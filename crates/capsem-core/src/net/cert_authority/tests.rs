use super::*;

const CA_KEY: &str = include_str!("../../../resources/ca/capsem-ca.key");
const CA_CERT: &str = include_str!("../../../resources/ca/capsem-ca.crt");

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
        leaf.as_ref().windows(domain_bytes.len()).any(|w| w == domain_bytes),
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
            ca.certified_key_for_domain(&format!("thread{i}.example.com")).unwrap();
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(ca.cache_size(), 10);
}

// -- Leaf cache identity and bounds --
//
// The guest chooses the SNI. The cache it feeds must key on the host's
// normalized identity (so `A.example`, `a.example`, and `a.example.` share
// one leaf, matching what policy evaluates) and must not grow without bound
// under a flood of distinct names.

#[test]
fn cache_key_ignores_case_and_trailing_dot() {
    let ca = load_ca();
    let upper = ca.certified_key_for_domain("A.example").unwrap();
    let lower = ca.certified_key_for_domain("a.example").unwrap();
    let dotted = ca.certified_key_for_domain("a.example.").unwrap();
    assert!(Arc::ptr_eq(&upper, &lower), "case must not split the cache");
    assert!(Arc::ptr_eq(&upper, &dotted), "a trailing dot must not split the cache");
    assert_eq!(ca.cache_size(), 1);
}

#[test]
fn minted_leaf_names_the_normalized_host() {
    let ca = load_ca();
    let key = ca.certified_key_for_domain("Leaf.Example.").unwrap();
    let leaf = key.cert[0].as_ref();
    assert!(leaf.windows(12).any(|w| w == b"leaf.example"));
    assert!(!leaf.windows(13).any(|w| w == b"Leaf.Example."));
}

#[test]
fn cache_is_bounded_under_a_distinct_sni_flood() {
    let ca = load_ca();
    for i in 0..1500 {
        ca.certified_key_for_domain(&format!("h{i}.flood.example")).unwrap();
    }
    assert!(
        ca.cache_size() < 1500,
        "a guest must not be able to grow the leaf cache without bound"
    );
}

/// Rewrite the SNI host inside a ClientHello record produced by a rustls
/// client. rustls refuses to *send* a trailing dot (RFC 6066), but its
/// server accepts one, so the adversarial spelling has to be spliced in.
fn client_hello_with_sni(name: &str) -> Vec<u8> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    let mut conn = rustls::ClientConnection::new(Arc::new(config), "placeholder.test".try_into().unwrap()).unwrap();
    let mut hello = Vec::new();
    conn.write_tls(&mut hello).unwrap();

    let u16_at = |b: &[u8], i: usize| u16::from_be_bytes([b[i], b[i + 1]]) as usize;
    let bump = |b: &mut [u8], i: usize, width: usize, delta: isize| {
        let mut v = 0usize;
        for k in 0..width {
            v = (v << 8) | b[i + k] as usize;
        }
        let v = (v as isize + delta) as usize;
        for k in 0..width {
            b[i + k] = (v >> (8 * (width - 1 - k))) as u8;
        }
    };

    assert_eq!(hello[0], 0x16, "handshake record");
    assert_eq!(hello[5], 1, "ClientHello");
    let mut pos = 5 + 4 + 2 + 32; // record + handshake headers, version, random
    pos += 1 + hello[pos] as usize; // session id
    pos += 2 + u16_at(&hello, pos); // cipher suites
    pos += 1 + hello[pos] as usize; // compression methods
    let extensions_len_at = pos;
    let mut ext = pos + 2;
    let end = ext + u16_at(&hello, extensions_len_at);
    let sni = loop {
        assert!(ext < end, "ClientHello carries no SNI extension");
        let (ty, len) = (u16_at(&hello, ext), u16_at(&hello, ext + 2));
        if ty == 0 {
            break ext;
        }
        ext += 4 + len;
    };
    let list_len_at = sni + 4;
    let name_len_at = list_len_at + 2 + 1; // list length, then name_type
    let name_at = name_len_at + 2;
    let old_len = u16_at(&hello, name_len_at);
    let delta = name.len() as isize - old_len as isize;
    hello.splice(name_at..name_at + old_len, name.bytes());
    for (offset, width) in [
        (name_len_at, 2),
        (list_len_at, 2),
        (sni + 2, 2),
        (extensions_len_at, 2),
        (6, 3),
        (3, 2),
    ] {
        bump(&mut hello, offset, width, delta);
    }
    hello
}

/// Drive a rustls server backed by the MITM resolver with one raw
/// ClientHello and return what the resolver recorded.
fn resolve_raw_sni(sni: &str) -> (Arc<CertAuthority>, Arc<MitmCertResolver>) {
    let ca = Arc::new(load_ca());
    let resolver = Arc::new(MitmCertResolver::new(Arc::clone(&ca)));
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::clone(&resolver) as _);
    let mut conn = rustls::ServerConnection::new(Arc::new(config)).unwrap();
    let hello = client_hello_with_sni(sni);
    conn.read_tls(&mut &hello[..]).unwrap();
    conn.process_new_packets()
        .expect("rustls accepts the spliced ClientHello");
    (ca, resolver)
}

#[test]
fn resolver_records_the_normalized_sni_from_a_raw_client_hello() {
    let (ca, resolver) = resolve_raw_sni("Example.COM.");
    assert_eq!(
        resolver.domain().as_deref(),
        Some("example.com"),
        "the domain handed to policy, dial, and telemetry is the normalized SNI"
    );
    assert_eq!(ca.cache_size(), 1);
    ca.certified_key_for_domain("example.com").unwrap();
    assert_eq!(
        ca.cache_size(),
        1,
        "the leaf minted for the raw SNI is the canonical entry"
    );
}

#[test]
fn raw_client_hello_splicer_round_trips_an_unchanged_name() {
    // Guard the test fixture itself: a ClientHello for the placeholder name
    // must resolve to the placeholder when the splice is a no-op.
    let (_ca, resolver) = resolve_raw_sni("placeholder.test");
    assert_eq!(resolver.domain().as_deref(), Some("placeholder.test"));
}
