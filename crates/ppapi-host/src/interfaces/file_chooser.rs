//! PPB_FileChooser(Dev);0.5/0.6 and PPB_FileChooserTrusted;0.5/0.6 implementation.
//!
//! Provides file chooser dialogs for Flash content. When Flash calls Show or
//! ShowWithoutUserGesture, we delegate to the `FileChooserProvider` trait
//! (implemented by the UI layer, e.g. via rfd in player-egui) to display
//! a native file picker. The chosen files are wrapped as PPB_FileRef resources
//! and returned via the PP_ArrayOutput callback.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::c_void;

use super::super::HOST;
use super::file_ref;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

pub struct FileChooserResource {
    pub instance: PP_Instance,
    pub mode: PP_FileChooserMode_Dev,
    pub accept_types: String,
}

impl Resource for FileChooserResource {
    fn resource_type(&self) -> &'static str {
        "PPB_FileChooser"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Vtables
// ---------------------------------------------------------------------------

static VTABLE_0_6: PPB_FileChooser_Dev_0_6 = PPB_FileChooser_Dev_0_6 {
    Create: Some(create),
    IsFileChooser: Some(is_file_chooser),
    Show: Some(show_0_6),
};

static VTABLE_0_5: PPB_FileChooser_Dev_0_5 = PPB_FileChooser_Dev_0_5 {
    Create: Some(create),
    IsFileChooser: Some(is_file_chooser),
    Show: Some(show_0_5),
    GetNextChosenFile: Some(get_next_chosen_file_0_5),
};

static VTABLE_TRUSTED_0_6: PPB_FileChooserTrusted_0_6 = PPB_FileChooserTrusted_0_6 {
    ShowWithoutUserGesture: Some(show_without_user_gesture_0_6),
};

static VTABLE_TRUSTED_0_5: PPB_FileChooserTrusted_0_5 = PPB_FileChooserTrusted_0_5 {
    ShowWithoutUserGesture: Some(show_without_user_gesture_0_5),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FILECHOOSER_DEV_INTERFACE_0_6, &VTABLE_0_6);
        registry.register(PPB_FILECHOOSER_DEV_INTERFACE_0_5, &VTABLE_0_5);
        registry.register(PPB_FILECHOOSER_TRUSTED_INTERFACE_0_6, &VTABLE_TRUSTED_0_6);
        registry.register_raw(
            PPB_FILECHOOSER_TRUSTED_INTERFACE_0_5,
            &VTABLE_TRUSTED_0_5 as *const _ as *const c_void,
        );
    }
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(
    instance: PP_Instance,
    mode: PP_FileChooserMode_Dev,
    accept_types: PP_Var,
) -> PP_Resource {
    tracing::trace!(
        "PPB_FileChooser::Create(instance={}, mode={}, accept_types={:?})",
        instance, mode, accept_types
    );
    let Some(host) = HOST.get() else { return 0 };

    let accept_str = host.vars.get_string(accept_types).unwrap_or_default();

    tracing::debug!(
        "PPB_FileChooser::Create(instance={}, mode={}, accept_types={:?})",
        instance, mode, accept_str
    );

    let res = FileChooserResource {
        instance,
        mode,
        accept_types: accept_str,
    };
    host.resources.insert(instance, Box::new(res))
}

unsafe extern "C" fn is_file_chooser(resource: PP_Resource) -> PP_Bool {
    tracing::trace!("PPB_FileChooser::IsFileChooser(resource={})", resource);
    let Some(host) = HOST.get() else { return PP_FALSE };
    pp_from_bool(host.resources.is_type(resource, "PPB_FileChooser"))
}

/// Show the file chooser dialog (0.6 API with PP_ArrayOutput).
unsafe extern "C" fn show_0_6(
    chooser: PP_Resource,
    output: PP_ArrayOutput,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_FileChooser::Show(chooser={})", chooser);
    do_show(chooser, PP_FALSE, "", output, callback)
}

/// Show the file chooser dialog (0.5 API without PP_ArrayOutput).
unsafe extern "C" fn show_0_5(
    chooser: PP_Resource,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_FileChooser(0.5)::Show(chooser={})", chooser);
    // We don't maintain a 0.5-style chosen-file iterator state yet.
    // Complete asynchronously with cancel, rather than exposing an ABI-mismatched vtable.
    fire_callback(callback, PP_ERROR_USERCANCEL);
    PP_OK_COMPLETIONPENDING
}

/// 0.5 API iterator accessor.
unsafe extern "C" fn get_next_chosen_file_0_5(_chooser: PP_Resource) -> PP_Resource {
    tracing::trace!("PPB_FileChooser(0.5)::GetNextChosenFile(chooser={})", _chooser);
    0
}

