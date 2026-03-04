//! PPB_TCPSocket_Private;0.5 / 0.4 / 0.3 implementation.
//!
//! Provides TCP socket operations: create, connect (by host:port or
//! by PP_NetAddress_Private), read, write, disconnect, and set-option.
//! SSL handshake is stubbed (returns PP_ERROR_NOTSUPPORTED).

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::{c_char, CStr};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};

use super::net_address::{addr_to_socketaddr, socketaddr_to_addr};
use crate::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

pub struct TcpSocketResource {
    pub instance: PP_Instance,
    /// The underlying OS TCP stream, present once connected.
    pub stream: Option<TcpStream>,
    /// Whether we've been explicitly disconnected.
    pub disconnected: bool,
    /// Whether TCP_NODELAY is requested (before or after connect).
    pub no_delay: bool,
}

impl Resource for TcpSocketResource {
    fn resource_type(&self) -> &'static str {
        "PPB_TCPSocket_Private"
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
    tracing::debug!("PPB_TCPSocket_Private::Create(instance={})", instance);
    let Some(host) = HOST.get() else { return 0 };
    let res = TcpSocketResource {
        instance,
        stream: None,
        disconnected: false,
        no_delay: false,
    };
    let id = host.resources.insert(instance, Box::new(res));
    tracing::debug!("PPB_TCPSocket_Private::Create -> resource={}", id);
    id
}

