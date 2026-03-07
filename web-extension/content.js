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
// Debug mode — set to true to show a live statistics panel below each canvas.
// ---------------------------------------------------------------------------
const FLASH_DEBUG = true;

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

/** Active native messaging port — used by the ExternalInterface bridge. */
let activePort = null;

// ---------------------------------------------------------------------------
// QOI WASM decoder — loaded once, reused for every frame.
// ---------------------------------------------------------------------------

/** @type {Function|null} decode(data_len) → output_ptr */
let _qoiDecode = null;
/** @type {WebAssembly.Memory|null} */
let _qoiMemory = null;

/** Resolves when the QOI WASM module is ready (or failed). */
const _qoiReady = (async () => {
  try {
    const url = chrome.runtime.getURL("qoiopt.wasm");
    const wasmBytes = await fetch(url).then((r) => r.arrayBuffer());
    const { instance } = await WebAssembly.instantiate(wasmBytes);
    _qoiDecode = instance.exports.decode;
    _qoiMemory = instance.exports.memory;
    console.log("[Flash Player] QOI WASM decoder ready");
  } catch (e) {
    console.error("[Flash Player] Failed to load QOI WASM decoder:", e);
  }
})();

/**
 * Per-instance metadata used for crash recovery.
 * instanceId -> { canvas, ctx, swfUrl, origWidth, origHeight, port, container, elem }
 */
const instanceMeta = new Map();

/** True while we are intentionally tearing down (page navigation). */
let navigatingAway = false;

// ---------------------------------------------------------------------------
// Debug statistics
// ---------------------------------------------------------------------------

/** Human-readable tag names for debug display. */
const TAG_NAMES = {
  [0x01]: "FRAME",
  [0x02]: "STATE",
  [0x03]: "CURSOR",
  [0x04]: "ERROR",
  [0x05]: "NAVIGATE",
  [0x10]: "SCRIPT",
  [0x20]: "AUDIO_INIT",
  [0x21]: "AUDIO_SAMPLES",
  [0x22]: "AUDIO_START",
  [0x23]: "AUDIO_STOP",
  [0x24]: "AUDIO_CLOSE",
  [0x30]: "AINPUT_OPEN",
  [0x31]: "AINPUT_START",
  [0x32]: "AINPUT_STOP",
  [0x33]: "AINPUT_CLOSE",
};

/**
 * Per-instance debug statistics collector.
 *
 * Tracks rolling 1-second windows of message counts, byte volumes,
 * per-type breakdowns, largest messages, decode/render timing, and
 * congestion (queue depth) information.
 */
class DebugStats {
  constructor() {
    // ---- Current window accumulators (reset every flush) ----
    this.msgCount = 0;          // messages received this window
    this.byteCount = 0;         // total decoded bytes this window
    this.b64ByteCount = 0;      // total base64 bytes this window
    /** Per-tag counters: tag -> { count, bytes } */
    this.tagCounters = new Map();
    /** Top-N largest messages this window: [{ tag, bytes }] */
    this.bigMessages = [];      // kept sorted, max 5
    this.decodeTimeSum = 0;     // ms spent in b64ToUint8
    this.renderTimeSum = 0;     // ms spent in putImageData (TAG_FRAME only)
    this.frameCount = 0;        // TAG_FRAME count this window

    // ---- Congestion tracking ----
    this.lastMsgTime = 0;       // performance.now() of last message
    this.interArrivalSum = 0;   // sum of inter-arrival gaps (ms)
    this.interArrivalCount = 0;
    this.minInterArrival = Infinity;
    this.pendingMessages = 0;   // incremented on receive, decremented after processing
    this.maxPending = 0;

    // ---- Snapshot (last flushed values for display) ----
    this.snap = {
      msgPerSec: 0, bytesPerSec: 0, b64BytesPerSec: 0,
      tags: [],           // [{ name, count, bytes }] sorted by bytes desc
      biggest: [],        // [{ name, bytes }]
      avgDecodeMs: 0, avgRenderMs: 0, fps: 0,
      avgInterArrivalMs: 0, minInterArrivalMs: 0,
      maxPending: 0,
      congested: false,
    };

    this._flushInterval = null;
    this._lastFlush = performance.now();
  }

  /** Record an incoming message before processing. */
  recordMessage(b64Len, decodedLen, tag, decodeMs) {
    this.msgCount++;
    this.byteCount += decodedLen;
    this.b64ByteCount += b64Len;
    this.decodeTimeSum += decodeMs;

    const name = TAG_NAMES[tag] || `0x${tag.toString(16).padStart(2, "0")}`;
    let tc = this.tagCounters.get(tag);
    if (!tc) { tc = { name, count: 0, bytes: 0 }; this.tagCounters.set(tag, tc); }
    tc.count++;
    tc.bytes += decodedLen;

    // Track top-5 biggest messages.
    if (this.bigMessages.length < 5 || decodedLen > this.bigMessages[this.bigMessages.length - 1].bytes) {
      this.bigMessages.push({ name, bytes: decodedLen });
      this.bigMessages.sort((a, b) => b.bytes - a.bytes);
      if (this.bigMessages.length > 5) this.bigMessages.length = 5;
    }

    // Inter-arrival timing.
    const now = performance.now();
    if (this.lastMsgTime > 0) {
      const gap = now - this.lastMsgTime;
      this.interArrivalSum += gap;
      this.interArrivalCount++;
      if (gap < this.minInterArrival) this.minInterArrival = gap;
    }
    this.lastMsgTime = now;
  }

  /** Record time spent in putImageData for a frame. */
  recordFrameRender(renderMs) {
    this.frameCount++;
    this.renderTimeSum += renderMs;
  }

  /** Call when a message starts processing (for congestion). */
  markProcessingStart() {
    this.pendingMessages++;
    if (this.pendingMessages > this.maxPending) this.maxPending = this.pendingMessages;
  }

  /** Call when a message finishes processing. */
  markProcessingEnd() {
    this.pendingMessages--;
  }

