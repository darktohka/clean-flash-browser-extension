/**
 * Flash Player Background Service Worker
 *
 * Manages the native messaging connection to the flash-player-host binary.
 * Each content-script instance opens a port here; the background worker
 * multiplexes them over a single native messaging connection (one host
 * process per extension lifetime).
 *
 * The host sends chunked binary messages:
 *   {"s": seq, "c": chunk_index, "t": total_chunks, "d": "base64_data"}
 *
 * This script reassembles multi-chunk messages, then forwards the
 * complete base64 blob to the content script for binary decoding.
 *
 * Protocol flow:
 *   content.js  --port-->  background.js  --nativePort-->  flash-player-host
 *               <--port--                 <--nativePort--
 */

"use strict";

const NATIVE_HOST_NAME = "org.cleanflash.flash_player";

/**
 * Background-global unique connection ID counter.
 * Content-script instance IDs can collide across tabs (each tab's counter
 * starts at 0), so the background uses its own IDs as map keys.
 */
let _nextBgId = 0;

/**
 * Map of bgId -> content script port.
 * Each Flash instance on a page gets its own port.
 */
const instances = new Map();

/** Map of bgId -> native messaging port. */
const nativePorts = new Map();

/**
 * In-progress chunked messages being reassembled, per connection.
 * Map of bgId -> Map of sequence_id -> { total, received, chunks: string[] }
 */
const pendingChunks = new Map();

/**
 * Reassemble a chunk. When all chunks for a sequence are received,
 * returns the concatenated base64 string. Otherwise returns null.
 */
/**
 * Reassemble a chunk for a specific instance. When all chunks for a sequence are received,
 * returns the concatenated base64 string. Otherwise returns null.
 */
function handleChunk(instanceId, msg) {
  const { s: seq, c: index, t: total, d: data } = msg;

  // Single-chunk message -- fast path.
  if (total === 1) {
    return data;
  }

  let instanceChunks = pendingChunks.get(instanceId);
  if (!instanceChunks) {
    instanceChunks = new Map();
    pendingChunks.set(instanceId, instanceChunks);
  }

  let entry = instanceChunks.get(seq);
  if (!entry) {
    entry = { total, received: 0, chunks: new Array(total) };
    instanceChunks.set(seq, entry);
  }

  entry.chunks[index] = data;
  entry.received++;

  if (entry.received === entry.total) {
    instanceChunks.delete(seq);
    return entry.chunks.join("");
  }

  return null;
}

/**
 * Forward a fully reassembled message to all connected content-script
 * instances.  The message is sent as `{b64: "<base64 binary blob>"}`.
 */
function broadcastMessage(b64) {
  for (const [, port] of instances) {
    try {
      port.postMessage({ b64 });
    } catch {
      // Port may have disconnected.
    }
  }
}

/**
 * Create a native messaging port for a SWF instance.
 *
 * @param {number} bgId - Background-unique connection ID (used as map key).
 * @param {Port} port - The content script's chrome.runtime port.
 * @returns {Port} The newly created native messaging port.
 */
function createNativePort(bgId, port) {
  const nativePort = chrome.runtime.connectNative(NATIVE_HOST_NAME);
  nativePorts.set(bgId, nativePort);

  nativePort.onMessage.addListener((msg) => {
    // All host messages are chunked: {s, c, t, d}.
    const b64 = handleChunk(bgId, msg);
    if (b64 !== null) {
      // Only send to the matching content script instance.
      try {
        port.postMessage({ b64 });
      } catch {
        // Port may have disconnected.
      }
    }
  });

  nativePort.onDisconnect.addListener(() => {
    const error = chrome.runtime.lastError;
    if (error) {
      console.error(`[Flash Player] Native host disconnected (bgId=${bgId}):`, error.message);
    }
    nativePorts.delete(bgId);
    pendingChunks.delete(bgId);

    // Detect "host not found / not installed" vs normal crash.
    const errMsg = error ? error.message || "" : "";
    const notInstalled =
      /not found|not installed|host.*not.*registered/i.test(errMsg);

    // Notify the instance with an inline error.
    try {
      port.postMessage({
        error: "Native host disconnected" + (error ? ": " + errMsg : ""),
        notInstalled,
      });
    } catch {
      // ignore
    }
    instances.delete(bgId);
  });

  return nativePort;
}

