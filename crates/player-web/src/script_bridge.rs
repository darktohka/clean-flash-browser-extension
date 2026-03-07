//! Script bridge — `ScriptProvider` implementation for the web player.
//!
//! Sends scripting requests to the Chrome Extension content script via the
//! native-messaging binary protocol (using [`protocol::send_host_message`])
//! and blocks until the content script sends back a JSON response over
//! stdin.  The stdin-reader thread recognises `"jsResponse"` messages and
//! routes them here instead of passing them to the normal command handler.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use parking_lot::Mutex;
use player_ui_traits::{JsValue, ScriptProvider};
use serde_json::{json, Value};

use crate::protocol;

// ===========================================================================
// ScriptBridge — shared state between the ScriptProvider and the stdin reader
// ===========================================================================

/// Shared state that connects the blocking `ScriptProvider` methods
/// (called from arbitrary PPAPI threads) with the background stdin-reader
/// thread that delivers `jsResponse` messages.
#[derive(Debug)]
pub struct ScriptBridge {
    next_id: AtomicU32,
    /// Pending request-id → oneshot sender.
    pending: Mutex<HashMap<u32, mpsc::Sender<Value>>>,
}

impl ScriptBridge {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU32::new(1),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Called by the stdin-reader thread when a `"jsResponse"` JSON message
    /// arrives.  Routes the response to the waiting `ScriptProvider` method.
    pub fn handle_response(&self, msg: &Value) {
        let id = match msg.get("id").and_then(|v| v.as_u64()) {
            Some(id) => id as u32,
            None => {
                tracing::warn!("jsResponse missing 'id': {:?}", msg);
                return;
            }
        };

        let mut pending = self.pending.lock();
        if let Some(tx) = pending.remove(&id) {
            let _ = tx.send(msg.clone());
        } else {
            tracing::warn!("jsResponse for unknown id {}: {:?}", id, msg);
        }
    }

    /// Send a scripting request and block until the browser responds.
    ///
    /// Returns the full JSON response (the caller picks out the fields it
    /// needs) or `None` on timeout / send failure.
    fn request(&self, payload: Value) -> Option<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        // Merge the request id into the payload.
        let mut obj = match payload {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        obj.insert("id".into(), json!(id));

        let json_str = Value::Object(obj).to_string();

        // Register the pending waiter *before* sending so we don't miss
        // a fast response.
        let (tx, rx) = mpsc::channel();
        self.pending.lock().insert(id, tx);

        // Send via the binary protocol (TAG_SCRIPT).
        if let Err(e) = protocol::send_host_message(&protocol::HostMessage::ScriptRequest(&json_str)) {
            tracing::error!("failed to send ScriptRequest: {}", e);
            self.pending.lock().remove(&id);
            return None;
        }

        // Block until the response arrives (with a generous timeout so
        // slow pages don't break, but we don't hang forever).
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(val) => Some(val),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                tracing::error!("ScriptRequest {} timed out", id);
                self.pending.lock().remove(&id);
                None
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                tracing::error!("ScriptRequest {} channel disconnected", id);
                None
            }
        }
    }
}

// ===========================================================================
// JSON ↔ JsValue helpers
// ===========================================================================

/// Encode a `JsValue` as a JSON `Value` for sending to the content script.
pub fn js_value_to_json(val: &JsValue) -> Value {
    match val {
        JsValue::Undefined => json!({"type": "undefined"}),
        JsValue::Null => json!({"type": "null"}),
        JsValue::Bool(b) => json!({"type": "bool", "v": b}),
        JsValue::Int(i) => json!({"type": "int", "v": i}),
        JsValue::Double(d) => json!({"type": "double", "v": d}),
        JsValue::String(s) => json!({"type": "string", "v": s}),
        JsValue::Object(id) => json!({"type": "object", "v": id}),
    }
}

/// Decode a JSON `Value` (from the content script) into a `JsValue`.
pub fn json_to_js_value(v: &Value) -> JsValue {
    let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("undefined");
    match ty {
        "undefined" => JsValue::Undefined,
        "null" => JsValue::Null,
        "bool" => JsValue::Bool(v.get("v").and_then(|x| x.as_bool()).unwrap_or(false)),
        "int" => JsValue::Int(v.get("v").and_then(|x| x.as_i64()).unwrap_or(0) as i32),
        "double" => JsValue::Double(v.get("v").and_then(|x| x.as_f64()).unwrap_or(0.0)),
        "string" => {
            JsValue::String(v.get("v").and_then(|x| x.as_str()).unwrap_or("").to_string())
        }
        "object" => JsValue::Object(v.get("v").and_then(|x| x.as_u64()).unwrap_or(0)),
        other => {
            tracing::warn!("unknown JsValue type {:?}", other);
            JsValue::Undefined
        }
    }
}

// ===========================================================================
// ScriptProvider implementation
// ===========================================================================

