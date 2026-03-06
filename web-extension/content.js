/**
 * Flash Player Content Script
 *
 * Detects <object> and <embed> elements that reference Flash content
 * and replaces them with a <canvas> driven by the native Flash Player
 * host via Native Messaging.
 *
 * Inspired by the ppMutationObserver pattern in cheerpflash/cheerpx/pp.js.
 */

"use strict";

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/**
 * Determine whether an element is a Flash <object> or <embed> and extract
 * its parameters. Returns null if the element is not Flash content.
 */
function getFlashParams(elem) {
  // Skip elements we have already processed.
  if (elem.getAttribute && elem.getAttribute("data-flash-player") != null) {
    return null;
  }

  const tag = elem.tagName;

  if (tag === "OBJECT") {
    // ActiveX classid check — only accept Flash classid.
    const classid = elem.getAttribute("classid");
    if (classid && classid.toLowerCase() !== "clsid:d27cdb6e-ae6d-11cf-96b8-444553540000") {
      return null;
    }
    // MIME type check.
    const type = elem.getAttribute("type");
    if (type && type !== "application/x-shockwave-flash") {
      return null;
    }

    const params = { src: elem.getAttribute("data") };
    for (let i = 0; i < elem.children.length; i++) {
      const c = elem.children[i];
      if (c.nodeName.toLowerCase() !== "param") continue;
      const name = c.getAttribute("name");
      const value = c.getAttribute("value");
      if (name != null && value != null) {
        params[name.toLowerCase()] = value;
      }
    }
    // Fallback: the "movie" param is sometimes used instead of "data".
    if (!params.src && params.movie) {
      params.src = params.movie;
    }
    return params;
  }

  if (tag === "EMBED") {
    const type = elem.getAttribute("type");
    if (type && type !== "application/x-shockwave-flash") {
      return null;
    }
    const params = {};
    const attrs = elem.attributes;
    for (let i = 0; i < attrs.length; i++) {
      params[attrs[i].name.toLowerCase()] = attrs[i].value;
    }
    return params;
  }

  return null;
}

/**
 * Resolve a potentially relative SWF URL against the document base.
 */
function resolveSwfUrl(src) {
  if (!src) return null;
  try {
    return new URL(src, document.baseURI).href;
  } catch {
    return src;
  }
}

// ---------------------------------------------------------------------------
// Canvas + port bridge
// ---------------------------------------------------------------------------

/** Unique instance counter for this content script. */
let nextInstanceId = 0;

/**
 * Replace a Flash element with a <canvas> and wire up the native messaging
 * bridge via the background service worker.
 *
 * Returns true if the element was successfully replaced.
 */
function replaceFlashElement(elem) {
  if (elem.parentNode == null) return false;

  const params = getFlashParams(elem);
  if (!params || !params.src) return false;

  const swfUrl = resolveSwfUrl(params.src);
  if (!swfUrl) return false;

  const instanceId = nextInstanceId++;

  // ---- Create the replacement <canvas> ----
  const canvas = document.createElement("canvas");
  canvas.setAttribute("data-flash-player", instanceId);
  canvas.style.border = "0";

  // Inherit dimensions from the original element.
  const origWidth = parseInt(elem.getAttribute("width") || elem.style.width, 10) || 550;
  const origHeight = parseInt(elem.getAttribute("height") || elem.style.height, 10) || 400;
  canvas.width = origWidth;
  canvas.height = origHeight;
  canvas.style.width = elem.style.width || origWidth + "px";
  canvas.style.height = elem.style.height || origHeight + "px";
  canvas.style.display = "inline-block";
  canvas.style.backgroundColor = "#000";
  canvas.tabIndex = 0; // Make focusable for keyboard events.

  const ctx = canvas.getContext("2d");

  // ---- Open a port to the background service worker ----
  const port = chrome.runtime.connect({ name: "flash-instance" });

  // Tell the background to start the native host and open the SWF.
  port.postMessage({
    type: "start",
    instanceId,
    url: swfUrl,
    width: origWidth,
    height: origHeight,
  });

  // ---- Handle messages from the native host (via background) ----
  port.onMessage.addListener((msg) => {
    // Extension-originated error (e.g. host disconnect).
    if (msg.error) {
      console.error("[Flash Player]", msg.error);
      return;
    }
    // Binary message from the host, base64-encoded.
    if (msg.b64) {
      handleBinaryMessage(ctx, canvas, msg.b64, port);
    }
  });

  port.onDisconnect.addListener(() => {
    console.warn("[Flash Player] Native host disconnected for instance", instanceId);
  });

  // ---- Wire up input events on the canvas ----
  bindInputEvents(canvas, port);

  // ---- Replace the original element in the DOM ----
  if (elem.tagName === "EMBED") {
    // For <embed>, insert the canvas and hide the original (some pages
    // reference the embed by id afterwards).
    elem.style.display = "none";
    elem.parentNode.insertBefore(canvas, elem);
  } else {
    // For <object>, remove children and append canvas inside.
    while (elem.firstChild) elem.removeChild(elem.firstChild);
    elem.style.display = "inline-block";
    elem.appendChild(canvas);
  }
  elem.setAttribute("data-flash-player", instanceId);

  return true;
}

