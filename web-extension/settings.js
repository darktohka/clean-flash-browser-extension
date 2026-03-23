/**
 * Clean Flash Settings popup script.
 *
 * Reads/writes settings to chrome.storage.sync (or chrome.storage.local
 * as fallback).  Settings keys mirror the Rust PlayerSettings struct.
 */

"use strict";

const DEFAULTS = {
  ruffleCompat: 1,              // 0=PreferRuffle, 1=PreferCleanFlash, 2=ForceCleanFlash
  networkBrowserOnly: true,
  networkFallbackNative: false,
  disableCrossdomainHttp: true,
  disableCrossdomainSockets: true,
  hardwareAcceleration: false,
  disableGeolocation: true,
  disableMicrophone: false,
  disableWebcam: false,
};

const storage = chrome.storage.sync || chrome.storage.local;

// ---- Elements ----

const ruffleCompat = document.getElementById("ruffleCompat");
const networkBrowserOnly = document.getElementById("networkBrowserOnly");
const networkFallbackNative = document.getElementById("networkFallbackNative");
const disableCrossdomainHttp = document.getElementById("disableCrossdomainHttp");
const disableCrossdomainSockets = document.getElementById("disableCrossdomainSockets");
const hardwareAcceleration = document.getElementById("hardwareAcceleration");
const disableGeolocation = document.getElementById("disableGeolocation");
const disableMicrophone = document.getElementById("disableMicrophone");
const disableWebcam = document.getElementById("disableWebcam");

// ---- Load saved settings ----

storage.get(DEFAULTS, (items) => {
  ruffleCompat.value = items.ruffleCompat;
  networkBrowserOnly.checked = items.networkBrowserOnly;
  networkFallbackNative.checked = items.networkFallbackNative;
  disableCrossdomainHttp.checked = items.disableCrossdomainHttp;
  disableCrossdomainSockets.checked = items.disableCrossdomainSockets;
  hardwareAcceleration.checked = items.hardwareAcceleration;
  disableGeolocation.checked = items.disableGeolocation;
  disableMicrophone.checked = items.disableMicrophone;
  disableWebcam.checked = items.disableWebcam;

  updateSliderLabels();
  updateNetworkDependencies();
});

// ---- Save on change ----

function save(key, value) {
  storage.set({ [key]: value }, () => {
    // Broadcast updated settings to all tabs so running Flash instances
    // can apply changes on-the-fly.
    broadcastSettings();
  });
}

/** Read all current settings and send a settingsUpdate message. */
function broadcastSettings() {
  storage.get(DEFAULTS, (items) => {
    chrome.runtime.sendMessage({
      type: "settingsUpdate",
      settings: items,
    }).catch(() => {});
  });
}

ruffleCompat.addEventListener("input", () => {
  const v = Number(ruffleCompat.value);
  save("ruffleCompat", v);
  updateSliderLabels();
});

networkBrowserOnly.addEventListener("change", () => {
  const checked = networkBrowserOnly.checked;
  save("networkBrowserOnly", checked);
  if (checked) {
    networkFallbackNative.checked = false;
    save("networkFallbackNative", false);
  }
  updateNetworkDependencies();
});

networkFallbackNative.addEventListener("change", () => {
  save("networkFallbackNative", networkFallbackNative.checked);
});

disableCrossdomainHttp.addEventListener("change", () => {
  save("disableCrossdomainHttp", disableCrossdomainHttp.checked);
});

disableCrossdomainSockets.addEventListener("change", () => {
  save("disableCrossdomainSockets", disableCrossdomainSockets.checked);
});

hardwareAcceleration.addEventListener("change", () => {
  save("hardwareAcceleration", hardwareAcceleration.checked);
});

disableGeolocation.addEventListener("change", () => {
  save("disableGeolocation", disableGeolocation.checked);
});

disableMicrophone.addEventListener("change", () => {
  save("disableMicrophone", disableMicrophone.checked);
});

disableWebcam.addEventListener("change", () => {
  save("disableWebcam", disableWebcam.checked);
});

// ---- UI helpers ----

function updateSliderLabels() {
  const v = Number(ruffleCompat.value);
  document.querySelectorAll(".slider-label").forEach((el) => {
    el.classList.toggle("active", Number(el.dataset.value) === v);
  });
}

function updateNetworkDependencies() {
  // "Fall back to native" is disabled when "always browser" is checked.
  networkFallbackNative.disabled = !networkBrowserOnly.checked;
}

// ---- Tooltip positioning (JS-based, stays within popup bounds) ----

let activeTooltip = null;

function showTooltip(el) {
  hideTooltip();
  const text = el.dataset.tooltip;
  if (!text) return;

  const tip = document.createElement("div");
  tip.className = "tooltip";
  tip.textContent = text;
  document.body.appendChild(tip);
  activeTooltip = tip;

  // Position above the element, clamped within the popup.
  const rect = el.getBoundingClientRect();
  const tipW = 240;
  const pad = 6;

  // Horizontal: center on the element, clamp to edges.
  let left = rect.left + rect.width / 2 - tipW / 2;
  left = Math.max(pad, Math.min(left, document.body.clientWidth - tipW - pad));

  // Vertical: prefer above, fall below if no room.
  tip.style.width = tipW + "px";
  tip.style.left = left + "px";
  tip.style.top = "0px";
  tip.style.visibility = "hidden";

  // Measure actual height.
  const tipH = tip.offsetHeight;
  let top = rect.top - tipH - pad;
  if (top < pad) {
    top = rect.bottom + pad;
  }
  tip.style.top = top + "px";
  tip.style.visibility = "";
}

function hideTooltip() {
  if (activeTooltip) {
    activeTooltip.remove();
    activeTooltip = null;
  }
}

document.querySelectorAll("[data-tooltip]").forEach((el) => {
  el.addEventListener("mouseenter", () => showTooltip(el));
  el.addEventListener("mouseleave", hideTooltip);
});
