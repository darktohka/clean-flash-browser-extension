/**
 * Clean Flash Settings popup script.
 *
 * Reads/writes settings to chrome.storage.sync (or chrome.storage.local
 * as fallback).  Settings keys mirror the Rust PlayerSettings struct.
 */

"use strict";

const DEFAULTS = {
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
  whitelistedFolders: []
};

const storage = chrome.storage.sync || chrome.storage.local;

// ---- Elements (General tab) ----

const ruffleCompat = document.getElementById("ruffleCompat");
const preferNetworkBrowser = document.getElementById("preferNetworkBrowser");
const networkFallbackNative = document.getElementById("networkFallbackNative");
const disableCrossdomainHttp = document.getElementById("disableCrossdomainHttp");
const disableCrossdomainSockets = document.getElementById("disableCrossdomainSockets");
const hardwareAcceleration = document.getElementById("hardwareAcceleration");
const audioBackend = document.getElementById("audioBackend");
const disableGeolocation = document.getElementById("disableGeolocation");
const spoofHardwareId = document.getElementById("spoofHardwareId");
const disableMicrophone = document.getElementById("disableMicrophone");
const disableWebcam = document.getElementById("disableWebcam");

// ---- Elements (Sandboxing tab) ----

const httpSandboxMode = document.getElementById("httpSandboxMode");
const tcpUdpSandboxMode = document.getElementById("tcpUdpSandboxMode");
const fileWhitelistEnabled = document.getElementById("fileWhitelistEnabled");

// ---- Tab switching ----

document.querySelectorAll(".tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab-btn").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".tab-content").forEach((c) => c.classList.remove("active"));
    btn.classList.add("active");
    const tab = document.getElementById("tab-" + btn.dataset.tab);
    if (tab) tab.classList.add("active");
  });
});

// ---- List editor helper ----

/**
 * Manages a list of strings backed by chrome.storage.
 * @param {string} storageKey - The key in chrome.storage
 * @param {HTMLElement} itemsContainer - The .list-items div
 * @param {HTMLInputElement} input - The text input
 * @param {HTMLButtonElement} addBtn - The add button
 * @param {object} [opts] - Options
 * @param {boolean} [opts.sorted] - Keep items sorted alphabetically
 * @param {HTMLButtonElement} [opts.browseBtn] - Browse button element
 * @param {"file"|"folder"} [opts.browseMode] - Whether to browse for files or folders
 */
function setupListEditor(storageKey, itemsContainer, input, addBtn, opts) {
  const sorted = opts && opts.sorted;
  let items = [];

  function render() {
    itemsContainer.innerHTML = "";
    if (items.length === 0) {
      const empty = document.createElement("div");
      empty.className = "list-empty";
      empty.textContent = "No entries";
      itemsContainer.appendChild(empty);
      return;
    }
    items.forEach((entry, idx) => {
      const row = document.createElement("div");
      row.className = "list-item";

      const text = document.createElement("span");
      text.className = "list-item-text";
      text.textContent = entry;
      row.appendChild(text);

      const removeBtn = document.createElement("button");
      removeBtn.className = "list-item-remove";
      removeBtn.textContent = "\u00d7";
      removeBtn.addEventListener("click", () => {
        items.splice(idx, 1);
        saveList();
        render();
      });
      row.appendChild(removeBtn);

      itemsContainer.appendChild(row);
    });
  }

  function saveList() {
    save(storageKey, items.slice());
  }

  function addEntry() {
    const val = input.value.trim();
    if (val && !items.includes(val)) {
      items.push(val);
      if (sorted) items.sort((a, b) => a.localeCompare(b));
      saveList();
      render();
    }
    input.value = "";
  }

  addBtn.addEventListener("click", addEntry);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") addEntry();
  });

  // Browse button: use a hidden <input type="file"> to pick files/folders
  if (opts && opts.browseBtn) {
    const hiddenInput = document.createElement("input");
    hiddenInput.type = "file";
    hiddenInput.style.display = "none";
    if (opts.browseMode === "folder") {
      hiddenInput.setAttribute("webkitdirectory", "");
    } else {
      hiddenInput.multiple = true;
    }
    document.body.appendChild(hiddenInput);

    opts.browseBtn.addEventListener("click", () => {
      hiddenInput.value = "";
      hiddenInput.click();
    });

    hiddenInput.addEventListener("change", () => {
      const files = hiddenInput.files;
      if (!files || files.length === 0) return;

      let changed = false;
      if (opts.browseMode === "folder") {
        // Extract the common folder path from webkitRelativePath
        // files[0].webkitRelativePath is like "folderName/file.txt"
        // We don't have access to the absolute path in a browser extension,
        // so we use the folder name as the entry.
        const paths = new Set();
        for (const f of files) {
          const rel = f.webkitRelativePath || f.name;
          const parts = rel.split("/");
          if (parts.length > 1) {
            paths.add(parts[0]);
          }
        }
        for (const p of paths) {
          if (p && !items.includes(p)) {
            items.push(p);
            changed = true;
          }
        }
      } else {
        for (const f of files) {
          const name = f.name;
          if (name && !items.includes(name)) {
            items.push(name);
            changed = true;
          }
        }
      }

      if (changed) {
        if (sorted) items.sort((a, b) => a.localeCompare(b));
        saveList();
        render();
      }
    });
  }

  return {
    load(arr) {
      items = Array.isArray(arr) ? arr.slice() : [];
      if (sorted) items.sort((a, b) => a.localeCompare(b));
      render();
    },
    getItems() { return items; }
  };
}

