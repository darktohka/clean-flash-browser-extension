//! PPB_TCPSocket_Private;0.5 / 0.4 / 0.3 implementation.
//!
//! Provides TCP socket operations: create, connect (by host:port or
//! by PP_NetAddress_Private), read, write, disconnect, and set-option.
//! SSL handshake is implemented using rustls.
//!
//! ## Flash socket policy handling
//!
//! Flash Player checks cross-domain socket policies before allowing a
//! connection.  It first tries port 843, then falls back to sending a
//! `<policy-file-request/>\0` on the application port itself.
//!
//! Our host intercepts both cases and returns a permissive
//! `<cross-domain-policy>` response locally so that:
//!   - Policy checks complete instantly (no server round-trip).
//!   - Game servers that don't serve policy files work out of the box.
//!   - The behaviour matches Chrome's PPAPI host (which allowed all
//!     socket access for trusted plugins).

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::{c_char, CStr};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;

use super::net_address::{addr_to_socketaddr, socketaddr_to_addr};
use crate::HOST;

// ---------------------------------------------------------------------------
// Flash socket policy auto-response
// ---------------------------------------------------------------------------

/// Standard Flash socket policy that permits connections from any domain to any
/// port.  The null terminator is required by the Flash policy protocol.
const PERMISSIVE_POLICY: &[u8] =
    b"<?xml version=\"1.0\"?>\
      <!DOCTYPE cross-domain-policy SYSTEM \"http://www.macromedia.com/xml/dtds/cross-domain-policy.dtd\">\
      <cross-domain-policy>\
        <allow-access-from domain=\"*\" to-ports=\"*\" />\
      </cross-domain-policy>\0";

/// The exact bytes Flash sends when requesting a socket policy file.
const POLICY_FILE_REQUEST: &[u8] = b"<policy-file-request/>\0";

/// Hosts that are always blocked through PPB_TCPSocket_Private.
const ALWAYS_BLOCKED_TCP_HOSTS: [&str; 1] = [
    "fpdownload.macromedia.com",
];

/// Geolocation hosts blocked when the `disable_geolocation` setting is true.
const GEO_BLOCKED_TCP_HOSTS: [&str; 2] = [
    "geo2.adobe.com",
    "geo.adobe.com",
];

fn is_blocked_tcp_host(host: &str) -> bool {
    let normalized = host.trim().trim_end_matches('.');

    // Always blocked.
    if ALWAYS_BLOCKED_TCP_HOSTS
        .iter()
        .any(|blocked| normalized.eq_ignore_ascii_case(blocked))
    {
        return true;
    }

    // Geolocation hosts are blocked only when the setting says so.
    let disable_geo = crate::HOST
        .get()
        .and_then(|h| h.get_settings_provider())
        .map(|sp| sp.get_settings().disable_geolocation)
        .unwrap_or(true);

    if disable_geo {
        if GEO_BLOCKED_TCP_HOSTS
            .iter()
            .any(|blocked| normalized.eq_ignore_ascii_case(blocked))
        {
            return true;
        }
    }

    false
}

/// Limit payload previews so trace logs stay useful and bounded.
const IO_TRACE_PREVIEW_LIMIT: usize = 2048;
/// Limit how long a read attempt can hold the shared stream lock.
const READ_SLICE_TIMEOUT: Duration = Duration::from_millis(50);

fn push_hex_byte(out: &mut String, b: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(HEX[(b >> 4) as usize] as char);
    out.push(HEX[(b & 0x0f) as usize] as char);
}

fn format_payload_preview(bytes: &[u8]) -> String {
    let shown = bytes.len().min(IO_TRACE_PREVIEW_LIMIT);
    let mut ascii = String::with_capacity(shown * 4);
    let mut hex = String::with_capacity(shown * 3);

    for &b in &bytes[..shown] {
        match b {
            b'\r' => ascii.push_str("\\r"),
            b'\n' => ascii.push_str("\\n"),
            b'\t' => ascii.push_str("\\t"),
            b'\\' => ascii.push_str("\\\\"),
            0x20..=0x7e => ascii.push(b as char),
            _ => {
                ascii.push_str("\\x");
                push_hex_byte(&mut ascii, b);
            }
        }

        if !hex.is_empty() {
            hex.push(' ');
        }
        push_hex_byte(&mut hex, b);
    }

    if bytes.len() > shown {
        format!(
            "len={} shown={} ascii=\"{}\"... hex={}...",
            bytes.len(), shown, ascii, hex
        )
    } else {
        format!("len={} ascii=\"{}\" hex={}", bytes.len(), ascii, hex)
    }
}