  /** Flush accumulators into a snapshot for display, then reset. */
  flush() {
    const now = performance.now();
    const elapsed = (now - this._lastFlush) / 1000; // seconds
    const s = this.snap;

    s.msgPerSec = elapsed > 0 ? Math.round(this.msgCount / elapsed) : 0;
    s.bytesPerSec = elapsed > 0 ? Math.round(this.byteCount / elapsed) : 0;
    s.b64BytesPerSec = elapsed > 0 ? Math.round(this.b64ByteCount / elapsed) : 0;
    s.fps = elapsed > 0 ? Math.round(this.frameCount / elapsed) : 0;
    s.avgDecodeMs = this.msgCount > 0 ? (this.decodeTimeSum / this.msgCount) : 0;
    s.avgRenderMs = this.frameCount > 0 ? (this.renderTimeSum / this.frameCount) : 0;
    s.avgInterArrivalMs = this.interArrivalCount > 0 ? (this.interArrivalSum / this.interArrivalCount) : 0;
    s.minInterArrivalMs = this.minInterArrival === Infinity ? 0 : this.minInterArrival;
    s.maxPending = this.maxPending;
    s.congested = this.maxPending > 3;

    // Per-tag breakdown sorted by bytes.
    s.tags = [];
    for (const [, tc] of this.tagCounters) {
      s.tags.push({ name: tc.name, count: tc.count, bytes: tc.bytes });
    }
    s.tags.sort((a, b) => b.bytes - a.bytes);

    // Biggest messages.
    s.biggest = this.bigMessages.slice();

    // Reset accumulators.
    this.msgCount = 0;
    this.byteCount = 0;
    this.b64ByteCount = 0;
    this.tagCounters.clear();
    this.bigMessages = [];
    this.decodeTimeSum = 0;
    this.renderTimeSum = 0;
    this.frameCount = 0;
    this.interArrivalSum = 0;
    this.interArrivalCount = 0;
    this.minInterArrival = Infinity;
    this.maxPending = 0;
    this._lastFlush = now;
  }

  /** Start periodic flushing and DOM updates. */
  start(panelEl) {
    this._panel = panelEl;
    this._flushInterval = setInterval(() => {
      this.flush();
      this._render();
    }, 500);
  }

  /** Stop the update loop. */
  stop() {
    if (this._flushInterval) { clearInterval(this._flushInterval); this._flushInterval = null; }
  }

  /** Format bytes into a human-readable string. */
  static fmtBytes(n) {
    if (n < 1024) return n + " B";
    if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
    return (n / (1024 * 1024)).toFixed(2) + " MB";
  }

  /** Render snapshot into the panel element. */
  _render() {
    const s = this.snap;
    const f = DebugStats.fmtBytes;
    let h = "";

    // Throughput row.
    h += `<b>Throughput:</b> ${s.msgPerSec} msg/s &nbsp;|&nbsp; `;
    h += `${f(s.bytesPerSec)}/s decoded &nbsp;|&nbsp; `;
    h += `${f(s.b64BytesPerSec)}/s base64<br>`;

    // FPS + timing.
    h += `<b>Frames:</b> ${s.fps} fps &nbsp;|&nbsp; `;
    h += `decode: ${s.avgDecodeMs.toFixed(2)} ms &nbsp;|&nbsp; `;
    h += `render: ${s.avgRenderMs.toFixed(2)} ms<br>`;

    // Inter-arrival / congestion.
    h += `<b>Arrival:</b> avg ${s.avgInterArrivalMs.toFixed(1)} ms &nbsp;|&nbsp; `;
    h += `min ${s.minInterArrivalMs.toFixed(1)} ms &nbsp;|&nbsp; `;
    h += `max queued: ${s.maxPending}`;
    if (s.congested) h += ` <span style="color:#ff6b6b;font-weight:700">⚠ CONGESTED</span>`;
    h += "<br>";

    // Per-type breakdown.
    if (s.tags.length) {
      h += `<b>By type:</b><br>`;
      for (const t of s.tags) {
        h += `&nbsp;&nbsp;${t.name}: ${t.count}× &nbsp; ${f(t.bytes)}<br>`;
      }
    }

    // Biggest messages.
    if (s.biggest.length) {
      h += `<b>Biggest msgs:</b> `;
      h += s.biggest.map(b => `${b.name} ${f(b.bytes)}`).join(", ");
      h += "<br>";
    }

    this._panel.innerHTML = h;
  }
}

/** Map of instanceId -> DebugStats (only populated when FLASH_DEBUG). */
const debugStatsMap = new Map();

/**
 * Create the debug stats DOM panel and attach it after the container.
 * Returns the DebugStats instance.
 */
