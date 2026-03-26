/**
 * Flash Player Content Script
 *
 * Detects <object> and <embed> elements that reference Flash content
 * and replaces them with a <canvas> driven by the native Flash Player
 * host via Native Messaging.
 */

"use strict";

// ---------------------------------------------------------------------------
// Debug mode - set to true to show a live statistics panel below each canvas.
// ---------------------------------------------------------------------------
const FLASH_DEBUG = false;
const LOG_HOST_EVENTS = false;
const IS_FIREFOX = typeof navigator !== "undefined" && /\bFirefox\//.test(navigator.userAgent);
const EXTENSION_VERSION = "1.0.0";

// ---------------------------------------------------------------------------
// Settings — read from chrome.storage.sync and exposed to page-script.js
// via a data attribute on <html>.
// ---------------------------------------------------------------------------
const SETTINGS_DEFAULTS = {
  ruffleCompat: 1,              // 0=PreferRuffle, 1=PreferCleanFlash, 2=ForceCleanFlash
  preferNetworkBrowser: true,
  networkFallbackNative: true,
  disableCrossdomainHttp: true,
  disableCrossdomainSockets: true,
  hardwareAcceleration: false,
  audioBackend: 0,              // 0=Browser, 1=Native
  disableGeolocation: true,
  spoofHardwareId: true,
  disableMicrophone: false,
  disableWebcam: false,
  // Sandboxing
  httpSandboxMode: "blacklist",
  httpBlacklist: [],
  httpWhitelist: [],
  tcpUdpSandboxMode: "blacklist",
  tcpUdpBlacklist: [],
  tcpUdpWhitelist: [],
  fileWhitelistEnabled: true,
  whitelistedFiles: [],
  whitelistedFolders: [],
  // Advanced
  urlRewriteRules: [],
};

function normalizeFlashSettings(raw) {
  const merged = Object.assign({}, SETTINGS_DEFAULTS, raw || {});
  if (!Array.isArray(merged.urlRewriteRules)) {
    merged.urlRewriteRules = [];
  }
  return merged;
}

let _flashSettings = normalizeFlashSettings();

/**
 * Apply settings to in-memory + DOM/localStorage cache, then optionally broadcast.
 * @param {Object} raw - Partial/full settings object.
 * @param {(settings: Object) => void} [broadcastFn] - Optional broadcast callback.
 */
function applyFlashSettings(raw, broadcastFn) {
  _flashSettings = normalizeFlashSettings(raw);
  try {
    const json = JSON.stringify(_flashSettings);
    document.documentElement.setAttribute("data-flash-settings", json);
    localStorage.setItem("_flashSettings", json);
  } catch (_) {}
  if (typeof broadcastFn === "function") {
    broadcastFn(_flashSettings);
  }
}

// Read settings and inject into the DOM for the MAIN-world page script.
(function loadSettings() {
  // Synchronously set cached settings so page-script.js (MAIN world) can
  // read them before the async chrome.storage call completes.
  try {
    const cached = localStorage.getItem("_flashSettings");
    if (cached) {
      document.documentElement.setAttribute("data-flash-settings", cached);
      // Also apply cached settings to the content-script's own copy.
      const parsed = JSON.parse(cached);
      if (parsed) _flashSettings = normalizeFlashSettings(parsed);
    }
  } catch (_) {}

  const storage = chrome.storage && (chrome.storage.sync || chrome.storage.local);
  if (!storage) return;
  storage.get(SETTINGS_DEFAULTS, (items) => {
    if (chrome.runtime.lastError) return;
    // Keep content-script cache + DOM/localStorage in sync.
    applyFlashSettings(items);
  });
})();

// Listen for live settings updates from the background service worker
// (triggered when the user changes settings in the popup).
chrome.runtime.onMessage.addListener((msg) => {
  if (msg.type === "settingsUpdate" && msg.settings) {
    console.log("[Flash Player] Received settings update:", msg.settings);
    applyFlashSettings(msg.settings, FlashInstance.broadcastSettingsUpdate);
  }
});

// Firefox content-script rendering pipeline.
// putImageData, createImageBitmap(ImageData), and cloneInto(ImageData) all
// fail in Firefox content scripts due to Xray wrapper security checks on
// typed arrays crossing compartment boundaries.  We encode the RGBA pixels
// as a minimal BMP blob and use createImageBitmap(Blob) + drawImage, which
// bypasses typed-array compartment checks entirely because the Blob
// constructor copies the data into an opaque internal store.
let _ffRenderChain = Promise.resolve();
const _FF_BMP_HDR_SIZE = 14 + 108; // BITMAPFILEHEADER + BITMAPV4HEADER

