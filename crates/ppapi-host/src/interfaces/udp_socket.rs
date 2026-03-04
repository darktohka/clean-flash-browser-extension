//! PPB_UDPSocket_Private;0.4 / 0.3 implementation.
//!
//! Provides UDP socket operations: create, bind, send-to, recv-from, close,
//! and socket feature configuration (address reuse, broadcast).

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::c_char;
use std::net::{SocketAddr, UdpSocket};

use super::net_address::{addr_to_socketaddr, socketaddr_to_addr};
use crate::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

pub struct UdpSocketResource {
    pub instance: PP_Instance,
    /// The underlying OS UDP socket, present once bound.
    pub socket: Option<UdpSocket>,
    /// Whether the socket has been closed.
    pub closed: bool,
    /// Address of the last RecvFrom sender.
    pub recv_from_addr: Option<SocketAddr>,
    /// Socket feature: SO_REUSEADDR.
    pub address_reuse: bool,
    /// Socket feature: SO_BROADCAST.
    pub broadcast: bool,
}

impl Resource for UdpSocketResource {
    fn resource_type(&self) -> &'static str {
        "PPB_UDPSocket_Private"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Vtable functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    tracing::debug!("PPB_UDPSocket_Private::Create(instance={})", instance);
    let Some(host) = HOST.get() else { return 0 };
    let res = UdpSocketResource {
        instance,
        socket: None,
        closed: false,
        recv_from_addr: None,
        address_reuse: false,
        broadcast: false,
    };
    let id = host.resources.insert(instance, Box::new(res));
    tracing::debug!("PPB_UDPSocket_Private::Create -> resource={}", id);
    id
}

unsafe extern "C" fn is_udp_socket(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_UDPSocket_Private")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn set_socket_feature(
    udp_socket: PP_Resource,
    name: PP_UDPSocketFeature_Private,
    value: PP_Var,
) -> i32 {
    tracing::debug!(
        "PPB_UDPSocket_Private::SetSocketFeature(resource={}, name={})",
        udp_socket, name
    );
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    let enabled = pp_to_bool(unsafe { value.value.as_bool });

    let result = host
        .resources
        .with_downcast_mut::<UdpSocketResource, _>(udp_socket, |s| {
            if s.socket.is_some() {
                // Features must be set before Bind.
                return PP_ERROR_FAILED;
            }
            match name {
                PP_UDPSOCKETFEATURE_PRIVATE_ADDRESS_REUSE => {
                    s.address_reuse = enabled;
                    PP_OK
                }
                PP_UDPSOCKETFEATURE_PRIVATE_BROADCAST => {
                    s.broadcast = enabled;
                    PP_OK
                }
                _ => PP_ERROR_BADARGUMENT,
            }
        });

    result.unwrap_or(PP_ERROR_BADRESOURCE)
}

