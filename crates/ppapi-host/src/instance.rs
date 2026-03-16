//! Plugin instance state tracking.
//!
//! Each `PP_Instance` corresponds to one Flash embed. In our projector there
//! is exactly one instance, but the architecture supports tracking multiple.

use ppapi_sys::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};

/// Per-instance state.
pub struct PluginInstance {
    /// The instance ID.
    pub id: PP_Instance,
    /// Bound Graphics2D resource (from BindGraphics).
    pub bound_graphics_2d: PP_Resource,
    /// Bound Graphics3D resource (from BindGraphics).
    pub bound_graphics_3d: PP_Resource,
    /// View rect (set by DidChangeView).
    pub view_rect: PP_Rect,
    /// Whether the instance has focus.
    pub has_focus: bool,
    /// Input event classes requested by the plugin.
    pub requested_input_events: u32,
    /// Input event classes requested for filtering.
    pub filtering_input_events: u32,
    /// Whether the instance is in fullscreen mode.
    pub is_fullscreen: bool,
    /// Whether the cursor is currently locked (pointer lock active).
    pub has_cursor_lock: bool,
    /// The SWF URL being loaded.
    pub swf_url: Option<String>,
    /// The page URL (document URL) for relative URL resolution.
    /// Eagerly cached at instance creation so it's always available
    /// even if the url_provider becomes unreachable.
    pub page_url: Option<String>,
    /// Whether a Graphics2D flush is in progress (only one at a time).
    pub graphics_2d_in_progress: bool,
    /// Whether a Graphics3D swap is in progress (only one at a time).
    pub graphics_3d_in_progress: bool,
    /// Stored completion callback for the in-flight flush.
    pub flush_callback: Option<PP_CompletionCallback>,
}

impl PluginInstance {
    pub fn new(id: PP_Instance) -> Self {
        Self {
            id,
            bound_graphics_2d: 0,
            bound_graphics_3d: 0,
            view_rect: PP_Rect::default(),
            has_focus: false,
            requested_input_events: 0,
            filtering_input_events: 0,
            is_fullscreen: false,
            has_cursor_lock: false,
            swf_url: None,
            page_url: None,
            graphics_2d_in_progress: false,
            graphics_3d_in_progress: false,
            flush_callback: None,
        }
    }

    /// Returns the "primary" bound graphics resource for legacy callers.
    /// Prefers 3D if both are bound, since 3D is the display surface.
    pub fn bound_graphics(&self) -> PP_Resource {
        if self.bound_graphics_3d != 0 {
            self.bound_graphics_3d
        } else {
            self.bound_graphics_2d
        }
    }
}

/// Manages all active plugin instances.
pub struct InstanceManager {
    next_id: AtomicI32,
    instances: parking_lot::Mutex<HashMap<PP_Instance, PluginInstance>>,
}

impl InstanceManager {
    pub fn new() -> Self {
        Self {
            next_id: AtomicI32::new(1),
            instances: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    /// Allocate a new instance ID and store a fresh PluginInstance.
    pub fn create_instance(&self) -> PP_Instance {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let inst = PluginInstance::new(id);
        self.instances.lock().insert(id, inst);
        id
    }

    /// Access an instance by ID.
    pub fn with_instance<R>(&self, id: PP_Instance, f: impl FnOnce(&PluginInstance) -> R) -> Option<R> {
        self.instances.lock().get(&id).map(f)
    }

    /// Mutably access an instance by ID.
    pub fn with_instance_mut<R>(
        &self,
        id: PP_Instance,
        f: impl FnOnce(&mut PluginInstance) -> R,
    ) -> Option<R> {
        self.instances.lock().get_mut(&id).map(f)
    }

    /// Remove an instance.
    pub fn destroy_instance(&self, id: PP_Instance) {
        self.instances.lock().remove(&id);
    }

    /// Check if an instance exists.
    pub fn exists(&self, id: PP_Instance) -> bool {
        self.instances.lock().contains_key(&id)
    }
}

impl Default for InstanceManager {
    fn default() -> Self {
        Self::new()
    }
}