// ---------------------------------------------------------------------------
// Binary message decoding
// ---------------------------------------------------------------------------

// Message type tags (must match protocol.rs).
const TAG_FRAME  = 0x01;
const TAG_STATE  = 0x02;
const TAG_CURSOR = 0x03;
const TAG_ERROR  = 0x04;
const TAG_SCRIPT = 0x10;

/**
 * Read a little-endian u32 from a DataView at the given offset.
 */
function readU32(dv, off) {
  return dv.getUint32(off, true);
}

/**
 * Read a little-endian i32 from a DataView at the given offset.
 */
function readI32(dv, off) {
  return dv.getInt32(off, true);
}

/**
 * Decode a base64 string into a Uint8Array.
 */
function b64ToUint8(b64) {
  const bin = atob(b64);
  const len = bin.length;
  const arr = new Uint8Array(len);
  for (let i = 0; i < len; i++) {
    arr[i] = bin.charCodeAt(i);
  }
  return arr;
}

/**
 * Handle a fully reassembled binary message (as a base64 string).
 */
function handleBinaryMessage(ctx, canvas, b64, port) {
  const bytes = b64ToUint8(b64);
  if (bytes.length === 0) return;

  const tag = bytes[0];
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);

  switch (tag) {
    case TAG_FRAME: {
      // 1 byte tag + 7 x u32 header (28 bytes) + BGRA pixels
      const x       = readU32(dv, 1);
      const y       = readU32(dv, 5);
      const width   = readU32(dv, 9);
      const height  = readU32(dv, 13);
      const frameW  = readU32(dv, 17);
      const frameH  = readU32(dv, 21);
      // stride at offset 25, not needed for drawing
      const pixelOffset = 29; // 1 + 7*4

      // Resize canvas if frame dimensions changed.
      if (ctx.canvas.width !== frameW || ctx.canvas.height !== frameH) {
        ctx.canvas.width = frameW;
        ctx.canvas.height = frameH;
      }

      // Convert BGRA -> RGBA in-place within the bytes buffer.
      const pixelLen = width * height * 4;
      const rgba = new Uint8Array(pixelLen);
      for (let i = 0; i < pixelLen; i += 4) {
        const off = pixelOffset + i;
        rgba[i]     = bytes[off + 2]; // R <- B
        rgba[i + 1] = bytes[off + 1]; // G
        rgba[i + 2] = bytes[off];     // B <- R
        rgba[i + 3] = bytes[off + 3]; // A
      }

      const imageData = new ImageData(new Uint8ClampedArray(rgba.buffer), width, height);
      ctx.putImageData(imageData, x, y);
      break;
    }

    case TAG_STATE: {
      // 1 byte tag + 1 byte code + u32 width + u32 height
      const code   = bytes[1];
      const width  = readU32(dv, 2);
      const height = readU32(dv, 6);
      const stateNames = ["idle", "loading", "running", "error"];
      const stateName = stateNames[code] || "unknown";
      if (code === 3) { // error
        console.error("[Flash Player] State: error");
      }
      break;
    }

    case TAG_CURSOR: {
      // 1 byte tag + i32 cursor type
      const cursor = readI32(dv, 1);
      canvas.style.cursor = ppCursorToCss(cursor);
      break;
    }

    case TAG_ERROR: {
      // 1 byte tag + u32 msg_len + UTF-8 bytes
      const msgLen = readU32(dv, 1);
      const msgBytes = bytes.subarray(5, 5 + msgLen);
      const decoder = new TextDecoder();
      const message = decoder.decode(msgBytes);
      console.error("[Flash Player]", message);
      break;
    }

    case TAG_SCRIPT: {
      // 1 byte tag + u32 json_len + UTF-8 JSON bytes
      const jsonLen = readU32(dv, 1);
      const jsonBytes = bytes.subarray(5, 5 + jsonLen);
      const decoder = new TextDecoder();
      const jsonStr = decoder.decode(jsonBytes);
      try {
        const req = JSON.parse(jsonStr);
        handleScriptRequest(req, port);
      } catch (e) {
        console.error("[Flash Player] Bad script request:", e, jsonStr);
      }
      break;
    }

    default:
      console.warn("[Flash Player] Unknown binary message tag:", tag);
  }
}

