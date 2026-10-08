use std::sync::Arc;

use rustls::pki_types::{pem::PemObject, CertificateDer};

use super::CA_CERT;

/// Build a rustls client config that trusts the Capsem MITM CA.
pub(super) fn make_tls_client_config() -> rustls::ClientConfig {
    let mut root_store = rustls::RootCertStore::empty();
    let certs: Vec<_> = CertificateDer::pem_slice_iter(CA_CERT.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    for cert in certs {
        root_store.add(cert).unwrap();
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}
