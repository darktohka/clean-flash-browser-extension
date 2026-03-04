//! PPB interface implementations.
//!
//! Each sub-module provides a static vtable and the `extern "C"` functions
//! that implement one PPB interface.

pub mod audio;
pub mod audio_config;
pub mod audio_input;
pub mod audio_output;
pub mod broker;
pub mod browser_font;
pub mod buffer;
pub mod char_set;
pub mod core;
pub mod crypto;
pub mod cursor_control;
pub mod flash;
pub mod flash_clipboard;
pub mod flash_drm;
pub mod flash_file;
pub mod flash_fullscreen;
pub mod flash_message_loop;
pub mod graphics2d;
pub mod image_data;
pub mod ime_input_event;
pub mod input_event;
pub mod instance;
pub mod instance_private;
pub mod memory;
pub mod message_loop;
pub mod net_address;
pub mod network_monitor;
pub mod opengles2;
pub mod pdf;
pub mod printing;
pub mod stubs;
pub mod tcp_socket;
pub mod text_input;
pub mod udp_socket;
pub mod url_loader;
pub mod url_request_info;
pub mod url_response_info;
pub mod url_util;
pub mod var;
pub mod var_deprecated;
pub mod video_capture;
pub mod view;

use crate::InterfaceRegistry;

/// Register all implemented PPB interfaces into the given registry.
///
/// # Safety
/// The vtable pointers are static and valid for the program lifetime.
pub unsafe fn register_all(registry: &mut InterfaceRegistry) {
    unsafe {
        self::audio_config::register(registry);
        self::audio::register(registry);
        self::audio_input::register(registry);
        self::audio_output::register(registry);
        self::broker::register(registry);
        self::browser_font::register(registry);
        self::buffer::register(registry);
        self::char_set::register(registry);
        self::cursor_control::register(registry);
        self::core::register(registry);
        self::instance::register(registry);
        self::instance_private::register(registry);
        self::var::register(registry);
        self::view::register(registry);
        self::message_loop::register(registry);
        self::graphics2d::register(registry);
        self::image_data::register(registry);
        self::input_event::register(registry);
        self::url_loader::register(registry);
        self::url_request_info::register(registry);
        self::url_response_info::register(registry);
        self::memory::register(registry);
        self::crypto::register(registry);
        self::flash::register(registry);
        self::flash_drm::register(registry);
        self::flash_fullscreen::register(registry);
        self::flash_clipboard::register(registry);
        self::flash_file::register(registry);
        self::flash_message_loop::register(registry);
        self::opengles2::register(registry);
        self::printing::register(registry);
        self::url_util::register(registry);
        self::net_address::register(registry);
        self::network_monitor::register(registry);
        self::tcp_socket::register(registry);
        self::udp_socket::register(registry);
        self::var_deprecated::register(registry);
        self::pdf::register(registry);
        self::text_input::register(registry);
        self::ime_input_event::register(registry);
        self::video_capture::register(registry);
    }
    // Register stub vtables for all remaining required interfaces
    self::stubs::register(registry);
}
