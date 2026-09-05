//! The guest-facing TLS server configuration, built once per VM.
//!
//! It used to be built per connection, with a fresh crypto provider and a
//! resolver that also carried the connection's SNI. A `ServerConfig` owns
//! the session cache, so a per-connection config meant an empty cache every
//! time and a full handshake for every connection a guest opened. One shared
//! config lets rustls resume sessions; the SNI is read back from the
//! `ServerConnection` after the handshake instead.

use std::sync::Arc;

use rustls::ServerConfig;

use crate::net::cert_authority::{CertAuthority, MitmCertResolver};

/// Build the shared guest-facing TLS config: leaf certificates minted on
/// demand from `ca` for whatever SNI the guest presents, HTTP/1.1 only.
pub fn make_server_tls_config(ca: &Arc<CertAuthority>) -> Arc<ServerConfig> {
    let resolver = Arc::new(MitmCertResolver::new(Arc::clone(ca)));
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("TLS server config: default protocol versions")
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

#[cfg(test)]
mod tests;
