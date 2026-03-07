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
    if (msg.type === "frame") {
      drawDirtyRegion(ctx, msg);
    } else if (msg.type === "cursor") {
      canvas.style.cursor = ppCursorToCss(msg.cursor);
    } else if (msg.type === "state") {
      if (msg.state === "error") {
        console.error("[Flash Player]", msg.message);
      }
    } else if (msg.type === "error") {
      console.error("[Flash Player]", msg.message);
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
// Drawing — paint dirty BGRA sub-regions onto the canvas
// ---------------------------------------------------------------------------

/**
 * Draw a dirty region received from the native host onto the canvas.
 *
 * The `msg` has: x, y, width, height, frameWidth, frameHeight, data (base64 BGRA).
 */
function drawDirtyRegion(ctx, msg) {
  const { x, y, width, height, frameWidth, frameHeight, data } = msg;

  // Resize canvas if frame dimensions changed.
  if (ctx.canvas.width !== frameWidth || ctx.canvas.height !== frameHeight) {
    ctx.canvas.width = frameWidth;
    ctx.canvas.height = frameHeight;
  }

  // Decode base64 → Uint8Array (BGRA premultiplied).
  const raw = atob(data);
  const len = raw.length;
  const rgba = new Uint8Array(len);

  // Convert BGRA → RGBA in-place.
  for (let i = 0; i < len; i += 4) {
    rgba[i]     = raw.charCodeAt(i + 2); // R ← B
    rgba[i + 1] = raw.charCodeAt(i + 1); // G
    rgba[i + 2] = raw.charCodeAt(i);     // B ← R
    rgba[i + 3] = raw.charCodeAt(i + 3); // A
  }

  const imageData = new ImageData(new Uint8ClampedArray(rgba.buffer), width, height);
  ctx.putImageData(imageData, x, y);
}

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
