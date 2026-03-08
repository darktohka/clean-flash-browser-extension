//! PPB_Flash_Menu;0.2 implementation.
//!
//! Flash uses context menus for right-click menus (Settings, About, etc.).
//! In a standalone player we don't need full GTK menu support. Instead we
//! parse the menu data, store it in a resource, and when Show() is called
//! we auto-select the first enabled normal item (or return USERCANCEL if none).
//!
//! This gives Flash a working code path without requiring GTK. Most Flash
//! content only uses the context menu for "Settings..." and "About" which
//! we can handle at the player level.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::CStr;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// A parsed menu item.
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub type_: PP_Flash_MenuItem_Type,
    pub name: String,
    pub id: i32,
    pub enabled: bool,
    pub checked: bool,
    pub submenu: Vec<MenuItem>,
}

pub struct FlashMenuResource {
    pub instance: PP_Instance,
    pub items: Vec<MenuItem>,
}

impl Resource for FlashMenuResource {
    fn resource_type(&self) -> &'static str {
        "PPB_Flash_Menu"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Menu parsing
// ---------------------------------------------------------------------------

/// Recursively parse the C PP_Flash_Menu tree into our owned MenuItem vec.
unsafe fn parse_menu(menu: *const PP_Flash_Menu) -> Vec<MenuItem> {
    if menu.is_null() {
        return Vec::new();
    }
    let m = unsafe { &*menu };
    let count = m.count as usize;
    let mut items = Vec::with_capacity(count);

    for i in 0..count {
        let item_ptr = unsafe { m.items.add(i) };
        if item_ptr.is_null() {
            continue;
        }
        let item = unsafe { &*item_ptr };
        let name = if item.name.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(item.name) }
                .to_string_lossy()
                .to_string()
        };

        let submenu = if item.type_ == PP_FLASH_MENUITEM_TYPE_SUBMENU {
            unsafe { parse_menu(item.submenu as *const PP_Flash_Menu) }
        } else {
            Vec::new()
        };

        items.push(MenuItem {
            type_: item.type_,
            name,
            id: item.id,
            enabled: pp_to_bool(item.enabled),
            checked: pp_to_bool(item.checked),
            submenu,
        });
    }

    items
}

/// Find the first enabled, normal (non-separator, non-submenu) menu item ID.
#[allow(dead_code)]
fn find_first_enabled_item(items: &[MenuItem]) -> Option<i32> {
    for item in items {
        match item.type_ {
            PP_FLASH_MENUITEM_TYPE_NORMAL | PP_FLASH_MENUITEM_TYPE_CHECKBOX => {
                if item.enabled {
                    return Some(item.id);
                }
            }
            PP_FLASH_MENUITEM_TYPE_SUBMENU => {
                if let Some(id) = find_first_enabled_item(&item.submenu) {
                    return Some(id);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Vtable
// ---------------------------------------------------------------------------

static VTABLE: PPB_Flash_Menu_0_2 = PPB_Flash_Menu_0_2 {
    Create: Some(create),
    IsFlashMenu: Some(is_flash_menu),
    Show: Some(show),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASH_MENU_INTERFACE_0_2, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(
    instance_id: PP_Instance,
    menu_data: *const PP_Flash_Menu,
) -> PP_Resource {
    tracing::debug!("PPB_Flash_Menu::Create(instance={})", instance_id);

    let Some(host) = HOST.get() else { return 0 };

    let items = unsafe { parse_menu(menu_data) };

    tracing::debug!(
        "PPB_Flash_Menu: parsed {} top-level items: {:?}",
        items.len(),
        items.iter().map(|i| &i.name).collect::<Vec<_>>()
    );

    let res = FlashMenuResource {
        instance: instance_id,
        items,
    };
    host.resources.insert(instance_id, Box::new(res))
}

unsafe extern "C" fn is_flash_menu(resource_id: PP_Resource) -> PP_Bool {
    let Some(host) = HOST.get() else { return PP_FALSE };
    pp_from_bool(host.resources.is_type(resource_id, "PPB_Flash_Menu"))
}

unsafe extern "C" fn show(
    menu_id: PP_Resource,
    location: *const PP_Point,
    selected_id: *mut i32,
    callback: PP_CompletionCallback,
) -> i32 {
    let Some(host) = HOST.get() else { return PP_ERROR_FAILED };

    let loc = if location.is_null() {
        PP_Point { x: 0, y: 0 }
    } else {
        unsafe { *location }
    };

    tracing::debug!(
        "PPB_Flash_Menu::Show(menu={}, location=({},{}))",
        menu_id, loc.x, loc.y
    );

    // Get the menu items
    let items = host.resources.with_downcast::<FlashMenuResource, _>(menu_id, |res| {
        res.items.clone()
    });

    let Some(items) = items else {
        tracing::error!("PPB_Flash_Menu::Show: bad resource {}", menu_id);
        return PP_ERROR_BADRESOURCE;
    };

    // Log the menu items for debugging
    for item in &items {
        tracing::debug!(
            "  Menu item: id={} name={:?} type={} enabled={}",
            item.id, item.name, item.type_, item.enabled
        );
    }

    // In a standalone player without GTK, we return USERCANCEL.
    // Flash will handle this gracefully.
    let result;
    if selected_id.is_null() {
        result = PP_ERROR_USERCANCEL;
    } else {
        // Auto-cancel: don't auto-select any menu item.
        // Flash context menus typically contain "Settings..." and "About"
        // which we don't need to handle.
        result = PP_ERROR_USERCANCEL;
    }

    // Fire the callback asynchronously
    // SAFETY: Convert raw pointers to usize for Send safety.
    if let Some(func) = callback.func {
        let user_data = callback.user_data as usize;
        crate::tokio_runtime().spawn_blocking(move || {
            unsafe { func(user_data as *mut std::ffi::c_void, result) };
        });
    }

    PP_OK_COMPLETIONPENDING
}
