use super::*;

const CA_KEY: &str = include_str!("../../../../../security/keys/capsem-ca.key");
const CA_CERT: &str = include_str!("../../../../../security/keys/capsem-ca.crt");

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