// ---------------------------------------------------------------------------
// JavaScript scripting bridge  (TAG_SCRIPT = 0x10)
// ---------------------------------------------------------------------------

/**
 * Object reference store.  Browser-side JS objects that have been returned
 * to the native host are stored here, keyed by a monotonically increasing
 * integer id.  The host sends the id back when it wants to access the
 * object again.
 */
const jsObjects = new Map();
let nextJsObjectId = 1;  // 0 is reserved for "window"

// Pre-register the window object.
jsObjects.set(0, window);

/**
 * Register a JS value and return its id.  Primitives are returned as-is;
 * only objects/functions get stored in the map.
 */
function registerJsObject(obj) {
  // Check if it's already registered (avoid duplicates for common objects).
  for (const [id, existing] of jsObjects) {
    if (existing === obj) return id;
  }
  const id = nextJsObjectId++;
  jsObjects.set(id, obj);
  return id;
}

/**
 * Encode a JS value into the JSON wire format expected by the native host.
 *   {type: "undefined|null|bool|int|double|string|object", v: ...}
 */
function encodeJsValue(val) {
  if (val === undefined) return { type: "undefined" };
  if (val === null) return { type: "null" };
  switch (typeof val) {
    case "boolean":
      return { type: "bool", v: val };
    case "number":
      if (Number.isInteger(val) && val >= -2147483648 && val <= 2147483647) {
        return { type: "int", v: val };
      }
      return { type: "double", v: val };
    case "string":
      return { type: "string", v: val };
    case "function":
    case "object":
      return { type: "object", v: registerJsObject(val) };
    default:
      return { type: "undefined" };
  }
}

/**
 * Decode a JSON-encoded value from the native host into a JS value.
 */
function decodeJsValue(encoded) {
  if (!encoded || !encoded.type) return undefined;
  switch (encoded.type) {
    case "undefined": return undefined;
    case "null": return null;
    case "bool": return !!encoded.v;
    case "int": return encoded.v | 0;
    case "double": return +encoded.v;
    case "string": return String(encoded.v ?? "");
    case "object": return jsObjects.get(encoded.v);
    default: return undefined;
  }
}

/**
 * Handle a scripting request from the native host.
 * @param {object} req  Parsed JSON request.
 * @param {MessagePort} port  Port to send the response back through.
 */
