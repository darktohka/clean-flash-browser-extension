//! TLS provider backed by [rustls](https://docs.rs/rustls).
//!
//! Desktop players (egui, win32, web-extension host) register this
//! provider so that PPB_TCPSocket_Private can perform SSL handshakes
//! without ppapi-host itself depending on rustls/aws-lc-sys.

use player_ui_traits::{TlsProvider, TlsStream, TlsStreamIo};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, OnceLock};

/// A [`TlsProvider`] that uses rustls with Mozilla root certificates.
pub struct RustlsTlsProvider;

impl RustlsTlsProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RustlsTlsProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns a shared `ClientConfig` with Mozilla root certificates.
fn client_config() -> Arc<rustls::ClientConfig> {
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

impl TlsProvider for RustlsTlsProvider {
    fn handshake(
        &self,
        mut tcp: TcpStream,
        server_name: &str,
    ) -> Result<TlsStream, String> {
        let config = client_config();

        let sni = rustls::pki_types::ServerName::try_from(server_name.to_owned())
            .map_err(|e| format!("invalid server name {:?}: {}", server_name, e))?;

        let mut conn = rustls::ClientConnection::new(config, sni)
            .map_err(|e| format!("ClientConnection::new failed: {}", e))?;

        // Drive the handshake to completion (blocking I/O).
        while conn.is_handshaking() {
            if let Err(e) = conn.complete_io(&mut tcp) {
                let _ = tcp.shutdown(std::net::Shutdown::Both);
                return Err(format!("handshake I/O error: {}", e));
            }
        }

        // Extract the server's leaf certificate (DER).
        let cert_der = conn
            .peer_certificates()
            .and_then(|certs| certs.first())
            .map(|c| c.as_ref().to_vec())
            .unwrap_or_default();

        let wrapped = rustls::StreamOwned::new(conn, tcp);

        Ok(TlsStream {
            stream: Box::new(RustlsStreamIo(wrapped)),
            server_cert_der: cert_der,
        })
    }
}

/// Wraps a rustls `StreamOwned` to implement [`TlsStreamIo`].
struct RustlsStreamIo(rustls::StreamOwned<rustls::ClientConnection, TcpStream>);

impl Read for RustlsStreamIo {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for RustlsStreamIo {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl TlsStreamIo for RustlsStreamIo {
    fn get_tcp_ref(&self) -> Option<&TcpStream> {
        Some(self.0.get_ref())
    }
}
