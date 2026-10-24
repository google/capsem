//! Shared real MITM config and one-connection owners.
use super::*;

/// Build a proxy config from allow/block lists for integration tests.
///
/// Enforcement intent is compiled into `SecurityRuleSet` so tests exercise the
/// same security-event/CEL rail as production. `NetworkMechanics` remains present
/// for non-enforcement proxy settings such as body capture and HTTP port gates.
pub(super) fn make_proxy_config(
    allowed: &[&str],
    blocked: &[&str],
    default_allow: bool,
) -> (Arc<MitmProxyConfig>, Arc<DbWriter>) {
    make_proxy_config_full(allowed, blocked, default_allow, &[80])
}

pub(super) fn host_pattern_condition(pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return None;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        let escaped = regex::escape(suffix);
        return Some(format!("http.host.matches(\"(^|.*\\\\.){escaped}$\")"));
    }
    Some(format!("http.host == \"{}\"", pattern.replace('"', "\\\"")))
}

pub(super) fn host_pattern_negative_condition(pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return None;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        let escaped = regex::escape(suffix);
        return Some(format!("http.host.matches(\"(^|.*\\\\.){escaped}$\") == false"));
    }
    Some(format!("http.host != \"{}\"", pattern.replace('"', "\\\"")))
}

pub(super) fn security_rules_for_proxy(
    allowed: &[&str],
    blocked: &[&str],
    default_allow: bool,
) -> capsem_core::net::policy_config::SecurityRuleSet {
    let mut toml = String::new();
    let blocked_conditions: Vec<String> = blocked
        .iter()
        .filter_map(|pattern| host_pattern_condition(pattern))
        .collect();
    if !blocked_conditions.is_empty() {
        toml.push_str(
            r#"
[profiles.rules.block_test_hosts]
name = "block_test_hosts"
action = "block"
reason = "test blocked host"
match = '''
"#,
        );
        toml.push_str(&blocked_conditions.join("\n|| "));
        toml.push_str(
            r#"
'''
"#,
        );
    }

    if !default_allow {
        let allowed_conditions: Vec<String> = allowed
            .iter()
            .filter_map(|pattern| host_pattern_negative_condition(pattern))
            .collect();
        toml.push_str(
            r#"
[profiles.rules.block_test_default_deny]
name = "block_test_default_deny"
action = "block"
reason = "test default deny"
match = '''
"#,
        );
        if allowed_conditions.is_empty() {
            toml.push_str("http.host != \"\"");
        } else {
            toml.push_str(&allowed_conditions.join("\n&& "));
        }
        toml.push_str(
            r#"
'''
"#,
        );
    }

    let profile =
        capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(&toml).expect("test security rule profile");
    capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        capsem_core::net::policy_config::SecurityRuleSource::User,
    )
    .expect("test security rules")
}

/// Like `make_proxy_config` but lets the caller override the
/// `http_upstream_ports` allowlist (T2.2). Used by T2.3's Ollama-shape
/// test that runs a fake upstream on an OS-assigned port.
pub(super) fn make_proxy_config_full(
    allowed: &[&str],
    blocked: &[&str],
    default_allow: bool,
    http_ports: &[u16],
) -> (Arc<MitmProxyConfig>, Arc<DbWriter>) {
    make_proxy_config_with_security_rules(security_rules_for_proxy(allowed, blocked, default_allow), http_ports)
}

pub(super) fn make_proxy_config_with_security_rules(
    security_rules: capsem_core::net::policy_config::SecurityRuleSet,
    http_ports: &[u16],
) -> (Arc<MitmProxyConfig>, Arc<DbWriter>) {
    let mut policy_inner = NetworkMechanics::new();
    policy_inner.http_upstream_ports = http_ports.to_vec();
    make_proxy_config_with_mechanics(security_rules, policy_inner)
}

