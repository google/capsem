use std::sync::Arc;

/// Re-exported so capsem-app can reference the type without depending on rustls.
pub type UpstreamTlsConfig = rustls::ClientConfig;

/// Build the upstream TLS client config (trusts standard webpki roots).
pub fn make_upstream_tls_config() -> Arc<rustls::ClientConfig> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("TLS config")
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Arc::new(config)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UpstreamConnectTarget {
    pub(super) address: String,
    pub(super) plaintext_tls: bool,
}

pub(super) fn upstream_connect_target(domain: &str, upstream_port: u16) -> UpstreamConnectTarget {
    #[cfg(any(test, debug_assertions))]
    if let Ok(overrides) = std::env::var("CAPSEM_TEST_UPSTREAM_OVERRIDES") {
        let key = format!("{domain}:{upstream_port}");
        for entry in overrides.split(',') {
            let Some((source, target)) = entry.split_once('=') else {
                continue;
            };
            if source.trim().eq_ignore_ascii_case(&key) {
                let target = target.trim();
                if !target.is_empty() {
                    if let Some(address) = target.strip_prefix("http://") {
                        return UpstreamConnectTarget {
                            address: address.to_string(),
                            plaintext_tls: true,
                        };
                    }
                    if let Some(address) = target.strip_prefix("https://") {
                        return UpstreamConnectTarget {
                            address: address.to_string(),
                            plaintext_tls: false,
                        };
                    }
                    return UpstreamConnectTarget {
                        address: target.to_string(),
                        plaintext_tls: false,
                    };
                }
            }
        }
    }

    UpstreamConnectTarget {
        address: format!("{domain}:{upstream_port}"),
        plaintext_tls: false,
    }
}
