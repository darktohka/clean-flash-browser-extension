/**
 * Flash Player Page Script  (runs in MAIN world)
 *
 * Handles JavaScript scripting requests from the native Flash Player host.
 * Communication with the content script (ISOLATED world) uses a shared
 * hidden DOM element:
 *
 *   content.js  →  writes request JSON to element attribute
 *               →  dispatches synchronous CustomEvent on the element
 *   page-script.js handles it, writes response JSON to another attribute
 *   content.js  ←  reads response attribute (same call stack, synchronous)
 *
 * This runs in the page's main JS context, so `eval()`, `window`, and all
 * DOM APIs are the real page objects (not the extension's isolated copies).
 */

"use strict";

(function () {
  // Unique element id — must match the one in content.js.
  const COMM_ID = "__flash_player_comm__";

  // Create the hidden communication element if it doesn't exist yet.
  let comm = document.getElementById(COMM_ID);
  if (!comm) {
    comm = document.createElement("div");
    comm.id = COMM_ID;
    comm.style.display = "none";
    (document.documentElement || document.body || document.head).appendChild(comm);
  }

  // -----------------------------------------------------------------
  // Object reference store
  // -----------------------------------------------------------------

  const jsObjects = new Map();
  let nextJsObjectId = 1; // 0 = window

  jsObjects.set(0, window);

  function registerJsObject(obj) {
    for (const [id, existing] of jsObjects) {
      if (existing === obj) return id;
    }
    const id = nextJsObjectId++;
    jsObjects.set(id, obj);
    return id;
  }

  // -----------------------------------------------------------------
  // Value encoding / decoding
  // -----------------------------------------------------------------

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

  function decodeJsValue(encoded) {
    if (!encoded || !encoded.type) return undefined;
    switch (encoded.type) {
      case "undefined":
        return undefined;
      case "null":
        return null;
      case "bool":
        return !!encoded.v;
      case "int":
        return encoded.v | 0;
      case "double":
        return +encoded.v;
      case "string":
        return String(encoded.v ?? "");
      case "object":
        return jsObjects.get(encoded.v);
      default:
        return undefined;
    }
  }

  // -----------------------------------------------------------------
  // Request handler
  // -----------------------------------------------------------------

  function handleRequest(req) {
    const op = req.op;

    switch (op) {
      case "getWindow":
        return { value: encodeJsValue(window) };

      case "hasProperty": {
        const obj = jsObjects.get(req.obj);
        const result = obj != null && req.name in Object(obj);
        return { value: { type: "bool", v: result } };
      }

      case "hasMethod": {
        const obj = jsObjects.get(req.obj);
        const result =
          obj != null && typeof Object(obj)[req.name] === "function";
        return { value: { type: "bool", v: result } };
      }

      case "getProperty": {
        const obj = jsObjects.get(req.obj);
        if (obj == null) return { value: { type: "undefined" } };
        const val = Object(obj)[req.name];
        return { value: encodeJsValue(val) };
      }

      case "setProperty": {
        const obj = jsObjects.get(req.obj);
        if (obj != null) {
          Object(obj)[req.name] = decodeJsValue(req.value);
        }
        return { value: { type: "undefined" } };
      }

      case "removeProperty": {
        const obj = jsObjects.get(req.obj);
        if (obj != null) delete Object(obj)[req.name];
        return { value: { type: "undefined" } };
      }

      case "getAllPropertyNames": {
        const obj = jsObjects.get(req.obj);
        const names = obj != null ? Object.keys(Object(obj)) : [];
        return { names };
      }

      case "callMethod": {
        const obj = jsObjects.get(req.obj);
        if (obj == null) return { error: "object not found" };
        const fn_ = Object(obj)[req.method];
        if (typeof fn_ !== "function")
          return { error: `${req.method} is not a function` };
        const args = (req.args || []).map(decodeJsValue);
        const result = fn_.apply(obj, args);
        return { value: encodeJsValue(result) };
      }

      case "call": {
        const fn_ = jsObjects.get(req.obj);
        if (typeof fn_ !== "function")
          return { error: "object is not callable" };
        const args = (req.args || []).map(decodeJsValue);
        const result = fn_(...args);
        return { value: encodeJsValue(result) };
      }

      case "construct": {
        const ctor = jsObjects.get(req.obj);
        if (typeof ctor !== "function")
          return { error: "object is not a constructor" };
        const args = (req.args || []).map(decodeJsValue);
        const result = new ctor(...args);
        return { value: encodeJsValue(result) };
      }

      case "executeScript": {
        // Indirect eval → runs in global scope of the page.
        const result = (0, eval)(req.script);
        return { value: encodeJsValue(result) };
      }

      case "release": {
        if (req.obj !== 0) jsObjects.delete(req.obj);
        return null; // no response needed
      }

      default:
        return { error: `unknown script op: ${op}` };
    }
  }

  // -----------------------------------------------------------------
  // Listen for requests from the content script
  // -----------------------------------------------------------------

  comm.addEventListener("__flash_req", () => {
    const reqJson = comm.getAttribute("data-req");
    if (!reqJson) return;

    let resp;
    try {
      const req = JSON.parse(reqJson);
      resp = handleRequest(req);
    } catch (e) {
      resp = { error: String(e) };
    }

    // Write response back (null means fire-and-forget, e.g. "release").
    comm.setAttribute("data-resp", resp ? JSON.stringify(resp) : "");
  });

  // Signal that the page script is ready.
  comm.setAttribute("data-ready", "1");
})();
