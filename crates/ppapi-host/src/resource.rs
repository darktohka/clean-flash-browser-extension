//! Resource manager - ID-based table for browser-side resources.
//!
//! Each PPB resource (Graphics2D, ImageData, Audio, URLLoader, etc.) is stored
//! as a `Box<dyn Resource>` and assigned a unique `PP_Resource` integer handle.
//! Resources are reference-counted; when the count drops to zero, the resource
//! is removed from the table.

use parking_lot::Mutex;
use ppapi_sys::PP_Resource;
use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};


/// Trait that all PPB resources implement.
pub trait Resource: Any + Send + Sync {
    /// The PPAPI interface name, e.g. "PPB_Graphics2D".
    fn resource_type(&self) -> &'static str;

    /// Downcast helper - returns self as `&dyn Any`.
    fn as_any(&self) -> &dyn Any;

    /// Downcast helper - returns self as `&mut dyn Any`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// A table entry wrapping a resource with its reference count.
pub struct ResourceEntry {
    pub resource: Box<dyn Resource>,
    pub ref_count: i32,
    pub instance: ppapi_sys::PP_Instance,
}

/// Global resource manager: assigns IDs and stores all live resources.
pub struct ResourceManager {
    next_id: AtomicI32,
    resources: Mutex<HashMap<PP_Resource, ResourceEntry>>,
}

impl ResourceManager {
    /// Create a new empty resource manager.
    pub fn new() -> Self {
        Self {
            // Start at 1; 0 is an invalid resource.
            next_id: AtomicI32::new(1),
            resources: Mutex::new(HashMap::new()),
        }
    }

    /// Insert a new resource into the table with ref_count=1.
    /// Returns the assigned PP_Resource handle.
    pub fn insert(
        &self,
        instance: ppapi_sys::PP_Instance,
        resource: Box<dyn Resource>,
    ) -> PP_Resource {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let entry = ResourceEntry {
            resource,
            ref_count: 1,
            instance,
        };
        self.resources.lock().insert(id, entry);
        id
    }

    /// Increment the reference count of a resource.
    pub fn add_ref(&self, id: PP_Resource) {
        if let Some(entry) = self.resources.lock().get_mut(&id) {
            entry.ref_count += 1;
        }
    }

    /// Decrement the reference count. Removes the resource if it reaches 0.
    pub fn release(&self, id: PP_Resource) {
        let mut map = self.resources.lock();
        let should_remove = if let Some(entry) = map.get_mut(&id) {
            entry.ref_count -= 1;
            entry.ref_count <= 0
        } else {
            false
        };
        if should_remove {
            tracing::trace!("Resource {} ref count reached zero, removing", id);
            map.remove(&id);
        }
    }

    /// Get a reference to a resource, executing a closure with it.
    pub fn with_resource<R>(&self, id: PP_Resource, f: impl FnOnce(&ResourceEntry) -> R) -> Option<R> {
        let map = self.resources.lock();
        map.get(&id).map(f)
    }

    /// Get a mutable reference to a resource, executing a closure with it.
    pub fn with_resource_mut<R>(
        &self,
        id: PP_Resource,
        f: impl FnOnce(&mut ResourceEntry) -> R,
    ) -> Option<R> {
        let mut map = self.resources.lock();
        map.get_mut(&id).map(f)
    }

    /// Attempt to downcast a resource to a concrete type and run a closure.
    pub fn with_downcast<T: 'static, R>(
        &self,
        id: PP_Resource,
        f: impl FnOnce(&T) -> R,
    ) -> Option<R> {
        let map = self.resources.lock();
        map.get(&id).and_then(|entry| {
            entry.resource.as_any().downcast_ref::<T>().map(f)
        })
    }

    /// Attempt to downcast a resource to a concrete mutable type and run a closure.
    pub fn with_downcast_mut<T: 'static, R>(
        &self,
        id: PP_Resource,
        f: impl FnOnce(&mut T) -> R,
    ) -> Option<R> {
        let mut map = self.resources.lock();
        map.get_mut(&id).and_then(|entry| {
            entry.resource.as_any_mut().downcast_mut::<T>().map(f)
        })
    }

    /// Access two *distinct* resources simultaneously: `src_id` immutably as `S`
    /// and `dst_id` mutably as `D`, without cloning either buffer.
    ///
    /// # Panics
    /// Panics if `src_id == dst_id`.
    pub fn with_downcast_pair<S: 'static, D: 'static, R>(
        &self,
        src_id: PP_Resource,
        dst_id: PP_Resource,
        f: impl FnOnce(&S, &mut D) -> R,
    ) -> Option<R> {
        assert_ne!(src_id, dst_id, "source and destination resources must differ");
        let mut map = self.resources.lock();
        // Use a raw pointer to the map to obtain two non-overlapping
        // references (one shared, one exclusive) to entries at different
        // keys.  This is sound because HashMap entries at distinct keys
        // occupy disjoint memory.
        let map_ptr: *mut HashMap<PP_Resource, ResourceEntry> = &mut *map;
        unsafe {
            let src_entry = (*map_ptr).get(&src_id)?;
            let dst_entry = (*map_ptr).get_mut(&dst_id)?;
            let src = src_entry.resource.as_any().downcast_ref::<S>()?;
            let dst = dst_entry.resource.as_any_mut().downcast_mut::<D>()?;
            Some(f(src, dst))
        }
    }

    /// Check if a resource exists and is of the expected type.
    pub fn is_type(&self, id: PP_Resource, type_name: &str) -> bool {
        self.with_resource(id, |entry| entry.resource.resource_type() == type_name)
            .unwrap_or(false)
    }

    /// Get the instance associated with a resource.
    pub fn get_instance(&self, id: PP_Resource) -> Option<ppapi_sys::PP_Instance> {
        self.with_resource(id, |entry| entry.instance)
    }

    /// Remove all resources belonging to a given instance.
    pub fn remove_instance_resources(&self, instance: ppapi_sys::PP_Instance) {
        let mut map = self.resources.lock();
        map.retain(|_, entry| entry.instance != instance);
    }

    /// Collect the IDs of all live resources whose `resource_type()` matches
    /// the given name.  The returned vec can be iterated *outside* the lock
    /// and each ID accessed individually via `with_downcast` / `with_downcast_mut`.
    pub fn ids_by_type(&self, type_name: &str) -> Vec<PP_Resource> {
        let map = self.resources.lock();
        map.iter()
            .filter(|(_, entry)| entry.resource.resource_type() == type_name)
            .map(|(&id, _)| id)
            .collect()
    }

    /// Number of live resources.
    pub fn len(&self) -> usize {
        self.resources.lock().len()
    }

    /// Whether there are no live resources.
    pub fn is_empty(&self) -> bool {
        self.resources.lock().is_empty()
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}
