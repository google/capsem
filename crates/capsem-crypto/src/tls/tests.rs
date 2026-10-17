use super::*;

#[test]
fn issuer_preserves_chain_and_keys_rejected_without_the_capsem_ca() {
    let ca = CertificateIssuer::load(
        include_str!("../../../capsem-core/resources/ca/capsem-ca.key"),
        include_str!("../../../capsem-core/resources/ca/capsem-ca.crt"),
    )
    .unwrap();
    let leaf = ca.mint_server_certificate("example.com").unwrap();
    assert_eq!(leaf.cert.len(), 2);
    assert_eq!(&leaf.cert[1], ca.certificate_der());
    assert!(leaf.keys_match().is_ok());
    assert!(CertificateIssuer::load("invalid", "invalid").is_err());
    assert!(ca.mint_server_certificate("invalid-☃.example").is_err());
    let (certificate, key) = self_signed(vec!["localhost".into()]).unwrap();
    let configuration = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key);
    assert!(configuration.is_ok());
}
