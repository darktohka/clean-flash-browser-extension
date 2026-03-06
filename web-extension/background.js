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

const NATIVE_HOST_NAME = "org.nickvision.flash_player";

/**
 * Map of instanceId -> content script port.
 * Each Flash instance on a page gets its own port.
 */
const instances = new Map();

/** The single native messaging port (lazy-connected). */
let nativePort = null;

/**
 * In-progress chunked messages being reassembled.
 * Map of sequence_id -> { total, received, chunks: string[] }
 */
const pendingChunks = new Map();

/**
 * Reassemble a chunk. When all chunks for a sequence are received,
 * returns the concatenated base64 string. Otherwise returns null.
 */
function handleChunk(msg) {
  const { s: seq, c: index, t: total, d: data } = msg;

  // Single-chunk message -- fast path.
  if (total === 1) {
    return data;
  }

  let entry = pendingChunks.get(seq);
  if (!entry) {
    entry = { total, received: 0, chunks: new Array(total) };
    pendingChunks.set(seq, entry);
  }

  entry.chunks[index] = data;
  entry.received++;

  if (entry.received === entry.total) {
    pendingChunks.delete(seq);
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
 * Connect to the native host if not already connected.
 */
function ensureNativePort() {
  if (nativePort) return;

  nativePort = chrome.runtime.connectNative(NATIVE_HOST_NAME);

  nativePort.onMessage.addListener((msg) => {
    // All host messages are chunked: {s, c, t, d}.
    const b64 = handleChunk(msg);
    if (b64 !== null) {
      broadcastMessage(b64);
    }
  });

  nativePort.onDisconnect.addListener(() => {
    const error = chrome.runtime.lastError;
    if (error) {
      console.error("[Flash Player] Native host disconnected:", error.message);
    }
    nativePort = null;
    pendingChunks.clear();

    // Notify all instances with an inline error (not chunked since
    // it originates from the extension, not the host).
    for (const [, port] of instances) {
      try {
        port.postMessage({
          error: "Native host disconnected" + (error ? ": " + error.message : ""),
        });
      } catch {
        // ignore
      }
    }
    instances.clear();
  });
}

/**
 * Handle a new port connection from a content script.
 */
chrome.runtime.onConnect.addListener((port) => {
  if (port.name !== "flash-instance") return;

  let instanceId = null;

  port.onMessage.addListener((msg) => {
    if (msg.type === "start") {
      instanceId = msg.instanceId;
      instances.set(instanceId, port);

      // Start the native host and send the open command.
      ensureNativePort();
      nativePort.postMessage({
        type: "open",
        url: msg.url,
      });

      // Send initial resize.
      if (msg.width && msg.height) {
        nativePort.postMessage({
          type: "resize",
          width: msg.width,
          height: msg.height,
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
    }
    // If no more instances, close the native host.
    if (instances.size === 0 && nativePort) {
      nativePort.postMessage({ type: "close" });
      nativePort.disconnect();
      nativePort = null;
      pendingChunks.clear();
    }
  });
});