fn trace_socket_payload(kind: &str, resource_id: PP_Resource, payload: &[u8]) {
    tracing::trace!(
        "PPB_TCPSocket_Private::{}(resource={}): {}",
        kind,
        resource_id,
        format_payload_preview(payload)
    );
}

// ---------------------------------------------------------------------------
// Socket stream - plain TCP or TLS-wrapped
// ---------------------------------------------------------------------------

pub enum SocketStream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl SocketStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Plain(s) => s.set_read_timeout(timeout),
            Self::Tls(s) => s.get_mut().set_read_timeout(timeout),
        }
    }
}

impl Read for SocketStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(buf),
            Self::Tls(s) => s.read(buf),
        }
    }
}

impl Write for SocketStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.write(buf),
            Self::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(s) => s.flush(),
            Self::Tls(s) => s.flush(),
        }
    }
}

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

pub struct TcpSocketResource {
    pub instance: PP_Instance,
    /// The I/O stream - plain TCP or TLS-wrapped.  Protected by a Mutex
    /// so background read/write tasks can access it safely (required for
    /// TLS where the connection state cannot be cloned).
    pub stream: Arc<Mutex<Option<SocketStream>>>,
    /// Cloned raw TCP handle for shutdown signals and address queries.
    /// Kept outside the stream mutex so `disconnect` can call `shutdown()`
    /// to interrupt blocking I/O held under the stream lock.
    pub raw_tcp: Option<TcpStream>,
    /// Whether we've been explicitly disconnected.
    pub disconnected: bool,
    /// Whether TCP_NODELAY is requested (before or after connect).
    pub no_delay: bool,
    /// Cancellation token - set to `true` on Disconnect so that
    /// background threads know they should not fire their callback.
    pub cancel: Arc<AtomicBool>,
    /// Pre-loaded policy response bytes to hand back on the next read.
    /// Set when we intercept a `<policy-file-request/>` write or when
    /// connecting to port 843.
    pub pending_policy_response: Option<Vec<u8>>,
    /// DER-encoded server certificate from TLS handshake.
    pub server_cert_der: Option<Vec<u8>>,
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
        stream: Arc::new(Mutex::new(None)),
        raw_tcp: None,
        disconnected: false,
        no_delay: false,
        cancel: Arc::new(AtomicBool::new(false)),
        pending_policy_response: None,
        server_cert_der: None,
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

    if is_blocked_tcp_host(&host_str) {
        tracing::warn!(
            "PPB_TCPSocket_Private::Connect(resource={}, host={}, port={}): blocked by host denylist",
            tcp_socket,
            host_str,
            port
        );
        return PP_ERROR_NOACCESS;
    }

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    // -----------------------------------------------------------------
    // Port 843 interception - Flash socket-policy master server.
    // Instead of connecting to the remote host (which usually has no
    // policy server), immediately pretend we connected and pre-load a
    // permissive policy response so the subsequent Read returns it.
    // Only intercept when disable_crossdomain_sockets is enabled.
    // -----------------------------------------------------------------
    if port == 843 {
        let should_intercept = HOST
            .get()
            .and_then(|h| h.get_settings_provider())
            .map(|sp| sp.get_settings().disable_crossdomain_sockets)
            .unwrap_or(true);

        if should_intercept {
            tracing::info!(
                "PPB_TCPSocket_Private::Connect: intercepting port 843 policy \
                 request for resource={} - will auto-respond with permissive policy",
                tcp_socket
            );
            host.resources
                .with_downcast_mut::<TcpSocketResource, _>(tcp_socket, |s| {
                    s.pending_policy_response = Some(PERMISSIVE_POLICY.to_vec());
                });
            // Fire the connect callback on the main loop with PP_OK.
            if let Some(poster) = &*host.main_loop_poster.lock() {
                poster.post_work(callback, 0, PP_OK);
            }
            return PP_OK_COMPLETIONPENDING;
        } else {
            tracing::info!(
                "PPB_TCPSocket_Private::Connect: port 843 interception disabled \
                 by settings for resource={} - connecting normally",
                tcp_socket
            );
        }
    }

