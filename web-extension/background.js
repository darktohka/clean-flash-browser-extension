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
 * Map of instanceId -> content script port.
 * Each Flash instance on a page gets its own port.
 */
const instances = new Map();

/** Map of instanceId -> native messaging port. */
const nativePorts = new Map();

/**
 * In-progress chunked messages being reassembled, per instance.
 * Map of instanceId -> Map of sequence_id -> { total, received, chunks: string[] }
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
 * Returns the port, or null if creation failed.
 */
function createNativePort(instanceId, port) {
  const nativePort = chrome.runtime.connectNative(NATIVE_HOST_NAME);
  nativePorts.set(instanceId, nativePort);

  nativePort.onMessage.addListener((msg) => {
    // All host messages are chunked: {s, c, t, d}.
    const b64 = handleChunk(instanceId, msg);
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
      console.error(`[Flash Player] Native host disconnected for instance ${instanceId}:`, error.message);
    }
    nativePorts.delete(instanceId);
    pendingChunks.delete(instanceId);
    // Notify the instance with an inline error.
    try {
      port.postMessage({
        error: "Native host disconnected" + (error ? ": " + error.message : ""),
      });
    } catch {
      // ignore
    }
    instances.delete(instanceId);
  });

  return nativePort;
}

/**
 * Handle a new port connection from a content script.
 */
chrome.runtime.onConnect.addListener((port) => {
  if (port.name !== "flash-instance") return;

  let instanceId = null;
  let nativePort = null;

  port.onMessage.addListener((msg) => {
    if (msg.type === "start") {
      instanceId = msg.instanceId;
      instances.set(instanceId, port);

      // Detect incognito mode from the sender tab.
      const incognito = !!(port.sender && port.sender.tab && port.sender.tab.incognito);

      // Create a native host for this instance and send the open command.
      nativePort = createNativePort(instanceId, port);
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
    if (instanceId != null) {
      instances.delete(instanceId);
      // Close the native host for this instance.
      const np = nativePorts.get(instanceId);
      if (np) {
        np.postMessage({ type: "close" });
        np.disconnect();
        nativePorts.delete(instanceId);
        pendingChunks.delete(instanceId);
      }
    }
  });
});