function handleScriptRequest(req, port) {
  const id = req.id;
  const op = req.op;

  try {
    switch (op) {
      case "getWindow": {
        // window is always object id 0.
        sendScriptResponse(port, id, encodeJsValue(window));
        break;
      }

      case "hasProperty": {
        const obj = jsObjects.get(req.obj);
        const result = obj != null && req.name in Object(obj);
        sendScriptResponse(port, id, { type: "bool", v: result });
        break;
      }

      case "hasMethod": {
        const obj = jsObjects.get(req.obj);
        const result = obj != null && typeof Object(obj)[req.name] === "function";
        sendScriptResponse(port, id, { type: "bool", v: result });
        break;
      }

      case "getProperty": {
        const obj = jsObjects.get(req.obj);
        if (obj == null) {
          sendScriptResponse(port, id, { type: "undefined" });
        } else {
          const val = Object(obj)[req.name];
          sendScriptResponse(port, id, encodeJsValue(val));
        }
        break;
      }

      case "setProperty": {
        const obj = jsObjects.get(req.obj);
        if (obj != null) {
          Object(obj)[req.name] = decodeJsValue(req.value);
        }
        sendScriptResponse(port, id, { type: "undefined" });
        break;
      }

      case "removeProperty": {
        const obj = jsObjects.get(req.obj);
        if (obj != null) {
          delete Object(obj)[req.name];
        }
        sendScriptResponse(port, id, { type: "undefined" });
        break;
      }

      case "getAllPropertyNames": {
        const obj = jsObjects.get(req.obj);
        const names = obj != null ? Object.keys(Object(obj)) : [];
        // Special response: names array instead of value.
        port.postMessage({ type: "jsResponse", id, names });
        break;
      }

      case "callMethod": {
        const obj = jsObjects.get(req.obj);
        if (obj == null) {
          sendScriptError(port, id, "object not found");
          break;
        }
        const fn_ = Object(obj)[req.method];
        if (typeof fn_ !== "function") {
          sendScriptError(port, id, `${req.method} is not a function`);
          break;
        }
        const args = (req.args || []).map(decodeJsValue);
        const result = fn_.apply(obj, args);
        sendScriptResponse(port, id, encodeJsValue(result));
        break;
      }

      case "call": {
        const fn_ = jsObjects.get(req.obj);
        if (typeof fn_ !== "function") {
          sendScriptError(port, id, "object is not callable");
          break;
        }
        const args = (req.args || []).map(decodeJsValue);
        const result = fn_(...args);
        sendScriptResponse(port, id, encodeJsValue(result));
        break;
      }

      case "construct": {
        const ctor = jsObjects.get(req.obj);
        if (typeof ctor !== "function") {
          sendScriptError(port, id, "object is not a constructor");
          break;
        }
        const args = (req.args || []).map(decodeJsValue);
        const result = new ctor(...args);
        sendScriptResponse(port, id, encodeJsValue(result));
        break;
      }

      case "executeScript": {
        // Use indirect eval so it runs in global scope.
        const result = (0, eval)(req.script);
        sendScriptResponse(port, id, encodeJsValue(result));
        break;
      }

      case "release": {
        const objId = req.obj;
        // Never release the window object (id 0).
        if (objId !== 0) {
          jsObjects.delete(objId);
        }
        // No response needed for release (id is 0 = fire-and-forget).
        break;
      }

      default:
        sendScriptError(port, id, `unknown script op: ${op}`);
    }
  } catch (e) {
    sendScriptError(port, id, String(e));
  }
}

function sendScriptResponse(port, id, value) {
  try {
    port.postMessage({ type: "jsResponse", id, value });
  } catch {
    // Port may have disconnected.
  }
}

function sendScriptError(port, id, error) {
  try {
    port.postMessage({ type: "jsResponse", id, error });
  } catch {
    // Port may have disconnected.
  }
}

// ---------------------------------------------------------------------------
// Drawing helpers (kept for reference; frame drawing is now in handleBinaryMessage)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Input event binding
// ---------------------------------------------------------------------------

/**
 * Build modifier flags matching PP_InputEvent_Modifier.
 */
