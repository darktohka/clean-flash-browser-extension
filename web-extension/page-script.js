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
  // ---------------------------------------------------------------------------
  // Flash plugin spoofing
  // ---------------------------------------------------------------------------

  /**
   * Inject a fake "Shockwave Flash" plugin into navigator.plugins and
   * navigator.mimeTypes so that:
   *
   *  1. Pages that feature-detect Flash via the plugin list believe Flash is
   *     installed and proceed to emit <object>/<embed> elements we intercept.
   *
   *  2. The Ruffle extension (if installed) sees a "real" Flash plugin and
   *     backs off from replacing our content.  Ruffle's two critical checks:
   *       - installPlugin():        navigator.plugins.namedItem("Shockwave Flash")  → truthy ⇒ skip
   *       - isFlashEnabledBrowser(): …?.filename !== "ruffle.js"                    → true   ⇒ skip
   *
   * We therefore create a Plugin-shaped object whose `.filename` is NOT
   * "ruffle.js" and patch both `PluginArray.prototype.namedItem` and
   * `MimeTypeArray.prototype.namedItem` so the fake entries are discoverable
   * through the standard API.
   */
  function ppSpoofFlash() {
    try {
      // Bail out if a real Flash plugin (or a previous spoof) already exists.
      if (navigator.plugins.namedItem("Shockwave Flash")) return;

      // ---- Fake Plugin object ----
      const flashPlugin = Object.create(Plugin.prototype, {
        name:        { value: "Shockwave Flash",        configurable: false, enumerable: true, writable: false },
        description: { value: "Shockwave Flash 34.0 r0", configurable: false, enumerable: true, writable: false },
        filename:    { value: "pepflashplayer.dll",       configurable: false, enumerable: true, writable: false },
        length:      { value: 2,                          configurable: false, enumerable: true, writable: false },
      });

      // ---- Fake MimeType objects ----
      const swfMime = Object.create(MimeType.prototype, {
        type:          { value: "application/x-shockwave-flash", configurable: false, enumerable: true, writable: false },
        description:   { value: "Shockwave Flash",               configurable: false, enumerable: true, writable: false },
        suffixes:      { value: "swf",                            configurable: false, enumerable: true, writable: false },
        enabledPlugin: { value: flashPlugin,                      configurable: false, enumerable: true, writable: false },
      });
      const futureMime = Object.create(MimeType.prototype, {
        type:          { value: "application/futuresplash",  configurable: false, enumerable: true, writable: false },
        description:   { value: "Shockwave Flash",           configurable: false, enumerable: true, writable: false },
        suffixes:      { value: "spl",                        configurable: false, enumerable: true, writable: false },
        enabledPlugin: { value: flashPlugin,                  configurable: false, enumerable: true, writable: false },
      });

      // Make the plugin indexable by position (Plugin[0], Plugin[1]).
      Object.defineProperties(flashPlugin, {
        0: { value: swfMime,    configurable: false, enumerable: true, writable: false },
        1: { value: futureMime, configurable: false, enumerable: true, writable: false },
      });

      // ---- Patch PluginArray ----
      // Add the fake plugin at the next index and update length.
      const pluginIdx = navigator.plugins.length;
      const pluginProps = {
        length: { value: pluginIdx + 1, configurable: true, enumerable: true, writable: false },
      };
      pluginProps[pluginIdx] = { value: flashPlugin, configurable: false, enumerable: true, writable: false };
      Object.defineProperties(PluginArray.prototype, pluginProps);

      // Make it accessible by name (navigator.plugins["Shockwave Flash"]).
      navigator.plugins["Shockwave Flash"] = flashPlugin;

      // Patch namedItem() so Ruffle's namedItem("Shockwave Flash") lookup succeeds.
      const origPluginNamedItem = PluginArray.prototype.namedItem;
      PluginArray.prototype.namedItem = function (name) {
        if (name === "Shockwave Flash") return flashPlugin;
        return origPluginNamedItem.call(this, name);
      };

      // ---- Patch MimeTypeArray ----
      const mimeBase = navigator.mimeTypes.length;
      const mimeProps = {
        length: { value: mimeBase + 2, configurable: true, enumerable: true, writable: false },
      };
      mimeProps[mimeBase]     = { value: swfMime,    configurable: false, enumerable: true, writable: false };
      mimeProps[mimeBase + 1] = { value: futureMime, configurable: false, enumerable: true, writable: false };
      Object.defineProperties(MimeTypeArray.prototype, mimeProps);

      navigator.mimeTypes["application/x-shockwave-flash"] = swfMime;
      navigator.mimeTypes["application/futuresplash"]       = futureMime;

      const origMimeNamedItem = MimeTypeArray.prototype.namedItem;
      MimeTypeArray.prototype.namedItem = function (name) {
        if (name === "application/x-shockwave-flash") return swfMime;
        if (name === "application/futuresplash")       return futureMime;
        return origMimeNamedItem.call(this, name);
      };
    } catch (e) {
    }
  }

  ppSpoofFlash();

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
  // ExternalInterface support
  //
  // PepperFlash defines __flash__addCallback(instance, name) via
  // ExecuteScript.  The default implementation sets instance[name]
  // to a wrapper that calls instance.CallFunction(xml) synchronously.
  // Since our native messaging bridge is async, we override
  // __flash__addCallback after Flash defines it so that registered
  // callbacks send the invoke XML asynchronously.
  // -----------------------------------------------------------------

  /**
   * Set up CallFunction on a DOM element so that any code that calls
   * it directly still works (fire-and-forget, returns undefined).
   */
  function setupCallFunction(elem) {
    if (typeof elem.CallFunction === "function") {
      return;
    }
    elem.CallFunction = function (xml) {
      sendCallFunctionAsync(xml);
      // Cannot return synchronously via native messaging.
      return undefined;
    };
  }

  /**
   * Send a CallFunction XML invocation to the native host asynchronously
   * via the content script.
   */
  function sendCallFunctionAsync(xml) {
    comm.setAttribute("data-callfn", xml);
    comm.dispatchEvent(new CustomEvent("__flash_callfn"));
  }

  // -----------------------------------------------------------------
  // Flash element Proxy
  //
  // In real Chrome, the <object> element transparently proxied unknown
  // property/method accesses to the plugin's scriptable object.  We
  // replicate this with a JavaScript Proxy: any property not found on
  // the real DOM element returns a callable stub that builds
  // ExternalInterface invoke XML and sends it to PepperFlash.
  //
  // Supports multiple Flash instances: each <object>/<embed> gets its
  // own proxy, tracked by a WeakMap.
  // -----------------------------------------------------------------

  /** Elements whose prototype we've already patched in this execution. */
  const patchedElements = new WeakSet();

  /**
   * Build a function stub for an unknown property on a Flash element.
   * When called, it constructs ExternalInterface invoke XML and sends
   * it to the native host.
   */
  function makeExternalInterfaceStub(name) {
    return function () {
      const argsXml =
        typeof __flash__argumentsToXML === "function"
          ? __flash__argumentsToXML(arguments, 0)
          : "<arguments/>";
      const invokeXml =
        '<invoke name="' +
        name +
        '" returntype="javascript">' +
        argsXml +
        "</invoke>";
      sendCallFunctionAsync(invokeXml);
      return undefined;
    };
  }

  /**
   * Given an element with [data-flash-player], find the actual
   * <object> or <embed> that page JS references (by name/id).
   *
   * content.js sets data-flash-player on both the canvas AND the
   * original <object>/<embed>.  For <object>, the canvas is a child;
   * for <embed>, the canvas is a preceding sibling and the embed is
   * hidden.  We want the <object> or <embed>, not the <canvas>.
   */
  function resolveFlashContainer(elem) {
    const tag = elem.tagName;
    if (tag === "OBJECT" || tag === "EMBED") return elem;

    // elem is probably the <canvas>.  Look for a parent <object> or a
    // sibling <embed> that also has data-flash-player.
    if (elem.parentElement) {
      const parentTag = elem.parentElement.tagName;
      if (parentTag === "OBJECT" || parentTag === "EMBED") return elem.parentElement;
    }

    // Check next sibling (content.js inserts canvas before a hidden <embed>).
    const next = elem.nextElementSibling;
    if (next && (next.tagName === "EMBED" || next.tagName === "OBJECT") &&
        next.hasAttribute("data-flash-player")) {
      return next;
    }

    // Fallback: return the element itself.
    return elem;
  }

  /**
   * Patch a Flash <object>/<embed> element's prototype so that unknown
   * property accesses are forwarded to PepperFlash's scriptable object
   * via CallFunction (ExternalInterface).
   *
   * Instead of wrapping the element in a Proxy and overriding window
   * globals, we replace the element's prototype with a Proxy.  This
   * means window.game (via browser named-element resolution) returns
   * the real <object> element, but game.startup() still gets
   * intercepted and routed through ExternalInterface.
   */
  function proxyFlashElement(elem) {
    const container = resolveFlashContainer(elem);

    // If already patched (even by a previous page-script.js execution),
    // just record in this execution's WeakSet and return.
    if (patchedElements.has(container) || container.__flashProtoPatched) {
      patchedElements.add(container);
      return container;
    }

    const origProto = Object.getPrototypeOf(container);
    const proxyProto = new Proxy(origProto, {
      get(target, prop, receiver) {
        // Symbols always pass through (toString, iterator, etc.).
        if (typeof prop === "symbol") return Reflect.get(target, prop, receiver);

        // If the property exists in the original prototype chain, use it.
        // This covers all DOM properties/methods like tagName, setAttribute, etc.
        if (prop in target) return Reflect.get(target, prop, receiver);

        // Promise-related / serialisation — never proxy these.
        if (prop === "then" || prop === "toJSON") return undefined;

        // Unknown property — return an ExternalInterface stub.
        return makeExternalInterfaceStub(prop);
      },

      has(target, prop) {
        // Promise/serialisation probing should not match.
        if (prop === "then" || prop === "toJSON") return Reflect.has(target, prop);
        // Claim all string properties exist so that
        // "startup" in game returns true.
        if (typeof prop === "string") return true;
        return Reflect.has(target, prop);
      },
    });

    Object.setPrototypeOf(container, proxyProto);
    patchedElements.add(container);

    // Mark the element so a re-execution of page-script.js knows
    // the prototype is already patched (avoids nested proxies).
    try { container.__flashProtoPatched = true; } catch (_) {}

    // Register the element in the object store so that native-host
    // references to the owner element resolve correctly.
    registerJsObject(container);

    return container;
  }

  /**
   * Patch a single [data-flash-player] element: set up CallFunction
   * and patch its prototype for ExternalInterface.
   */
  function patchFlashElement(elem) {
    const container = resolveFlashContainer(elem);
    if (patchedElements.has(container) || container.__flashProtoPatched) {
      patchedElements.add(container);
      return;
    }
    setupCallFunction(container);
    proxyFlashElement(container);
  }

  /**
   * After every executeScript, check whether PepperFlash just defined the
   * __flash__addCallback / __flash__removeCallback helpers and replace
   * them with our async-bridge-aware versions.
   */
  function patchFlashCallbacks() {
    if (
      typeof window.__flash__addCallback === "function" &&
      !window.__flash__addCallback.__patched
    ) {
      window.__flash__addCallback = function (instance, name) {
        // Also set up CallFunction on the element as a fallback.
        if (instance && typeof instance === "object") {
          setupCallFunction(instance);
        }
        instance[name] = function () {
          // Build the invoke XML using Flash's own helper (defined in the
          // same script block).
          const argsXml =
            typeof __flash__argumentsToXML === "function"
              ? __flash__argumentsToXML(arguments, 0)
              : "<arguments/>";
          const invokeXml =
            '<invoke name="' +
            name +
            '" returntype="javascript">' +
            argsXml +
            "</invoke>";
          sendCallFunctionAsync(invokeXml);
          // Fire-and-forget — returning undefined for now.
          return undefined;
        };
      };
      window.__flash__addCallback.__patched = true;
    }

    if (
      typeof window.__flash__removeCallback === "function" &&
      !window.__flash__removeCallback.__patched
    ) {
      window.__flash__removeCallback = function (instance, name) {
        instance[name] = null;
      };
      window.__flash__removeCallback.__patched = true;
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

      case "getOwnerElement": {
        // Find the Flash <object> or <embed> element annotated by
        // content.js (it carries the data-flash-player attribute).
        // Prefer the actual <object>/<embed> over a <canvas>.
        let elem = document.querySelector(
          "object[data-flash-player], embed[data-flash-player]"
        );
        if (!elem) elem = document.querySelector("[data-flash-player]");
        if (!elem) {
          return { value: { type: "undefined" } };
        }
        const container = resolveFlashContainer(elem);
        // Ensure CallFunction is available and prototype is patched.
        patchFlashElement(container);
        return { value: encodeJsValue(container) };
      }

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
        if (obj == null) {
          return { value: { type: "undefined" } };
        }
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
        // Check whether Flash just defined its ExternalInterface helpers
        // and override them with our async-bridge-aware versions.
        patchFlashCallbacks();
        return { value: encodeJsValue(result) };
      }

      case "release": {
        if (req.obj !== 0) jsObjects.delete(req.obj);
        return null; // no response needed
      }

      default:
        console.warn("[flash] unknown script op: %s", op);
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
      console.error("[flash] handleRequest threw: %o", e);
      resp = { error: String(e) };
    }
    // Write response back (null means fire-and-forget, e.g. "release").
    comm.setAttribute("data-resp", resp ? JSON.stringify(resp) : "");
  });
  // Proactively proxy all Flash elements that already exist in the DOM
  // so that page JS can call methods (e.g. game.startup) even before
  // the native host calls getOwnerElement.
  //
  // 1) Elements already annotated by content.js:
  document.querySelectorAll("[data-flash-player]").forEach((el) => {
    patchFlashElement(el);
  });
  //
  // 2) Also scan <object> and <embed> tags that look like Flash content
  //    even if content.js hasn't processed them yet.
  document.querySelectorAll(
    'object[type="application/x-shockwave-flash"], ' +
    'object[classid="clsid:d27cdb6e-ae6d-11cf-96b8-444553540000"], ' +
    'embed[type="application/x-shockwave-flash"]'
  ).forEach((el) => {
    if (!patchedElements.has(el)) {
      patchFlashElement(el);
    }
  });

  // Watch for new [data-flash-player] elements added dynamically.
  // content.js may replace <object>/<embed> with <canvas> at any time.
  const flashObserver = new MutationObserver((mutations) => {
    for (const mut of mutations) {
      // Check added nodes for [data-flash-player].
      for (const node of mut.addedNodes) {
        if (node.nodeType !== Node.ELEMENT_NODE) continue;
        if (node.hasAttribute && node.hasAttribute("data-flash-player")) {
          patchFlashElement(node);
        }
        // Also check children (e.g. <object> with a <canvas> child inserted).
        if (node.querySelectorAll) {
          node.querySelectorAll("[data-flash-player]").forEach((child) => {
            patchFlashElement(child);
          });
        }
      }

      // Also watch for the attribute being set on an existing element.
      if (
        mut.type === "attributes" &&
        mut.attributeName === "data-flash-player" &&
        mut.target.hasAttribute("data-flash-player")
      ) {
        patchFlashElement(mut.target);
      }
    }
  });
  flashObserver.observe(document.documentElement, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ["data-flash-player"],
  });

  // Signal that the page script is ready.
  comm.setAttribute("data-ready", "1");
})();