/**
 * Handle a new port connection from a content script.
 */
chrome.runtime.onConnect.addListener((port) => {
  if (port.name !== "flash-instance") return;

  // Allocate a background-unique ID for this connection so that
  // duplicate tabs (which reuse content-script instance IDs starting
  // at 0) don't collide in the background's maps.
  const bgId = _nextBgId++;
  let nativePort = null;

  port.onMessage.addListener((msg) => {
    if (msg.type === "start") {
      instances.set(bgId, port);

      // Detect incognito mode from the sender tab.
      const incognito = !!(port.sender && port.sender.tab && port.sender.tab.incognito);

      // Create a native host for this instance and send the open command.
      nativePort = createNativePort(bgId, port);
      if (!nativePort) return;
      nativePort.postMessage({
        type: "open",
        url: msg.url,
        args: msg.args || [],
        incognito,
        language: msg.language || "",
        deviceScale: msg.deviceScale,
        cssScale: msg.cssScale,
        scrollX: msg.scrollX,
        scrollY: msg.scrollY,
        isFullscreen: msg.isFullscreen,
        isVisible: msg.isVisible,
        isPageVisible: msg.isPageVisible,
        width: msg.width,
        height: msg.height,
        settings: msg.settings || {},
      });

      // Send initial resize with view info.
      if (msg.width && msg.height) {
        nativePort.postMessage({
          type: "resize",
          width: msg.width,
          height: msg.height,
          deviceScale: msg.deviceScale,
          cssScale: msg.cssScale,
          scrollX: msg.scrollX,
          scrollY: msg.scrollY,
          isFullscreen: msg.isFullscreen,
          isVisible: msg.isVisible,
          isPageVisible: msg.isPageVisible,
        });
      }
    } else {
      // Forward input events and other commands directly.
      if (nativePort) {
        nativePort.postMessage(msg);
      }
    }
  });

  port.onDisconnect.addListener(() => {
    instances.delete(bgId);
    // Close the native host for this connection.
    if (nativePort) {
      try { nativePort.postMessage({ type: "close" }); } catch { /* already gone */ }
      nativePort.disconnect();
      nativePorts.delete(bgId);
      pendingChunks.delete(bgId);
      nativePort = null;
    }
  });
});

// ---------------------------------------------------------------------------
// Cookie API — chrome.cookies access for content scripts
//
// Content scripts cannot use chrome.cookies directly, so they send
// messages here.  We use chrome.cookies.getAll() to retrieve matching
// cookies for a URL, and chrome.cookies.set() to store Set-Cookie
// response headers received by the native HTTP client.
// ---------------------------------------------------------------------------

chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg.type === "openSettings") {
    // Open the extension popup/settings page.
    chrome.action.openPopup().catch(() => {
      // Fallback: open settings.html in a new tab if popup can't be opened.
      chrome.tabs.create({ url: chrome.runtime.getURL("settings.html") });
    });
    return false;
  }

  if (msg.type === "settingsUpdate") {
    // Forward settings update to all connected native host instances.
    const settings = msg.settings || {};
    for (const [, nativePort] of nativePorts) {
      try {
        nativePort.postMessage({ type: "settingsUpdate", settings });
      } catch {
        // Port may have disconnected.
      }
    }
    // Also forward to all content script tabs so they update page-script.js
    // data attribute for future page loads.
    chrome.tabs.query({}, (tabs) => {
      for (const tab of tabs) {
        chrome.tabs.sendMessage(tab.id, {
          type: "settingsUpdate",
          settings,
        }).catch(() => {});
      }
    });
    return false;
  }

  if (msg.type === "getCookies") {
    const url = msg.url;
    if (!url || !chrome.cookies) {
      sendResponse({ cookies: "" });
      return false;
    }
    chrome.cookies.getAll({ url })
      .then((cookies) => {
        const cookieStr = cookies
          .map((c) => c.name + "=" + c.value)
          .join("; ");
        sendResponse({ cookies: cookieStr });
      })
      .catch((e) => {
        console.warn("[Flash Player] chrome.cookies.getAll error:", e);
        sendResponse({ cookies: "" });
      });
    return true; // async sendResponse
  }

  if (msg.type === "setCookies") {
    const url = msg.url;
    const cookieHeaders = msg.cookies; // array of Set-Cookie header strings
    if (!url || !cookieHeaders || !chrome.cookies) {
      sendResponse({ ok: true });
      return false;
    }
    // Parse each Set-Cookie header and store via chrome.cookies.set().
    const promises = cookieHeaders.map((header) => {
      const parsed = parseSetCookieHeader(header, url);
      if (!parsed) return Promise.resolve();
      return chrome.cookies.set(parsed).catch((e) => {
        console.warn("[Flash Player] chrome.cookies.set error:", e, parsed);
      });
    });
    Promise.all(promises)
      .then(() => sendResponse({ ok: true }))
      .catch(() => sendResponse({ ok: true }));
    return true; // async sendResponse
  }

  return false;
});

/**
 * Parse a Set-Cookie header string into a chrome.cookies.set() details object.
 *
 * @param {string} header - Raw Set-Cookie header value, e.g.
 *   "name=value; Path=/; Domain=.example.com; Secure; HttpOnly; SameSite=Lax; Max-Age=3600"
 * @param {string} requestUrl - The URL that produced this Set-Cookie header,
 *   used as fallback for domain/path and as the `url` parameter.
 * @returns {object|null} chrome.cookies.set() details, or null if unparseable.
 */
function parseSetCookieHeader(header, requestUrl) {
  const parts = header.split(";").map((s) => s.trim());
  if (parts.length === 0) return null;

  // First part is "name=value".
  const firstEq = parts[0].indexOf("=");
  if (firstEq < 0) return null;
  const name = parts[0].substring(0, firstEq).trim();
  const value = parts[0].substring(firstEq + 1).trim();
  if (!name) return null;

  let domain = null;
  let path = null;
  let secure = false;
  let httpOnly = false;
  let sameSite = undefined;
  let expirationDate = undefined;

  for (let i = 1; i < parts.length; i++) {
    const part = parts[i];
    const eqIdx = part.indexOf("=");
    const attrName = (eqIdx >= 0 ? part.substring(0, eqIdx) : part)
      .trim()
      .toLowerCase();
    const attrVal = eqIdx >= 0 ? part.substring(eqIdx + 1).trim() : "";

    switch (attrName) {
      case "domain":
        domain = attrVal;
        break;
      case "path":
        path = attrVal;
        break;
      case "secure":
        secure = true;
        break;
      case "httponly":
        httpOnly = true;
        break;
      case "samesite":
        switch (attrVal.toLowerCase()) {
          case "strict":
            sameSite = "strict";
            break;
          case "none":
            sameSite = "no_restriction";
            break;
          case "lax":
          default:
            sameSite = "lax";
            break;
        }
        break;
      case "max-age": {
        const secs = parseInt(attrVal, 10);
        if (!isNaN(secs)) {
          expirationDate = Math.floor(Date.now() / 1000) + secs;
        }
        break;
      }
      case "expires": {
        const d = new Date(attrVal);
        if (!isNaN(d.getTime())) {
          expirationDate = Math.floor(d.getTime() / 1000);
        }
        break;
      }
    }
  }

  const details = {
    url: requestUrl,
    name,
    value,
  };
  if (domain) details.domain = domain;
  if (path) details.path = path;
  if (secure) details.secure = true;
  if (httpOnly) details.httpOnly = true;
  if (sameSite) details.sameSite = sameSite;
  if (expirationDate !== undefined) details.expirationDate = expirationDate;

  return details;
}