unsafe extern "C" fn is_tcp_socket(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_TCPSocket_Private")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn connect(
    tcp_socket: PP_Resource,
    host_ptr: *const c_char,
    port: u16,
    callback: PP_CompletionCallback,
) -> i32 {
    if host_ptr.is_null() {
        tracing::warn!("PPB_TCPSocket_Private::Connect: null host");
        return PP_ERROR_BADARGUMENT;
    }
    let host_str = unsafe { CStr::from_ptr(host_ptr) }
        .to_str()
        .unwrap_or("")
        .to_owned();
    tracing::debug!(
        "PPB_TCPSocket_Private::Connect(resource={}, host={}, port={})",
        tcp_socket, host_str, port
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    // Get no_delay preference before spawning.
    let no_delay = host
        .resources
        .with_downcast::<TcpSocketResource, _>(tcp_socket, |s| s.no_delay)
        .unwrap_or(false);

    // Perform DNS resolution + connect asynchronously.
    let cb = callback;
    let resource_id = tcp_socket;
    std::thread::spawn(move || {
        let result = do_connect_host(&host_str, port, no_delay);
        finish_connect(resource_id, result, cb);
    });

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn connect_with_net_address(
    tcp_socket: PP_Resource,
    addr: *const PP_NetAddress_Private,
    callback: PP_CompletionCallback,
) -> i32 {
    if addr.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let pp_addr = unsafe { &*addr };
    let Some(sa) = addr_to_socketaddr(pp_addr) else {
        tracing::warn!("PPB_TCPSocket_Private::ConnectWithNetAddress: invalid address");
        return PP_ERROR_ADDRESS_INVALID;
    };
    tracing::debug!(
        "PPB_TCPSocket_Private::ConnectWithNetAddress(resource={}, addr={})",
        tcp_socket, sa
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    let no_delay = host
        .resources
        .with_downcast::<TcpSocketResource, _>(tcp_socket, |s| s.no_delay)
        .unwrap_or(false);

    let cb = callback;
    let resource_id = tcp_socket;
    std::thread::spawn(move || {
        let result = do_connect_addr(&sa, no_delay);
        finish_connect(resource_id, result, cb);
    });

    PP_OK_COMPLETIONPENDING
}

/// Blocking DNS + TCP connect.
fn do_connect_host(
    host: &str,
    port: u16,
    no_delay: bool,
) -> Result<TcpStream, i32> {
    let addr_str = format!("{}:{}", host, port);
    let addrs: Vec<SocketAddr> = addr_str
        .to_socket_addrs()
        .map_err(|e| {
            tracing::warn!("PPB_TCPSocket_Private: DNS resolution failed for {}: {}", host, e);
            PP_ERROR_NAME_NOT_RESOLVED
        })?
        .collect();

    if addrs.is_empty() {
        return Err(PP_ERROR_NAME_NOT_RESOLVED);
    }

    for addr in &addrs {
        match TcpStream::connect_timeout(addr, std::time::Duration::from_secs(30)) {
            Ok(stream) => {
                let _ = stream.set_nodelay(no_delay);
                tracing::debug!("PPB_TCPSocket_Private: connected to {}", addr);
                return Ok(stream);
            }
            Err(e) => {
                tracing::debug!("PPB_TCPSocket_Private: connect to {} failed: {}", addr, e);
            }
        }
    }

    Err(PP_ERROR_CONNECTION_FAILED)
}

fn do_connect_addr(addr: &SocketAddr, no_delay: bool) -> Result<TcpStream, i32> {
    match TcpStream::connect_timeout(addr, std::time::Duration::from_secs(30)) {
        Ok(stream) => {
            let _ = stream.set_nodelay(no_delay);
            tracing::debug!("PPB_TCPSocket_Private: connected to {}", addr);
            Ok(stream)
        }
        Err(e) => {
            tracing::warn!("PPB_TCPSocket_Private: connect to {} failed: {}", addr, e);
            Err(PP_ERROR_CONNECTION_FAILED)
        }
    }
}

fn finish_connect(
    resource_id: PP_Resource,
    result: Result<TcpStream, i32>,
    cb: PP_CompletionCallback,
) {
    let Some(host) = HOST.get() else { return };
    let code = match result {
        Ok(stream) => {
            host.resources
                .with_downcast_mut::<TcpSocketResource, _>(resource_id, |s| {
                    s.stream = Some(stream);
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
}

unsafe extern "C" fn get_local_address(
    tcp_socket: PP_Resource,
    local_addr: *mut PP_NetAddress_Private,
) -> PP_Bool {
    if local_addr.is_null() {
        return PP_FALSE;
    }
    let Some(host) = HOST.get() else { return PP_FALSE };
    host.resources
        .with_downcast::<TcpSocketResource, _>(tcp_socket, |s| {
            if let Some(ref stream) = s.stream {
                if let Ok(sa) = stream.local_addr() {
                    let out = unsafe { &mut *local_addr };
                    socketaddr_to_addr(&sa, out);
                    return PP_TRUE;
                }
            }
            PP_FALSE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_remote_address(
    tcp_socket: PP_Resource,
    remote_addr: *mut PP_NetAddress_Private,
) -> PP_Bool {
    if remote_addr.is_null() {
        return PP_FALSE;
    }
    let Some(host) = HOST.get() else { return PP_FALSE };
    host.resources
        .with_downcast::<TcpSocketResource, _>(tcp_socket, |s| {
            if let Some(ref stream) = s.stream {
                if let Ok(sa) = stream.peer_addr() {
                    let out = unsafe { &mut *remote_addr };
                    socketaddr_to_addr(&sa, out);
                    return PP_TRUE;
                }
            }
            PP_FALSE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn ssl_handshake(
    tcp_socket: PP_Resource,
    _server_name: *const c_char,
    _server_port: u16,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::warn!(
        "PPB_TCPSocket_Private::SSLHandshake(resource={}) — not supported",
        tcp_socket
    );
    // SSL is not implemented; report failure.
    crate::callback::complete_immediately(callback, PP_ERROR_NOTSUPPORTED)
}

unsafe extern "C" fn get_server_certificate(tcp_socket: PP_Resource) -> PP_Resource {
    tracing::warn!(
        "PPB_TCPSocket_Private::GetServerCertificate(resource={}) — stub",
        tcp_socket
    );
    0
}

unsafe extern "C" fn add_chain_building_certificate(
    _tcp_socket: PP_Resource,
    _certificate: PP_Resource,
    _is_trusted: PP_Bool,
) -> PP_Bool {
    PP_FALSE
}

unsafe extern "C" fn read(
    tcp_socket: PP_Resource,
    buffer: *mut c_char,
    bytes_to_read: i32,
    callback: PP_CompletionCallback,
) -> i32 {
    if buffer.is_null() || bytes_to_read <= 0 {
        return PP_ERROR_BADARGUMENT;
    }
    tracing::trace!(
        "PPB_TCPSocket_Private::Read(resource={}, bytes_to_read={})",
        tcp_socket, bytes_to_read
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    // Clone the TcpStream so we can read on a background thread.
    let stream_clone = host
        .resources
        .with_downcast::<TcpSocketResource, _>(tcp_socket, |s| {
            s.stream.as_ref().and_then(|st| st.try_clone().ok())
        })
        .flatten();

    let Some(mut stream) = stream_clone else {
        tracing::warn!("PPB_TCPSocket_Private::Read: not connected");
        return PP_ERROR_FAILED;
    };

    // Cap at 1 MB as per spec.
    let max_read = bytes_to_read.min(1024 * 1024) as usize;
    let buf_ptr = buffer as usize; // raw pointer sent to thread
    let cb = callback;

    std::thread::spawn(move || {
        let slice = unsafe { std::slice::from_raw_parts_mut(buf_ptr as *mut u8, max_read) };
        let result = match stream.read(slice) {
            Ok(0) => 0, // EOF
            Ok(n) => n as i32,
            Err(e) => {
                tracing::warn!("PPB_TCPSocket_Private::Read error: {}", e);
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

unsafe extern "C" fn write(
    tcp_socket: PP_Resource,
    buffer: *const c_char,
    bytes_to_write: i32,
    callback: PP_CompletionCallback,
) -> i32 {
    if buffer.is_null() || bytes_to_write <= 0 {
        return PP_ERROR_BADARGUMENT;
    }
    tracing::trace!(
        "PPB_TCPSocket_Private::Write(resource={}, bytes_to_write={})",
        tcp_socket, bytes_to_write
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    let stream_clone = host
        .resources
        .with_downcast::<TcpSocketResource, _>(tcp_socket, |s| {
            s.stream.as_ref().and_then(|st| st.try_clone().ok())
        })
        .flatten();

    let Some(mut stream) = stream_clone else {
        tracing::warn!("PPB_TCPSocket_Private::Write: not connected");
        return PP_ERROR_FAILED;
    };

    // Cap at 1 MB as per spec.
    let max_write = bytes_to_write.min(1024 * 1024) as usize;
    // Copy data so the thread owns it.
    let data = unsafe { std::slice::from_raw_parts(buffer as *const u8, max_write) }.to_vec();
    let cb = callback;

    std::thread::spawn(move || {
        let result = match stream.write(&data) {
            Ok(n) => n as i32,
            Err(e) => {
                tracing::warn!("PPB_TCPSocket_Private::Write error: {}", e);
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

unsafe extern "C" fn disconnect(tcp_socket: PP_Resource) {
    tracing::debug!("PPB_TCPSocket_Private::Disconnect(resource={})", tcp_socket);
    let Some(host) = HOST.get() else { return };
    host.resources
        .with_downcast_mut::<TcpSocketResource, _>(tcp_socket, |s| {
            s.disconnected = true;
            if let Some(ref stream) = s.stream {
                let _ = stream.shutdown(Shutdown::Both);
            }
            s.stream = None;
        });
}

unsafe extern "C" fn set_option(
    tcp_socket: PP_Resource,
    name: PP_TCPSocketOption_Private,
    value: PP_Var,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!(
        "PPB_TCPSocket_Private::SetOption(resource={}, name={})",
        tcp_socket, name
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    if name == PP_TCPSOCKETOPTION_PRIVATE_INVALID {
        return crate::callback::complete_immediately(callback, PP_ERROR_BADARGUMENT);
    }

    if name == PP_TCPSOCKETOPTION_PRIVATE_NO_DELAY {
        let enabled = pp_to_bool(unsafe { value.value.as_bool });
        let result = host
            .resources
            .with_downcast_mut::<TcpSocketResource, _>(tcp_socket, |s| {
                s.no_delay = enabled;
                if let Some(ref stream) = s.stream {
                    stream.set_nodelay(enabled).map_err(|e| {
                        tracing::warn!("set_nodelay failed: {}", e);
                        PP_ERROR_FAILED
                    })
                } else {
                    // Will apply on connect.
                    Ok(())
                }
            });
        let code = match result {
            Some(Ok(())) => PP_OK,
            Some(Err(e)) => e,
            None => PP_ERROR_BADRESOURCE,
        };
        return crate::callback::complete_immediately(callback, code);
    }

    crate::callback::complete_immediately(callback, PP_ERROR_BADARGUMENT)
}

// ---------------------------------------------------------------------------
// Vtables
// ---------------------------------------------------------------------------

static VTABLE_0_5: PPB_TCPSocket_Private_0_5 = PPB_TCPSocket_Private_0_5 {
    Create: Some(create),
    IsTCPSocket: Some(is_tcp_socket),
    Connect: Some(connect),
    ConnectWithNetAddress: Some(connect_with_net_address),
    GetLocalAddress: Some(get_local_address),
    GetRemoteAddress: Some(get_remote_address),
    SSLHandshake: Some(ssl_handshake),
    GetServerCertificate: Some(get_server_certificate),
    AddChainBuildingCertificate: Some(add_chain_building_certificate),
    Read: Some(read),
    Write: Some(write),
    Disconnect: Some(disconnect),
    SetOption: Some(set_option),
};

static VTABLE_0_4: PPB_TCPSocket_Private_0_4 = PPB_TCPSocket_Private_0_4 {
    Create: Some(create),
    IsTCPSocket: Some(is_tcp_socket),
    Connect: Some(connect),
    ConnectWithNetAddress: Some(connect_with_net_address),
    GetLocalAddress: Some(get_local_address),
    GetRemoteAddress: Some(get_remote_address),
    SSLHandshake: Some(ssl_handshake),
    GetServerCertificate: Some(get_server_certificate),
    AddChainBuildingCertificate: Some(add_chain_building_certificate),
    Read: Some(read),
    Write: Some(write),
    Disconnect: Some(disconnect),
};

static VTABLE_0_3: PPB_TCPSocket_Private_0_3 = PPB_TCPSocket_Private_0_3 {
    Create: Some(create),
    IsTCPSocket: Some(is_tcp_socket),
    Connect: Some(connect),
    ConnectWithNetAddress: Some(connect_with_net_address),
    GetLocalAddress: Some(get_local_address),
    GetRemoteAddress: Some(get_remote_address),
    SSLHandshake: Some(ssl_handshake),
    Read: Some(read),
    Write: Some(write),
    Disconnect: Some(disconnect),
};

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_TCPSOCKET_PRIVATE_INTERFACE_0_5, &VTABLE_0_5);
        registry.register(PPB_TCPSOCKET_PRIVATE_INTERFACE_0_4, &VTABLE_0_4);
        registry.register(PPB_TCPSOCKET_PRIVATE_INTERFACE_0_3, &VTABLE_0_3);
    }
}
