use super::*;

#[test]
fn generation_parser_requires_canonical_nonzero_hex() {
    assert_eq!(
        parse_generation("07070707070707070707070707070707").unwrap().as_bytes(),
        [7; 16]
    );
    for invalid in [
        "",
        "00000000000000000000000000000000",
        "0707070707070707070707070707070",
        "070707070707070707070707070707070",
        "0707070707070707070707070707070G",
        "0707070707070707070707070707070A",
    ] {
        assert!(parse_generation(invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn failed_policy_update_preserves_the_running_engine_revision() {
    let active_policy = br#"
[network]
[user_rules.profiles.rules.worker_http]
name = "worker_http"
action = "allow"
match = 'http.host == "worker.example"'
[corp_rules]
"#;
    let mut state = ProxyRuntimeState::default();
    let digest = state.apply(active_policy).unwrap();

    assert!(state.apply(b"[network]\nunknown = true").is_err());
    assert_eq!(state.engine.as_ref().unwrap().policy().snapshot().digest(), digest);
}

#[tokio::test]
async fn http_runtime_requires_every_host_capability() {
    let active_policy = br#"
[network]
[user_rules.profiles.rules.worker_http]
name = "worker_http"
action = "allow"
match = 'http.host == "worker.example"'
[corp_rules]
"#;
    let mut state = ProxyRuntimeState::default();
    state.apply(active_policy).unwrap();
    assert!(state.http_config().unwrap().is_none());

    state.attach_ledger(Arc::new(capsem_logger::DbWriter::open_in_memory(8).unwrap()));
    assert!(state.http_config().unwrap().is_none());

    state.attach_credentials(Arc::new(UnavailableCredentials));
    assert!(state.http_config().unwrap().is_none());

    let (client, _broker) = UnixStream::pair().unwrap();
    state.attach_upstream(Arc::new(
        capsem_core::net::upstream_grant::UpstreamGrantClient::start(client).unwrap(),
    ));
    assert!(state.http_config().unwrap().is_none());
    let (mcp, _requests) = capsem_proto::mcp_aggregator::AggregatorClient::channel(1);
    state.attach_mcp(
        mcp,
        capsem_proto::proxy_mcp::ProxyMcpHello::new(Vec::new(), 1, 1, 1, 1).unwrap(),
    );
    let config = state.http_config().unwrap().expect("complete capability set");
    assert!(config.upstream_grants.is_some());
    assert!(config.mcp_endpoint.is_some());
    assert_eq!(
        config.engine.policy().snapshot().digest(),
        state.engine.as_ref().unwrap().policy().snapshot().digest()
    );
}

#[tokio::test]
async fn standalone_http_runtime_pins_provider_without_mcp_capability() {
    let active_policy = br#"
[network]
[user_rules.profiles.rules.worker_http]
name = "worker_http"
action = "allow"
match = 'http.host == "api.openai.com"'
[corp_rules]
"#;
    let mut state = ProxyRuntimeState::standalone("openai".to_string());
    state.apply(active_policy).unwrap();
    state.attach_ledger(Arc::new(capsem_logger::DbWriter::open_in_memory(8).unwrap()));
    state.attach_credentials(Arc::new(UnavailableCredentials));
    let (client, _broker) = UnixStream::pair().unwrap();
    state.attach_upstream(Arc::new(
        capsem_core::net::upstream_grant::UpstreamGrantClient::start(client).unwrap(),
    ));

    let runtime = state.http_runtime().unwrap().expect("standalone runtime is ready");

    assert!(runtime.config.mcp_endpoint.is_none());
    let target = runtime.target.expect("standalone target");
    assert_eq!(target.provider_id(), "openai");
    assert_eq!(target.domain(), "api.openai.com");
}

struct EmptyPrivateNames;

impl capsem_core::net::dns::private::PrivateNames for EmptyPrivateNames {
    fn address_of<'a>(&'a self, _name: &'a str) -> capsem_core::net::dns::private::Lookup<'a, std::net::Ipv4Addr> {
        Box::pin(async { None })
    }

    fn name_of(&self, _address: std::net::Ipv4Addr) -> capsem_core::net::dns::private::Lookup<'_, String> {
        Box::pin(async { None })
    }
}

#[tokio::test]
async fn dns_runtime_requires_ledger_upstream_and_private_names() {
    let mut state = ProxyRuntimeState::default();
    state
        .apply(
            br#"
[network]
[user_rules.profiles.rules.worker_dns]
name = "worker_dns"
action = "allow"
match = 'dns.qname == "worker.example"'
[corp_rules]
"#,
        )
        .unwrap();
    state.attach_ledger(Arc::new(capsem_logger::DbWriter::open_in_memory(8).unwrap()));
    let (client, _broker) = UnixStream::pair().unwrap();
    state.attach_upstream(Arc::new(
        capsem_core::net::upstream_grant::UpstreamGrantClient::start(client).unwrap(),
    ));
    assert!(state.dns_runtime().is_none());

    state.attach_private_names(Arc::new(EmptyPrivateNames));
    assert!(state.dns_runtime().is_some());
}