function _buildBmpBlob(rgba, w, h) {
  const pxBytes = w * h * 4;
  const hdr = new ArrayBuffer(_FF_BMP_HDR_SIZE);
  const u8 = new Uint8Array(hdr);
  const dv = new DataView(hdr);
  u8[0] = 0x42; u8[1] = 0x4D;                      // 'BM'
  dv.setUint32(2, _FF_BMP_HDR_SIZE + pxBytes, true); // file size
  dv.setUint32(10, _FF_BMP_HDR_SIZE, true);           // pixel data offset
  dv.setUint32(14, 108, true);                        // V4 header size
  dv.setInt32(18, w, true);                        // width
  dv.setInt32(22, -h, true);                        // height (neg = top-down)
  dv.setUint16(26, 1, true);                        // planes
  dv.setUint16(28, 32, true);                        // 32 bpp
  dv.setUint32(30, 3, true);                        // BI_BITFIELDS
  dv.setUint32(34, pxBytes, true);                    // image data size
  // RGBA channel masks — matches native RGBA byte order, no swap needed.
  dv.setUint32(54, 0x000000FF, true);                 // R mask
  dv.setUint32(58, 0x0000FF00, true);                 // G mask
  dv.setUint32(62, 0x00FF0000, true);                 // B mask
  dv.setUint32(66, 0xFF000000, true);                 // A mask
  dv.setUint32(70, 0x73524742, true);                 // bV4CSType = 'sRGB'
  // Blob() copies the typed-array data, so it's safe even though rgba
  // points into WASM memory that may be overwritten by the next frame.
  return new Blob([u8, rgba], { type: "image/bmp" });
}

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
    // ActiveX classid check - only accept Flash classid.
    const classid = elem.getAttribute("classid");
    if (classid && classid.toLowerCase() !== "clsid:d27cdb6e-ae6d-11cf-96b8-444553540000") {
      return null;
    }
    // MIME type check.
    const type = elem.getAttribute("type");
    if (type && type !== "application/x-shockwave-flash") {
      return null;
    }

    const params = {};
    const attrs = elem.attributes;
    for (let i = 0; i < attrs.length; i++) {
      params[attrs[i].name.toLowerCase()] = attrs[i].value;
    }
    if (!params.src && params.data) {
      params.src = params.data;
    }
    for (let i = 0; i < elem.children.length; i++) {
      const c = elem.children[i];
      if (c.nodeName.toLowerCase() === "param") {
        const name = c.getAttribute("name");
        const value = c.getAttribute("value");
        if (name != null && value != null) {
          params[name.toLowerCase()] = value;
        }
      } else if (c.tagName === "EMBED") {
        // Merge attributes from nested <embed> fallback - fill in any
        // keys not already set by <object> attributes or <param> tags.
        const embedAttrs = c.attributes;
        for (let j = 0; j < embedAttrs.length; j++) {
          const key = embedAttrs[j].name.toLowerCase();
          if (!(key in params)) {
            params[key] = embedAttrs[j].value;
          }
        }
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
 * Convert parsed <object>/<embed> params into DidCreate argn/argv pairs.
 */
function buildDidCreateArgs(params, swfUrl) {
  const args = [];
  const seen = new Set();

  const push = (name, value) => {
    if (name == null) return;
    const key = String(name).trim().toLowerCase();
    if (!key || seen.has(key)) return;
    args.push({ name: key, value: value == null ? "" : String(value) });
    seen.add(key);
  };

  for (const [name, value] of Object.entries(params || {})) {
    push(name, value);
  }

  // Keep core Flash keys present even if not explicitly specified in HTML.
  push("type", "application/x-shockwave-flash");
  push("src", swfUrl);
  push("movie", swfUrl);
  push("data", swfUrl);
  // Flash uses the "base" param to resolve relative URLs internally.
  // Without it, it falls back to loaderInfo.url (the SWF origin).
  push("base", document.baseURI);

  return args;
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
// QOI WASM decoder - lazily loaded on first frame decode.
// ---------------------------------------------------------------------------

/** @type {Function|null} decode(data_len) → output_ptr */
let _qoiDecode = null;
/** @type {WebAssembly.Memory|null} */
let _qoiMemory = null;

function _ensureQoiDecoderReady() {
  if (_qoiDecode && _qoiMemory) return true;
  try {
    const b64 = "AGFzbQEAAAABDgNgAX8AYAABf2ABfwF/AwoJAAABAAAAAAACBAUBcAEEBAUDAQABBhoFfwFBAAt/AUEAC38BQQALfwFBAAt/AUEACwcvBQZtZW1vcnkCAAtvdXRwdXRfYmFzZQMBBGlwdHIDAwRvcHRyAwQGZGVjb2RlAAgJCgEAQQALBAQGBwUK3gUJJAAjBCAAaiQEA0AjBD8AQRB0TwRAQQFAAEEASARAAAsMAQsLCw0AIwQgADYCAEEEEAALKgECf0EEIQADQCMDLQAAIAFBCHdyIQEjA0EBaiQDIABBAWsiAA0ACyABC0oAIwAgADYCACMAIABBCHZB/wFxQQVsIABB/wFxQQNsaiAAQRB2Qf8BcUEHbGogAEEYdkELbGpBP3FBAnRBBGpqIAA2AgAgABABCxIAIwAgAEECdEEEamooAgAQAwscACAAQQFqIQADQCMAKAIAEAMgAEEBayIADQALC3ABAX8jACgCACIBQf//g3hxIABBA3FBAmsgAUEQdkH/AXFqQf8BcUEQdHIiAUH/gXxxIABBAnYiAEEDcUECayABQQh2Qf8BcWpB/wFxQQh0ciIBQYB+cSAAQQJ2QQNxQQJrIAFB/wFxakH/AXFyEAMLewECfyMAKAIAIgFBgH5xIwMtAAAhAiMDQQFqJAMgAEEgayIAIAJBBHZBCGtqIAFB/wFxakH/AXFyIgFB/4F8cSAAIAFBCHZB/wFxakH/AXFBCHRyIgFB//+DeHEgAkEPcUEIayAAaiABQRB2Qf8BcWpB/wFxQRB0chADC5UCACAAJAJBACQDIABBfHFBBGokACMAQYQCaiQBIwBBgICAeDYCACMBJARBABAAIwMtAAAjA0EBaiQDQfEARwRAAAsjAy0AACMDQQFqJANB7wBHBEAACyMDLQAAIwNBAWokA0HpAEcEQAALIwMtAAAjA0EBaiQDQeYARwRAAAsQAhABEAIQASMDLQAAGiMDQQFqJAMjAy0AABojA0EBaiQDA0AjAy0AACEAIwNBAWokAwJAIABB/gFGBEAjAygCACMDQQRqJAMjA0EBayQDQf///wdxIwAoAgBBgICAeHFyEAMMAQsgAEH/AUYEQCMDKAIAIwNBBGokAxADDAELIABBP3EgAEEGdhEAAAsjAyMCRw0ACyMBCw==";

    let wasmBytes;
    if (typeof Uint8Array.fromBase64 === "function") {
      wasmBytes = Uint8Array.fromBase64(b64);
    } else {
      const bin = atob(b64);
      const out = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
      wasmBytes = out;
    }

    const module = new WebAssembly.Module(wasmBytes);
    const instance = new WebAssembly.Instance(module);
    _qoiDecode = instance.exports.decode;
    _qoiMemory = instance.exports.memory;
    console.log("[Flash Player] QOI WASM decoder ready");
    return true;
  } catch (e) {
    console.error("[Flash Player] Failed to load QOI WASM decoder:", e);
    return false;
  }
}

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
  [0x40]: "CONTEXT_MENU",
  [0x50]: "PRINT",
  [0x60]: "VIDCAP_OPEN",
  [0x61]: "VIDCAP_START",
  [0x62]: "VIDCAP_STOP",
  [0x63]: "VIDCAP_CLOSE",
  [0x70]: "VERSION",
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

// ---------------------------------------------------------------------------
// FlashInstance - encapsulates a single Flash Player instance.
// ---------------------------------------------------------------------------

class FlashInstance {
  // ---- Static state ----

  /** @type {Map<number, FlashInstance>} All active instances. */
  static _instances = new Map();
  static _nextId = 0;
  /** @type {Port|null} Most recently started port (ExternalInterface bridge). */
  static _activePort = null;
  /** True while we are intentionally tearing down (page navigation). */
  static _navigatingAway = false;
  /** True while all ports are intentionally closed due to version mismatch. */
  static _closingForVersionMismatch = false;
  /** Pending broadcastViewUpdate timer. */
  static _viewUpdateTimer = null;

  // ---- Static lookup helpers ----

  /**
   * Look up a FlashInstance by its numeric instance ID.
   * @param {number} id - The instance ID.
   * @returns {FlashInstance|undefined}
   */
  static getById(id) {
    return FlashInstance._instances.get(id);
  }

  /**
   * Find the FlashInstance that owns a particular native messaging port.
   * @param {Port} port - The chrome.runtime port to search for.
   * @returns {FlashInstance|null}
   */
  static getByPort(port) {
    for (const inst of FlashInstance._instances.values()) {
      if (inst.port === port) return inst;
    }
    return null;
  }

  /**
   * Find the instance ID for the FlashInstance that owns a particular port.
   * @param {Port} port - The chrome.runtime port to search for.
   * @returns {number|null}
   */
  static getIdByPort(port) {
    for (const [id, inst] of FlashInstance._instances) {
      if (inst.port === port) return id;
    }
    return null;
  }

  // ---- Static lifecycle ----

  /**
   * Create a FlashInstance by replacing a Flash <object> or <embed> element.
   * Returns the instance, or null if the element is not Flash content.
   */
  static fromElement(elem) {
    if (elem.parentNode == null) return null;
    const params = getFlashParams(elem);
    if (!params || !params.src) return null;
    const swfUrl = resolveSwfUrl(params.src);
    if (!swfUrl) return null;
    return new FlashInstance(elem, params, swfUrl);
  }

  /**
   * Destroy all instances whose element belongs to the given document.
   * Also closes any associated audio/video streams.
   */
  static destroyForDocument(doc) {
    for (const [, inst] of FlashInstance._instances) {
      if (inst.ownerDocument === doc) {
        inst.destroy();
      }
    }
  }

  /**
   * Restart all instances whose element belongs to the given document.
   * Used when the page is restored from bfcache.
   */
  static restartForDocument(doc) {
    for (const [, inst] of FlashInstance._instances) {
      if (inst.ownerDocument === doc) {
        inst.ctx.clearRect(0, 0, inst.canvas.width, inst.canvas.height);
        inst.start();
      }
    }
  }

  /**
   * Close all active native messaging ports when host/extension versions differ.
   * Leaves a mismatch overlay so users can install the matching version.
   *
   * @param {string} hostVersion - Version reported by the native host.
   */
  static closeAllForVersionMismatch(hostVersion) {
    if (FlashInstance._closingForVersionMismatch) return;
    FlashInstance._closingForVersionMismatch = true;
    try {
      for (const inst of FlashInstance._instances.values()) {
        inst.destroy();
        inst.showVersionMismatchOverlay(hostVersion);
      }
    } finally {
      FlashInstance._closingForVersionMismatch = false;
    }
  }

  // ---- Static broadcast ----

  /**
   * Send a viewUpdate message to all active Flash instances when the
   * browser view state changes (tab visibility, scroll, fullscreen).
   */
  static broadcastViewUpdate() {
    for (const inst of FlashInstance._instances.values()) {
      inst.sendViewUpdate();
    }
  }

  /**
   * Send a settingsUpdate message to all active Flash instances so the
   * native host applies changed settings on-the-fly.
   * @param {Object} settings - The full settings object.
   */
  static broadcastSettingsUpdate(settings) {
    for (const inst of FlashInstance._instances.values()) {
      if (inst.port) {
        try {
          inst.port.postMessage({ type: "settingsUpdate", settings });
        } catch (_) {}
      }
    }
  }

  /**
   * Debounce broadcastViewUpdate to avoid flooding the native host
   * during rapid scroll or resize events.
   * @param {number} [delayMs=100] - Minimum delay in milliseconds.
   */
  static scheduleBroadcastViewUpdate(delayMs = 100) {
    if (FlashInstance._viewUpdateTimer) return;
    FlashInstance._viewUpdateTimer = setTimeout(() => {
      FlashInstance._viewUpdateTimer = null;
      FlashInstance.broadcastViewUpdate();
    }, delayMs);
  }

  // ---- Static event handlers ----

  /**
   * Handle fullscreen enter/exit for all Flash instances.
   * Saves pre-fullscreen CSS dimensions on enter and restores them on exit.
   * On enter, the canvas and container expand to fill the screen.
   */
  static handleFullscreenChange() {
    const fsEl = document.fullscreenElement || document.webkitFullscreenElement;
    for (const inst of FlashInstance._instances.values()) {
      const { canvas, container } = inst;
      if (fsEl && (fsEl === container || fsEl.contains(canvas))) {
        // Save pre-fullscreen CSS sizes so we can restore them on exit.
        inst._preFsContainerW = container.style.width;
        inst._preFsContainerH = container.style.height;
        inst._preFsCanvasW = canvas.style.width;
        inst._preFsCanvasH = canvas.style.height;
        // Entering fullscreen - expand container and canvas to screen size.
        container.style.width = "100vw";
        container.style.height = "100vh";
        container.style.backgroundColor = "#000";
        canvas.style.width = "100%";
        canvas.style.height = "100%";
        const w = screen.width;
        const h = screen.height;
        canvas.width = w;
        canvas.height = h;
        canvas.focus();
        if (inst.port) {
          inst.port.postMessage({ type: "resize", width: w, height: h, ...inst.collectViewInfo() });
        }
      } else if (!fsEl && inst._preFsContainerW != null) {
        // Exiting fullscreen - restore original dimensions.
        container.style.width = inst._preFsContainerW;
        container.style.height = inst._preFsContainerH;
        container.style.backgroundColor = "";
        canvas.style.width = inst._preFsCanvasW;
        canvas.style.height = inst._preFsCanvasH;
        canvas.width = inst.origWidth;
        canvas.height = inst.origHeight;
        inst._preFsContainerW = null;
        if (inst.port) {
          inst.port.postMessage({ type: "resize", width: inst.origWidth, height: inst.origHeight, ...inst.collectViewInfo() });
        }
      }
    }
    FlashInstance.broadcastViewUpdate();
  }

  /**
   * Notify all active Flash instances when the pointer lock state changes
   * (e.g. user pressed Escape to unlock the cursor).
   */
  static handlePointerLockChange() {
    const locked = !!document.pointerLockElement;
    for (const inst of FlashInstance._instances.values()) {
      if (inst.port) {
        inst.port.postMessage({ type: "cursorLockChanged", locked });
      }
    }
  }

  // ---- Constructor ----

  /**
   * Create a new FlashInstance by replacing a Flash <object> or <embed>
   * element with a <canvas> and connecting to the native messaging host.
   *
   * @param {Element} elem - The original <object> or <embed> DOM element.
   * @param {Object} params - Key-value pairs extracted from the element's
   *   attributes and <param> children.
   * @param {string} swfUrl - Resolved absolute URL of the SWF file.
   */
  constructor(elem, params, swfUrl) {
    this.instanceId = FlashInstance._nextId++;
    this.elem = elem;
    this.ownerDocument = elem.ownerDocument;
    this.swfUrl = swfUrl;
    this.didCreateArgs = buildDidCreateArgs(params, swfUrl);
    this.port = null;
    this._started = false;
    this._resizeObserver = null;
    this._mutationObserver = null;
    this._debugStats = null;
    this._preFsContainerW = null;
    this._preFsContainerH = null;
    this._preFsCanvasW = null;
    this._preFsCanvasH = null;

    // Track associated audio/video/input stream IDs so that destroy()
    // can close them when the instance is torn down.
    this._audioStreamIds = new Set();
    this._audioInputStreamIds = new Set();
    this._videoCaptureStreamIds = new Set();
    /** True while a Flash context menu is open — suppresses input events. */
    this.contextMenuOpen = false;

    // URL rewriter JS callback object ID (resolved in start()).
    this._urlRewriterCallbackId = null;
    // Store the raw attribute for deferred resolution.
    this._urlRewriterAttr = elem.getAttribute("data-url-rewriter") || params["data-url-rewriter"] || null;

    // ---- Create the replacement <canvas> ----
    const canvas = document.createElement("canvas");
    canvas.setAttribute("data-flash-player", this.instanceId);
    canvas.style.border = "0";

    // Inherit dimensions from the original element.
    // Detect percentage / non-pixel values - parseInt("100%") wrongly gives 100.
    const widthRaw = elem.getAttribute("width") || elem.style.width || "";
    const heightRaw = elem.getAttribute("height") || elem.style.height || "";
    const isRelativeWidth = widthRaw && !/^\d+$/.test(widthRaw.trim()) && !/^\d+px$/i.test(widthRaw.trim());
    const isRelativeHeight = heightRaw && !/^\d+$/.test(heightRaw.trim()) && !/^\d+px$/i.test(heightRaw.trim());

    // For the CSS display size, preserve percentage / relative values.
    const cssWidth = isRelativeWidth ? widthRaw.trim() : (parseInt(widthRaw, 10) || 550) + "px";
    const cssHeight = isRelativeHeight ? heightRaw.trim() : (parseInt(heightRaw, 10) || 400) + "px";
    canvas.style.width = elem.style.width || cssWidth;
    canvas.style.height = elem.style.height || cssHeight;

    // For the internal canvas buffer, use absolute pixels.  When the size is
    // relative we pick a sensible default - we will correct it after DOM
    // insertion once the actual rendered size can be measured.
    const origWidth = isRelativeWidth ? 550 : (parseInt(widthRaw, 10) || 550);
    const origHeight = isRelativeHeight ? 400 : (parseInt(heightRaw, 10) || 400);
    canvas.width = origWidth;
    canvas.height = origHeight;
    canvas.style.display = "inline-block";
    canvas.tabIndex = 0; // Make focusable for keyboard events.

    const ctx = canvas.getContext("2d");

    // ---- Wrap canvas in a container for crash overlay support ----
    const container = document.createElement("div");
    container.style.position = "relative";
    container.style.display = "inline-block";
    container.style.width = canvas.style.width;
    container.style.height = canvas.style.height;
    container.setAttribute("data-flash-container", this.instanceId);
    container.appendChild(canvas);

    this.canvas = canvas;
    this.ctx = ctx;
    this.origWidth = origWidth;
    this.origHeight = origHeight;
    this.container = container;
    this.usesRelativeSize = isRelativeWidth || isRelativeHeight;

    // Register in the global map.
    FlashInstance._instances.set(this.instanceId, this);

    // ---- Replace the original element in the DOM ----
    // Must happen BEFORE start() so that collectViewInfo can measure the
    // canvas via getBoundingClientRect and report isVisible correctly.
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
    elem.setAttribute("data-flash-player", this.instanceId);

    // When the embed lives inside a ruffle-player custom element (our
    // RufflePlayer override), stretch the container and canvas to fill
    // the parent element completely.
    const parentTag = (container.parentElement && container.parentElement.tagName || "").toLowerCase();
    if (parentTag.startsWith("ruffle-player")) {
      container.style.width = "100%";
      container.style.height = "100%";
      canvas.style.width = "100%";
      canvas.style.height = "100%";
    }

    // ---- Connect and start ----
    this.start();

    // Now that the container is in the DOM, measure the actual rendered size.
    // This corrects the initial dimensions when the element uses percentage or
    // other relative CSS units (e.g. width="100%" should map to the real pixel
    // width of the laid-out element, not parseInt("100%") == 100).
    if (this.usesRelativeSize) {
      const measured = this._measureRenderedSize();
      if (measured.w !== this.origWidth || measured.h !== this.origHeight) {
        this.canvas.width = measured.w;
        this.canvas.height = measured.h;
        this.origWidth = measured.w;
        this.origHeight = measured.h;
        if (this.port) {
          this.port.postMessage({ type: "resize", width: measured.w, height: measured.h, ...this.collectViewInfo() });
        }
      }
    }

    // Watch for layout changes (window resize, parent container resize) so
    // that percentage-based Flash elements stay in sync.
    this._observeResize();

    // Watch for attribute/style changes on the original <embed>/<object> so
    // that dimension changes made by page scripts propagate to the canvas.
    this._observeElementMutations();

    // Now that the container is in the DOM, create the debug panel if needed.
    // For <object>, the container lives *inside* the element, so anchor the
    // panel to the <object> itself so it appears outside/after it.
    if (FLASH_DEBUG) {
      const anchor = elem.tagName === "OBJECT" ? elem : container;
      this._createDebugStatsPanel(anchor);
    }
  }

  /**
   * Collect current view metadata from browser APIs.
   * Included in resize and viewUpdate messages so the native host can
   * populate PPB_View resources with accurate values.
   */
  collectViewInfo() {
    // Determine visibility by checking both page visibility and whether the
    // canvas has a non-zero bounding rect inside the viewport.
    const canvas = this.canvas;
    const isPageVisible = document.visibilityState === "visible";
    const isFullscreen = !!(document.fullscreenElement || document.webkitFullscreenElement);
    let isVisible = isPageVisible;
    if (isVisible && canvas) {
      const rect = canvas.getBoundingClientRect();
      isVisible = rect.width > 0 && rect.height > 0 &&
        rect.bottom > 0 && rect.right > 0 &&
        rect.top < window.innerHeight && rect.left < window.innerWidth;
    }
    return {
      deviceScale: window.devicePixelRatio || 1.0,
      cssScale: 1.0,  // TO-DO: is it just window.devicePixelRatio?
      scrollX: Math.round(window.scrollX || 0),
      scrollY: Math.round(window.scrollY || 0),
      isFullscreen,
      isVisible,
      isPageVisible,
    };
  }

  /**
   * Send a viewUpdate message to the native host for this instance.
   */
  sendViewUpdate() {
    if (this.port) {
      console.log("[Flash Player] Broadcasting view update for instance with", this.collectViewInfo());
      this.port.postMessage({ type: "viewUpdate", ...this.collectViewInfo() });
    }
  }

  /**
   * Measure the actual rendered pixel size of this instance's canvas.
   * Falls back to the provided defaults if the element has zero dimensions.
   */
  _measureRenderedSize() {
    const rect = this.canvas.getBoundingClientRect();
    const w = Math.round(rect.width) || this.origWidth;
    const h = Math.round(rect.height) || this.origHeight;
    return { w, h };
  }

  /**
   * Set up a ResizeObserver on the canvas so that when the element's CSS
   * layout size changes, the internal canvas buffer and native host are updated.
   */
  _observeResize() {
    if (typeof ResizeObserver === "undefined") return;
    const inst = this;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const cr = entry.contentRect;
        const w = Math.round(cr.width);
        const h = Math.round(cr.height);
        if (w <= 0 || h <= 0) continue;
        if (w === inst.canvas.width && h === inst.canvas.height) continue;
        inst.canvas.width = w;
        inst.canvas.height = h;
        inst.origWidth = w;
        inst.origHeight = h;
        if (inst.port) {
          console.log("[Flash Player] Resize detected for instance updating with", { width: w, height: h, ...inst.collectViewInfo() });
          inst.port.postMessage({ type: "resize", width: w, height: h, ...inst.collectViewInfo() });
        }
      }
    });
    ro.observe(this.canvas);
    this._resizeObserver = ro;
  }

  /**
   * Observe attribute changes (width, height, style) on the original
   * <embed> or <object> element so that dimension changes made by page
   * scripts propagate down to the internal container and canvas.
   */
  _observeElementMutations() {
    const elem = this.elem;
    if (!elem) return;
    const inst = this;

    const mo = new MutationObserver((mutations) => {
      let needsUpdate = false;
      for (const m of mutations) {
        if (m.type === "attributes") {
          const attr = m.attributeName.toLowerCase();
          if (attr === "width" || attr === "height" || attr === "style") {
            needsUpdate = true;
            break;
          }
        }
      }
      if (!needsUpdate) return;
      inst._syncDimensionsFromElement();
    });

    mo.observe(elem, { attributes: true, attributeFilter: ["width", "height", "style"] });
    this._mutationObserver = mo;
  }

  /**
   * Read the current dimensions from the original <embed>/<object> element
   * and propagate them to the container <div> and canvas CSS display size.
   */
  _syncDimensionsFromElement() {
    const { elem, canvas, container } = this;
    if (!elem || !canvas || !container) return;

    if (document.fullscreenElement || document.webkitFullscreenElement) return;

    const widthRaw = elem.getAttribute("width") || elem.style.width || "";
    const heightRaw = elem.getAttribute("height") || elem.style.height || "";
    if (!widthRaw && !heightRaw) return;

    const isRelativeWidth = widthRaw && !/^\d+$/.test(widthRaw.trim()) && !/^\d+px$/i.test(widthRaw.trim());
    const isRelativeHeight = heightRaw && !/^\d+$/.test(heightRaw.trim()) && !/^\d+px$/i.test(heightRaw.trim());

    // Update CSS display dimensions on container and canvas.
    // The canvas ResizeObserver will pick up the layout change and
    // update the internal buffer size + send the resize event to the host.
    const cssWidth = isRelativeWidth ? widthRaw.trim() : (parseInt(widthRaw, 10) || this.origWidth) + "px";
    const cssHeight = isRelativeHeight ? heightRaw.trim() : (parseInt(heightRaw, 10) || this.origHeight) + "px";

    container.style.width = cssWidth;
    container.style.height = cssHeight;
    canvas.style.width = cssWidth;
    canvas.style.height = cssHeight;

    // After the CSS change has been applied, measure the actual laid-out size
    // on the next frame and update the canvas buffer.  We cannot rely solely
    // on ResizeObserver because Firefox content scripts may not fire it
    // reliably due to Xray wrapper compartment isolation.  The existing
    // ResizeObserver (observeResize) has a same-size guard so there is no
    // conflict if both paths run.
    const inst = this;
    requestAnimationFrame(() => {
      const measured = inst._measureRenderedSize();
      const w = measured.w;
      const h = measured.h;
      if (w <= 0 || h <= 0) return;
      if (w === canvas.width && h === canvas.height) return;

      canvas.width = w;
      canvas.height = h;
      inst.origWidth = w;
      inst.origHeight = h;

      if (inst.port) {
        inst.port.postMessage({ type: "resize", width: w, height: h, ...inst.collectViewInfo() });
      }
    });
  }

  /**
   * Start (or restart) this Flash instance by opening a new port to the
   * background service worker and wiring up message handlers.
   */
  async start() {
    const port = chrome.runtime.connect({ name: "flash-instance" });
    this.port = port;
    FlashInstance._activePort = port;

    const inst = this;

    // Resolve the JS URL rewriter callback if the attribute was set.
    if (this._urlRewriterAttr) {
      try {
        const resp = sendToPageScript({
          op: "executeScript",
          script: "(" + this._urlRewriterAttr + ")",
        });
        if (resp && resp.value && resp.value.type === "object") {
          this._urlRewriterCallbackId = resp.value.v;
          console.log("[Flash Player] URL rewriter callback resolved, object ID:", resp.value.v);
        }
      } catch (e) {
        console.warn("[Flash Player] Failed to resolve data-url-rewriter:", e);
      }
    }

    console.log("[Flash Player] Starting instance with", _flashSettings);

    const startMsg = {
      type: "start",
      instanceId: this.instanceId,
      url: this.swfUrl,
      args: this.didCreateArgs,
      width: this.origWidth,
      height: this.origHeight,
      language: navigator.language || "en-US",
      settings: _flashSettings,
      ...this.collectViewInfo(),
    };
    if (this._urlRewriterCallbackId != null) {
      startMsg.urlRewriterCallbackId = this._urlRewriterCallbackId;
    }
    port.postMessage(startMsg);

    // ---- Handle messages from the native host (via background) ----
    port.onMessage.addListener((msg) => {
      // Extension-originated error (e.g. host disconnect).
      if (msg.error) {
        console.error("[Flash Player]", msg.error);
        if (!FlashInstance._navigatingAway) {
          if (msg.notInstalled) {
            inst.showNotInstalledOverlay();
          } else {
            inst.showCrashOverlay();
          }
        }
        return;
      }
      if (msg.b64) {
        handleBinaryMessage(inst, msg.b64);
      }
    });

    port.onDisconnect.addListener(() => {
      console.warn("[Flash Player] Native host disconnected for instance", inst.instanceId);
      if (!FlashInstance._navigatingAway && !FlashInstance._closingForVersionMismatch) {
        inst.showCrashOverlay();
      }
    });

    // On restart, clone the canvas to strip old event listeners.
    if (this._started) {
      const oldCanvas = this.canvas;
      const freshCanvas = oldCanvas.cloneNode(false);
      freshCanvas.getContext("2d");
      this.canvas = freshCanvas;
      this.ctx = freshCanvas.getContext("2d");
      if (oldCanvas.parentNode) {
        oldCanvas.parentNode.replaceChild(freshCanvas, oldCanvas);
      }
      // Re-observe the new canvas for resize.
      if (this._resizeObserver) {
        this._resizeObserver.disconnect();
        this._resizeObserver.observe(freshCanvas);
      }
      this._bindInputEvents(freshCanvas, port);
    } else {
      this._started = true;
      this._bindInputEvents(this.canvas, port);
    }
  }

  /**
   * Remove the crash overlay and restart the native host for this instance.
   */
  restart() {
    const overlay = this.container.querySelector(".flash-crash-overlay");
    if (overlay) overlay.remove();
    const vOverlay = this.container.querySelector(".flash-version-mismatch-overlay");
    if (vOverlay) vOverlay.remove();
    this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    this.start();
  }

  /**
   * Tear down this instance's connection and observers.
   * The instance remains in the registry for potential bfcache restart.
   */
  destroy() {
    if (this.port) {
      try { this.port.disconnect(); } catch { /* already gone */ }
      if (FlashInstance._activePort === this.port) {
        FlashInstance._activePort = null;
      }
      this.port = null;
    }
    // Remove overlays.
    for (const cls of [".flash-crash-overlay", ".flash-notinstalled-overlay", ".flash-version-mismatch-overlay"]) {
      const overlay = this.container.querySelector(cls);
      if (overlay) overlay.remove();
    }
    // Disconnect observers.
    if (this._mutationObserver) { this._mutationObserver.disconnect(); this._mutationObserver = null; }
    if (this._resizeObserver) { this._resizeObserver.disconnect(); this._resizeObserver = null; }
    // Stop debug stats.
    if (this._debugStats) { this._debugStats.stop(); this._debugStats = null; }
    // Close associated audio/video streams.
    for (const sid of this._audioStreamIds) audioClose(sid);
    this._audioStreamIds.clear();
    for (const sid of this._audioInputStreamIds) audioInputClose(sid);
    this._audioInputStreamIds.clear();
    for (const sid of this._videoCaptureStreamIds) videoCaptureClose(sid);
    this._videoCaptureStreamIds.clear();
  }

  /**
   * Create the debug stats DOM panel and attach it after the container.
   */
  _createDebugStatsPanel(anchorEl) {
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
      maxWidth: this.container.style.width,
      boxSizing: "border-box",
      overflowX: "auto",
      userSelect: "text",
    });
    panel.innerHTML = "<i>Collecting statistics…</i>";

    const insertAfter = anchorEl || this.container;
    if (insertAfter.nextSibling) {
      insertAfter.parentNode.insertBefore(panel, insertAfter.nextSibling);
    } else if (insertAfter.parentNode) {
      insertAfter.parentNode.appendChild(panel);
    }

    const stats = new DebugStats();
    stats.start(panel);
    this._debugStats = stats;
  }

  // ---- Shared overlay infrastructure ----

  /**
   * Create a styled overlay on top of the canvas with shared structure
   * (background, grid pattern, icon, title, subtitle) and an optional
   * action element (button or link).
   *
   * @param {Object} opts
   * @param {string} opts.className      - CSS class for the overlay element.
   * @param {string[]} [opts.removeBefore] - Selectors of overlays to remove first.
   * @param {string} opts.title          - Main heading text.
   * @param {string} opts.subtitle       - Secondary description (plain text or HTML).
   * @param {boolean} [opts.subtitleHtml] - If true, subtitle is set via innerHTML.
   * @param {Object} [opts.iconStyle]    - Extra styles merged onto the icon element.
   * @param {Element} [opts.actionEl]    - An action element (button/link) to append.
   * @returns {Element|null} The overlay element, or null if it already exists.
   */
  _showOverlay({ className, removeBefore, title, subtitle, subtitleHtml, iconStyle, actionEl }) {
    const { container } = this;

    if (container.querySelector(`.${className}`)) return null;

    if (removeBefore) {
      for (const sel of removeBefore) {
        const el = container.querySelector(sel);
        if (el) el.remove();
      }
    }

    const overlay = document.createElement("div");
    overlay.className = className;
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
      overflow: "hidden",
      userSelect: "none",
      padding: "24px",
    });

    // Subtle grid-line pattern.
    const pattern = document.createElement("div");
    Object.assign(pattern.style, {
      position: "absolute",
      inset: "0",
      backgroundImage:
        "linear-gradient(rgba(255,255,255,.03) 1px, transparent 1px), " +
        "linear-gradient(90deg, rgba(255,255,255,.03) 1px, transparent 1px)",
      backgroundSize: "40px 40px",
      pointerEvents: "none",
    });
    overlay.appendChild(pattern);

    // Warning icon.
    const icon = document.createElement("div");
    icon.textContent = "\u26A0";
    Object.assign(icon.style, {
      fontSize: "48px",
      marginBottom: "12px",
      position: "relative",
      zIndex: "1",
    }, iconStyle || {});
    overlay.appendChild(icon);

    // Title.
    const titleEl = document.createElement("div");
    titleEl.textContent = title;
    Object.assign(titleEl.style, {
      fontSize: "20px",
      fontWeight: "700",
      letterSpacing: "0.3px",
      marginBottom: "10px",
      textShadow: "0 2px 12px rgba(0,0,0,0.5)",
      position: "relative",
      zIndex: "1",
    });
    overlay.appendChild(titleEl);

    // Subtitle.
    const subtitleEl = document.createElement("div");
    if (subtitleHtml) {
      subtitleEl.innerHTML = subtitle;
    } else {
      subtitleEl.textContent = subtitle;
    }
    Object.assign(subtitleEl.style, {
      fontSize: "13px",
      opacity: "0.8",
      maxWidth: "420px",
      lineHeight: "1.5",
      marginBottom: "20px",
      position: "relative",
      zIndex: "1",
    });
    overlay.appendChild(subtitleEl);

    // Action element.
    if (actionEl) {
      overlay.appendChild(actionEl);
    }

    container.appendChild(overlay);
    return overlay;
  }

  /**
   * Create a styled button or link element for use in overlays.
   * @param {Object} opts
   * @param {string} opts.text         - Button label text.
   * @param {string} [opts.href]       - If set, creates an <a> link instead of a <button>.
   * @param {Function} [opts.onClick]  - Click handler (for buttons).
   * @returns {Element}
   */
  static _createOverlayAction({ text, href, onClick }) {
    const isLink = !!href;
    const btn = document.createElement(isLink ? "a" : "button");
    if (isLink) {
      btn.href = href;
      btn.target = "_blank";
      btn.rel = "noopener noreferrer";
      btn.style.textDecoration = "none";
      btn.style.display = "inline-block";
    }
    btn.textContent = text;
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
      btn.style.background = "linear-gradient(135deg, #8ed0f4, #c0e5fb)";
    });
    btn.addEventListener("mouseleave", () => {
      btn.style.background = "linear-gradient(135deg, #7ec8f2, #b4dff7)";
    });
    if (onClick) {
      btn.addEventListener("click", onClick);
    }
    return btn;
  }

  /**
   * Show a styled crash overlay on top of the canvas.
   */
  showCrashOverlay() {
    // Inject keyframe animation (once).
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

    const inst = this;
    this._showOverlay({
      className: "flash-crash-overlay",
      title: "Oops! Flash Player has crashed!",
      subtitle: "The native host process ended unexpectedly.",
      iconStyle: {
        filter: "drop-shadow(0 2px 8px rgba(0,0,0,0.4))",
        animation: "flash-crash-pulse 2s ease-in-out infinite",
      },
      actionEl: FlashInstance._createOverlayAction({
        text: "\u21BB  Restart",
        onClick: () => inst.restart(),
      }),
    });
  }

  /**
   * Show an overlay explaining that Flash Player (the native host) is missing.
   */
  showNotInstalledOverlay() {
    this._showOverlay({
      className: "flash-notinstalled-overlay",
      removeBefore: [".flash-crash-overlay"],
      title: "Clean Flash Player not found",
      subtitle:
        "The Clean Flash Player browser extension is working,<br />" +
        "but Flash Player is not yet installed on your system.",
      subtitleHtml: true,
      actionEl: FlashInstance._createOverlayAction({
        text: "\u2B07  Download Installer",
        href: "https://gitlab.com/cleanflash/installer/-/releases",
      }),
    });
  }

  /**
   * Show an overlay when the native host version does not match the extension.
   * @param {string} hostVersion - The version string reported by the native host.
   */
  showVersionMismatchOverlay(hostVersion) {
    this._showOverlay({
      className: "flash-version-mismatch-overlay",
      removeBefore: [".flash-crash-overlay"],
      title: "Flash Player version mismatch",
      subtitle:
        "The installed native Clean Flash Player version (v" + hostVersion + ") does not match " +
        "the browser extension version (v" + EXTENSION_VERSION + ").<br /><br />" +
        "Please download the latest installer to update.<br />Ensure your extensions are also updated.",
      subtitleHtml: true,
      actionEl: FlashInstance._createOverlayAction({
        text: "\u2B07  Download Installer",
        href: "https://gitlab.com/cleanflash/installer/-/releases",
      }),
    });
  }

  /**
   * Bind mouse, keyboard, wheel, composition, and focus events on a canvas.
   * The port parameter is captured by closure so that on restart, the
   * fresh canvas's handlers use the new port.
   */
  _bindInputEvents(canvas, port) {
    const inst = this;

    // Use pointer capture so that mouseup fires on the canvas even when the
    // pointer is released outside the canvas, outside an iframe, or outside
    // the browser window.  This matches how the real PPAPI Flash plugin
    // receives mouse-release events.
    canvas.addEventListener("pointerdown", (e) => {
      canvas.setPointerCapture(e.pointerId);
    });

    // Forward mousedown events that occur outside the canvas while it is
    // focused, so Flash games can detect "click-away" and close their own
    // internal context menus / popups.  Use the window (not document)
    // capturing phase so this fires before any page-level listeners.
    window.addEventListener("mousedown", (e) => {
      if (e.target === canvas || canvas.contains(e.target)) return;
      if (inst.contextMenuOpen) return;
      // Only forward if this canvas was the focused element.
      if (document.activeElement !== canvas) return;
      const pos = canvasPos(canvas, e, inst);
      port.postMessage({
        type: "mousedown",
        x: pos.x,
        y: pos.y,
        button: mapButton(e),
        modifiers: getModifiers(e),
      });
    }, true);

    canvas.addEventListener("mousedown", (e) => {
      e.preventDefault();
      if (inst.contextMenuOpen) return;
      canvas.focus();
      const pos = canvasPos(canvas, e, inst);
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
      if (inst.contextMenuOpen) return;
      const pos = canvasPos(canvas, e, inst);
      port.postMessage({
        type: "mouseup",
        x: pos.x,
        y: pos.y,
        button: mapButton(e),
        modifiers: getModifiers(e),
      });
    });

    // Throttle mousemove to one message per animation frame.
    let _pendingMove = null;
    let _moveTimer = 0;
    function _flushMove() {
      if (!_pendingMove) return;
      if (inst.contextMenuOpen) { _pendingMove = null; return; }
      const pos = canvasPos(canvas, _pendingMove, inst);
      try {
        port.postMessage({
          type: "mousemove",
          x: pos.x,
          y: pos.y,
          modifiers: getModifiers(_pendingMove),
        });
      } catch (_) {
        // Extension context invalidated (e.g. extension updated/reloaded).
        _pendingMove = null;
        return;
      }
      _pendingMove = null;
      if (_moveTimer) { clearTimeout(_moveTimer); _moveTimer = 0; }
    }
    canvas._flushPendingMove = _flushMove;
    canvas.addEventListener("mousemove", (e) => {
      if (inst.contextMenuOpen) return;
      _pendingMove = e;
      if (!_moveTimer) {
        _moveTimer = setTimeout(() => { _moveTimer = 0; _flushMove(); }, 50);
      }
    });

    canvas.addEventListener("mouseenter", () => {
      if (inst.contextMenuOpen) return;
      port.postMessage({ type: "mouseenter" });
    });

    canvas.addEventListener("mouseleave", () => {
      if (inst.contextMenuOpen) return;
      port.postMessage({ type: "mouseleave" });
    });

    canvas.addEventListener("wheel", (e) => {
      e.preventDefault();
      if (inst.contextMenuOpen) return;
      port.postMessage({
        type: "wheel",
        deltaX: -e.deltaX,
        deltaY: -e.deltaY,
        modifiers: getModifiers(e),
      });
    }, { passive: false });

    canvas.addEventListener("keydown", (e) => {
      e.preventDefault();
      if (inst.contextMenuOpen) return;
      port.postMessage({
        type: "rawkeydown",
        keyCode: e.keyCode,
        code: e.code,
        modifiers: getModifiers(e),
      });

      if (!e.ctrlKey && !e.metaKey) {
        if (e.key.length === 1) {
          port.postMessage({
            type: "char",
            keyCode: e.key.charCodeAt(0),
            text: e.key,
            code: e.code,
            modifiers: getModifiers(e),
          });
        } else if (!e.altKey) {
          let charCode = 0, charText = "";
          switch (e.key) {
            case "Enter": charCode = 13; charText = "\r"; break;
            case "Tab": charCode = 9; charText = "\t"; break;
            case "Backspace": charCode = 8; charText = ""; break;
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
      if (inst.contextMenuOpen) return;
      port.postMessage({
        type: "keyup",
        keyCode: e.keyCode,
        code: e.code,
        modifiers: getModifiers(e),
      });
    });

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

    canvas.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      const pos = canvasPos(canvas, e, inst);
      port.postMessage({
        type: "contextmenu",
        x: pos.x,
        y: pos.y,
        button: mapButton(e),
        modifiers: getModifiers(e),
      });
    });
  }
}