// Setup list editors
const httpBlacklistEditor = setupListEditor("httpBlacklist",
  document.getElementById("httpBlacklistItems"),
  document.getElementById("httpBlacklistInput"),
  document.getElementById("httpBlacklistAdd"));

const httpWhitelistEditor = setupListEditor("httpWhitelist",
  document.getElementById("httpWhitelistItems"),
  document.getElementById("httpWhitelistInput"),
  document.getElementById("httpWhitelistAdd"));

const tcpUdpBlacklistEditor = setupListEditor("tcpUdpBlacklist",
  document.getElementById("tcpUdpBlacklistItems"),
  document.getElementById("tcpUdpBlacklistInput"),
  document.getElementById("tcpUdpBlacklistAdd"));

const tcpUdpWhitelistEditor = setupListEditor("tcpUdpWhitelist",
  document.getElementById("tcpUdpWhitelistItems"),
  document.getElementById("tcpUdpWhitelistInput"),
  document.getElementById("tcpUdpWhitelistAdd"));

const whitelistedFilesEditor = setupListEditor("whitelistedFiles",
  document.getElementById("whitelistedFilesItems"),
  document.getElementById("whitelistedFilesInput"),
  document.getElementById("whitelistedFilesAdd"),
  { sorted: true, browseBtn: document.getElementById("whitelistedFilesBrowse"), browseMode: "file" });

const whitelistedFoldersEditor = setupListEditor("whitelistedFolders",
  document.getElementById("whitelistedFoldersItems"),
  document.getElementById("whitelistedFoldersInput"),
  document.getElementById("whitelistedFoldersAdd"),
  { sorted: true, browseBtn: document.getElementById("whitelistedFoldersBrowse"), browseMode: "folder" });

// ---- Load saved settings ----

storage.get(DEFAULTS, (items) => {
  ruffleCompat.value = items.ruffleCompat;
  preferNetworkBrowser.checked = items.preferNetworkBrowser;
  networkFallbackNative.checked = items.networkFallbackNative;
  disableCrossdomainHttp.checked = items.disableCrossdomainHttp;
  disableCrossdomainSockets.checked = items.disableCrossdomainSockets;
  hardwareAcceleration.checked = items.hardwareAcceleration;
  audioBackend.value = items.audioBackend;
  disableGeolocation.checked = items.disableGeolocation;
  spoofHardwareId.checked = items.spoofHardwareId;
  disableMicrophone.checked = items.disableMicrophone;
  disableWebcam.checked = items.disableWebcam;

  // Sandboxing
  httpSandboxMode.value = items.httpSandboxMode;
  tcpUdpSandboxMode.value = items.tcpUdpSandboxMode;
  fileWhitelistEnabled.checked = items.fileWhitelistEnabled;

  httpBlacklistEditor.load(items.httpBlacklist);
  httpWhitelistEditor.load(items.httpWhitelist);
  tcpUdpBlacklistEditor.load(items.tcpUdpBlacklist);
  tcpUdpWhitelistEditor.load(items.tcpUdpWhitelist);
  whitelistedFilesEditor.load(items.whitelistedFiles);
  whitelistedFoldersEditor.load(items.whitelistedFolders);

  updateSliderLabels();
  updateNetworkDependencies();
  updateHttpSandboxVisibility();
  updateTcpUdpSandboxVisibility();
  updateFileWhitelistVisibility();
});

