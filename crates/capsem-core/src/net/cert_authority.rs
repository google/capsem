/// Certificate authority for MITM proxy: loads the static Capsem CA keypair
/// and mints short-lived leaf certificates on demand for each domain.
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use lru::LruCache;

use rcgen::{CertificateParams, IsCa, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::ClientHello;
use rustls::sign::CertifiedKey;

/// Most leaf certificates kept alive at once. The guest chooses the SNI, so
/// an unbounded map let a loop over random names grow host memory for the
/// life of the VM; a real workload talks to far fewer hosts than this.
const LEAF_CACHE_CAPACITY: usize = 1024;

/// Holds the static CA keypair and caches minted leaf certificates.
pub struct CertAuthority {
    ca_cert: rcgen::Certificate,
    ca_key: KeyPair,
    ca_cert_der: CertificateDer<'static>,
    cache: Mutex<LruCache<String, Arc<CertifiedKey>>>,
}

impl CertAuthority {
    /// Load a CA from PEM-encoded private key and certificate.
    ///
    /// Typically called with `include_str!("../../resources/ca/capsem-ca.key")`.
    pub fn load(key_pem: &str, cert_pem: &str) -> anyhow::Result<Self> {
        let ca_key = KeyPair::from_pem(key_pem)?;

        // Parse the existing CA cert PEM to extract params, then re-sign to get Certificate.
        let ca_params = CertificateParams::from_ca_cert_pem(cert_pem)?;
        let ca_cert = ca_params.self_signed(&ca_key)?;
        let ca_cert_der = CertificateDer::from(ca_cert.der().to_vec());

        Ok(Self {
            ca_cert,
            ca_key,
            ca_cert_der,
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(LEAF_CACHE_CAPACITY).expect("leaf cache capacity is non-zero"),
            )),
        })
    }

    /// Get or mint a `CertifiedKey` for the given domain, keyed on its
    /// normalized form. Leaf certs are ECDSA P-256, valid from 2026-01-01 to
    /// now+1y, with SAN=domain. Minting takes well under a millisecond, so it
    /// happens under the lock rather than racing two mints for one name.
    pub fn certified_key_for_domain(&self, domain: &str) -> anyhow::Result<Arc<CertifiedKey>> {
        let host = crate::net::hostname::normalize_host(domain);
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(key) = cache.get(&host) {
            return Ok(Arc::clone(key));
        }
        let arc = Arc::new(self.mint_leaf(&host)?);
        cache.put(host, Arc::clone(&arc));
        drop(cache);
        Ok(arc)
    }

    /// The CA certificate in DER, for a client that must trust minted leaves.
    pub fn ca_cert_der(&self) -> &CertificateDer<'static> {
        &self.ca_cert_der
    }

    /// Number of cached certificates.
    pub fn cache_size(&self) -> usize {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Mint a leaf certificate for the given domain, signed by the CA.
    fn mint_leaf(&self, domain: &str) -> anyhow::Result<CertifiedKey> {
        let leaf_key = KeyPair::generate()?;

        let mut params = CertificateParams::new(vec![domain.to_string()])?;
        params.distinguished_name.push(rcgen::DnType::CommonName, domain);
        params.subject_alt_names = vec![SanType::DnsName(domain.try_into()?)];
        params.not_before = time::Date::from_calendar_date(2026, time::Month::January, 1)
            .unwrap()
            .midnight()
            .assume_utc();
        params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(365);
        params.is_ca = IsCa::NoCa;
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];

        // Sign the leaf with our CA.
        let leaf_cert = params.signed_by(&leaf_key, &self.ca_cert, &self.ca_key)?;
        let leaf_der = CertificateDer::from(leaf_cert.der().to_vec());

        // Build the chain: [leaf, ca].
        let chain = vec![leaf_der, self.ca_cert_der.clone()];

        // Build the rustls signing key from the leaf's private key.
        let leaf_key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&leaf_key_der)?;

        Ok(CertifiedKey::new(chain, signing_key))
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