// ---------------------------------------------------------------------------
// Browser view-change listeners (visibility, scroll, fullscreen)
// ---------------------------------------------------------------------------

// Page visibility changes (tab switch, minimize).
document.addEventListener("visibilitychange", () => FlashInstance.broadcastViewUpdate());

// Scroll position changes - throttled to avoid flooding the native host.
window.addEventListener("scroll", () => {
  FlashInstance.scheduleBroadcastViewUpdate(100);
}, { passive: true });

// Viewport changes (window resize and browser/page zoom).
window.addEventListener("resize", () => {
  FlashInstance.scheduleBroadcastViewUpdate(50);
}, { passive: true });

// Visual viewport changes are a strong signal for zoom on mobile and desktop.
// Observed separately from window.resize because VisualViewport fires when
// pinch-zoom or on-screen keyboard changes the visual area without changing
// the layout viewport.
if (window.visualViewport) {
  window.visualViewport.addEventListener("resize", () => {
    FlashInstance.scheduleBroadcastViewUpdate(50);
  }, { passive: true });
  window.visualViewport.addEventListener("scroll", () => {
    FlashInstance.scheduleBroadcastViewUpdate(50);
  }, { passive: true });
}

// Fallback watcher for zoom implementations that change devicePixelRatio
// without reliably dispatching resize events.  There is no native DPR-change
// event, so we poll every 250 ms with an epsilon comparison.
let lastDevicePixelRatio = window.devicePixelRatio || 1.0;
setInterval(() => {
  const dpr = window.devicePixelRatio || 1.0;
  if (Math.abs(dpr - lastDevicePixelRatio) > 0.0001) {
    lastDevicePixelRatio = dpr;
    FlashInstance.scheduleBroadcastViewUpdate(0);
  }
}, 250);