    // Get no_delay preference and cancel token before spawning.
    let (no_delay, cancel) = host
        .resources
        .with_downcast::<TcpSocketResource, _>(tcp_socket, |s| {
            (s.no_delay, s.cancel.clone())
        })
        .unwrap_or((false, Arc::new(AtomicBool::new(true))));

    // Perform DNS resolution + connect asynchronously.
    let cb = callback;
    let resource_id = tcp_socket;
    crate::tokio_runtime().spawn_blocking(move || {
        let result = do_connect_host(&host_str, port, no_delay);
        finish_connect(resource_id, result, cb, &cancel);
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

    let (no_delay, cancel) = host
        .resources
        .with_downcast::<TcpSocketResource, _>(tcp_socket, |s| {
            (s.no_delay, s.cancel.clone())
        })
        .unwrap_or((false, Arc::new(AtomicBool::new(true))));

    let cb = callback;
    let resource_id = tcp_socket;
    crate::tokio_runtime().spawn_blocking(move || {
        let result = do_connect_addr(&sa, no_delay);
        finish_connect(resource_id, result, cb, &cancel);
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
    cancel: &AtomicBool,
) {
    // If the socket was already disconnected / resource freed while
    // we were connecting, do NOT fire the callback - the plugin has
    // already cleaned up and the user_data pointer may be stale.
    if cancel.load(Ordering::Acquire) {
        tracing::debug!(
            "PPB_TCPSocket_Private: connect finished for cancelled resource {} - dropping callback",
            resource_id
        );
        return;
    }

    let Some(host) = HOST.get() else { return };
    let code = match result {
        Ok(stream) => {
            host.resources
                .with_downcast_mut::<TcpSocketResource, _>(resource_id, |s| {
                    let raw_clone = stream.try_clone().ok();
                    *s.stream.lock() = Some(SocketStream::Plain(stream));
                    s.raw_tcp = raw_clone;
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
            if let Some(ref tcp) = s.raw_tcp {
                if let Ok(sa) = tcp.local_addr() {
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
            if let Some(ref tcp) = s.raw_tcp {
                if let Ok(sa) = tcp.peer_addr() {
                    let out = unsafe { &mut *remote_addr };
                    socketaddr_to_addr(&sa, out);
                    return PP_TRUE;
                }
            }
            PP_FALSE
        })
        .unwrap_or(PP_FALSE)
}

// ---------------------------------------------------------------------------
// TLS support - rustls
// ---------------------------------------------------------------------------

/// Returns a shared `ClientConfig` with Mozilla root certificates.
fn tls_client_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut root_store = rustls::RootCertStore::empty();
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(
                rustls::ClientConfig::builder()
                    .with_root_certificates(root_store)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

/// Perform TLS handshake on the stream inside the mutex, upgrading
/// `SocketStream::Plain` → `SocketStream::Tls`.
fn do_tls_handshake(
    stream: &Mutex<Option<SocketStream>>,
    server_name: &str,
) -> Result<Vec<u8>, i32> {
    let config = tls_client_config();

    let sni = rustls::pki_types::ServerName::try_from(server_name.to_owned()).map_err(|e| {
        tracing::warn!("SSLHandshake: invalid server name {:?}: {}", server_name, e);
        PP_ERROR_BADARGUMENT
    })?;

    let mut conn = rustls::ClientConnection::new(config, sni).map_err(|e| {
        tracing::warn!("SSLHandshake: ClientConnection::new failed: {}", e);
        PP_ERROR_FAILED
    })?;

    // Lock the stream and take the plain TcpStream out.
    let mut guard = stream.lock();
    match guard.as_ref() {
        None => return Err(PP_ERROR_FAILED),
        Some(SocketStream::Tls(_)) => return Err(PP_ERROR_FAILED),
        Some(SocketStream::Plain(_)) => {}
    }
    let mut tcp = match guard.take() {
        Some(SocketStream::Plain(tcp)) => tcp,
        _ => unreachable!(),
    };

    // Drive the TLS handshake to completion (blocking I/O).
    // We hold the mutex during the handshake.  Per the PPAPI spec, no
    // pending reads/writes should exist during SSLHandshake, so this
    // will not cause contention.
    while conn.is_handshaking() {
        if let Err(e) = conn.complete_io(&mut tcp) {
            tracing::warn!("SSLHandshake: handshake I/O error: {}", e);
            let _ = tcp.shutdown(Shutdown::Both);
            return Err(PP_ERROR_FAILED);
        }
    }

    // Extract the server's leaf certificate (DER).
    let cert_der = conn
        .peer_certificates()
        .and_then(|certs| certs.first())
        .map(|c| c.as_ref().to_vec())
        .unwrap_or_default();

    // Wrap in StreamOwned and store back.
    *guard = Some(SocketStream::Tls(Box::new(rustls::StreamOwned::new(
        conn, tcp,
    ))));

    Ok(cert_der)
}

unsafe extern "C" fn ssl_handshake(
    tcp_socket: PP_Resource,
    server_name: *const c_char,
    _server_port: u16,
    callback: PP_CompletionCallback,
) -> i32 {
    if server_name.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let name_str = unsafe { CStr::from_ptr(server_name) }
        .to_str()
        .unwrap_or("")
        .to_owned();
    tracing::debug!(
        "PPB_TCPSocket_Private::SSLHandshake(resource={}, server={})",
        tcp_socket,
        name_str
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    let Some((stream_arc, cancel)) = host
        .resources
        .with_downcast::<TcpSocketResource, _>(tcp_socket, |s| {
            if s.disconnected {
                return None;
            }
            Some((s.stream.clone(), s.cancel.clone()))
        })
        .flatten()
    else {
        return PP_ERROR_FAILED;
    };

    let cb = callback;
    let resource_id = tcp_socket;

    crate::tokio_runtime().spawn_blocking(move || {
        let result = do_tls_handshake(&stream_arc, &name_str);

        if cancel.load(Ordering::Acquire) {
            return;
        }

        let Some(host) = HOST.get() else { return };
        let code = match result {
            Ok(cert_der) => {
                host.resources
                    .with_downcast_mut::<TcpSocketResource, _>(resource_id, |s| {
                        s.server_cert_der = if cert_der.is_empty() {
                            None
                        } else {
                            Some(cert_der)
                        };
                    });
                tracing::info!(
                    "PPB_TCPSocket_Private::SSLHandshake(resource={}): TLS handshake complete",
                    resource_id
                );
                PP_OK
            }
            Err(e) => {
                // Disconnect on failure per spec.
                host.resources
                    .with_downcast_mut::<TcpSocketResource, _>(resource_id, |s| {
                        s.disconnected = true;
                        if let Some(ref tcp) = s.raw_tcp {
                            let _ = tcp.shutdown(Shutdown::Both);
                        }
                        *s.stream.lock() = None;
                        s.raw_tcp = None;
                    });
                e
            }
        };

        if let Some(poster) = &*host.main_loop_poster.lock() {
            poster.post_work(cb, 0, code);
        } else {
            unsafe { cb.run(code) };
        }
    });

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn get_server_certificate(tcp_socket: PP_Resource) -> PP_Resource {
    tracing::debug!(
        "PPB_TCPSocket_Private::GetServerCertificate(resource={})",
        tcp_socket
    );
    // PPB_X509Certificate_Private resource is not implemented;
    // return null resource.
    0
}

unsafe extern "C" fn add_chain_building_certificate(
    _tcp_socket: PP_Resource,
    _certificate: PP_Resource,
    _is_trusted: PP_Bool,
) -> PP_Bool {
    // Not implemented per spec.
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

    // -----------------------------------------------------------------
    // If we have a pending policy response (from an intercepted port-843
    // connect or a `<policy-file-request/>` write), serve it directly
    // instead of reading from the network.
    // -----------------------------------------------------------------
    let policy_data = host
        .resources
        .with_downcast_mut::<TcpSocketResource, _>(tcp_socket, |s| {
            s.pending_policy_response.take()
        })
        .flatten();

    if let Some(policy_bytes) = policy_data {
        let max_read = bytes_to_read.min(1024 * 1024) as usize;
        let n = policy_bytes.len().min(max_read);
        unsafe {
            std::ptr::copy_nonoverlapping(
                policy_bytes.as_ptr(),
                buffer as *mut u8,
                n,
            );
        }
        tracing::info!(
            "PPB_TCPSocket_Private::Read(resource={}): serving {} bytes of \
             auto-generated policy response",
            tcp_socket, n
        );
        trace_socket_payload("Read response", tcp_socket, &policy_bytes[..n]);
        // Fire the read callback with the byte count.
        if let Some(poster) = &*host.main_loop_poster.lock() {
            poster.post_work(callback, 0, n as i32);
        }
        return PP_OK_COMPLETIONPENDING;
    }

    // -----------------------------------------------------------------
    // Normal read path - lock the stream and read on a bg thread.
    // -----------------------------------------------------------------
    let Some((stream_arc, cancel)) = host
        .resources
        .with_downcast::<TcpSocketResource, _>(tcp_socket, |s| {
            (s.stream.clone(), s.cancel.clone())
        })
    else {
        return PP_ERROR_BADRESOURCE;
    };

    // Cap at 1 MB as per spec.
    let max_read = bytes_to_read.min(1024 * 1024) as usize;
    // Carry the plugin buffer address as a usize so the closure is Send.
    // We only use it *after* the read completes, right before posting the
    // callback - at which point the PPAPI contract guarantees the buffer
    // is still valid.
    let buf_addr = buffer as usize;
    let cb = callback;
    let resource_id = tcp_socket;

    crate::tokio_runtime().spawn_blocking(move || {
        // Read into an owned buffer so we never construct a &mut [u8] to the
        // plugin's memory on a background thread.
        let mut owned_buf = vec![0u8; max_read];
        enum ReadPoll {
            Done(i32),
            Retry,
        }

        let result = loop {
            let poll = {
                let mut guard = stream_arc.lock();
                match guard.as_mut() {
                    Some(ss) => {
                        if let Err(e) = ss.set_read_timeout(Some(READ_SLICE_TIMEOUT)) {
                            tracing::warn!(
                                "PPB_TCPSocket_Private::Read(resource={}): set_read_timeout error: {}",
                                resource_id, e
                            );
                            ReadPoll::Done(PP_ERROR_FAILED)
                        } else {
                            let read_result = ss.read(&mut owned_buf);
                            if let Err(e) = ss.set_read_timeout(None) {
                                tracing::warn!(
                                    "PPB_TCPSocket_Private::Read(resource={}): clearing read timeout failed: {}",
                                    resource_id, e
                                );
                            }

                            match read_result {
                                Ok(0) => {
                                    tracing::debug!(
                                        "PPB_TCPSocket_Private::Read(resource={}): EOF (0 bytes)",
                                        resource_id
                                    );
                                    ReadPoll::Done(0)
                                }
                                Ok(n) => {
                                    tracing::debug!(
                                        "PPB_TCPSocket_Private::Read(resource={}): received {} bytes",
                                        resource_id, n
                                    );
                                    trace_socket_payload("Read response", resource_id, &owned_buf[..n]);
                                    ReadPoll::Done(n as i32)
                                }
                                Err(e)
                                    if matches!(
                                        e.kind(),
                                        ErrorKind::WouldBlock
                                            | ErrorKind::TimedOut
                                            | ErrorKind::Interrupted
                                    ) =>
                                {
                                    ReadPoll::Retry
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "PPB_TCPSocket_Private::Read(resource={}): error: {}",
                                        resource_id, e
                                    );
                                    ReadPoll::Done(PP_ERROR_FAILED)
                                }
                            }
                        }
                    }
                    None => {
                        tracing::warn!(
                            "PPB_TCPSocket_Private::Read(resource={}): not connected",
                            resource_id
                        );
                        ReadPoll::Done(PP_ERROR_FAILED)
                    }
                }
            };

            match poll {
                ReadPoll::Done(code) => break code,
                ReadPoll::Retry => {
                    if cancel.load(Ordering::Acquire) {
                        tracing::debug!(
                            "PPB_TCPSocket_Private::Read(resource={}): cancelled - dropping callback",
                            resource_id
                        );
                        return;
                    }
                    std::thread::yield_now();
                    continue;
                }
            }
        };

        if cancel.load(Ordering::Acquire) {
            tracing::debug!(
                "PPB_TCPSocket_Private::Read(resource={}): cancelled - dropping callback",
                resource_id
            );
            return;
        }

        // Copy the data into the plugin's buffer before posting the callback.
        // This is safe because the plugin buffer remains valid until the
        // callback fires, and we copy before posting.
        if result > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    owned_buf.as_ptr(),
                    buf_addr as *mut u8,
                    result as usize,
                );
            }
        }

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

    // Cap at 1 MB as per spec.
    let max_write = bytes_to_write.min(1024 * 1024) as usize;
    // Copy data so the thread owns it.
    let data = unsafe { std::slice::from_raw_parts(buffer as *const u8, max_write) }.to_vec();
    trace_socket_payload("Write request", tcp_socket, &data);

    // -----------------------------------------------------------------
    // Policy-file-request interception.  If Flash writes exactly the
    // 23-byte `<policy-file-request/>\0` payload to ANY socket, we
    // treat it as a policy check: don't forward the data to the server
    // and instead queue a permissive response for the next Read.
    // Only intercept when disable_crossdomain_sockets is enabled.
    // -----------------------------------------------------------------
    if data == POLICY_FILE_REQUEST {
        let should_intercept = HOST
            .get()
            .and_then(|h| h.get_settings_provider())
            .map(|sp| sp.get_settings().disable_crossdomain_sockets)
            .unwrap_or(true);

        if should_intercept {
            tracing::info!(
                "PPB_TCPSocket_Private::Write(resource={}): intercepted policy-file-request \
                 - queuing permissive policy response",
                tcp_socket
            );
            host.resources
                .with_downcast_mut::<TcpSocketResource, _>(tcp_socket, |s| {
                    s.pending_policy_response = Some(PERMISSIVE_POLICY.to_vec());
                });
            // Report success immediately (all bytes "written").
            if let Some(poster) = &*host.main_loop_poster.lock() {
                poster.post_work(callback, 0, max_write as i32);
            }
            return PP_OK_COMPLETIONPENDING;
        } else {
            tracing::info!(
                "PPB_TCPSocket_Private::Write(resource={}): policy-file-request \
                 interception disabled by settings - forwarding to server",
                tcp_socket
            );
        }
    }

    // -----------------------------------------------------------------
    // Normal write path.
    // -----------------------------------------------------------------
    let Some((stream_arc, cancel)) = host
        .resources
        .with_downcast::<TcpSocketResource, _>(tcp_socket, |s| {
            (s.stream.clone(), s.cancel.clone())
        })
    else {
        return PP_ERROR_BADRESOURCE;
    };

    let cb = callback;
    let resource_id = tcp_socket;

    crate::tokio_runtime().spawn_blocking(move || {
        let result = {
            let mut guard = stream_arc.lock();
            match guard.as_mut() {
                Some(ss) => match ss.write(&data) {
                    Ok(n) => {
                        tracing::debug!(
                            "PPB_TCPSocket_Private::Write(resource={}): wrote {} bytes",
                            resource_id, n
                        );
                        trace_socket_payload("Write sent", resource_id, &data[..n]);
                        n as i32
                    }
                    Err(e) => {
                        tracing::warn!(
                            "PPB_TCPSocket_Private::Write(resource={}): error: {}",
                            resource_id, e
                        );
                        PP_ERROR_FAILED
                    }
                },
                None => {
                    tracing::warn!(
                        "PPB_TCPSocket_Private::Write(resource={}): not connected",
                        resource_id
                    );
                    PP_ERROR_FAILED
                }
            }
        };

        if cancel.load(Ordering::Acquire) {
            tracing::debug!(
                "PPB_TCPSocket_Private::Write(resource={}): cancelled - dropping callback",
                resource_id
            );
            return;
        }

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
            // Signal cancellation so background threads drop their
            // callbacks instead of posting stale completions.
            s.cancel.store(true, Ordering::Release);
            // Shut down the raw TCP stream first - this interrupts any
            // blocking I/O held under the stream lock.
            if let Some(ref tcp) = s.raw_tcp {
                let _ = tcp.shutdown(Shutdown::Both);
            }
            *s.stream.lock() = None;
            s.raw_tcp = None;
            s.pending_policy_response = None;
            s.server_cert_der = None;
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
                if let Some(ref tcp) = s.raw_tcp {
                    tcp.set_nodelay(enabled).map_err(|e| {
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
