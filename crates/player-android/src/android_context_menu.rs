//! Android context menu provider — shows menu via IPC and blocks for response.

use crate::ipc_transport::IpcTransport;
use crate::protocol::{tags, PayloadReader, PayloadWriter};
use player_ui_traits::{ContextMenuItem, ContextMenuItemType, ContextMenuProvider};
use std::sync::Arc;
use std::time::Duration;

const MENU_TIMEOUT: Duration = Duration::from_secs(120);

pub struct AndroidContextMenuProvider {
    ipc: Arc<IpcTransport>,
}

impl AndroidContextMenuProvider {
    pub fn new(ipc: Arc<IpcTransport>) -> Self {
        Self { ipc }
    }
}

fn serialize_menu_items(pw: &mut PayloadWriter, items: &[ContextMenuItem]) {
    pw.write_u32(items.len() as u32);
    for item in items {
        pw.write_u8(match item.item_type {
            ContextMenuItemType::Normal => 0,
            ContextMenuItemType::Checkbox => 1,
            ContextMenuItemType::Separator => 2,
            ContextMenuItemType::Submenu => 3,
        });
        pw.write_string(&item.name);
        pw.write_i32(item.id);
        pw.write_u8(if item.enabled { 1 } else { 0 });
        pw.write_u8(if item.checked { 1 } else { 0 });
        serialize_menu_items(pw, &item.submenu);
    }
}

impl ContextMenuProvider for AndroidContextMenuProvider {
    fn show_context_menu(&self, items: &[ContextMenuItem], x: i32, y: i32) -> Option<i32> {
        let mut pw = PayloadWriter::new();
        pw.write_i32(x);
        pw.write_i32(y);
        serialize_menu_items(&mut pw, items);

        let response = self
            .ipc
            .request_blocking(tags::CONTEXT_MENU_SHOW, pw.finish(), MENU_TIMEOUT)
            .ok()?;

        let mut pr = PayloadReader::new(&response.payload);
        let selected_id = pr.read_i32().ok()?;
        if selected_id < 0 {
            None
        } else {
            Some(selected_id)
        }
    }
}