/// `ScriptProvider` backed by the Chrome Extension native-messaging bridge.
///
/// Each method serialises a JSON request, sends it over the saved stdout
/// (binary protocol tag 0x10), then blocks until the stdin reader delivers
/// the matching `jsResponse`.
pub struct WebScriptProvider {
    bridge: std::sync::Arc<ScriptBridge>,
}

impl WebScriptProvider {
    pub fn new(bridge: std::sync::Arc<ScriptBridge>) -> Self {
        Self { bridge }
    }
}

impl ScriptProvider for WebScriptProvider {
    fn get_window_object(&self) -> JsValue {
        let resp = self.bridge.request(json!({"op": "getWindow"}));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .map(json_to_js_value)
            .unwrap_or(JsValue::Undefined)
    }

    fn get_owner_element(&self) -> JsValue {
        let resp = self.bridge.request(json!({"op": "getOwnerElement"}));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .map(json_to_js_value)
            .unwrap_or(JsValue::Undefined)
    }

    fn has_property(&self, object_id: u64, name: &str) -> bool {
        let resp = self.bridge.request(json!({
            "op": "hasProperty",
            "obj": object_id,
            "name": name,
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn has_method(&self, object_id: u64, name: &str) -> bool {
        let resp = self.bridge.request(json!({
            "op": "hasMethod",
            "obj": object_id,
            "name": name,
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn get_property(&self, object_id: u64, name: &str) -> JsValue {
        let resp = self.bridge.request(json!({
            "op": "getProperty",
            "obj": object_id,
            "name": name,
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .map(json_to_js_value)
            .unwrap_or(JsValue::Undefined)
    }

    fn set_property(&self, object_id: u64, name: &str, value: &JsValue) {
        let _ = self.bridge.request(json!({
            "op": "setProperty",
            "obj": object_id,
            "name": name,
            "value": js_value_to_json(value),
        }));
    }

    fn remove_property(&self, object_id: u64, name: &str) {
        let _ = self.bridge.request(json!({
            "op": "removeProperty",
            "obj": object_id,
            "name": name,
        }));
    }

    fn get_all_property_names(&self, object_id: u64) -> Vec<String> {
        let resp = self.bridge.request(json!({
            "op": "getAllPropertyNames",
            "obj": object_id,
        }));
        resp.as_ref()
            .and_then(|r| r.get("names"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn call_method(
        &self,
        object_id: u64,
        method_name: &str,
        args: &[JsValue],
    ) -> Result<JsValue, String> {
        let js_args: Vec<Value> = args.iter().map(js_value_to_json).collect();
        let resp = self.bridge.request(json!({
            "op": "callMethod",
            "obj": object_id,
            "method": method_name,
            "args": js_args,
        }));
        match resp {
            Some(r) => {
                if let Some(err) = r.get("error").and_then(|e| e.as_str()) {
                    Err(err.to_string())
                } else {
                    Ok(r.get("value").map(json_to_js_value).unwrap_or(JsValue::Undefined))
                }
            }
            None => Err("no response from browser".into()),
        }
    }

    fn call(&self, object_id: u64, args: &[JsValue]) -> Result<JsValue, String> {
        let js_args: Vec<Value> = args.iter().map(js_value_to_json).collect();
        let resp = self.bridge.request(json!({
            "op": "call",
            "obj": object_id,
            "args": js_args,
        }));
        match resp {
            Some(r) => {
                if let Some(err) = r.get("error").and_then(|e| e.as_str()) {
                    Err(err.to_string())
                } else {
                    Ok(r.get("value").map(json_to_js_value).unwrap_or(JsValue::Undefined))
                }
            }
            None => Err("no response from browser".into()),
        }
    }

    fn construct(&self, object_id: u64, args: &[JsValue]) -> Result<JsValue, String> {
        let js_args: Vec<Value> = args.iter().map(js_value_to_json).collect();
        let resp = self.bridge.request(json!({
            "op": "construct",
            "obj": object_id,
            "args": js_args,
        }));
        match resp {
            Some(r) => {
                if let Some(err) = r.get("error").and_then(|e| e.as_str()) {
                    Err(err.to_string())
                } else {
                    Ok(r.get("value").map(json_to_js_value).unwrap_or(JsValue::Undefined))
                }
            }
            None => Err("no response from browser".into()),
        }
    }

    fn execute_script(&self, script: &str) -> Result<JsValue, String> {
        let resp = self.bridge.request(json!({
            "op": "executeScript",
            "script": script,
        }));
        match resp {
            Some(r) => {
                if let Some(err) = r.get("error").and_then(|e| e.as_str()) {
                    Err(err.to_string())
                } else {
                    Ok(r.get("value").map(json_to_js_value).unwrap_or(JsValue::Undefined))
                }
            }
            None => Err("no response from browser".into()),
        }
    }

    fn release_object(&self, object_id: u64) {
        // Fire-and-forget — no response needed.
        let json_str = json!({
            "id": 0,
            "op": "release",
            "obj": object_id,
        })
        .to_string();

        let _ = protocol::send_host_message(&protocol::HostMessage::ScriptRequest(&json_str));
    }
}