function getModifiers(e) {
  let m = 0;
  if (e.shiftKey) m |= 1;       // PP_INPUTEVENT_MODIFIER_SHIFTKEY
  if (e.ctrlKey)  m |= 2;       // PP_INPUTEVENT_MODIFIER_CONTROLKEY
  if (e.altKey)   m |= 4;       // PP_INPUTEVENT_MODIFIER_ALTKEY
  if (e.metaKey)  m |= 8;       // PP_INPUTEVENT_MODIFIER_METAKEY
  if (e.buttons & 1) m |= 16;   // PP_INPUTEVENT_MODIFIER_LEFTBUTTONDOWN
  if (e.buttons & 4) m |= 32;   // PP_INPUTEVENT_MODIFIER_MIDDLEBUTTONDOWN
  if (e.buttons & 2) m |= 64;   // PP_INPUTEVENT_MODIFIER_RIGHTBUTTONDOWN
  return m;
}

/**
 * Map a DOM MouseEvent.button to our protocol button index.
 */
function mapButton(e) {
  // DOM: 0=left, 1=middle, 2=right — matches our protocol.
  return e.button;
}

/**
 * Compute mouse position relative to the canvas, accounting for CSS scaling.
 */
function canvasPos(canvas, e) {
  const rect = canvas.getBoundingClientRect();
  const scaleX = canvas.width / rect.width;
  const scaleY = canvas.height / rect.height;
  return {
    x: Math.round((e.clientX - rect.left) * scaleX),
    y: Math.round((e.clientY - rect.top) * scaleY),
  };
}

function bindInputEvents(canvas, port) {
  canvas.addEventListener("mousedown", (e) => {
    e.preventDefault();
    canvas.focus();
    const pos = canvasPos(canvas, e);
    port.postMessage({
      type: "mousedown",
      x: pos.x,
      y: pos.y,
      button: mapButton(e),
      modifiers: getModifiers(e),
    });
  });

  canvas.addEventListener("mouseup", (e) => {
    e.preventDefault();
    const pos = canvasPos(canvas, e);
    port.postMessage({
      type: "mouseup",
      x: pos.x,
      y: pos.y,
      button: mapButton(e),
      modifiers: getModifiers(e),
    });
  });

  canvas.addEventListener("mousemove", (e) => {
    const pos = canvasPos(canvas, e);
    port.postMessage({
      type: "mousemove",
      x: pos.x,
      y: pos.y,
      modifiers: getModifiers(e),
    });
  });

  canvas.addEventListener("mouseenter", () => {
    port.postMessage({ type: "mouseenter" });
  });

  canvas.addEventListener("mouseleave", () => {
    port.postMessage({ type: "mouseleave" });
  });

  canvas.addEventListener("wheel", (e) => {
    e.preventDefault();
    port.postMessage({
      type: "wheel",
      deltaX: -e.deltaX,
      deltaY: -e.deltaY,
      modifiers: getModifiers(e),
    });
  }, { passive: false });

  canvas.addEventListener("keydown", (e) => {
    e.preventDefault();
    port.postMessage({
      type: "keydown",
      keyCode: e.keyCode,
      code: e.code,
      modifiers: getModifiers(e),
    });
  });

  canvas.addEventListener("keyup", (e) => {
    e.preventDefault();
    port.postMessage({
      type: "keyup",
      keyCode: e.keyCode,
      code: e.code,
      modifiers: getModifiers(e),
    });
  });

  canvas.addEventListener("keypress", (e) => {
    e.preventDefault();
    port.postMessage({
      type: "char",
      keyCode: e.charCode || e.keyCode,
      text: e.key.length === 1 ? e.key : "",
      code: e.code,
      modifiers: getModifiers(e),
    });
  });

  canvas.addEventListener("focus", () => {
    port.postMessage({ type: "focus", hasFocus: true });
  });

  canvas.addEventListener("blur", () => {
    port.postMessage({ type: "focus", hasFocus: false });
  });

  // Prevent context menu on right-click.
  canvas.addEventListener("contextmenu", (e) => {
    e.preventDefault();
  });
}

// ---------------------------------------------------------------------------
// Cursor mapping  (PP_CursorType_Dev → CSS cursor)
// ---------------------------------------------------------------------------