function createDebugStatsPanel(instanceId, container, anchorEl) {
  const panel = document.createElement("div");
  panel.className = "flash-debug-stats";
  Object.assign(panel.style, {
    fontFamily: "'Consolas', 'Menlo', 'Monaco', monospace",
    fontSize: "11px",
    lineHeight: "1.5",
    color: "#c8d6e5",
    background: "linear-gradient(135deg, #0b1a2e 0%, #142744 100%)",
    border: "1px solid #1e3a5f",
    borderTop: "none",
    padding: "8px 12px",
    maxWidth: container.style.width,
    boxSizing: "border-box",
    overflowX: "auto",
    userSelect: "text",
  });
  panel.innerHTML = "<i>Collecting statistics…</i>";

  // Insert right after anchorEl (defaults to container).
  // For <object> elements pass the <object> itself so the panel lands
  // outside it — otherwise it ends up inside the collapsed object and
  // has zero height.
  const insertAfter = anchorEl || container;
  if (insertAfter.nextSibling) {
    insertAfter.parentNode.insertBefore(panel, insertAfter.nextSibling);
  } else if (insertAfter.parentNode) {
    insertAfter.parentNode.appendChild(panel);
  }

  const stats = new DebugStats();
  stats.start(panel);
  debugStatsMap.set(instanceId, stats);
  return stats;
}

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

  // ---- Wrap canvas in a container for crash overlay support ----
  const container = document.createElement("div");
  container.style.position = "relative";
  container.style.display = "inline-block";
  container.style.width = canvas.style.width;
  container.style.height = canvas.style.height;
  container.appendChild(canvas);

  // ---- Store instance metadata for crash recovery ----
  const meta = { canvas, ctx, swfUrl, origWidth, origHeight, port: null, container, elem };
  instanceMeta.set(instanceId, meta);

  // ---- Debug stats panel (below the canvas) ----
  if (FLASH_DEBUG) {
    // Defer panel creation until the container is in the DOM.
    meta._pendingDebugPanel = true;
  }

  // ---- Connect and start ----
  startInstance(instanceId, meta);

  // ---- Replace the original element in the DOM ----
  if (elem.tagName === "EMBED") {
    // For <embed>, insert the container and hide the original (some pages
    // reference the embed by id afterwards).
    elem.style.display = "none";
    elem.parentNode.insertBefore(container, elem);
  } else {
    // For <object>, remove children and append container inside.
    while (elem.firstChild) elem.removeChild(elem.firstChild);
    elem.style.display = "inline-block";
    elem.appendChild(container);
  }
  elem.setAttribute("data-flash-player", instanceId);

  // Now that the container is in the DOM, create the debug panel if needed.
  // For <object>, the container lives *inside* the element, so anchor the
  // panel to the <object> itself so it appears outside/after it.
  if (FLASH_DEBUG && meta._pendingDebugPanel) {
    const anchor = elem.tagName === "OBJECT" ? elem : container;
    createDebugStatsPanel(instanceId, container, anchor);
    meta._pendingDebugPanel = false;
  }

  return true;
}

// ---------------------------------------------------------------------------
// Instance lifecycle (start / restart)
// ---------------------------------------------------------------------------

/**
 * Start (or restart) a Flash instance by opening a new port to the
 * background service worker and wiring up message handlers.
 */
function startInstance(instanceId, meta) {
  const { canvas, ctx, swfUrl, origWidth, origHeight } = meta;

  const port = chrome.runtime.connect({ name: "flash-instance" });
  meta.port = port;
  activePort = port;

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
      if (!navigatingAway) showCrashOverlay(instanceId);
      return;
    }
    // Binary message from the host, base64-encoded.
    // Always read from meta so we use the current canvas/ctx after restarts.
    if (msg.b64) {
      handleBinaryMessage(meta.ctx, meta.canvas, msg.b64, port);
    }
  });

  port.onDisconnect.addListener(() => {
    console.warn("[Flash Player] Native host disconnected for instance", instanceId);
    if (!navigatingAway) showCrashOverlay(instanceId);
  });

  // ---- Wire up input events on the canvas ----
  // On restart, clone the canvas to strip old event listeners.
  if (meta._started) {
    const freshCanvas = canvas.cloneNode(false);
    freshCanvas.getContext("2d"); // ensure context
    meta.canvas = freshCanvas;
    meta.ctx = freshCanvas.getContext("2d");
    if (canvas.parentNode) {
      canvas.parentNode.replaceChild(freshCanvas, canvas);
    }
    bindInputEvents(freshCanvas, port);
  } else {
    meta._started = true;
    bindInputEvents(canvas, port);
  }
}

// ---------------------------------------------------------------------------
// Crash overlay
// ---------------------------------------------------------------------------

/**
 * Show a styled crash overlay on top of the canvas for the given instance.
 */
function showCrashOverlay(instanceId) {
  const meta = instanceMeta.get(instanceId);
  if (!meta) return;

  const { container } = meta;

  // Don't add a second overlay.
  if (container.querySelector(".flash-crash-overlay")) return;

  const overlay = document.createElement("div");
  overlay.className = "flash-crash-overlay";
  Object.assign(overlay.style, {
    position: "absolute",
    inset: "0",
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    justifyContent: "center",
    background: "linear-gradient(135deg, #0b1a3e 0%, #162d6b 40%, #1e3f8f 70%, #2557b8 100%)",
    color: "#fff",
    fontFamily: "'Segoe UI', system-ui, -apple-system, sans-serif",
    textAlign: "center",
    zIndex: "999999",
    borderRadius: "0",
    overflow: "hidden",
    userSelect: "none",
  });

  // Subtle grid-line pattern for depth.
  const patternOverlay = document.createElement("div");
  Object.assign(patternOverlay.style, {
    position: "absolute",
    inset: "0",
    backgroundImage:
      "linear-gradient(rgba(255,255,255,.03) 1px, transparent 1px), " +
      "linear-gradient(90deg, rgba(255,255,255,.03) 1px, transparent 1px)",
    backgroundSize: "40px 40px",
    pointerEvents: "none",
  });
  overlay.appendChild(patternOverlay);

  // Icon
  const icon = document.createElement("div");
  icon.textContent = "\u26A0"; // ⚠
  Object.assign(icon.style, {
    fontSize: "48px",
    marginBottom: "12px",
    filter: "drop-shadow(0 2px 8px rgba(0,0,0,0.4))",
    animation: "flash-crash-pulse 2s ease-in-out infinite",
    position: "relative",
    zIndex: "1",
  });
  overlay.appendChild(icon);

  // Title
  const title = document.createElement("div");
  title.textContent = "Oops! Flash Player has crashed!";
  Object.assign(title.style, {
    fontSize: "22px",
    fontWeight: "700",
    letterSpacing: "0.3px",
    marginBottom: "8px",
    textShadow: "0 2px 12px rgba(0,0,0,0.5)",
    position: "relative",
    zIndex: "1",
  });
  overlay.appendChild(title);

  // Subtitle
  const subtitle = document.createElement("div");
  subtitle.textContent = "The native host process ended unexpectedly.";
  Object.assign(subtitle.style, {
    fontSize: "13px",
    opacity: "0.7",
    marginBottom: "24px",
    position: "relative",
    zIndex: "1",
  });
  overlay.appendChild(subtitle);

  // Restart button
  const btn = document.createElement("button");
  btn.textContent = "\u21BB  Restart";
  Object.assign(btn.style, {
    padding: "10px 32px",
    fontSize: "14px",
    fontWeight: "600",
    color: "#0b1a3e",
    background: "linear-gradient(135deg, #7ec8f2, #b4dff7)",
    border: "none",
    borderRadius: "8px",
    cursor: "pointer",
    boxShadow: "0 4px 20px rgba(30, 63, 143, 0.45), inset 0 1px 0 rgba(255,255,255,0.4)",
    transition: "0.5s ease",
    position: "relative",
    zIndex: "1",
    letterSpacing: "0.5px",
  });
  btn.addEventListener("mouseenter", () => {
    // Slightly brighten the button on hover, do not grow.
    btn.style.background = "linear-gradient(135deg, #8ed0f4, #c0e5fb)";
  });
  btn.addEventListener("mouseleave", () => {
    btn.style.background = "linear-gradient(135deg, #7ec8f2, #b4dff7)";
  });
  btn.addEventListener("click", () => {
    restartInstance(instanceId);
  });
  overlay.appendChild(btn);

  // Inject keyframe animation (once)
  if (!document.getElementById("flash-crash-styles")) {
    const style = document.createElement("style");
    style.id = "flash-crash-styles";
    style.textContent = `
      @keyframes flash-crash-pulse {
        0%, 100% { opacity: 1; transform: scale(1); }
        50% { opacity: 0.6; transform: scale(1.08); }
      }
    `;
    document.head.appendChild(style);
  }

  container.appendChild(overlay);
}

