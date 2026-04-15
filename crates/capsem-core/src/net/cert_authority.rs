/// Certificate authority for MITM proxy: loads the static Capsem CA keypair
/// and mints short-lived leaf certificates on demand for each domain.
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};

use rcgen::{CertificateParams, IsCa, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::ClientHello;
use rustls::sign::CertifiedKey;

/// Holds the static CA keypair and caches minted leaf certificates.
pub struct CertAuthority {
    ca_cert: rcgen::Certificate,
    ca_key: KeyPair,
    ca_cert_der: CertificateDer<'static>,
    cache: RwLock<HashMap<String, Arc<CertifiedKey>>>,
}

impl CertAuthority {
    /// Load a CA from PEM-encoded private key and certificate.
    ///
    /// Typically called with `include_str!("../../../config/capsem-ca.key")`.
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
            cache: RwLock::new(HashMap::new()),
        })
    }

    /// Get or mint a `CertifiedKey` for the given domain.
    ///
    /// Uses a `RwLock` cache: read-lock for cache hits, write-lock only on miss.
    /// Leaf certs are ECDSA P-256, valid from 2026-01-01 to now+1y, with SAN=domain.
    pub fn certified_key_for_domain(&self, domain: &str) -> anyhow::Result<Arc<CertifiedKey>> {
        // Fast path: cache hit under read lock.
        {
            let cache = self.cache.read().unwrap();
            if let Some(key) = cache.get(domain) {
                return Ok(Arc::clone(key));
            }
        }

        // Slow path: mint and cache under write lock.
        let mut cache = self.cache.write().unwrap();
        // Double-check after acquiring write lock (another thread may have raced).
        if let Some(key) = cache.get(domain) {
            return Ok(Arc::clone(key));
        }

        let certified_key = self.mint_leaf(domain)?;
        let arc = Arc::new(certified_key);
        cache.insert(domain.to_string(), Arc::clone(&arc));
        Ok(arc)
    }

    /// Number of cached certificates.
    pub fn cache_size(&self) -> usize {
        self.cache.read().unwrap().len()
    }

    /// Mint a leaf certificate for the given domain, signed by the CA.
    fn mint_leaf(&self, domain: &str) -> anyhow::Result<CertifiedKey> {
        let leaf_key = KeyPair::generate()?;

        let mut params = CertificateParams::new(vec![domain.to_string()])?;
        params.distinguished_name
            .push(rcgen::DnType::CommonName, domain);
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
        let leaf_key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            leaf_key.serialize_der(),
        ));
        let signing_key =
            rustls::crypto::aws_lc_rs::sign::any_supported_type(&leaf_key_der)?;

        Ok(CertifiedKey::new(chain, signing_key))
    }
}

/// rustls SNI-based certificate resolver that mints certs on demand.
///
/// Also captures the resolved domain name for use after the TLS handshake
/// (replaces the old separate SNI parser). Always mints certs even for
/// blocked domains so we can complete the TLS handshake, read the HTTP
/// request (capturing method/path), and return a proper 403 response.
pub struct MitmCertResolver {
    pub ca: Arc<CertAuthority>,
    /// Domain captured during TLS handshake from ClientHello SNI.
    pub resolved_domain: std::sync::Mutex<Option<String>>,
}

impl MitmCertResolver {
    /// Create a new resolver wrapping the given CA.
    pub fn new(ca: Arc<CertAuthority>) -> Self {
        Self {
            ca,
            resolved_domain: std::sync::Mutex::new(None),
        }
    }

    /// Get the domain captured during the last TLS handshake.
    pub fn domain(&self) -> Option<String> {
        self.resolved_domain.lock().unwrap().clone()
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
        let domain = hello.server_name()?;
        *self.resolved_domain.lock().unwrap() = Some(domain.to_owned());

        // Always mint a cert, even for blocked domains. This lets us complete
        // the TLS handshake, read the HTTP request (method, path), and return
        // a proper 403 response with telemetry.
        self.ca.certified_key_for_domain(domain).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CA_KEY: &str = include_str!("../../../../config/capsem-ca.key");
    const CA_CERT: &str = include_str!("../../../../config/capsem-ca.crt");

    fn load_ca() -> CertAuthority {
        CertAuthority::load(CA_KEY, CA_CERT).expect("failed to load CA")
    }

    #[test]
    fn load_static_ca() {
        let ca = load_ca();
        assert_eq!(ca.cache_size(), 0);
    }

    #[test]
    fn mint_domain_cert_correct_san() {
        let ca = load_ca();
        let key = ca.certified_key_for_domain("example.com").unwrap();
        // The chain should have exactly 2 certs: leaf + CA.
        assert_eq!(key.cert.len(), 2);
        // Verify the leaf cert contains the domain as a UTF-8 substring in DER.
        let leaf = &key.cert[0];
        let domain_bytes = b"example.com";
        assert!(
            leaf.as_ref()
                .windows(domain_bytes.len())
                .any(|w| w == domain_bytes),
            "leaf cert should contain example.com in DER"
        );
    }

    #[test]
    fn cache_hit_ptr_eq() {
        let ca = load_ca();
        let a = ca.certified_key_for_domain("cache-test.com").unwrap();
        let b = ca.certified_key_for_domain("cache-test.com").unwrap();
        assert!(Arc::ptr_eq(&a, &b), "cache should return same Arc");
        assert_eq!(ca.cache_size(), 1);
    }

    #[test]
    fn different_domains_different_certs() {
        let ca = load_ca();
        let a = ca.certified_key_for_domain("a.com").unwrap();
        let b = ca.certified_key_for_domain("b.com").unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(ca.cache_size(), 2);
    }

    #[test]
    fn chain_includes_ca() {
        let ca = load_ca();
        let key = ca.certified_key_for_domain("chain-test.com").unwrap();
        // Second cert in chain should be the CA cert.
        assert_eq!(key.cert[1].as_ref(), ca.ca_cert_der.as_ref());
    }

    #[test]
    fn resolver_debug_output() {
        let ca = Arc::new(load_ca());
        let resolver = MitmCertResolver::new(ca);
        let debug = format!("{:?}", resolver);
        assert!(debug.contains("MitmCertResolver"));
    }

    #[test]
    fn concurrent_minting_safe() {
        let ca = Arc::new(load_ca());
        let mut handles = Vec::new();
        for i in 0..10 {
            let ca = Arc::clone(&ca);
            handles.push(std::thread::spawn(move || {
                ca.certified_key_for_domain(&format!("thread{i}.example.com"))
                    .unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(ca.cache_size(), 10);
    }
}
