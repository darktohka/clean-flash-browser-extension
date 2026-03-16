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

  function installInlineModernSwfobject() {
    if (typeof window.swfobject !== "undefined") return;

    window.swfobject = (function () {
      var FLASH_MIME = "application/x-shockwave-flash";
      var doc = document;
      var win = window;

      var domLoadFns = [];
      var isDomLoaded = false;
      var autoHideShow = true;
      var encodeURIEnabled = false;

      function fireDomReady() {
        if (isDomLoaded) return;
        isDomLoaded = true;
        for (var i = 0; i < domLoadFns.length; i++) {
          domLoadFns[i]();
        }
        domLoadFns.length = 0;
      }

      if (doc.readyState === "complete" || doc.readyState === "interactive") {
        setTimeout(fireDomReady, 0);
      } else {
        doc.addEventListener("DOMContentLoaded", fireDomReady, false);
      }

      function addDomLoadEvent(fn) {
        if (isDomLoaded) {
          fn();
        } else {
          domLoadFns.push(fn);
        }
      }

      function addLoadEvent(fn) {
        win.addEventListener("load", fn, false);
      }

      function isElement(id) {
        return id && id.nodeType === 1;
      }

      function getEl(id) {
        if (isElement(id)) return id;
        try {
          return doc.getElementById(id);
        } catch (_) {
          return null;
        }
      }

      function getId(thing) {
        return isElement(thing) ? thing.id : thing;
      }

      function setVisibility(id, visible) {
        if (!autoHideShow) return;
        var el = getEl(id);
        if (el) {
          el.style.visibility = visible ? "visible" : "hidden";
        }
      }

      function createObjParam(el, name, value) {
        var p = doc.createElement("param");
        p.setAttribute("name", name);
        p.setAttribute("value", value);
        el.appendChild(p);
      }

      function createSWF(attObj, parObj, replaceElemIdStr) {
        var el = getEl(replaceElemIdStr);
        if (!el) return undefined;

        var o = doc.createElement("object");

        if (typeof attObj.id === "undefined") {
          attObj.id = getId(replaceElemIdStr);
        }

        for (var param in parObj) {
          if (parObj.hasOwnProperty(param) && param.toLowerCase() !== "movie") {
            createObjParam(o, param, parObj[param]);
          }
        }

        for (var attr in attObj) {
          if (attObj.hasOwnProperty(attr)) {
            var lower = attr.toLowerCase();
            if (lower === "styleclass") {
              o.setAttribute("class", attObj[attr]);
            } else if (lower !== "classid" && lower !== "data") {
              o.setAttribute(attr, attObj[attr]);
            }
          }
        }

        o.setAttribute("type", FLASH_MIME);
        o.setAttribute("data", attObj.data);

        el.parentNode.replaceChild(o, el);
        return o;
      }

      function removeSWF(id) {
        var obj = getEl(id);
        if (obj && obj.nodeName.toUpperCase() === "OBJECT") {
          obj.parentNode.removeChild(obj);
        }
      }

      var styleEl = null;

      function createCSS(sel, decl, media, newStyle) {
        var head = doc.getElementsByTagName("head")[0];
        if (!head) return;
        if (newStyle) styleEl = null;
        if (!styleEl) {
          styleEl = doc.createElement("style");
          styleEl.setAttribute("media", typeof media === "string" ? media : "screen");
          head.appendChild(styleEl);
        }
        styleEl.appendChild(doc.createTextNode(sel + " {" + decl + "}"));
      }

      function getQueryParamValue(param) {
        var q = doc.location.search || doc.location.hash;
        if (!q) return "";
        if (/\?/.test(q)) q = q.split("?")[1];
        if (!param) return q;
        var pairs = q.split("&");
        for (var i = 0; i < pairs.length; i++) {
          var idx = pairs[i].indexOf("=");
          if (idx !== -1 && pairs[i].substring(0, idx) === param) {
            return pairs[i].substring(idx + 1);
          }
        }
        return "";
      }

      function getObjectById(id) {
        var o = getEl(id);
        if (!o) return null;
        if (o.nodeName.toUpperCase() !== "OBJECT") return o;
        if (typeof o.SetVariable !== "undefined") return o;
        return o.getElementsByTagName("object")[0] || o;
      }

      return {
        registerObject: function (objectIdStr, _swfVersionStr, _xiSwfUrlStr, callbackFn) {
          addDomLoadEvent(function () {
            var el = getEl(objectIdStr);
            if (el) {
              setVisibility(objectIdStr, true);
              if (callbackFn) {
                var ref = getObjectById(objectIdStr);
                callbackFn({ success: !!ref, ref: ref || null, id: objectIdStr });
              }
            } else if (callbackFn) {
              callbackFn({ success: false, id: objectIdStr });
            }
          });
        },

        getObjectById: function (id) {
          return getObjectById(id);
        },

        embedSWF: function (
          swfUrlStr,
          replaceElemIdStr,
          widthStr,
          heightStr,
          _swfVersionStr,
          _xiSwfUrlStr,
          flashvarsObj,
          parObj,
          attObj,
          callbackFn
        ) {
          var id = getId(replaceElemIdStr);
          var callbackObj = { success: false, id: id };

          if (!swfUrlStr || !replaceElemIdStr || !widthStr || !heightStr) {
            if (callbackFn) callbackFn(callbackObj);
            return;
          }

          setVisibility(id, false);

          addDomLoadEvent(function () {
            widthStr += "";
            heightStr += "";

            var att = {};
            if (attObj && typeof attObj === "object") {
              for (var a in attObj) {
                if (attObj.hasOwnProperty(a)) att[a] = attObj[a];
              }
            }
            att.data = swfUrlStr;
            att.width = widthStr;
            att.height = heightStr;

            var par = {};
            if (parObj && typeof parObj === "object") {
              for (var p in parObj) {
                if (parObj.hasOwnProperty(p)) par[p] = parObj[p];
              }
            }

            if (flashvarsObj && typeof flashvarsObj === "object") {
              for (var k in flashvarsObj) {
                if (flashvarsObj.hasOwnProperty(k)) {
                  var key = encodeURIEnabled ? encodeURIComponent(k) : k;
                  var val = encodeURIEnabled ? encodeURIComponent(flashvarsObj[k]) : flashvarsObj[k];
                  par.flashvars = (par.flashvars ? par.flashvars + "&" : "") + key + "=" + val;
                }
              }
            }

            var obj = createSWF(att, par, replaceElemIdStr);
            if (obj) {
              if (att.id === id) setVisibility(id, true);
              callbackObj.success = true;
              callbackObj.ref = obj;
              callbackObj.id = obj.id;
            } else {
              setVisibility(id, true);
            }

            if (callbackFn) callbackFn(callbackObj);
          });
        },

        switchOffAutoHideShow: function () {
          autoHideShow = false;
        },

        enableUriEncoding: function (bool) {
          encodeURIEnabled = typeof bool === "undefined" ? true : bool;
        },

        ua: {
          w3: true,
          pv: [99, 0, 0],
          wk: false,
          ie: false,
          win: /win/i.test(navigator.platform),
          mac: /mac/i.test(navigator.platform)
        },

        getFlashPlayerVersion: function () {
          return { major: 99, minor: 0, release: 0 };
        },

        hasFlashPlayerVersion: function () {
          return true;
        },

        createSWF: function (attObj, parObj, replaceElemIdStr) {
          return createSWF(attObj, parObj, replaceElemIdStr);
        },

        showExpressInstall: function () {
        },

        removeSWF: function (id) {
          removeSWF(id);
        },

        createCSS: function (sel, decl, media, newStyle) {
          createCSS(sel, decl, media, newStyle);
        },

        addDomLoadEvent: addDomLoadEvent,

        addLoadEvent: addLoadEvent,

        getQueryParamValue: getQueryParamValue,

        expressInstallCallback: function () {
        },

        version: "2.3"
      };
    }());
  }

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

  installInlineModernSwfobject();
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
  // Per-instance element cache
  //
  // Maps instanceId (from data-flash-player attribute) to
  // { ownerElement, container } so we never need to querySelector
  // by attribute – critical when there are multiple Flash SWFs.
  // -----------------------------------------------------------------

  const flashInstances = new Map();

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
   * Resolve the canvas element for a specific Flash instance.
   * For <embed>, this ensures fullscreen/pointer-lock target the
   * rendered canvas rather than the hidden embed element.
   */
  function resolveFlashCanvas(instanceId) {
    const id = Number(instanceId);
    const cached = Number.isFinite(id) ? flashInstances.get(id) : null;
    if (cached) {
      if (cached.container && cached.container.querySelector) {
        const specific = cached.container.querySelector(
          'canvas[data-flash-player="' + id + '"]'
        );
        if (specific) return specific;
        const anyCanvas = cached.container.querySelector("canvas");
        if (anyCanvas) return anyCanvas;
      }
      if (cached.ownerElement && cached.ownerElement.querySelector) {
        const ownerCanvas = cached.ownerElement.querySelector("canvas");
        if (ownerCanvas) return ownerCanvas;
      }
    }

    // DOM fallback if cache is cold.
    const byId = document.querySelector(
      'canvas[data-flash-player="' + id + '"]'
    );
    if (byId) return byId;
    return document.querySelector("[data-flash-container] canvas") ||
           document.querySelector("canvas");
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

    // Cache element references keyed by instanceId so that request
    // handlers can find the right element without querySelector.
    const instId = elem.getAttribute && elem.getAttribute("data-flash-player");
    if (instId != null) {
      const id = Number(instId);
      // The container div is the element's parent (for <object>) or
      // the preceding sibling's parent (for <embed>).  Prefer the
      // element with data-flash-container if we can find it cheaply.
      let containerDiv = container.closest
        ? container.closest("[data-flash-container]")
        : null;
      if (!containerDiv && container.parentElement &&
          container.parentElement.hasAttribute &&
          container.parentElement.hasAttribute("data-flash-container")) {
        containerDiv = container.parentElement;
      }
      flashInstances.set(id, { ownerElement: container, container: containerDiv || container });
    }
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
        // Use the per-instance cache when an instanceId is provided
        // (supports multiple Flash SWFs on a page).
        const instId = req.instanceId;
        let cached = instId != null ? flashInstances.get(Number(instId)) : null;
        if (cached && cached.ownerElement) {
          patchFlashElement(cached.ownerElement);
          return { value: encodeJsValue(cached.ownerElement) };
        }
        // Fallback: query the DOM (single-instance or first load).
        let elem = instId != null
          ? document.querySelector(
              'object[data-flash-player="' + instId + '"], ' +
              'embed[data-flash-player="' + instId + '"]'
            )
          : null;
        if (!elem) elem = document.querySelector(
          "object[data-flash-player], embed[data-flash-player]"
        );
        if (!elem) elem = document.querySelector("[data-flash-player]");
        if (!elem) {
          return { value: { type: "undefined" } };
        }
        const container = resolveFlashContainer(elem);
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

      // ---------------------------------------------------------------
      // Clipboard operations for PPB_Flash_Clipboard
      // ---------------------------------------------------------------

      case "clipboardIsAvailable": {
        const fmt = req.format; // "plaintext" | "html" | "rtf"
        if (fmt === "rtf") return { value: encodeJsValue(false) };
        // We can always report plaintext/html as available if we have
        // data in our internal buffer, or attempt a read.
        if (window.__flashClipboard && window.__flashClipboard[fmt]) {
          return { value: encodeJsValue(true) };
        }
        // Try reading from the real clipboard via a hidden textarea.
        // This only works for plaintext during a user gesture.
        if (fmt === "plaintext") {
          const prevFocus = document.activeElement;
          try {
            const ta = document.createElement("textarea");
            ta.style.cssText = "position:fixed;left:-9999px;top:-9999px;opacity:0";
            document.body.appendChild(ta);
            ta.focus();
            const ok = document.execCommand("paste");
            const text = ta.value;
            document.body.removeChild(ta);
            if (prevFocus && prevFocus.focus) prevFocus.focus();
            if (ok && text.length > 0) return { value: encodeJsValue(true) };
          } catch (_) {
            if (prevFocus && prevFocus.focus) prevFocus.focus();
          }
        }
        return { value: encodeJsValue(false) };
      }

      case "clipboardRead": {
        const fmt = req.format; // "plaintext" | "html"
        // Try reading from the system clipboard first so that external
        // clipboard changes (from outside Flash) are picked up.
        if (fmt === "plaintext") {
          const prevFocus = document.activeElement;
          try {
            const ta = document.createElement("textarea");
            ta.style.cssText = "position:fixed;left:-9999px;top:-9999px;opacity:0";
            document.body.appendChild(ta);
            ta.focus();
            const ok = document.execCommand("paste");
            const text = ta.value;
            document.body.removeChild(ta);
            if (prevFocus && prevFocus.focus) prevFocus.focus();
            if (ok && text) return { value: encodeJsValue(text) };
          } catch (_) {
            if (prevFocus && prevFocus.focus) prevFocus.focus();
          }
        }
        // Fall back to our internal buffer (covers HTML and cases where
        // execCommand is unavailable).
        if (window.__flashClipboard && window.__flashClipboard[fmt]) {
          return { value: encodeJsValue(window.__flashClipboard[fmt]) };
        }
        return { value: encodeJsValue(null) };
      }

      case "clipboardWrite": {
        // Store in internal buffer for reads within the same page.
        if (!window.__flashClipboard) window.__flashClipboard = {};
        if (req.plaintext != null) window.__flashClipboard.plaintext = req.plaintext;
        else delete window.__flashClipboard.plaintext;
        if (req.html != null) window.__flashClipboard.html = req.html;
        else delete window.__flashClipboard.html;

        // Also write to the system clipboard.
        const text = req.plaintext || req.html || "";
        if (navigator.clipboard && navigator.clipboard.writeText) {
          // Modern Clipboard API — async, does not steal focus.
          navigator.clipboard.writeText(text).catch(() => {});
        } else {
          // Fallback: textarea + execCommand, with focus preservation.
          const prevFocus = document.activeElement;
          try {
            const ta = document.createElement("textarea");
            ta.style.cssText = "position:fixed;left:-9999px;top:-9999px;opacity:0";
            ta.value = text;
            document.body.appendChild(ta);
            ta.select();
            document.execCommand("copy");
            document.body.removeChild(ta);
          } catch (_) { /* ignore - clipboard write may fail without user gesture */ }
          if (prevFocus && prevFocus.focus) prevFocus.focus();
        }
        return { value: encodeJsValue(true) };
      }

      // ---------------------------------------------------------------
      // Fullscreen operations for PPB_FlashFullscreen / PPB_Fullscreen
      // ---------------------------------------------------------------

      case "fullscreenIsActive": {
        return { value: encodeJsValue(!!document.fullscreenElement) };
      }

      case "fullscreenSet": {
        const enter = !!req.fullscreen;
        try {
          if (enter) {
            // Prefer the instance canvas so fullscreen targets the rendered
            // Flash surface instead of a hidden <embed>.
            const fInstId = req.instanceId;
            const fCached = fInstId != null ? flashInstances.get(Number(fInstId)) : null;
            const canvasEl = fInstId != null ? resolveFlashCanvas(fInstId) : null;
            const el = canvasEl ||
                       (fCached && fCached.container) ||
                       document.querySelector("[data-flash-container]") ||
                       document.documentElement;
            if (el.requestFullscreen) {
              el.requestFullscreen();
            } else if (el.webkitRequestFullscreen) {
              el.webkitRequestFullscreen();
            }
          } else {
            if (document.exitFullscreen) {
              document.exitFullscreen();
            } else if (document.webkitExitFullscreen) {
              document.webkitExitFullscreen();
            }
          }
          return { value: encodeJsValue(true) };
        } catch (e) {
          console.warn("[flash] fullscreenSet failed:", e);
          return { value: encodeJsValue(false) };
        }
      }

      case "fullscreenGetScreenSize": {
        return {
          value: encodeJsValue({ w: screen.width, h: screen.height }),
        };
      }

      // ---------------------------------------------------------------
      // Cursor lock (Pointer Lock API) for PPB_CursorControl
      // ---------------------------------------------------------------

      case "cursorLock": {
        console.log("[flash] cursorLock requested");
        try {
          // Prefer the instance canvas so pointer lock is bound to the
          // rendered Flash surface.
          const cInstId = req.instanceId;
          const cCached = cInstId != null ? flashInstances.get(Number(cInstId)) : null;
          const canvasEl = cInstId != null ? resolveFlashCanvas(cInstId) : null;
          const el = canvasEl ||
                     (cCached && cCached.container) ||
                     document.querySelector("[data-flash-container]") ||
                     document.documentElement;
          if (el.requestPointerLock) {
            el.requestPointerLock();
            return { value: encodeJsValue(true) };
          }
          return { value: encodeJsValue(false) };
        } catch (e) {
          console.warn("[flash] cursorLock failed:", e);
          return { value: encodeJsValue(false) };
        }
      }

      case "cursorUnlock": {
        try {
          if (document.exitPointerLock) {
            document.exitPointerLock();
          }
          return { value: encodeJsValue(true) };
        } catch (e) {
          console.warn("[flash] cursorUnlock failed:", e);
          return { value: encodeJsValue(false) };
        }
      }

      case "hasCursorLock": {
        return { value: encodeJsValue(!!document.pointerLockElement) };
      }

      case "canLockCursor": {
        // Pointer lock is available when in fullscreen mode.
        const inFullscreen = !!(document.fullscreenElement || document.webkitFullscreenElement);
        return { value: encodeJsValue(inFullscreen) };
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