// Fullscreen changes - resize canvas to fill screen on enter, restore on exit.
document.addEventListener("fullscreenchange", () => FlashInstance.handleFullscreenChange());
document.addEventListener("webkitfullscreenchange", () => FlashInstance.handleFullscreenChange());

// Pointer lock changes - notify the native host so Flash knows if the
// cursor was locked or unlocked (e.g. user pressed Escape).
document.addEventListener("pointerlockchange", () => FlashInstance.handlePointerLockChange());

// ---------------------------------------------------------------------------
// Binary message decoding
// ---------------------------------------------------------------------------

// Message type tags (must match protocol.rs).
const TAG_FRAME = 0x01;
const TAG_STATE = 0x02;
const TAG_CURSOR = 0x03;
const TAG_ERROR = 0x04;
const TAG_SCRIPT = 0x10;
const TAG_NAVIGATE = 0x05;
const TAG_AUDIO_INIT = 0x20;
const TAG_AUDIO_SAMPLES = 0x21;
const TAG_AUDIO_START = 0x22;
const TAG_AUDIO_STOP = 0x23;
const TAG_AUDIO_CLOSE = 0x24;
const TAG_AUDIO_INPUT_OPEN = 0x30;
const TAG_AUDIO_INPUT_START = 0x31;
const TAG_AUDIO_INPUT_STOP = 0x32;
const TAG_AUDIO_INPUT_CLOSE = 0x33;
const TAG_CONTEXT_MENU = 0x40;
const TAG_PRINT = 0x50;
const TAG_VIDEO_CAPTURE_OPEN = 0x60;
const TAG_VIDEO_CAPTURE_START = 0x61;
const TAG_VIDEO_CAPTURE_STOP = 0x62;
const TAG_VIDEO_CAPTURE_CLOSE = 0x63;
const TAG_VERSION = 0x70;

