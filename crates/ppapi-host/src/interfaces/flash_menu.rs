//! PPB_Flash_Menu;0.2 implementation.
//!
//! Flash uses context menus for right-click menus (Settings, About, etc.).
//! When a `ContextMenuProvider` is set on the host, the menu items are
//! forwarded to the UI layer which displays a real context menu and
//! returns the selected item ID.  Without a provider, Show() returns
//! PP_ERROR_USERCANCEL (the menu is silently dismissed).

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
    tracing::trace!("PPB_Flash_Menu::Create(instance={})", instance_id);

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
    tracing::trace!("PPB_Flash_Menu::IsFlashMenu(resource={})", resource_id);
    let Some(host) = HOST.get() else { return PP_FALSE };
    pp_from_bool(host.resources.is_type(resource_id, "PPB_Flash_Menu"))
}

unsafe extern "C" fn show(
    menu_id: PP_Resource,
    location: *const PP_Point,
    selected_id: *mut i32,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::trace!("PPB_Flash_Menu::Show(menu_id={}, location={:?})", menu_id, location);
    let Some(host) = HOST.get() else { return PP_ERROR_FAILED };

    let loc = if location.is_null() {
        PP_Point { x: 0, y: 0 }
    } else {
        unsafe { *location }
    };

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

    // Try to use the context menu provider if available.
    let provider = host.get_context_menu_provider();

    // Grab the main-loop poster so we can deliver the completion callback
    // on the main thread, matching Chrome's behaviour.
    let poster = host.main_loop_poster.lock().clone();

    // Convert raw pointers to usize for Send safety.
    let selected_id_usize = selected_id as usize;

    // Signal that an interactive operation is pending so the Flash nested
    // message loop doesn't apply its safety-net timeout.
    host.pending_interactive_ops
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    crate::tokio_runtime().spawn_blocking(move || {
        let result = if let Some(ref provider) = provider {
            let ui_items = convert_items_to_ui(&items);
            match provider.show_context_menu(&ui_items, loc.x, loc.y) {
                Some(id) => {
                    if selected_id_usize != 0 {
                        unsafe { *(selected_id_usize as *mut i32) = id };
                    }
                    tracing::debug!("PPB_Flash_Menu::Show: user selected item id={}", id);
                    PP_OK
                }
                None => {
                    tracing::debug!("PPB_Flash_Menu::Show: user cancelled menu");
                    PP_ERROR_USERCANCEL
                }
            }
        } else {
            tracing::debug!("PPB_Flash_Menu::Show: no context menu provider, auto-cancel");
            PP_ERROR_USERCANCEL
        };

        // Interactive operation complete.
        if let Some(h) = HOST.get() {
            h.pending_interactive_ops
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }

        // Post the completion callback to the main message loop so Flash
        // receives it on the expected thread.
        if let Some(ref poster) = poster {
            poster.post_work(callback, 0, result);
        } else if let Some(func) = callback.func {
            unsafe { func(callback.user_data, result) };
        }
    });

    PP_OK_COMPLETIONPENDING
}

/// Convert internal MenuItem list to player_ui_traits ContextMenuItem list.
fn convert_items_to_ui(items: &[MenuItem]) -> Vec<player_ui_traits::ContextMenuItem> {
    items
        .iter()
        .map(|item| {
            let item_type = match item.type_ {
                PP_FLASH_MENUITEM_TYPE_NORMAL => player_ui_traits::ContextMenuItemType::Normal,
                PP_FLASH_MENUITEM_TYPE_CHECKBOX => player_ui_traits::ContextMenuItemType::Checkbox,
                PP_FLASH_MENUITEM_TYPE_SEPARATOR => {
                    player_ui_traits::ContextMenuItemType::Separator
                }
                PP_FLASH_MENUITEM_TYPE_SUBMENU => player_ui_traits::ContextMenuItemType::Submenu,
                _ => player_ui_traits::ContextMenuItemType::Normal,
            };
            player_ui_traits::ContextMenuItem {
                item_type,
                name: item.name.clone(),
                id: item.id,
                enabled: item.enabled,
                checked: item.checked,
                submenu: convert_items_to_ui(&item.submenu),
            }
        })
        .collect()
}