/**
 * Remove the crash overlay and restart the native host for an instance.
 */
function restartInstance(instanceId) {
  const meta = instanceMeta.get(instanceId);
  if (!meta) return;

  const { container } = meta;

  // Remove the crash overlay.
  const overlay = container.querySelector(".flash-crash-overlay");
  if (overlay) overlay.remove();

  // Clear the canvas.
  meta.ctx.clearRect(0, 0, meta.canvas.width, meta.canvas.height);

  // Re-connect.
  startInstance(instanceId, meta);
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
const TAG_NAVIGATE = 0x05;
const TAG_AUDIO_INIT    = 0x20;
const TAG_AUDIO_SAMPLES = 0x21;
const TAG_AUDIO_START   = 0x22;
const TAG_AUDIO_STOP    = 0x23;
const TAG_AUDIO_CLOSE   = 0x24;
const TAG_AUDIO_INPUT_OPEN  = 0x30;
const TAG_AUDIO_INPUT_START = 0x31;
const TAG_AUDIO_INPUT_STOP  = 0x32;
const TAG_AUDIO_INPUT_CLOSE = 0x33;

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

// Pre-computed base64 decode lookup table (avoids atob + charCodeAt overhead).
const _B64 = new Uint8Array(128);
for (let _i = 0; _i < 64; _i++) _B64["ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".charCodeAt(_i)] = _i;

/** True if the fast native Uint8Array.fromBase64 API is available (Chrome 128+). */
const _hasFromBase64 = typeof Uint8Array.fromBase64 === "function";

/** True if the fast native Uint8Array.prototype.toBase64 API is available. */
const _hasToBase64 = typeof Uint8Array.prototype.toBase64 === "function";

/**
 * Decode a base64 string into a Uint8Array.
 *
 * Fast path: native Uint8Array.fromBase64 (Chrome 128+, all-native, ~5× faster).
 * Fallback : pure-JS lookup-table decoder — skips the intermediate binary string
 *            that atob() creates and processes 4 input chars → 3 output bytes per
 *            iteration instead of 1 byte per iteration.
 */
function b64ToUint8(b64) {
  if (_hasFromBase64) return Uint8Array.fromBase64(b64);

  const len = b64.length;
  let outLen = (len * 3) >>> 2;
  if (len > 0 && b64.charCodeAt(len - 1) === 0x3D) outLen--;
  if (len > 1 && b64.charCodeAt(len - 2) === 0x3D) outLen--;

  const out = new Uint8Array(outLen);
  for (let i = 0, j = 0; i < len; i += 4) {
    const a = _B64[b64.charCodeAt(i)];
    const b = _B64[b64.charCodeAt(i + 1)];
    const c = _B64[b64.charCodeAt(i + 2)];
    const d = _B64[b64.charCodeAt(i + 3)];
    out[j++] = (a << 2) | (b >> 4);
    if (j < outLen) out[j++] = ((b & 0xF) << 4) | (c >> 2);
    if (j < outLen) out[j++] = ((c & 0x3) << 6) | d;
  }
  return out;
}

/**
 * Encode a Uint8Array to a base64 string.
 * Uses the native toBase64() when available, otherwise batched btoa.
 */
function uint8ToB64(bytes) {
  if (_hasToBase64) return bytes.toBase64();
  const parts = [];
  for (let off = 0; off < bytes.length; off += 8192) {
    parts.push(String.fromCharCode.apply(null, bytes.subarray(off, off + 8192)));
  }
  return btoa(parts.join(""));
}

/** Shared TextDecoder — avoids allocating a new one on every message. */
const _textDecoder = new TextDecoder();

// ---------------------------------------------------------------------------
// Web Audio playback — receives PCM from native host, plays via AudioContext
//
// Uses an adaptive jitter buffer: tracks the variance of inter-arrival
// times and keeps enough scheduling headroom to absorb spikes without
// audible gaps, while keeping latency as low as possible.
// ---------------------------------------------------------------------------

/**
 * Active audio streams.
 * stream_id -> {
 *   ctx,              // AudioContext
 *   nextTime,         // next scheduled start (ctx.currentTime units)
 *   sampleRate,
 *   frameCount,
 *   bufferDuration,   // seconds per buffer (frameCount / sampleRate)
 *   lastArrival,      // performance.now() of last write_samples
 *   jitterEma,        // exponential moving average of |inter-arrival − expected|
 *   targetAhead,      // current adaptive headroom (seconds)
 * }
 */
const audioStreams = new Map();

/** Minimum scheduling headroom (seconds). */
const MIN_AHEAD = 0.04;
/** Maximum scheduling headroom (seconds) — caps latency. */
const MAX_AHEAD = 0.25;
/** EMA smoothing factor for jitter measurement (0 < α < 1). */
const JITTER_ALPHA = 0.05;
/**
 * How many multiples of the jitter EMA to add on top of MIN_AHEAD.
 * Higher = more resilient to spikes, but adds latency.
 */
const JITTER_MULTIPLIER = 3.0;

/**
 * Create a new Web Audio stream.
 */
function audioInit(streamId, sampleRate, frameCount) {
  // Close any existing stream with the same id.
  audioClose(streamId);

  const ctx = new AudioContext({ sampleRate });
  const bufferDuration = frameCount / sampleRate;

  audioStreams.set(streamId, {
    ctx,
    nextTime: 0,
    sampleRate,
    frameCount,
    bufferDuration,
    lastArrival: 0,
    jitterEma: 0,
    targetAhead: MIN_AHEAD,
  });
  console.log("[Flash Player] Audio stream created:", streamId,
              "rate:", sampleRate, "frames:", frameCount,
              "bufDur:", bufferDuration.toFixed(4) + "s");
}

/**
 * Schedule a buffer of PCM samples for playback on a stream.
 * `pcmBytes` is a Uint8Array of interleaved stereo i16 LE samples.
 */
function audioWriteSamples(streamId, pcmBytes) {
  const stream = audioStreams.get(streamId);
  if (!stream) return;

  const { ctx, sampleRate, frameCount, bufferDuration } = stream;

  // Resume the context if it was suspended (autoplay policy).
  if (ctx.state === "suspended") {
    ctx.resume();
  }

  // --- Adaptive jitter measurement ---
  const arrivalNow = performance.now();
  if (stream.lastArrival > 0) {
    const interArrival = (arrivalNow - stream.lastArrival) / 1000; // seconds
    const deviation = Math.abs(interArrival - bufferDuration);
    // Exponential moving average of the absolute deviation.
    stream.jitterEma = stream.jitterEma * (1 - JITTER_ALPHA)
                     + deviation * JITTER_ALPHA;
    // Adaptive headroom: base + multiple of jitter.
    stream.targetAhead = Math.min(
      MAX_AHEAD,
      Math.max(MIN_AHEAD, MIN_AHEAD + stream.jitterEma * JITTER_MULTIPLIER),
    );
  }
  stream.lastArrival = arrivalNow;

  // --- Decode interleaved stereo i16 LE → float32 per-channel ---
  const dv = new DataView(pcmBytes.buffer, pcmBytes.byteOffset, pcmBytes.byteLength);
  const actualFrames = Math.min(frameCount, (pcmBytes.byteLength / 4) | 0);
  const buffer = ctx.createBuffer(2, actualFrames, sampleRate);
  const left = buffer.getChannelData(0);
  const right = buffer.getChannelData(1);

  for (let i = 0; i < actualFrames; i++) {
    left[i]  = dv.getInt16(i * 4,     true) / 32768.0;
    right[i] = dv.getInt16(i * 4 + 2, true) / 32768.0;
  }

  const source = ctx.createBufferSource();
  source.buffer = buffer;
  source.connect(ctx.destination);

  const now = ctx.currentTime;
  const ahead = stream.targetAhead;

  if (stream.nextTime <= now) {
    // First buffer after init, resume, or an underrun — schedule from
    // `now + targetAhead` to build up a safe cushion before playback
    // reaches the scheduled point.
    stream.nextTime = now + ahead;
  } else if (stream.nextTime > now + ahead + bufferDuration * 4) {
    // We're scheduled way too far in the future (clock jump?).
    // Re-anchor to avoid growing latency.
    stream.nextTime = now + ahead;
  }

  source.start(stream.nextTime);
  stream.nextTime += actualFrames / sampleRate;
}

/**
 * Start (resume) playback on a stream.
 */
function audioStart(streamId) {
  const stream = audioStreams.get(streamId);
  if (stream && stream.ctx.state === "suspended") {
    stream.ctx.resume();
  }
}

/**
 * Stop (suspend) playback on a stream.
 */
function audioStop(streamId) {
  const stream = audioStreams.get(streamId);
  if (stream) {
    stream.ctx.suspend();
    stream.nextTime = 0;
    stream.lastArrival = 0;
    stream.jitterEma = 0;
    stream.targetAhead = MIN_AHEAD;
  }
}

/**
 * Close and release a stream.
 */
function audioClose(streamId) {
  const stream = audioStreams.get(streamId);
  if (stream) {
    stream.ctx.close().catch(() => {});
    audioStreams.delete(streamId);
    console.log("[Flash Player] Audio stream closed:", streamId);
  }
}

// ---------------------------------------------------------------------------
// Web Audio input capture — uses getUserMedia + ScriptProcessorNode to
// capture mono i16 PCM from the microphone and send it back to the host.
// ---------------------------------------------------------------------------

/**
 * Active audio input streams.
 * stream_id -> {
 *   ctx,           // AudioContext
 *   mediaStream,   // MediaStream from getUserMedia
 *   source,        // MediaStreamAudioSourceNode
 *   processor,     // ScriptProcessorNode
 *   port,          // native messaging port for sending data back
 *   sampleRate,
 *   frameCount,
 *   capturing,     // boolean
 *   ready,         // Promise that resolves when the stream is fully set up
 *   pendingStart,  // true if audioInputStart was called before ready
 * }
 */
const audioInputStreams = new Map();

/**
 * Open a new audio input capture stream.
 * This requests microphone permission and sets up the audio graph,
 * but does not start sending data until audioInputStart() is called.
 */
function audioInputOpen(streamId, sampleRate, frameCount, port) {
  // Close any existing stream with the same id.
  audioInputClose(streamId);

  // Create a placeholder entry immediately so that audioInputStart()
  // can find it even before the async setup completes.
  const streamState = {
    ctx: null,
    mediaStream: null,
    source: null,
    processor: null,
    port,
    sampleRate,
    frameCount,
    capturing: false,
    ready: null,
    pendingStart: false,
  };

  // The ready promise tracks the async setup.
  streamState.ready = (async () => {
    try {
      const mediaStream = await navigator.mediaDevices.getUserMedia({
        audio: {
          sampleRate: { ideal: sampleRate },
          channelCount: { exact: 1 },
          echoCancellation: false,
          noiseSuppression: false,
          autoGainControl: false,
        },
      });

      const ctx = new AudioContext({ sampleRate });
      const source = ctx.createMediaStreamSource(mediaStream);

      // ScriptProcessorNode: captures PCM in buffers of `frameCount` frames.
      // (AudioWorklet would be preferred but requires a separate module file
      // and complicates the extension packaging; ScriptProcessorNode works
      // for the buffer sizes Flash typically requests.)
      const processor = ctx.createScriptProcessor(frameCount, 1, 1);

      streamState.ctx = ctx;
      streamState.mediaStream = mediaStream;
      streamState.source = source;
      streamState.processor = processor;

      processor.onaudioprocess = (e) => {
        if (!streamState.capturing) return;

        const input = e.inputBuffer.getChannelData(0);
        // Convert float32 [-1, 1] to i16 LE bytes.
        const i16 = new Int16Array(input.length);
        for (let i = 0; i < input.length; i++) {
          const s = Math.max(-1, Math.min(1, input[i]));
          i16[i] = s < 0 ? s * 0x8000 : s * 0x7FFF;
        }

        // Encode as base64 and send to the host.
        const bytes = new Uint8Array(i16.buffer);
        const b64 = uint8ToB64(bytes);

        streamState.port.postMessage({
          type: "audioInputData",
          streamId,
          data: b64,
        });
      };

      // Connect the graph: source → processor → destination
      // (destination connection is required for ScriptProcessorNode to fire)
      source.connect(processor);
      processor.connect(ctx.destination);

      // If audioInputStart() was already called while we were setting up,
      // start capturing immediately.  Otherwise suspend the context.
      if (streamState.pendingStart) {
        streamState.capturing = true;
        streamState.pendingStart = false;
        console.log("[Flash Player] Audio input stream opened + auto-started:", streamId,
                    "rate:", sampleRate, "frames:", frameCount);
      } else {
        await ctx.suspend();
        console.log("[Flash Player] Audio input stream opened (suspended):", streamId,
                    "rate:", sampleRate, "frames:", frameCount);
      }
    } catch (e) {
      console.error("[Flash Player] Failed to open audio input:", e);
    }
  })();

  audioInputStreams.set(streamId, streamState);
}

/**
 * Start capturing on an audio input stream.
 */
function audioInputStart(streamId) {
  const stream = audioInputStreams.get(streamId);
  if (!stream) return;

  // If the async setup hasn't completed yet, flag it so that
  // audioInputOpen's ready handler will auto-start.
  if (!stream.ctx) {
    stream.pendingStart = true;
    console.log("[Flash Player] Audio input start queued (still opening):", streamId);
    return;
  }

  stream.capturing = true;
  if (stream.ctx.state === "suspended") {
    stream.ctx.resume();
  }
  console.log("[Flash Player] Audio input capture started:", streamId);
}

/**
 * Stop capturing on an audio input stream.
 */
function audioInputStop(streamId) {
  const stream = audioInputStreams.get(streamId);
  if (!stream) return;
  stream.capturing = false;
  stream.pendingStart = false;
  if (stream.ctx) {
    stream.ctx.suspend();
  }
  console.log("[Flash Player] Audio input capture stopped:", streamId);
}

/**
 * Close and release an audio input stream.
 */
function audioInputClose(streamId) {
  const stream = audioInputStreams.get(streamId);
  if (!stream) return;
  stream.capturing = false;
  stream.pendingStart = false;
  if (stream.processor) stream.processor.disconnect();
  if (stream.source) stream.source.disconnect();
  if (stream.mediaStream) stream.mediaStream.getTracks().forEach(t => t.stop());
  if (stream.ctx) stream.ctx.close().catch(() => {});
  audioInputStreams.delete(streamId);
  console.log("[Flash Player] Audio input stream closed:", streamId);
}

/**
 * Handle a fully reassembled binary message (as a base64 string).
 */
function handleBinaryMessage(ctx, canvas, b64, port) {
  const b64Len = b64.length;
  const t0 = FLASH_DEBUG ? performance.now() : 0;
  const bytes = b64ToUint8(b64);
  const decodeMs = FLASH_DEBUG ? performance.now() - t0 : 0;
  if (bytes.length === 0) return;

  const tag = bytes[0];
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);

  // ---- Debug stats recording ----
  // Find the stats object for this canvas (there may be multiple instances).
  let _ds = null;
  if (FLASH_DEBUG) {
    const instId = canvas.getAttribute("data-flash-player");
    if (instId != null) _ds = debugStatsMap.get(Number(instId));
    if (_ds) {
      _ds.recordMessage(b64Len, bytes.length, tag, decodeMs);
      _ds.markProcessingStart();
    }
  }

  switch (tag) {
    case TAG_FRAME: {
      // 1 byte tag + 7 x u32 header (28 bytes) + QOI-encoded RGBA pixels
      const x       = readU32(dv, 1);
      const y       = readU32(dv, 5);
      const width   = readU32(dv, 9);
      const height  = readU32(dv, 13);
      const frameW  = readU32(dv, 17);
      const frameH  = readU32(dv, 21);
      // stride at offset 25 — ignored for QOI
      const qoiOffset = 29; // 1 + 7*4
      const qoiLen = bytes.length - qoiOffset;

      // Resize canvas if frame dimensions changed.
      if (ctx.canvas.width !== frameW || ctx.canvas.height !== frameH) {
        ctx.canvas.width = frameW;
        ctx.canvas.height = frameH;
      }

      if (!_qoiDecode || !_qoiMemory) break; // WASM not ready yet

      // Ensure WASM memory can hold input + working space + decoded output.
      const needBytes = qoiLen + 268 + width * height * 4;
      const needPages = Math.ceil(needBytes / 65536);
      const currPages = _qoiMemory.buffer.byteLength / 65536;
      if (needPages > currPages) {
        _qoiMemory.grow(needPages - currPages);
      }

      // Copy QOI data into WASM memory at address 0.
      new Uint8Array(_qoiMemory.buffer, 0, qoiLen).set(
        bytes.subarray(qoiOffset, qoiOffset + qoiLen),
      );

      // Decode — returns pointer to [u32 width, u32 height, ...RGBA pixels].
      const ptr = _qoiDecode(qoiLen);
      const pixelLen = width * height * 4;
      const rgba = new Uint8ClampedArray(_qoiMemory.buffer, ptr + 8, pixelLen);

      const imageData = new ImageData(rgba, width, height);
      const rt0 = FLASH_DEBUG ? performance.now() : 0;
      ctx.putImageData(imageData, x, y);
      if (FLASH_DEBUG && _ds) _ds.recordFrameRender(performance.now() - rt0);
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
      const message = _textDecoder.decode(msgBytes);
      console.error("[Flash Player]", message);
      break;
    }

    case TAG_SCRIPT: {
      // 1 byte tag + u32 json_len + UTF-8 JSON bytes
      const jsonLen = readU32(dv, 1);
      const jsonBytes = bytes.subarray(5, 5 + jsonLen);
      const jsonStr = _textDecoder.decode(jsonBytes);
      try {
        const req = JSON.parse(jsonStr);
        handleScriptRequest(req, port);
      } catch (e) {
        console.error("[Flash Player] Bad script request:", e, jsonStr);
      }
      break;
    }

    case TAG_NAVIGATE: {
      // 1 byte tag + u32 url_len + UTF-8 url + u32 target_len + UTF-8 target
      const urlLen = readU32(dv, 1);
      const url = _textDecoder.decode(bytes.subarray(5, 5 + urlLen));
      const targetLen = readU32(dv, 5 + urlLen);
      const target = _textDecoder.decode(bytes.subarray(9 + urlLen, 9 + urlLen + targetLen));
      console.log("[Flash Player] Navigate:", url, "target:", target);
      try {
        if (target === "_blank") {
          window.open(url, "_blank");
        } else if (target === "_self" || target === "" || target === "_top") {
          window.location.href = url;
        } else if (target === "_parent") {
          (window.parent || window).location.href = url;
        } else {
          // Named target — try window.open with that name.
          window.open(url, target);
        }
      } catch (e) {
        console.error("[Flash Player] Navigate failed:", e);
      }
      break;
    }

    // ---- Audio messages ----

    case TAG_AUDIO_INIT: {
      // 1 byte tag + u32 stream_id + u32 sample_rate + u32 frame_count
      const streamId   = readU32(dv, 1);
      const sampleRate = readU32(dv, 5);
      const frameCount = readU32(dv, 9);
      audioInit(streamId, sampleRate, frameCount);
      break;
    }

    case TAG_AUDIO_SAMPLES: {
      // 1 byte tag + u32 stream_id + PCM bytes
      const streamId = readU32(dv, 1);
      const pcmBytes = bytes.subarray(5);
      audioWriteSamples(streamId, pcmBytes);
      break;
    }

    case TAG_AUDIO_START: {
      const streamId = readU32(dv, 1);
      audioStart(streamId);
      break;
    }

    case TAG_AUDIO_STOP: {
      const streamId = readU32(dv, 1);
      audioStop(streamId);
      break;
    }

    case TAG_AUDIO_CLOSE: {
      const streamId = readU32(dv, 1);
      audioClose(streamId);
      break;
    }

    // ---- Audio input messages ----

    case TAG_AUDIO_INPUT_OPEN: {
      // 1 byte tag + u32 stream_id + u32 sample_rate + u32 frame_count
      const streamId   = readU32(dv, 1);
      const sampleRate = readU32(dv, 5);
      const frameCount = readU32(dv, 9);
      audioInputOpen(streamId, sampleRate, frameCount, port);
      break;
    }

    case TAG_AUDIO_INPUT_START: {
      const streamId = readU32(dv, 1);
      audioInputStart(streamId);
      break;
    }

    case TAG_AUDIO_INPUT_STOP: {
      const streamId = readU32(dv, 1);
      audioInputStop(streamId);
      break;
    }

    case TAG_AUDIO_INPUT_CLOSE: {
      const streamId = readU32(dv, 1);
      audioInputClose(streamId);
      break;
    }

    default:
      console.warn("[Flash Player] Unknown binary message tag:", tag);
  }

  // ---- Debug: mark processing done ----
  if (FLASH_DEBUG && _ds) _ds.markProcessingEnd();
}