function tagHex(tag) {
  return `0x${tag.toString(16).padStart(2, "0")}`;
}

/**
 * Log a host-originated event to the console (only when LOG_HOST_EVENTS is on).
 */
function logHostEvent(tag, payloadBytes) {
  if (!LOG_HOST_EVENTS) return;
  const name = TAG_NAMES[tag] || `UNKNOWN(${tagHex(tag)})`;
  console.log("[Flash Player] Host event:", name, `tag=${tagHex(tag)}`, `bytes=${payloadBytes}`);
}

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

/** Pre-computed base-64 character → 6-bit value lookup table. */
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
 * Fallback : pure-JS lookup-table decoder.
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

/** Shared TextDecoder - avoids allocating a new one on every message. */
const _textDecoder = new TextDecoder();

// ---------------------------------------------------------------------------
// Web Audio playback - receives PCM from native host, plays via AudioContext
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
/** Maximum scheduling headroom (seconds) - caps latency. */
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
 *
 * Uses an adaptive jitter buffer: tracks the variance of inter-arrival
 * times and keeps enough scheduling headroom to absorb spikes without
 * audible gaps, while keeping latency as low as possible.
 *
 * @param {number} streamId - The stream to write to.
 * @param {Uint8Array} pcmBytes - Interleaved stereo i16 LE samples.
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
    left[i] = dv.getInt16(i * 4, true) / 32768.0;
    right[i] = dv.getInt16(i * 4 + 2, true) / 32768.0;
  }

  const source = ctx.createBufferSource();
  source.buffer = buffer;
  source.connect(ctx.destination);

  const now = ctx.currentTime;
  const ahead = stream.targetAhead;

  if (stream.nextTime <= now) {
    // First buffer after init, resume, or an underrun - schedule from
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
    stream.ctx.close().catch(() => { });
    audioStreams.delete(streamId);
    console.log("[Flash Player] Audio stream closed:", streamId);
  }
}

// ---------------------------------------------------------------------------
// Web Audio input capture - uses getUserMedia + ScriptProcessorNode to
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

        // Convert float32 [-1, 1] to i16 LE bytes.
        const input = e.inputBuffer.getChannelData(0);
        const i16 = new Int16Array(input.length);
        for (let i = 0; i < input.length; i++) {
          const s = Math.max(-1, Math.min(1, input[i]));
          i16[i] = s < 0 ? s * 0x8000 : s * 0x7FFF;
        }

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
  if (stream.ctx) stream.ctx.close().catch(() => { });
  audioInputStreams.delete(streamId);
  console.log("[Flash Player] Audio input stream closed:", streamId);
}

// ---------------------------------------------------------------------------
// Web Video Capture - uses getUserMedia({ video }) + canvas to capture
// video frames and send I420 YUV data back to the host.
// ---------------------------------------------------------------------------

/**
 * Active video capture streams.
 * stream_id -> {
 *   video,        // HTMLVideoElement (hidden)
 *   canvas,       // OffscreenCanvas or HTMLCanvasElement for frame extraction
 *   canvasCtx,    // 2D rendering context
 *   mediaStream,  // MediaStream from getUserMedia
 *   port,         // native messaging port for sending data back
 *   width,        // requested width
 *   height,       // requested height
 *   fps,          // requested frames per second
 *   capturing,    // boolean
 *   intervalId,   // setInterval ID for frame capture loop
 *   ready,        // Promise that resolves when the stream is set up
 *   pendingStart, // true if videoCaptureStart was called before ready
 * }
 */
const videoCaptureStreams = new Map();

/**
 * Convert RGBA pixel data to planar I420 (YUV 4:2:0).
 * @param {Uint8ClampedArray} rgba - RGBA pixel data (width * height * 4 bytes).
 * @param {number} width
 * @param {number} height
 * @returns {Uint8Array} I420 data (width * height * 3 / 2 bytes).
 */
function rgbaToI420(rgba, width, height) {
  const ySize = width * height;
  const uvSize = (width >> 1) * (height >> 1);
  const i420 = new Uint8Array(ySize + uvSize * 2);

  let yIdx = 0;
  let uIdx = ySize;
  let vIdx = ySize + uvSize;

  for (let row = 0; row < height; row++) {
    for (let col = 0; col < width; col++) {
      const idx = (row * width + col) * 4;
      const r = rgba[idx];
      const g = rgba[idx + 1];
      const b = rgba[idx + 2];

      // ITU-R BT.601 conversion.
      i420[yIdx++] = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;

      // Subsample U and V for every 2x2 block.
      if ((row & 1) === 0 && (col & 1) === 0) {
        i420[uIdx++] = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
        i420[vIdx++] = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
      }
    }
  }

  return i420;
}

/**
 * Open a new video capture stream.
 * Requests camera permission and sets up the video element + canvas,
 * but does not start sending frames until videoCaptureStart() is called.
 */
function videoCaptureOpen(streamId, width, height, fps, port) {
  // Close any existing stream with the same id.
  videoCaptureClose(streamId);

  const streamState = {
    video: null,
    canvas: null,
    canvasCtx: null,
    mediaStream: null,
    port,
    width,
    height,
    fps,
    capturing: false,
    intervalId: null,
    ready: null,
    pendingStart: false,
  };

  streamState.ready = (async () => {
    try {
      console.log("[Flash Player] Video capture requesting getUserMedia:", streamId,
        width + "x" + height, "@", fps, "fps");
      const mediaStream = await navigator.mediaDevices.getUserMedia({
        video: {
          width: { ideal: width },
          height: { ideal: height },
          frameRate: { ideal: fps },
        },
      });

      // Create a hidden video element to receive the camera stream.
      const video = document.createElement("video");
      video.srcObject = mediaStream;
      video.width = width;
      video.height = height;
      video.muted = true;
      video.playsInline = true;
      video.style.position = "fixed";
      video.style.left = "-9999px";
      video.style.top = "-9999px";
      document.body.appendChild(video);
      await video.play();

      // Wait until the video has decoded at least one frame so that
      // drawImage produces actual pixel data instead of a blank frame.
      if (video.readyState < 2) {
        await new Promise((resolve) => {
          video.addEventListener("loadeddata", resolve, { once: true });
        });
      }

      // Create a canvas for extracting pixel data.
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const canvasCtx = canvas.getContext("2d", { willReadFrequently: true });

      streamState.video = video;
      streamState.canvas = canvas;
      streamState.canvasCtx = canvasCtx;
      streamState.mediaStream = mediaStream;

      // Always auto-start capture once the stream is ready.
      // The Rust side sends Start shortly after Open, but due to the async
      // getUserMedia + video.play() pipeline the Start message may arrive
      // before or after we reach this point.  Starting eagerly avoids a
      // race where the Start message is processed while video is still null
      // and gets queued, or arrives before the stream entry exists.
      streamState.capturing = true;
      streamState.pendingStart = false;
      startVideoCaptureLoop(streamId, streamState);
      console.log("[Flash Player] Video capture stream opened + started:", streamId,
        width + "x" + height, "@", fps, "fps");
    } catch (e) {
      console.error("[Flash Player] Failed to open video capture:", e);
    }
  })();

  videoCaptureStreams.set(streamId, streamState);
}

/**
 * Start the periodic frame capture loop for a video stream.
 * Grabs frames from the video element at the requested FPS, converts
 * to I420, and sends them as base64 to the native host.
 */
function startVideoCaptureLoop(streamId, streamState) {
  if (streamState.intervalId !== null) return;

  const intervalMs = Math.max(1, Math.round(1000 / streamState.fps));
  let framesSent = 0;

  streamState.intervalId = setInterval(() => {
    if (!streamState.capturing || !streamState.video || !streamState.canvasCtx) return;

    const { video, canvasCtx, canvas, width, height, port: p } = streamState;

    // Skip frames until the video has decoded actual pixel data.
    if (video.readyState < 2) return;

    // Draw current video frame to canvas.
    canvasCtx.drawImage(video, 0, 0, width, height);
    const imageData = canvasCtx.getImageData(0, 0, width, height);

    // Convert RGBA to I420.
    const i420 = rgbaToI420(imageData.data, width, height);

    // Encode as base64 and send to the host.
    const b64 = uint8ToB64(i420);

    p.postMessage({
      type: "videoCaptureData",
      streamId,
      width,
      height,
      data: b64,
    });

    framesSent++;
    if (framesSent === 1) {
      console.log("[Flash Player] Video capture first frame sent:", streamId,
        width + "x" + height, "I420 bytes:", i420.length);
    } else if (framesSent % 300 === 0) {
      console.log("[Flash Player] Video capture frames sent:", framesSent, "stream:", streamId);
    }
  }, intervalMs);
}

