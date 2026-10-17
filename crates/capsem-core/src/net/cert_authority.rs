/// Certificate authority for MITM proxy: loads the static Capsem CA keypair
/// and mints short-lived leaf certificates on demand for each domain.
use std::fmt;
use std::sync::Arc;

use capsem_cache::BoundedCache;
use capsem_crypto::tls::CertificateIssuer;
use rustls::pki_types::CertificateDer;
use rustls::server::ClientHello;
use rustls::sign::CertifiedKey;

/// Holds the static CA keypair and caches minted leaf certificates.
pub struct CertAuthority {
    issuer: CertificateIssuer,
    cache: BoundedCache<String, CertifiedKey>,
}

impl CertAuthority {
    /// Load a CA from PEM-encoded private key and certificate.
    ///
    /// Typically called with `include_str!("../../resources/ca/capsem-ca.key")`.
    pub fn load(key_pem: &str, cert_pem: &str) -> anyhow::Result<Self> {
        let issuer = CertificateIssuer::load(key_pem, cert_pem)?;
        Ok(Self {
            issuer,
            cache: BoundedCache::new("core.tls_leaf", capsem_proto::CACHE_TLS_LEAF_ENTRIES),
        })
    }

    /// Get or mint a `CertifiedKey` for the given domain, keyed on its
    /// normalized form. Leaf certs are ECDSA P-256, valid from 2026-01-01 to
    /// now+1y, with SAN=domain. Minting takes well under a millisecond, so it
    /// happens under the lock rather than racing two mints for one name.
    pub fn certified_key_for_domain(&self, domain: &str) -> anyhow::Result<Arc<CertifiedKey>> {
        let host = crate::net::hostname::normalize_host(domain);
        self.cache
            .get_or_try_insert_with(host.as_str(), || self.mint_leaf(&host))
    }

    /// The CA certificate in DER, for a client that must trust minted leaves.
    pub fn ca_cert_der(&self) -> &CertificateDer<'static> {
        self.issuer.certificate_der()
    }

    /// Number of cached certificates.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }

    /// Mint a leaf certificate for the given domain, signed by the CA.
    fn mint_leaf(&self, domain: &str) -> anyhow::Result<CertifiedKey> {
        self.issuer.mint_server_certificate(domain)
    }
}

/// rustls SNI-based certificate resolver that mints certs on demand.
///
/// Stateless, so one `ServerConfig` (and its session cache) can serve every
/// connection; the proxy reads the negotiated SNI back from the
/// `ServerConnection` after the handshake. Always mints certs even for
/// blocked domains so we can complete the TLS handshake, read the HTTP
/// request (capturing method/path), and return a proper 403 response.
pub struct MitmCertResolver {
    pub ca: Arc<CertAuthority>,
}

impl MitmCertResolver {
    /// Create a new resolver wrapping the given CA.
    pub fn new(ca: Arc<CertAuthority>) -> Self {
        Self { ca }
    }
}

impl fmt::Debug for MitmCertResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MitmCertResolver")
            .field("cache_size", &self.ca.cache_size())
            .finish()
    }
}

impl rustls::server::ResolvesServerCert for MitmCertResolver {
    fn resolve(&self, hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        // The SNI is guest-controlled. Normalize it once here so the domain
        // handed to policy, dial, and telemetry is the same identity the leaf
        // is minted for: rustls accepts `Example.COM.` and a rule on
        // `example.com` must still fire.
        let domain = crate::net::hostname::normalize_host(hello.server_name()?);
        if domain.is_empty() {
            return None;
        }

        // Always mint a cert, even for blocked domains. This lets us complete
        // the TLS handshake, read the HTTP request (method, path), and return
        // a proper 403 response with telemetry.
        self.ca.certified_key_for_domain(&domain).ok()
    }
}

#[cfg(test)]
mod tests;