// ---------------------------------------------------------------------------
// JavaScript scripting bridge  (TAG_SCRIPT = 0x10)
//
// The actual JS execution runs in page-script.js (MAIN world).  This
// content script (ISOLATED world) proxies requests/responses through a
// shared hidden DOM element using synchronous dispatchEvent.
// ---------------------------------------------------------------------------

const COMM_ID = "__flash_player_comm__";

/**
 * Ensure the hidden communication element exists in the DOM.
 * Both the content script and the page script reference it by id.
 */
function getCommElement() {
  let el = document.getElementById(COMM_ID);
  if (!el) {
    el = document.createElement("div");
    el.id = COMM_ID;
    el.style.display = "none";
    (document.documentElement || document.body || document.head).appendChild(el);
  }
  return el;
}

/**
 * Send a scripting request to the MAIN-world page script and return
 * the JSON-parsed response synchronously.
 *
 * Works because `dispatchEvent` is synchronous — the page script's
 * listener runs in the same call stack, writes the response attribute,
 * and then control returns here.
 */
function sendToPageScript(req) {
  const comm = getCommElement();
  comm.setAttribute("data-req", JSON.stringify(req));
  comm.setAttribute("data-resp", ""); // clear previous
  comm.dispatchEvent(new CustomEvent("__flash_req"));
  const respStr = comm.getAttribute("data-resp");
  if (!respStr) return null;
  try {
    return JSON.parse(respStr);
  } catch {
    return null;
  }
}