/**
 * Start capturing on a video capture stream.
 */
function videoCaptureStart(streamId) {
  const stream = videoCaptureStreams.get(streamId);
  if (!stream) {
    console.warn("[Flash Player] videoCaptureStart: stream not found:", streamId);
    return;
  }

  if (!stream.video) {
    stream.pendingStart = true;
    console.log("[Flash Player] Video capture start queued (still opening):", streamId);
    return;
  }

  if (stream.capturing) {
    console.log("[Flash Player] Video capture already running:", streamId);
    return;
  }

  stream.capturing = true;
  startVideoCaptureLoop(streamId, stream);
  console.log("[Flash Player] Video capture started:", streamId);
}

/**
 * Stop capturing on a video capture stream.
 */
function videoCaptureStop(streamId) {
  const stream = videoCaptureStreams.get(streamId);
  if (!stream) return;
  stream.capturing = false;
  stream.pendingStart = false;
  if (stream.intervalId !== null) {
    clearInterval(stream.intervalId);
    stream.intervalId = null;
  }
  console.log("[Flash Player] Video capture stopped:", streamId);
}

/**
 * Close and release a video capture stream.
 */
function videoCaptureClose(streamId) {
  const stream = videoCaptureStreams.get(streamId);
  if (!stream) return;
  stream.capturing = false;
  stream.pendingStart = false;
  if (stream.intervalId !== null) {
    clearInterval(stream.intervalId);
    stream.intervalId = null;
  }
  if (stream.video) {
    stream.video.pause();
    stream.video.srcObject = null;
    if (stream.video.parentNode) stream.video.parentNode.removeChild(stream.video);
  }
  if (stream.mediaStream) stream.mediaStream.getTracks().forEach(t => t.stop());
  videoCaptureStreams.delete(streamId);
  console.log("[Flash Player] Video capture stream closed:", streamId);
}

// ---------------------------------------------------------------------------
// Flash context menu  (TAG_CONTEXT_MENU = 0x40)
// ---------------------------------------------------------------------------

/**
 * Show a Flash context menu at the given position on top of the canvas.
 *
 * @param {Array} items - Menu item tree from the host.
 * @param {number} x - X position in plugin coordinates.
 * @param {number} y - Y position in plugin coordinates.
 * @param {HTMLCanvasElement} canvas - The Flash canvas element.
 * @param {Port} port - Native messaging port to send menuResponse back.
 */
function showFlashContextMenu(items, x, y, canvas, port) {
  // Remove any existing Flash context menu.
  removeFlashContextMenu();

  // Mark the instance so input events are suppressed while the menu is open.
  const instId = canvas.getAttribute("data-flash-player");
  const inst = instId != null ? FlashInstance.getById(Number(instId)) : null;

  if (inst) inst.contextMenuOpen = true;

  const menu = document.createElement("div");
  menu.className = "flash-context-menu";
  Object.assign(menu.style, {
    position: "fixed",
    zIndex: "2147483647",
    background: "#f0f0f0",
    border: "1px solid #a0a0a0",
    borderRadius: "3px",
    boxShadow: "2px 2px 6px rgba(0,0,0,0.3)",
    padding: "2px 0",
    fontFamily: "'Segoe UI', Tahoma, Geneva, Verdana, sans-serif",
    fontSize: "12px",
    color: "#1a1a1a",
    minWidth: "160px",
    cursor: "default",
    userSelect: "none",
    textAlign: "left",
  });

  let responded = false;
  function sendResponse(selectedId) {
    if (responded) return;
    responded = true;
    if (inst) inst.contextMenuOpen = false;
    port.postMessage({ type: "menuResponse", selectedId: selectedId });
    removeFlashContextMenu();
  }

  /** Recursively build nested DOM menu items from the host's data structure. */
  function buildMenuItems(parentEl, itemList) {
    for (const item of itemList) {
      if (item.type === "separator") {
        const sep = document.createElement("div");
        Object.assign(sep.style, {
          height: "1px",
          background: "#c0c0c0",
          margin: "3px 0"
        });
        parentEl.appendChild(sep);
        continue;
      }

      const row = document.createElement("div");
      Object.assign(row.style, {
        padding: "4px 24px 4px 24px",
        position: "relative",
        whiteSpace: "nowrap",
        color: item.enabled ? "#1a1a1a" : "#a0a0a0",
        cursor: item.enabled ? "default" : "not-allowed",
      });

      // Checkbox indicator
      if (item.type === "checkbox" && item.checked) {
        const check = document.createElement("span");
        check.textContent = "\u2713";
        Object.assign(check.style, {
          position: "absolute",
          left: "6px",
        });
        row.appendChild(check);
      }

      const label = document.createElement("span");
      label.textContent = item.name || "";
      row.appendChild(label);

      // Submenu arrow
      if (item.type === "submenu") {
        const arrow = document.createElement("span");
        arrow.textContent = "\u25B6";
        Object.assign(arrow.style, {
          position: "absolute",
          right: "8px",
          fontSize: "10px",
        });
        row.appendChild(arrow);
      }

      if (item.enabled && item.type !== "submenu") {
        row.addEventListener("mouseenter", () => {
          row.style.background = "#0078d4";
          row.style.color = "#fff";
        });
        row.addEventListener("mouseleave", () => {
          row.style.background = "";
          row.style.color = "#1a1a1a";
        });
        row.addEventListener("click", (e) => {
          e.stopPropagation();
          sendResponse(item.id);
        });
      }

      // Submenu hover behavior
      if (item.type === "submenu" && item.submenu && item.submenu.length > 0) {
        const sub = document.createElement("div");
        sub.className = "flash-context-submenu";
        Object.assign(sub.style, {
          display: "none",
          position: "absolute",
          left: "100%",
          top: "0",
          background: "#f0f0f0",
          border: "1px solid #a0a0a0",
          borderRadius: "3px",
          boxShadow: "2px 2px 6px rgba(0,0,0,0.3)",
          padding: "2px 0",
          minWidth: "160px",
          zIndex: "2147483647",
        });
        buildMenuItems(sub, item.submenu);
        row.appendChild(sub);

        row.addEventListener("mouseenter", () => {
          sub.style.display = "block";
          row.style.background = "#0078d4";
          row.style.color = "#fff";
        });
        row.addEventListener("mouseleave", () => {
          sub.style.display = "none";
          row.style.background = "";
          row.style.color = item.enabled ? "#1a1a1a" : "#a0a0a0";
        });
      }

      parentEl.appendChild(row);
    }
  }

  buildMenuItems(menu, items);

  // ---- Inject "Clean Flash Settings" as the pen-ultimate entry ----
  {
    const sep = document.createElement("div");
    Object.assign(sep.style, {
      height: "1px",
      background: "#c0c0c0",
      margin: "3px 0",
    });

    const settingsRow = document.createElement("div");
    Object.assign(settingsRow.style, {
      padding: "4px 24px 4px 24px",
      position: "relative",
      whiteSpace: "nowrap",
      color: "#1a1a1a",
      cursor: "default",
    });
    const settingsLabel = document.createElement("span");
    settingsLabel.textContent = "Clean Flash Settings\u2026";
    settingsRow.appendChild(settingsLabel);

    settingsRow.addEventListener("mouseenter", () => {
      settingsRow.style.background = "#0078d4";
      settingsRow.style.color = "#fff";
    });
    settingsRow.addEventListener("mouseleave", () => {
      settingsRow.style.background = "";
      settingsRow.style.color = "#1a1a1a";
    });
    settingsRow.addEventListener("click", (e) => {
      e.stopPropagation();
      // Open the extension settings popup.
      chrome.runtime.sendMessage({ type: "openSettings" });
      sendResponse(null);
    });

    // Insert before the last child (pen-ultimate position).
    const lastChild = menu.lastElementChild;
    if (lastChild) {
      menu.insertBefore(sep, lastChild);
      menu.insertBefore(settingsRow, lastChild);
    } else {
      menu.appendChild(sep);
      menu.appendChild(settingsRow);
    }
  }

  // Position relative to the canvas.
  // Map Flash view coordinates (DIPs) to CSS pixels for positioning.
  // Use the logical Flash dimensions, not the canvas buffer size which
  // may be at a higher device resolution.
  const rect = canvas.getBoundingClientRect();
  const logicalW = (inst && inst.origWidth) || canvas.width;
  const logicalH = (inst && inst.origHeight) || canvas.height;
  const scaleX = rect.width / (logicalW || 1);
  const scaleY = rect.height / (logicalH || 1);
  let menuX = rect.left + x * scaleX;
  let menuY = rect.top + y * scaleY;

  document.body.appendChild(menu);

  // Adjust if the menu would overflow the viewport.
  const menuRect = menu.getBoundingClientRect();
  if (menuX + menuRect.width > window.innerWidth) {
    menuX = window.innerWidth - menuRect.width - 2;
  }
  if (menuY + menuRect.height > window.innerHeight) {
    menuY = window.innerHeight - menuRect.height - 2;
  }
  if (menuX < 0) menuX = 0;
  if (menuY < 0) menuY = 0;

  menu.style.left = menuX + "px";
  menu.style.top = menuY + "px";

  // Close menu on click outside or pressing Escape.
  function onDocClick(e) {
    if (!menu.contains(e.target)) {
      sendResponse(null);
    }
  }
  function onDocKeydown(e) {
    if (e.key === "Escape") {
      sendResponse(null);
    }
  }
  // Close menu if the browser window loses focus entirely.
  function onWindowBlur() {
    sendResponse(null);
  }

  // Defer event registration so the current right-click event doesn't
  // immediately close the menu.
  setTimeout(() => {
    window.addEventListener("mousedown", onDocClick, true);
    window.addEventListener("keydown", onDocKeydown, true);
    window.addEventListener("blur", onWindowBlur);
  }, 0);

  // Store cleanup reference so removeFlashContextMenu can tidy up.
  menu._flashCleanup = () => {
    window.removeEventListener("mousedown", onDocClick, true);
    window.removeEventListener("keydown", onDocKeydown, true);
    window.removeEventListener("blur", onWindowBlur);
  };
}

/**
 * Remove any existing Flash context menu from the DOM.
 */
function removeFlashContextMenu() {
  const existing = document.querySelectorAll(".flash-context-menu");
  for (const el of existing) {
    if (el._flashCleanup) el._flashCleanup();
    el.remove();
  }
}

// ---------------------------------------------------------------------------
// Binary message handler
// ---------------------------------------------------------------------------

/**
 * Handle a fully reassembled binary message (as a base64 string).
 * The binary payload is LZ4-compressed (with prepended uncompressed size).
 *
 * @param {FlashInstance} instance - The instance this message belongs to.
 * @param {string} b64 - Base64-encoded binary payload.
 */