/// ShowWithoutUserGesture (0.6 API with PP_ArrayOutput).
unsafe extern "C" fn show_without_user_gesture_0_6(
    chooser: PP_Resource,
    save_as: PP_Bool,
    suggested_file_name: PP_Var,
    output: PP_ArrayOutput,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!(
        "PPB_FileChooserTrusted::ShowWithoutUserGesture(chooser={}, save_as={}, suggested_file_name={:?})",
        chooser, save_as, suggested_file_name
    );
    let Some(host) = HOST.get() else { return PP_ERROR_FAILED };
    let suggested = host.vars.get_string(suggested_file_name).unwrap_or_default();

    tracing::debug!(
        "PPB_FileChooserTrusted::ShowWithoutUserGesture(chooser={}, save_as={}, suggested={:?})",
        chooser, save_as, suggested
    );

    do_show(chooser, save_as, &suggested, output, callback)
}

/// ShowWithoutUserGesture (0.5 API — no PP_ArrayOutput, we ignore).
unsafe extern "C" fn show_without_user_gesture_0_5(
    chooser: PP_Resource,
    save_as: PP_Bool,
    suggested_file_name: PP_Var,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!(
        "PPB_FileChooserTrusted(0.5)::ShowWithoutUserGesture(chooser={}, save_as={}, suggested_file_name={:?})",
        chooser, save_as, suggested_file_name
    );
    let Some(host) = HOST.get() else { return PP_ERROR_FAILED };
    let _suggested = host.vars.get_string(suggested_file_name).unwrap_or_default();

    tracing::debug!(
        "PPB_FileChooserTrusted(0.5)::ShowWithoutUserGesture(chooser={}, save_as={})",
        chooser, save_as
    );

    // 0.5 doesn't have ArrayOutput; fire callback with USERCANCEL for now.
    fire_callback(callback, PP_ERROR_USERCANCEL);
    PP_OK_COMPLETIONPENDING
}

/// Common implementation for showing a file chooser.
fn do_show(
    chooser: PP_Resource,
    save_as: PP_Bool,
    suggested_name: &str,
    output: PP_ArrayOutput,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!(
        "PPB_FileChooser::do_show(chooser={}, save_as={}, suggested_name={:?})",
        chooser, save_as, suggested_name
    );
    let Some(host) = HOST.get() else { return PP_ERROR_FAILED };

    // Read chooser resource data
    let (instance, mode, accept_types) = match host.resources.with_downcast::<FileChooserResource, _>(chooser, |fc| {
        (fc.instance, fc.mode, fc.accept_types.clone())
    }) {
        Some(data) => data,
        None => {
            tracing::error!("PPB_FileChooser::Show: bad resource {}", chooser);
            return PP_ERROR_BADRESOURCE;
        }
    };

    // Determine mode
    let ui_mode = if pp_to_bool(save_as) {
        player_ui_traits::FileChooserMode::Save
    } else if mode == PP_FILECHOOSERMODE_OPENMULTIPLE {
        player_ui_traits::FileChooserMode::OpenMultiple
    } else {
        player_ui_traits::FileChooserMode::Open
    };

    let suggested_name = suggested_name.to_string();
    let accept_types_clone = accept_types.clone();

    // Spawn a blocking task for the file dialog so we don't block the plugin thread.
    // The callback will be fired from this task.
    // SAFETY: PP_CompletionCallback contains raw pointers that the plugin expects
    // to be called back on; we trust the plugin's threading model here.
    let cb_func = callback.func;
    let cb_user_data = callback.user_data as usize; // convert to usize for Send
    let output_get = output.GetDataBuffer;
    let output_user = output.user_data as usize;
    crate::tokio_runtime().spawn_blocking(move || {
        let Some(host) = HOST.get() else { return };

        let chosen_files = {
            let provider = host.file_chooser_provider.lock();
            match provider.as_ref() {
                Some(p) => p.show_file_chooser(ui_mode, &accept_types_clone, &suggested_name),
                None => {
                    tracing::warn!("No FileChooserProvider set — returning cancel");
                    Vec::new()
                }
            }
        };

        let result_code;
        if chosen_files.is_empty() {
            result_code = PP_ERROR_USERCANCEL;
        } else {
            result_code = PP_OK;

            // Allocate the output array
            if let Some(get_data_buffer) = output_get {
                let count = chosen_files.len() as u32;
                let buf = unsafe {
                    get_data_buffer(
                        output_user as *mut c_void,
                        count,
                        std::mem::size_of::<PP_Resource>() as u32,
                    )
                };
                if !buf.is_null() {
                    let file_refs = buf as *mut PP_Resource;
                    for (i, path) in chosen_files.iter().enumerate() {
                        let fr = file_ref::create_file_ref_unrestricted(path, instance);
                        unsafe { *file_refs.add(i) = fr };
                    }
                }
            }
        }

        if let Some(func) = cb_func {
            unsafe { func(cb_user_data as *mut c_void, result_code) };
        }
    });

    PP_OK_COMPLETIONPENDING
}

/// Fire a completion callback on the current thread.
fn fire_callback(callback: PP_CompletionCallback, result: i32) {
    if let Some(func) = callback.func {
        unsafe { func(callback.user_data, result) };
    }
}