/**
 * Handle a scripting request from the native host.
 * Forwards to the MAIN-world page script and sends the response back.
 */
function handleScriptRequest(req, port) {
  const id = req.id;
  const op = req.op;

  // Fire-and-forget operations (e.g. "release") don't need a response.
  if (op === "release") {
    sendToPageScript(req);
    return;
  }

  const resp = sendToPageScript(req);
  if (!resp) {
    sendScriptError(port, id, "no response from page script");
    return;
  }

  if (resp.error) {
    sendScriptError(port, id, resp.error);
  } else if (resp.names) {
    // getAllPropertyNames returns {names: [...]}
    try {
      port.postMessage({ type: "jsResponse", id, names: resp.names });
    } catch { /* port disconnected */ }
  } else {
    sendScriptResponse(port, id, resp.value);
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
// ExternalInterface: JS → AS (CallFunction bridge)
//
// page-script.js dispatches "__flash_callfn" CustomEvents on the shared
// comm element when JavaScript calls a registered ExternalInterface
// callback (e.g. game.startup(…)).  We forward the invoke XML to the
// native host which routes it to PepperFlash's scriptable object.
// ---------------------------------------------------------------------------

/**
 * Set up the listener that forwards CallFunction invocations from the
 * MAIN-world page script to the native messaging host.
 */
function initCallFunctionBridge() {
  const comm = getCommElement();
  comm.addEventListener("__flash_callfn", () => {
    const xml = comm.getAttribute("data-callfn");
    if (xml && activePort) {
      try {
        activePort.postMessage({ type: "callFunction", xml });
      } catch {
        // Port may have disconnected.
      }
    }
  });
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

  // Throttle mousemove to one message per animation frame to avoid
  // flooding the native messaging channel with hundreds of events/sec.
  let _pendingMove = null;
  let _moveRaf = 0;
  canvas.addEventListener("mousemove", (e) => {
    _pendingMove = e;
    if (!_moveRaf) {
      _moveRaf = requestAnimationFrame(() => {
        _moveRaf = 0;
        if (_pendingMove) {
          const pos = canvasPos(canvas, _pendingMove);
          port.postMessage({
            type: "mousemove",
            x: pos.x,
            y: pos.y,
            modifiers: getModifiers(_pendingMove),
          });
          _pendingMove = null;
        }
      });
    }
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
    // Send RAWKEYDOWN — matches Chrome's PPAPI behaviour.
    // PepperFlash expects RAWKEYDOWN (type 6), not KEYDOWN (type 7).
    port.postMessage({
      type: "rawkeydown",
      keyCode: e.keyCode,
      code: e.code,
      modifiers: getModifiers(e),
    });

    // Synthesize a CHAR event for character-producing keys.
    // This replaces the deprecated 'keypress' event and is more reliable
    // across browsers.  Ctrl/Meta combos are shortcuts, not characters.
    if (!e.ctrlKey && !e.metaKey) {
      if (e.key.length === 1) {
        // Printable character (letters, digits, symbols, space).
        port.postMessage({
          type: "char",
          keyCode: e.key.charCodeAt(0),
          text: e.key,
          code: e.code,
          modifiers: getModifiers(e),
        });
      } else if (!e.altKey) {
        // Special keys that produce character events in PPAPI.
        let charCode = 0, charText = "";
        switch (e.key) {
          case "Enter":     charCode = 13; charText = "\r"; break;
          case "Tab":       charCode = 9;  charText = "\t"; break;
          case "Backspace": charCode = 8;  charText = "";   break;
        }
        if (charCode) {
          port.postMessage({
            type: "char",
            keyCode: charCode,
            text: charText,
            code: e.code,
            modifiers: getModifiers(e),
          });
        }
      }
    }
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

  // --- IME composition events ---
  // These fire on any focused element when an Input Method Editor is active
  // (e.g. CJK input, dead-key sequences on European keyboards).

  canvas.addEventListener("compositionstart", (e) => {
    port.postMessage({ type: "compositionstart" });
  });

  canvas.addEventListener("compositionupdate", (e) => {
    port.postMessage({
      type: "compositionupdate",
      text: e.data || "",
    });
  });

  canvas.addEventListener("compositionend", (e) => {
    port.postMessage({
      type: "compositionend",
      text: e.data || "",
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

/**
 * Tear down all active Flash instances — disconnects ports so the
 * background service worker shuts down the native hosts.
 * Also closes any active Web Audio streams.
 */
function destroyAllInstances() {
  for (const [id, meta] of instanceMeta) {
    if (meta.port) {
      try { meta.port.disconnect(); } catch { /* already gone */ }
      meta.port = null;
    }
    // Remove any crash overlay that may be showing.
    const overlay = meta.container.querySelector(".flash-crash-overlay");
    if (overlay) overlay.remove();
    // Stop debug stats.
    const ds = debugStatsMap.get(id);
    if (ds) { ds.stop(); debugStatsMap.delete(id); }
  }
  // Close all audio streams.
  for (const [streamId] of audioStreams) {
    audioClose(streamId);
  }
  activePort = null;
}

/**
 * Restart every known Flash instance from its saved metadata.
 * Used when the page is restored from bfcache.
 */
function restartAllInstances() {
  for (const [id, meta] of instanceMeta) {
    // Clear the canvas and reconnect.
    meta.ctx.clearRect(0, 0, meta.canvas.width, meta.canvas.height);
    startInstance(id, meta);
  }
}

function init() {
  // Set up the ExternalInterface CallFunction bridge (JS → AS).
  initCallFunctionBridge();

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

// ---------------------------------------------------------------------------
// Page lifecycle: tear down on navigate-away, restart on bfcache restore
// ---------------------------------------------------------------------------

window.addEventListener("pagehide", () => {
  navigatingAway = true;
  destroyAllInstances();
});

window.addEventListener("pageshow", (e) => {
  if (e.persisted) {
    // Page was restored from bfcache — ports are dead, restart everything.
    navigatingAway = false;
    restartAllInstances();
  }
});