function handleBinaryMessage(instance, b64) {
  const ctx = instance.ctx;
  const canvas = instance.canvas;
  const port = instance.port;

  const b64Len = b64.length;
  const t0 = FLASH_DEBUG ? performance.now() : 0;
  const compressed = b64ToUint8(b64);
  // LZ4 decompress (lz4_decompress is provided by lz4.js loaded before us).
  const bytes = lz4_decompress(compressed);
  const decodeMs = FLASH_DEBUG ? performance.now() - t0 : 0;
  if (bytes.length === 0) return;

  const tag = bytes[0];
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);

  // Log every host-originated event type as it arrives.
  logHostEvent(tag, bytes.length);

  // ---- Debug stats recording ----
  let _ds = null;
  if (FLASH_DEBUG) {
    _ds = instance._debugStats;
    if (_ds) {
      _ds.recordMessage(b64Len, bytes.length, tag, decodeMs);
      _ds.markProcessingStart();
    }
  }

  switch (tag) {
    case TAG_FRAME: {
      // Lazy-init the QOI decoder only when frame data actually arrives.
      if (!_ensureQoiDecoderReady()) break;

      // 1 byte tag + 7 x u32 header (28 bytes) + QOI-encoded RGBA pixels
      const x = readU32(dv, 1);
      const y = readU32(dv, 5);
      const width = readU32(dv, 9);
      const height = readU32(dv, 13);
      const frameW = readU32(dv, 17);
      const frameH = readU32(dv, 21);
      // stride at offset 25 - ignored for QOI
      const qoiOffset = 29; // 1 + 7*4
      const qoiLen = bytes.length - qoiOffset;

      // Resize canvas if frame dimensions changed.
      if (ctx.canvas.width !== frameW || ctx.canvas.height !== frameH) {
        ctx.canvas.width = frameW;
        ctx.canvas.height = frameH;
      }

      if (!_qoiDecode || !_qoiMemory) break;

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

      // Decode - returns pointer to [u32 width, u32 height, ...RGBA pixels].
      const ptr = _qoiDecode(qoiLen);
      const pixelLen = width * height * 4;
      const rgbaView = new Uint8ClampedArray(_qoiMemory.buffer, ptr + 8, pixelLen);

      const rt0 = FLASH_DEBUG ? performance.now() : 0;
      if (IS_FIREFOX) {
        // Encode as BMP blob and blit via createImageBitmap + drawImage,
        // completely avoiding typed-array compartment checks.
        const blob = _buildBmpBlob(rgbaView, width, height);
        const localCtx = ctx;
        const dx = x, dy = y;
        _ffRenderChain = _ffRenderChain.then(() =>
          createImageBitmap(blob)
        ).then(bitmap => {
          localCtx.drawImage(bitmap, dx, dy);
          bitmap.close();
        });
      } else {
        const imageData = new ImageData(rgbaView, width, height);
        ctx.putImageData(imageData, x, y);
      }
      if (FLASH_DEBUG && _ds) _ds.recordFrameRender(performance.now() - rt0);

      if (canvas._flushPendingMove) canvas._flushPendingMove();

      break;
    }

    case TAG_STATE: {
      // 1 byte tag + 1 byte code + u32 width + u32 height
      // State codes: 0=idle, 1=loading, 2=running, 3=error.
      // When the player reaches "running" (code 2) we immediately send
      // a viewUpdate so it has accurate viewport data.
      const code = bytes[1];
      const width = readU32(dv, 2);
      const height = readU32(dv, 6);
      const stateNames = ["idle", "loading", "running", "error"];
      const stateName = stateNames[code] || "unknown";
      if (code === 2) {
        console.log(`[Flash Player] State: ${stateName}, view: ${width}x${height}`);
        console.log(`[Flash Player] View info:`, instance.collectViewInfo());
        port.postMessage({ type: "viewUpdate", ...instance.collectViewInfo() });
      } else if (code === 3) {
        console.error("[Flash Player] State: error");
      }
      break;
    }

    case TAG_CURSOR: {
      // 1 byte tag + i32 cursor type (PP_CursorType_Dev enum).
      const cursor = readI32(dv, 1);
      canvas.style.cursor = ppCursorToCss(cursor);
      break;
    }

    case TAG_ERROR: {
      // 1 byte tag + u32 msg_len + UTF-8 error message.
      const msgLen = readU32(dv, 1);
      const msgBytes = bytes.subarray(5, 5 + msgLen);
      const message = _textDecoder.decode(msgBytes);
      console.error("[Flash Player]", message);
      break;
    }

    case TAG_SCRIPT: {
      // 1 byte tag + u32 json_len + UTF-8 JSON request from Flash scripting.
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
      // Flash navigation targets follow standard HTML window naming:
      // _blank, _self, _top, _parent, or a named window.
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
          // Named target - try window.open with that name.
          window.open(url, target);
        }
      } catch (e) {
        console.error("[Flash Player] Navigate failed:", e);
      }
      break;
    }

    // ---- Audio messages ----

    case TAG_AUDIO_INIT: {
      const streamId = readU32(dv, 1);
      const sampleRate = readU32(dv, 5);
      const frameCount = readU32(dv, 9);
      audioInit(streamId, sampleRate, frameCount);
      instance._audioStreamIds.add(streamId);
      break;
    }

    case TAG_AUDIO_SAMPLES: {
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
      instance._audioStreamIds.delete(streamId);
      break;
    }

    // ---- Audio input messages ----

    case TAG_AUDIO_INPUT_OPEN: {
      const streamId = readU32(dv, 1);
      const sampleRate = readU32(dv, 5);
      const frameCount = readU32(dv, 9);
      audioInputOpen(streamId, sampleRate, frameCount, port);
      instance._audioInputStreamIds.add(streamId);
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
      instance._audioInputStreamIds.delete(streamId);
      break;
    }

    // ---- Video capture messages ----

    case TAG_VIDEO_CAPTURE_OPEN: {
      const streamId = readU32(dv, 1);
      const width = readU32(dv, 5);
      const height = readU32(dv, 9);
      const fps = readU32(dv, 13);
      videoCaptureOpen(streamId, width, height, fps, port);
      instance._videoCaptureStreamIds.add(streamId);
      break;
    }

    case TAG_VIDEO_CAPTURE_START: {
      const streamId = readU32(dv, 1);
      videoCaptureStart(streamId);
      break;
    }

    case TAG_VIDEO_CAPTURE_STOP: {
      const streamId = readU32(dv, 1);
      videoCaptureStop(streamId);
      break;
    }

    case TAG_VIDEO_CAPTURE_CLOSE: {
      const streamId = readU32(dv, 1);
      videoCaptureClose(streamId);
      instance._videoCaptureStreamIds.delete(streamId);
      break;
    }

    case TAG_CONTEXT_MENU: {
      // 1 byte tag + u32 json_len + UTF-8 JSON with menu item tree.
      const jsonLen = readU32(dv, 1);
      const jsonBytes = bytes.subarray(5, 5 + jsonLen);
      const jsonStr = _textDecoder.decode(jsonBytes);
      try {
        const menuData = JSON.parse(jsonStr);
        showFlashContextMenu(menuData.items, menuData.x, menuData.y, canvas, port);
      } catch (e) {
        console.error("[Flash Player] Bad context menu data:", e, jsonStr);
        port.postMessage({ type: "menuResponse", selectedId: null });
      }
      break;
    }

    case TAG_PRINT: {
      // 1 byte tag, no payload - print the Flash canvas content only.
      // Use a hidden iframe to avoid popup-blocker restrictions (this
      // message arrives from native messaging, not a user gesture).
      console.log("[Flash Player] Print requested by Flash content");
      try {
        const dataUrl = canvas.toDataURL("image/png");
        const iframe = document.createElement("iframe");
        iframe.style.position = "fixed";
        iframe.style.left = "-9999px";
        iframe.style.top = "-9999px";
        iframe.style.width = canvas.width + "px";
        iframe.style.height = canvas.height + "px";
        iframe.style.border = "none";
        document.body.appendChild(iframe);

        const doc = iframe.contentDocument || iframe.contentWindow.document;
        doc.open();
        doc.write(
          "<!DOCTYPE html><html><head><title>Print Flash Content</title>" +
          "<style>@page { margin: 0; } body { margin: 0; }</style>" +
          "</head><body style='margin:0'>" +
          "<img src='" + dataUrl + "' style='max-width:100%;height:auto'>" +
          "</body></html>"
        );
        doc.close();

        const img = doc.querySelector("img");
        const doPrint = () => {
          try {
            iframe.contentWindow.focus();
            iframe.contentWindow.print();
          } catch (e) {
            console.error("[Flash Player] Print failed:", e);
          }
          setTimeout(() => { iframe.remove(); }, 5000);
        };

        if (img.complete) {
          doPrint();
        } else {
          img.onload = doPrint;
          img.onerror = () => {
            console.error("[Flash Player] Failed to load canvas image for printing");
            iframe.remove();
          };
        }
      } catch (e) {
        console.error("[Flash Player] Print error:", e);
      }
      break;
    }

    case TAG_VERSION: {
      // 1 byte tag + u32 version_len + UTF-8 version string
      const versionLen = readU32(dv, 1);
      const versionBytes = bytes.subarray(5, 5 + versionLen);
      const hostVersion = _textDecoder.decode(versionBytes);
      console.log(`[Flash Player] Host version: ${hostVersion}, extension version: ${EXTENSION_VERSION}`);
      if (hostVersion !== EXTENSION_VERSION) {
        console.warn(`[Flash Player] Version mismatch! Host=${hostVersion} Extension=${EXTENSION_VERSION}`);
        FlashInstance.closeAllForVersionMismatch(hostVersion);
      }
      break;
    }

    default:
      console.warn("[Flash Player] Unknown binary message tag:", tag);
  }

  if (FLASH_DEBUG && _ds) _ds.markProcessingEnd();
}

// ---------------------------------------------------------------------------
// JavaScript scripting bridge  (TAG_SCRIPT = 0x10)
//
// The actual JS execution runs in page-script.js (MAIN world).  This
// content script (ISOLATED world) proxies requests/responses through a
// shared hidden DOM element using synchronous dispatchEvent.
// ---------------------------------------------------------------------------

/** Element ID used for the hidden DOM element that bridges ISOLATED ↔ MAIN worlds. */
const COMM_ID = "__flash_player_comm__";
let _pageAsyncRequestId = 1;

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
 * Works because `dispatchEvent` is synchronous - the page script's
 * listener runs in the same call stack, writes the response attribute,
 * and then control returns here.
 */