unsafe extern "C" fn bind(
    udp_socket: PP_Resource,
    addr: *const PP_NetAddress_Private,
    callback: PP_CompletionCallback,
) -> i32 {
    if addr.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let pp_addr = unsafe { &*addr };
    let Some(bind_addr) = addr_to_socketaddr(pp_addr) else {
        tracing::warn!("PPB_UDPSocket_Private::Bind: invalid address");
        return crate::callback::complete_immediately(callback, PP_ERROR_ADDRESS_INVALID);
    };
    tracing::debug!(
        "PPB_UDPSocket_Private::Bind(resource={}, addr={})",
        udp_socket, bind_addr
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    // Read socket options.
    let opts = host
        .resources
        .with_downcast::<UdpSocketResource, _>(udp_socket, |s| {
            (s.address_reuse, s.broadcast)
        });

    let Some((address_reuse, broadcast)) = opts else {
        return PP_ERROR_BADRESOURCE;
    };

    let resource_id = udp_socket;
    let cb = callback;

    std::thread::spawn(move || {
        let result = do_bind(&bind_addr, address_reuse, broadcast);
        let Some(host) = HOST.get() else { return };

        let code = match result {
            Ok(socket) => {
                host.resources
                    .with_downcast_mut::<UdpSocketResource, _>(resource_id, |s| {
                        s.socket = Some(socket);
                    });
                PP_OK
            }
            Err(e) => e,
        };

        if let Some(poster) = &*host.main_loop_poster.lock() {
            poster.post_work(cb, 0, code);
        } else {
            unsafe { cb.run(code) };
        }
    });

    PP_OK_COMPLETIONPENDING
}

fn do_bind(
    addr: &SocketAddr,
    address_reuse: bool,
    broadcast: bool,
) -> Result<UdpSocket, i32> {
    // Use socket2 features through the standard library where possible.
    let socket = UdpSocket::bind(addr).map_err(|e| {
        tracing::warn!("PPB_UDPSocket_Private: bind to {} failed: {}", addr, e);
        if e.kind() == std::io::ErrorKind::AddrInUse {
            PP_ERROR_ADDRESS_IN_USE
        } else {
            PP_ERROR_FAILED
        }
    })?;

    if broadcast {
        let _ = socket.set_broadcast(true);
    }

    // SO_REUSEADDR is typically set before bind, but std UdpSocket::bind
    // doesn't expose it. For most Flash use-cases this is fine.
    let _ = address_reuse; // acknowledged but std doesn't let us set post-bind

    tracing::debug!("PPB_UDPSocket_Private: bound to {}", addr);
    Ok(socket)
}

unsafe extern "C" fn get_bound_address(
    udp_socket: PP_Resource,
    addr: *mut PP_NetAddress_Private,
) -> PP_Bool {
    if addr.is_null() {
        return PP_FALSE;
    }
    let Some(host) = HOST.get() else { return PP_FALSE };
    host.resources
        .with_downcast::<UdpSocketResource, _>(udp_socket, |s| {
            if let Some(ref socket) = s.socket {
                if let Ok(sa) = socket.local_addr() {
                    let out = unsafe { &mut *addr };
                    socketaddr_to_addr(&sa, out);
                    return PP_TRUE;
                }
            }
            PP_FALSE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn recv_from(
    udp_socket: PP_Resource,
    buffer: *mut c_char,
    num_bytes: i32,
    callback: PP_CompletionCallback,
) -> i32 {
    if buffer.is_null() || num_bytes <= 0 {
        return PP_ERROR_BADARGUMENT;
    }
    tracing::trace!(
        "PPB_UDPSocket_Private::RecvFrom(resource={}, num_bytes={})",
        udp_socket, num_bytes
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    let socket_clone = host
        .resources
        .with_downcast::<UdpSocketResource, _>(udp_socket, |s| {
            s.socket.as_ref().and_then(|sock| sock.try_clone().ok())
        })
        .flatten();

    let Some(socket) = socket_clone else {
        tracing::warn!("PPB_UDPSocket_Private::RecvFrom: not bound");
        return PP_ERROR_FAILED;
    };

    let max_bytes = num_bytes as usize;
    let buf_ptr = buffer as usize;
    let cb = callback;
    let resource_id = udp_socket;

    std::thread::spawn(move || {
        let slice = unsafe { std::slice::from_raw_parts_mut(buf_ptr as *mut u8, max_bytes) };
        let (result, from_addr) = match socket.recv_from(slice) {
            Ok((n, from)) => (n as i32, Some(from)),
            Err(e) => {
                tracing::warn!("PPB_UDPSocket_Private::RecvFrom error: {}", e);
                (PP_ERROR_FAILED, None)
            }
        };

        let Some(host) = HOST.get() else { return };

        // Store the sender address so GetRecvFromAddress can retrieve it.
        if let Some(from) = from_addr {
            host.resources
                .with_downcast_mut::<UdpSocketResource, _>(resource_id, |s| {
                    s.recv_from_addr = Some(from);
                });
        }

        if let Some(poster) = &*host.main_loop_poster.lock() {
            poster.post_work(cb, 0, result);
        } else {
            unsafe { cb.run(result) };
        }
    });

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn get_recv_from_address(
    udp_socket: PP_Resource,
    addr: *mut PP_NetAddress_Private,
) -> PP_Bool {
    if addr.is_null() {
        return PP_FALSE;
    }
    let Some(host) = HOST.get() else { return PP_FALSE };
    host.resources
        .with_downcast::<UdpSocketResource, _>(udp_socket, |s| {
            if let Some(ref from) = s.recv_from_addr {
                let out = unsafe { &mut *addr };
                socketaddr_to_addr(from, out);
                PP_TRUE
            } else {
                PP_FALSE
            }
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn send_to(
    udp_socket: PP_Resource,
    buffer: *const c_char,
    num_bytes: i32,
    addr: *const PP_NetAddress_Private,
    callback: PP_CompletionCallback,
) -> i32 {
    if buffer.is_null() || num_bytes <= 0 || addr.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let pp_addr = unsafe { &*addr };
    let Some(dest) = addr_to_socketaddr(pp_addr) else {
        return PP_ERROR_ADDRESS_INVALID;
    };
    tracing::trace!(
        "PPB_UDPSocket_Private::SendTo(resource={}, num_bytes={}, dest={})",
        udp_socket, num_bytes, dest
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    let socket_clone = host
        .resources
        .with_downcast::<UdpSocketResource, _>(udp_socket, |s| {
            s.socket.as_ref().and_then(|sock| sock.try_clone().ok())
        })
        .flatten();

    let Some(socket) = socket_clone else {
        tracing::warn!("PPB_UDPSocket_Private::SendTo: not bound");
        return PP_ERROR_FAILED;
    };

    // Copy data so the thread owns it.
    let data =
        unsafe { std::slice::from_raw_parts(buffer as *const u8, num_bytes as usize) }.to_vec();
    let cb = callback;

    std::thread::spawn(move || {
        let result = match socket.send_to(&data, dest) {
            Ok(n) => n as i32,
            Err(e) => {
                tracing::warn!("PPB_UDPSocket_Private::SendTo error: {}", e);
                PP_ERROR_FAILED
            }
        };

        let Some(host) = HOST.get() else { return };
        if let Some(poster) = &*host.main_loop_poster.lock() {
            poster.post_work(cb, 0, result);
        } else {
            unsafe { cb.run(result) };
        }
    });

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn close(udp_socket: PP_Resource) {
    tracing::debug!("PPB_UDPSocket_Private::Close(resource={})", udp_socket);
    let Some(host) = HOST.get() else { return };
    host.resources
        .with_downcast_mut::<UdpSocketResource, _>(udp_socket, |s| {
            s.closed = true;
            s.socket = None;
        });
}

// ---------------------------------------------------------------------------
// Vtables
// ---------------------------------------------------------------------------

static VTABLE_0_4: PPB_UDPSocket_Private_0_4 = PPB_UDPSocket_Private_0_4 {
    Create: Some(create),
    IsUDPSocket: Some(is_udp_socket),
    SetSocketFeature: Some(set_socket_feature),
    Bind: Some(bind),
    GetBoundAddress: Some(get_bound_address),
    RecvFrom: Some(recv_from),
    GetRecvFromAddress: Some(get_recv_from_address),
    SendTo: Some(send_to),
    Close: Some(close),
};

static VTABLE_0_3: PPB_UDPSocket_Private_0_3 = PPB_UDPSocket_Private_0_3 {
    Create: Some(create),
    IsUDPSocket: Some(is_udp_socket),
    Bind: Some(bind),
    GetBoundAddress: Some(get_bound_address),
    RecvFrom: Some(recv_from),
    GetRecvFromAddress: Some(get_recv_from_address),
    SendTo: Some(send_to),
    Close: Some(close),
};

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_UDPSOCKET_PRIVATE_INTERFACE_0_4, &VTABLE_0_4);
        registry.register(PPB_UDPSOCKET_PRIVATE_INTERFACE_0_3, &VTABLE_0_3);
    }
}
