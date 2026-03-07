/**
 * Flash Player Background Service Worker
 *
 * Manages the native messaging connection to the flash-player-host binary.
 * Each content-script instance opens a port here; the background worker
 * multiplexes them over a single native messaging connection (one host
 * process per extension lifetime).
 *
 * Protocol flow:
 *   content.js  ──port──▶  background.js  ──nativePort──▶  flash-player-host
 *               ◀──port──                 ◀──nativePort──
 */

"use strict";

const NATIVE_HOST_NAME = "org.nickvision.flash_player";

/**
 * Map of instanceId → content script port.
 * Each Flash instance on a page gets its own port.
 */
const instances = new Map();

/** The single native messaging port (lazy-connected). */
let nativePort = null;

/**
 * Connect to the native host if not already connected.
 */
function ensureNativePort() {
  if (nativePort) return;

  nativePort = chrome.runtime.connectNative(NATIVE_HOST_NAME);

  nativePort.onMessage.addListener((msg) => {
    // Forward host messages to the appropriate content-script instance.
    // Currently we support a single instance per host process.
    // Broadcast to all connected instances.
    for (const [, port] of instances) {
      try {
        port.postMessage(msg);
      } catch {
        // Port may have disconnected.
      }
    }
  });

  nativePort.onDisconnect.addListener(() => {
    const error = chrome.runtime.lastError;
    if (error) {
      console.error("[Flash Player] Native host disconnected:", error.message);
    }
    nativePort = null;

    // Notify all instances.
    for (const [, port] of instances) {
      try {
        port.postMessage({
          type: "error",
          message: "Native host disconnected" + (error ? ": " + error.message : ""),
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
    }
  });
});