function sendToPageScript(req) {
  const comm = getCommElement();
  comm.setAttribute("data-req", JSON.stringify(req));
  comm.setAttribute("data-resp", "");
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
 * Send an async scripting request to the MAIN-world page script and
 * resolve with the JSON response.
 */
function sendToPageScriptAsync(req, timeoutMs = 30000) {
  const comm = getCommElement();
  const requestId = String(_pageAsyncRequestId++);

  return new Promise((resolve) => {
    let settled = false;
    let timer = null;

    const finish = (resp) => {
      if (settled) return;
      settled = true;
      comm.removeEventListener("__flash_resp_async", onResp);
      if (timer) clearTimeout(timer);
      resolve(resp);
    };

    const onResp = () => {
      const respStr = comm.getAttribute("data-resp-async");
      if (!respStr) return;
      try {
        const payload = JSON.parse(respStr);
        if (!payload || payload.id !== requestId) return;
        finish(payload.resp || null);
      } catch {
        // Ignore malformed response payloads.
      }
    };

    comm.addEventListener("__flash_resp_async", onResp);
    timer = setTimeout(() => {
      finish({ error: "page script async timeout" });
    }, timeoutMs);

    comm.setAttribute("data-resp-async", "");
    comm.setAttribute(
      "data-req-async",
      JSON.stringify({ id: requestId, req })
    );
    comm.dispatchEvent(new CustomEvent("__flash_req_async"));
  });
}

/**
 * Handle a scripting request from the native host.
 * Routes the request to the appropriate handler (clipboard, cookies,
 * document queries, etc.) or forwards it to the MAIN-world page script
 * for JavaScript evaluation, and sends the response back.
 *
 * @param {Object} req - The parsed JSON request from the host.
 * @param {Port} port - Native messaging port for sending the response.
 */
async function handleScriptRequest(req, port) {
  const id = req.id;
  const op = req.op;

  if (op === "getDocumentUrl") {
    sendScriptResponse(port, id, { type: "string", v: window.location.href });
    return;
  }

  if (op === "getDocumentBaseUrl") {
    sendScriptResponse(port, id, { type: "string", v: document.baseURI || window.location.href });
    return;
  }

  if (op === "getPluginUrl") {
    const inst = FlashInstance.getByPort(port);
    if (inst && typeof inst.swfUrl === "string" && inst.swfUrl.length > 0) {
      sendScriptResponse(port, id, { type: "string", v: inst.swfUrl });
    } else {
      sendScriptResponse(port, id, { type: "undefined" });
    }
    return;
  }

  // ---------------------------------------------------------------
  // Clipboard
  // ---------------------------------------------------------------

  if (op === "clipboardRead") {
    const fmt = req.format;
    if (fmt === "plaintext" && navigator.clipboard && navigator.clipboard.readText) {
      try {
        const text = await navigator.clipboard.readText();
        if (text != null) {
          sendScriptResponse(port, id, { type: "string", v: text });
          return;
        }
      } catch (_) { /* permission denied or not focused - fall through */ }
    }
    if (fmt === "html" && navigator.clipboard && navigator.clipboard.read) {
      try {
        const items = await navigator.clipboard.read();
        for (const item of items) {
          if (item.types.includes("text/html")) {
            const blob = await item.getType("text/html");
            const html = await blob.text();
            if (html) {
              sendScriptResponse(port, id, { type: "string", v: html });
              return;
            }
          }
        }
      } catch (_) { /* fall through */ }
    }
    const resp = sendToPageScript(req);
    if (resp && resp.value) {
      sendScriptResponse(port, id, resp.value);
    } else {
      sendScriptResponse(port, id, { type: "null" });
    }
    return;
  }

  if (op === "clipboardWrite") {
    const resp = sendToPageScript(req);
    const text = req.plaintext || req.html || "";
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).catch(() => { });
    }
    if (resp && resp.value) {
      sendScriptResponse(port, id, resp.value);
    } else {
      sendScriptResponse(port, id, { type: "bool", v: true });
    }
    return;
  }

  if (op === "clipboardIsAvailable") {
    const fmt = req.format;
    if (fmt === "rtf") {
      sendScriptResponse(port, id, { type: "bool", v: false });
      return;
    }
    if (fmt === "plaintext" && navigator.clipboard && navigator.clipboard.readText) {
      try {
        const text = await navigator.clipboard.readText();
        if (text != null && text.length > 0) {
          sendScriptResponse(port, id, { type: "bool", v: true });
          return;
        }
      } catch (_) { /* fall through */ }
    }
    const resp = sendToPageScript(req);
    if (resp && resp.value) {
      sendScriptResponse(port, id, resp.value);
    } else {
      sendScriptResponse(port, id, { type: "bool", v: false });
    }
    return;
  }

  // ---------------------------------------------------------------
  // Cookies: use chrome.cookies API (via background service worker)
  // to get/set cookies for arbitrary URLs.
  // ---------------------------------------------------------------

  if (op === "getCookiesForUrl") {
    const url = req.url;
    try {
      const resp = await chrome.runtime.sendMessage({
        type: "getCookies",
        url: url,
      });
      if (resp && typeof resp.cookies === "string") {
        sendScriptResponse(port, id, { type: "string", v: resp.cookies });
      } else {
        sendScriptResponse(port, id, { type: "string", v: "" });
      }
    } catch (e) {
      console.warn("[Flash Player] getCookiesForUrl error:", e);
      sendScriptResponse(port, id, { type: "string", v: "" });
    }
    return;
  }

  if (op === "setCookiesFromResponse") {
    const url = req.url;
    const cookies = req.cookies; // array of Set-Cookie header strings
    try {
      await chrome.runtime.sendMessage({
        type: "setCookies",
        url: url,
        cookies: cookies,
      });
    } catch (e) {
      console.warn("[Flash Player] setCookiesFromResponse error:", e);
    }
    sendScriptResponse(port, id, { type: "bool", v: true });
    return;
  }

  // ---------------------------------------------------------------
  // HTTP Fetch: perform HTTP requests in MAIN world so page-level
  // window.fetch overrides are honored.
  // ---------------------------------------------------------------

  if (op === "httpFetch") {
    const resp = await sendToPageScriptAsync(req);
    if (!resp) {
      sendScriptError(port, id, "no response from page script");
    } else if (resp.error) {
      sendScriptError(port, id, resp.error);
    } else {
      sendScriptResponse(port, id, resp.value);
    }
    return;
  }

  // ---------------------------------------------------------------
  // URL rewriting: apply regex rewrite rules from extension settings
  // ---------------------------------------------------------------

  if (op === "rewriteUrl") {
    const url = req.url || "";
    const rules = Array.isArray(_flashSettings.urlRewriteRules)
      ? _flashSettings.urlRewriteRules
      : [];
    let current = url;
    let changed = false;
    for (const rule of rules) {
      try {
        const re = new RegExp(rule.source);
        if (re.test(current)) {
          const target = rule.target.replace(/\\(\d+)/g, "\$$1");
          current = current.replace(re, target);
          changed = true;
        }
      } catch (_) {
        // skip invalid regex
      }
    }
    if (changed) {
      console.log(`[Flash Player] URL rewritten:`, { original: url, rewritten: current });
      sendScriptResponse(port, id, { type: "string", v: current });
    } else {
      sendScriptResponse(port, id, { type: "null" });
    }
    return;
  }

  // ---------------------------------------------------------------
  // Settings: persist settings edits from the native host
  // ---------------------------------------------------------------

  if (op === "editSettings") {
    const edits = req.edits;
    if (edits && typeof edits === "object") {
      try {
        const store = chrome.storage.sync || chrome.storage.local;
        store.set(edits, () => {
          // After persisting, update the in-memory settings and broadcast
          // so all Flash instances (including the one that triggered this)
          // see the updated values immediately.
          store.get(null, (items) => {
            applyFlashSettings(items, FlashInstance.broadcastSettingsUpdate);
          });
        });
      } catch (e) {
        console.warn("[Flash Player] editSettings error:", e);
      }
    }
    sendScriptResponse(port, id, { type: "bool", v: true });
    return;
  }

  // Fire-and-forget operations (e.g. "release") don't need a response.
  if (op === "release") {
    sendToPageScript(req);
    return;
  }

  // Inject instanceId so that page-script.js can target the correct
  // Flash element when there are multiple SWFs on the page.
  if (req.instanceId == null) {
    const iid = FlashInstance.getIdByPort(port);
    if (iid != null) req.instanceId = iid;
  }

  const resp = sendToPageScript(req);
  if (!resp) {
    sendScriptError(port, id, "no response from page script");
    return;
  }

  if (resp.error) {
    sendScriptError(port, id, resp.error);
  } else if (resp.names) {
    try {
      port.postMessage({ type: "jsResponse", id, names: resp.names });
    } catch { /* port disconnected */ }
  } else {
    sendScriptResponse(port, id, resp.value);
  }
}

/** Send a successful scripting response back to the native host. */
function sendScriptResponse(port, id, value) {
  try {
    port.postMessage({ type: "jsResponse", id, value });
  } catch {
    // Port may have disconnected.
  }
}

/** Send an error scripting response back to the native host. */
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
 *
 * page-script.js dispatches "__flash_callfn" CustomEvents on the shared
 * comm element when JavaScript calls a registered ExternalInterface
 * callback (e.g. game.startup(…)).  We forward the invoke XML to the
 * native host which routes it to PepperFlash's scriptable object.
 */
function initCallFunctionBridge() {
  const comm = getCommElement();
  comm.addEventListener("__flash_callfn", () => {
    const xml = comm.getAttribute("data-callfn");
    if (xml && FlashInstance._activePort) {
      try {
        FlashInstance._activePort.postMessage({ type: "callFunction", xml });
      } catch {
        // Port may have disconnected.
      }
    }
  });
}

// ---------------------------------------------------------------------------
// Input event helpers
// ---------------------------------------------------------------------------

/**
 * Build modifier flags matching PP_InputEvent_Modifier.
 */
function getModifiers(e) {
  let m = 0;
  if (e.shiftKey) m |= 1;       // PP_INPUTEVENT_MODIFIER_SHIFTKEY
  if (e.ctrlKey) m |= 2;       // PP_INPUTEVENT_MODIFIER_CONTROLKEY
  if (e.altKey) m |= 4;       // PP_INPUTEVENT_MODIFIER_ALTKEY
  if (e.metaKey) m |= 8;       // PP_INPUTEVENT_MODIFIER_METAKEY
  if (e.buttons & 1) m |= 16;   // PP_INPUTEVENT_MODIFIER_LEFTBUTTONDOWN
  if (e.buttons & 4) m |= 32;   // PP_INPUTEVENT_MODIFIER_MIDDLEBUTTONDOWN
  if (e.buttons & 2) m |= 64;   // PP_INPUTEVENT_MODIFIER_RIGHTBUTTONDOWN
  return m;
}

/**
 * Map a DOM MouseEvent.button to our protocol button index.
 */
function mapButton(e) {
  return e.button;
}

/**
 * Compute mouse position relative to the canvas in Flash view coordinates
 * (DIPs). Uses the logical Flash dimensions (instance.origWidth/origHeight)
 * rather than canvas.width/canvas.height because Flash may render at a
 * higher device resolution, making the canvas buffer larger than the
 * view rect reported via DidChangeView.
 */
function canvasPos(canvas, e, instance) {
  const rect = canvas.getBoundingClientRect();
  const logicalW = (instance && instance.origWidth) || canvas.width;
  const logicalH = (instance && instance.origHeight) || canvas.height;
  const scaleX = logicalW / rect.width;
  const scaleY = logicalH / rect.height;
  return {
    x: Math.round((e.clientX - rect.left) * scaleX),
    y: Math.round((e.clientY - rect.top) * scaleY),
  };
}

// ---------------------------------------------------------------------------
// Cursor mapping  (PP_CursorType_Dev → CSS cursor)
// ---------------------------------------------------------------------------

/** Lookup table mapping PP_CursorType_Dev enum values → CSS cursor values. */
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

/**
 * Map a PepperFlash cursor type enum to its CSS cursor value.
 * @param {number} cursorType - PP_CursorType_Dev value.
 * @returns {string} CSS cursor keyword.
 */
function ppCursorToCss(cursorType) {
  return PP_CURSOR_MAP[cursorType] || "default";
}

// ---------------------------------------------------------------------------
// Mutation Observer - scan and watch for Flash elements
// ---------------------------------------------------------------------------

/**
 * MutationObserver callback.  Walks added nodes looking for <object> or
 * <embed> Flash elements and replaces them with FlashInstance canvases.
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
        if (FlashInstance.fromElement(node)) continue;
      }

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
 * Initialise the Flash Player content script.
 * Sets up the ExternalInterface bridge, scans existing DOM elements for
 * Flash content, and installs a MutationObserver for future elements.
 */
function init() {
  initCallFunctionBridge();

  const tags = ["object", "embed"];
  for (const tagName of tags) {
    const elems = document.getElementsByTagName(tagName);
    const snapshot = Array.from(elems);
    for (const elem of snapshot) {
      FlashInstance.fromElement(elem);
    }
  }

  const observer = new MutationObserver(flashMutationObserver);
  observer.observe(document.documentElement, {
    subtree: true,
    childList: true,
  });
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", init);
} else {
  init();
}

// ---------------------------------------------------------------------------
// Page lifecycle: tear down on navigate-away, restart on bfcache restore
//
// Only affects instances belonging to the current document, so that
// iframes with their own Flash content are not disturbed.
// ---------------------------------------------------------------------------

// Tear down instances belonging to THIS document on navigate-away.
window.addEventListener("pagehide", () => {
  FlashInstance._navigatingAway = true;
  FlashInstance.destroyForDocument(document);
});

// Guard: skip the very first pagereveal (initial page load).
let firstReveal = true;

// Restart instances belonging to THIS document when the page is restored
// from bfcache.  Other documents (e.g. iframes) keep their own state.
window.addEventListener("pagereveal", () => {
  if (firstReveal) {
    firstReveal = false;
    return;
  }
  // Page was restored from bfcache - ports are dead, restart everything
  // that belongs to this document.
  FlashInstance._navigatingAway = false;
  FlashInstance.restartForDocument(document);
});