// ---- Save on change ----

function save(key, value) {
  storage.set({ [key]: value }, () => {
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

// ---- General tab listeners ----

ruffleCompat.addEventListener("input", () => {
  const v = Number(ruffleCompat.value);
  save("ruffleCompat", v);
  updateSliderLabels();
});

preferNetworkBrowser.addEventListener("change", () => {
  const checked = preferNetworkBrowser.checked;
  save("preferNetworkBrowser", checked);
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

audioBackend.addEventListener("change", () => {
  const v = Number(audioBackend.value);
  save("audioBackend", v);
});

disableGeolocation.addEventListener("change", () => {
  save("disableGeolocation", disableGeolocation.checked);
});

spoofHardwareId.addEventListener("change", () => {
  save("spoofHardwareId", spoofHardwareId.checked);
});

disableMicrophone.addEventListener("change", () => {
  save("disableMicrophone", disableMicrophone.checked);
});

disableWebcam.addEventListener("change", () => {
  save("disableWebcam", disableWebcam.checked);
});

// ---- Sandboxing tab listeners ----

httpSandboxMode.addEventListener("change", () => {
  save("httpSandboxMode", httpSandboxMode.value);
  updateHttpSandboxVisibility();
});

tcpUdpSandboxMode.addEventListener("change", () => {
  save("tcpUdpSandboxMode", tcpUdpSandboxMode.value);
  updateTcpUdpSandboxVisibility();
});

fileWhitelistEnabled.addEventListener("change", () => {
  save("fileWhitelistEnabled", fileWhitelistEnabled.checked);
  updateFileWhitelistVisibility();
});

// ---- UI helpers ----

function updateSliderLabels() {
  const v = Number(ruffleCompat.value);
  document.querySelectorAll(".slider-label").forEach((el) => {
    if (el.closest(".slider-container") === ruffleCompat.closest(".slider-container")) {
      el.classList.toggle("active", Number(el.dataset.value) === v);
    }
  });
}

function updateNetworkDependencies() {
  networkFallbackNative.disabled = !preferNetworkBrowser.checked;
}

function updateHttpSandboxVisibility() {
  const mode = httpSandboxMode.value;
  document.getElementById("httpBlacklistSection").style.display = mode === "blacklist" ? "" : "none";
  document.getElementById("httpWhitelistSection").style.display = mode === "whitelist" ? "" : "none";
}

function updateTcpUdpSandboxVisibility() {
  const mode = tcpUdpSandboxMode.value;
  document.getElementById("tcpUdpBlacklistSection").style.display = mode === "blacklist" ? "" : "none";
  document.getElementById("tcpUdpWhitelistSection").style.display = mode === "whitelist" ? "" : "none";
}

function updateFileWhitelistVisibility() {
  document.getElementById("fileWhitelistDetails").style.display = fileWhitelistEnabled.checked ? "" : "none";
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

  const rect = el.getBoundingClientRect();
  const tipW = 240;
  const pad = 6;

  let left = rect.left + rect.width / 2 - tipW / 2;
  left = Math.max(pad, Math.min(left, document.body.clientWidth - tipW - pad));

  tip.style.width = tipW + "px";
  tip.style.left = left + "px";
  tip.style.top = "0px";
  tip.style.visibility = "hidden";

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

function bindTooltips() {
  document.querySelectorAll("[data-tooltip]").forEach((el) => {
    el.addEventListener("mouseenter", () => showTooltip(el));
    el.addEventListener("mouseleave", hideTooltip);
  });
}

bindTooltips();