/// Preserve a real guest-facing TLS/SNI connection while routing the upstream
/// leg to a deterministic loopback HTTP fixture. The override changes only
/// transport routing; policy and telemetry continue to observe `domain:443`.
pub(super) fn make_proxy_config_with_local_http_upstream(
    domain: &str,
    upstream_port: u16,
) -> (Arc<MitmProxyConfig>, Arc<DbWriter>) {
    let mut policy_inner = NetworkMechanics::new();
    policy_inner.upstream_overrides.insert(
        format!("{domain}:443"),
        UpstreamOverride {
            dial: format!("127.0.0.1:{upstream_port}"),
            protocol: UpstreamOverrideProtocol::Http,
        },
    );
    make_proxy_config_with_mechanics(security_rules_for_proxy(&[domain], &[], false), policy_inner)
}

pub(super) fn make_proxy_config_with_mechanics(
    security_rules: capsem_core::net::policy_config::SecurityRuleSet,
    policy_inner: NetworkMechanics,
) -> (Arc<MitmProxyConfig>, Arc<DbWriter>) {
    let ca = Arc::new(CertAuthority::load(CA_KEY, CA_CERT).unwrap());
    let policy = capsem_core::net::proxy_engine::ProxyPolicyHandle::new(
        capsem_core::net::proxy_engine::ProxyPolicySnapshot::new(
            "blake3:integration".into(),
            policy_inner,
            security_rules,
            BTreeMap::new(),
            capsem_core::net::policy_config::ProviderRuleProfile::builtin_defaults()
                .endpoint_registry()
                .expect("builtin provider endpoint registry"),
        ),
    );
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&dir.path().join("test.db"), 256).unwrap());
    std::mem::forget(dir); // the tempdir lives as long as the test
    let engine = Arc::new(capsem_core::net::proxy_engine::ProxyEngine::local(
        policy.clone(),
        Arc::clone(&db),
    ));
    let telemetry = Arc::new(mitm_proxy::telemetry_hook::TelemetryDeps {
        db: db.clone(),
        credentials: engine.credentials(),
        pricing: Arc::new(capsem_core::net::ai_traffic::pricing::PricingTable::load()),
        trace_state: Arc::new(std::sync::Mutex::new(capsem_core::net::ai_traffic::TraceState::new())),
    });
    let pipeline = mitm_proxy::make_production_pipeline(Arc::clone(&telemetry));
    let upstream_grants = Arc::new(IntegrationGrants::new(policy, UpstreamResolver::system()));
    let config = Arc::new(MitmProxyConfig {
        server_tls: mitm_proxy::make_server_tls_config(&ca),
        ca,
        engine,
        db: db.clone(),
        upstream_tls: mitm_proxy::make_upstream_tls_config(),
        telemetry,
        pipeline,
        mcp_endpoint: None,
        upstream_resolver: Default::default(),
        upstream_grants: Some(upstream_grants),
        semantic_body_budget: MitmProxyConfig::new_semantic_body_budget(),
        shutdown: Default::default(),
    });
    (config, db)
}

pub(super) fn security_rules_from_toml(toml: &str) -> capsem_core::net::policy_config::SecurityRuleSet {
    let profile =
        capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(toml).expect("test security rule profile");
    capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        capsem_core::net::policy_config::SecurityRuleSource::User,
    )
    .expect("test security rules")
}

/// Spawn the MITM proxy on a TCP listener and return the address.
/// The proxy handles exactly one connection then exits.
pub(super) async fn spawn_proxy(config: Arc<MitmProxyConfig>) -> (tokio::task::JoinHandle<()>, std::net::SocketAddr) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let std_stream = stream.into_std().unwrap();
        let fd = std_stream.into();
        mitm_proxy::handle_connection(fd, config).await;
    });

    (handle, addr)
}

/// Spawn the standalone model adapter for exactly one accepted connection.
pub(super) async fn spawn_standalone_proxy(
    config: Arc<MitmProxyConfig>,
    provider_id: &str,
) -> (tokio::task::JoinHandle<()>, std::net::SocketAddr) {
    let target = mitm_proxy::StandaloneModelTarget::from_registry(
        config.engine.policy().snapshot().model_endpoints(),
        provider_id,
    )
    .expect("configured standalone provider");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        mitm_proxy::handle_standalone_connection(stream.into_std().unwrap().into(), config, target).await;
    });
    (handle, addr)
}