const PP_CURSOR_MAP = [
  "default",      // 0 = POINTER
  "crosshair",    // 1 = CROSS
  "pointer",      // 2 = HAND
  "text",         // 3 = IBEAM
  "wait",         // 4 = WAIT
  "help",         // 5 = HELP
  "e-resize",     // 6 = EASTRESIZE
  "n-resize",     // 7 = NORTHRESIZE
  "ne-resize",    // 8 = NORTHEASTRESIZE
  "nw-resize",    // 9 = NORTHWESTRESIZE
  "s-resize",     // 10 = SOUTHRESIZE
  "se-resize",    // 11 = SOUTHEASTRESIZE
  "sw-resize",    // 12 = SOUTHWESTRESIZE
  "w-resize",     // 13 = WESTRESIZE
  "ns-resize",    // 14 = NORTHSOUTHRESIZE
  "ew-resize",    // 15 = EASTWESTRESIZE
  "nesw-resize",  // 16 = NORTHEASTSOUTHWESTRESIZE
  "nwse-resize",  // 17 = NORTHWESTSOUTHEASTRESIZE
  "col-resize",   // 18 = COLUMNRESIZE
  "row-resize",   // 19 = ROWRESIZE
  "move",         // 20 = MIDDLEPANNING
  "move",         // 21 = EASTPANNING
  "move",         // 22 = NORTHPANNING
  "move",         // 23 = NORTHEASTPANNING
  "move",         // 24 = NORTHWESTPANNING
  "move",         // 25 = SOUTHPANNING
  "move",         // 26 = SOUTHEASTPANNING
  "move",         // 27 = SOUTHWESTPANNING
  "move",         // 28 = WESTPANNING
  "move",         // 29 = MOVE
  "vertical-text",// 30 = VERTICALTEXT
  "cell",         // 31 = CELL
  "context-menu", // 32 = CONTEXTMENU
  "alias",        // 33 = ALIAS
  "progress",     // 34 = PROGRESS
  "no-drop",      // 35 = NODROP
  "copy",         // 36 = COPY
  "none",         // 37 = NONE
  "not-allowed",  // 38 = NOTALLOWED
  "zoom-in",      // 39 = ZOOMIN
  "zoom-out",     // 40 = ZOOMOUT
  "grab",         // 41 = GRAB
  "grabbing",     // 42 = GRABBING
];

function ppCursorToCss(cursorType) {
  return PP_CURSOR_MAP[cursorType] || "default";
}

// ---------------------------------------------------------------------------
// Mutation Observer — scan and watch for Flash elements
// ---------------------------------------------------------------------------

/**
 * MutationObserver callback (inspired by ppMutationObserver in pp.js).
 * Walks added nodes looking for <object> or <embed> Flash elements.
 */
function flashMutationObserver(mutations) {
  for (let i = 0; i < mutations.length; i++) {
    const addedNodes = Array.from(mutations[i].addedNodes);
    const stack = addedNodes.slice();

    while (stack.length) {
      const node = stack.pop();
      if (node.nodeType !== Node.ELEMENT_NODE) continue;

      const tag = node.nodeName.toUpperCase();
      if (tag === "OBJECT" || tag === "EMBED") {
        if (replaceFlashElement(node)) continue;
      }

      // Recurse into children.
      if (node.children && node.children.length) {
        for (let j = 0; j < node.children.length; j++) {
          stack.push(node.children[j]);
        }
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

function init() {
  // Scan existing elements.
  const tags = ["object", "embed"];
  for (const tagName of tags) {
    const elems = document.getElementsByTagName(tagName);
    // Snapshot the live collection before mutating the DOM.
    const snapshot = Array.from(elems);
    for (const elem of snapshot) {
      replaceFlashElement(elem);
    }
  }

  // Observe future DOM mutations.
  const observer = new MutationObserver(flashMutationObserver);
  observer.observe(document.documentElement, {
    subtree: true,
    childList: true,
  });
}

// Run init when the DOM is ready.
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", init);
} else {
  init();
}
