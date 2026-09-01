use super::*;

static TEST_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::path::Path>) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value.as_ref());
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn test_server_def() -> McpServerDef {
    McpServerDef {
        name: "test".to_string(),
        url: "https://mcp.example.com/v1".to_string(),
        headers: HashMap::new(),
        auth: None,
        enabled: true,
        source: "test".to_string(),
        command: None,
        args: vec![],
        env: HashMap::new(),
        pool_size: None,
        pool_safe_tools: Vec::new(),
    }
}

#[test]
fn new_manager_has_empty_catalogs() {
    let mgr = McpServerManager::new(vec![test_server_def()], reqwest::Client::new());
    assert!(mgr.tool_catalog().is_empty());
    assert!(mgr.resource_catalog().is_empty());
    assert!(mgr.prompt_catalog().is_empty());
    assert_eq!(mgr.definitions().len(), 1);
}

#[test]
fn disabled_server_definition_stored() {
    let mut def = test_server_def();
    def.enabled = false;
    let mgr = McpServerManager::new(vec![def], reqwest::Client::new());
    assert_eq!(mgr.definitions().len(), 1);
    assert!(!mgr.definitions()[0].enabled);
}

#[test]
fn stdio_server_stored() {
    let mut def = test_server_def();
    def.command = Some("/usr/bin/my-mcp-server".to_string());
    let mgr = McpServerManager::new(vec![def], reqwest::Client::new());
    assert_eq!(mgr.definitions().len(), 1);
    assert!(mgr.definitions()[0].is_stdio());
}

#[test]
fn tool_count_for_server_empty() {
    let mgr = McpServerManager::new(vec![test_server_def()], reqwest::Client::new());
    assert_eq!(mgr.tool_count_for_server("test"), 0);
}

#[test]
fn tool_count_for_server_nonexistent() {
    let mgr = McpServerManager::new(vec![], reqwest::Client::new());
    assert_eq!(mgr.tool_count_for_server("nonexistent"), 0);
}

#[tokio::test]
async fn call_tool_unknown_server_errors() {
    let mgr = McpServerManager::new(vec![], reqwest::Client::new());
    let result = mgr.call_tool("unknown__tool", serde_json::json!({})).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not running"));
}

#[tokio::test]
async fn call_tool_no_separator_errors() {
    let mgr = McpServerManager::new(vec![], reqwest::Client::new());
    let result = mgr.call_tool("noseparator", serde_json::json!({})).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("invalid namespaced"));
}

#[tokio::test]
async fn read_resource_invalid_uri_errors() {
    let mgr = McpServerManager::new(vec![], reqwest::Client::new());
    let result = mgr.read_resource("http://invalid").await;
    assert!(result.is_err());
}

#[test]
fn lookup_tool_peer_unknown_server_errors() {
    let mgr = McpServerManager::new(vec![], reqwest::Client::new());
    let result = mgr.lookup_tool_peer("unknown__tool");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not running"));
}

#[test]
fn lookup_tool_peer_no_separator_errors() {
    let mgr = McpServerManager::new(vec![], reqwest::Client::new());
    let result = mgr.lookup_tool_peer("noseparator");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("invalid namespaced"));
}

// ── ServerPool round-robin tests (T3 angle 2) ───────────────────

#[test]
fn next_peer_index_single_peer_always_zero() {
    let counter = AtomicUsize::new(0);
    // Pool of 1 peer ⇒ always idx 0 regardless of pool-safe flag.
    for _ in 0..10 {
        assert_eq!(next_peer_index(1, true, &counter), 0);
        assert_eq!(next_peer_index(1, false, &counter), 0);
    }
    // Counter never bumped (we early-return).
    assert_eq!(counter.load(Ordering::Relaxed), 0);
}

#[test]
fn next_peer_index_zero_peers_returns_zero() {
    // Defensive: 0 peers shouldn't crash; returns 0 (caller should
    // never invoke pick with empty pool but mod-by-zero would panic).
    let counter = AtomicUsize::new(0);
    assert_eq!(next_peer_index(0, true, &counter), 0);
    assert_eq!(next_peer_index(0, false, &counter), 0);
}

#[test]
fn next_peer_index_unsafe_tool_pins_to_zero() {
    let counter = AtomicUsize::new(0);
    // Pool of 4 peers, but tool is NOT pool-safe ⇒ always idx 0.
    for _ in 0..20 {
        assert_eq!(next_peer_index(4, false, &counter), 0);
    }
    assert_eq!(counter.load(Ordering::Relaxed), 0);
}

#[test]
fn next_peer_index_safe_tool_round_robins() {
    let counter = AtomicUsize::new(0);
    let peer_count = 4;
    // First peer_count calls cover every index exactly once.
    let mut seen = std::collections::HashSet::new();
    for _ in 0..peer_count {
        seen.insert(next_peer_index(peer_count, true, &counter));
    }
    assert_eq!(seen, (0..peer_count).collect::<HashSet<_>>());
}

#[test]
fn next_peer_index_safe_tool_balanced_over_many_calls() {
    let counter = AtomicUsize::new(0);
    let peer_count = 4;
    let n = 4_000;
    let mut hits = vec![0usize; peer_count];
    for _ in 0..n {
        hits[next_peer_index(peer_count, true, &counter)] += 1;
    }
    // Each peer hit exactly n / peer_count times (round-robin is
    // deterministic, not random).
    for h in &hits {
        assert_eq!(*h, n / peer_count);
    }
}

#[test]
fn next_peer_index_counter_wraps_cleanly_at_usize_overflow() {
    // Round-robin uses fetch_add + modulo; if peer_count doesn't
    // divide usize::MAX evenly the wraparound produces a non-uniform
    // step at the wrap point. We accept that — the cost is one
    // imbalanced bucket every 2^63 calls, which is irrelevant in
    // practice. This test just asserts no panic at the boundary.
    let counter = AtomicUsize::new(usize::MAX - 1);
    assert!(next_peer_index(4, true, &counter) < 4);
    assert!(next_peer_index(4, true, &counter) < 4); // wraps
    assert!(next_peer_index(4, true, &counter) < 4); // post-wrap
}

#[test]
fn server_pool_pick_routes_pool_safe_via_round_robin() {
    // Build a ServerPool by hand (no real RunningServer needed —
    // pick() returns &RunningServer but the test only inspects the
    // index it would have picked via next_peer_index).
    // We can't synthesize RunningServer (no public ctor), so this
    // test exercises the helper directly. Coverage of the
    // ServerPool::pick branching arrives via the live integration
    // test once a pool_size > 1 def is wired (see live integration
    // tests below).
    let counter = AtomicUsize::new(0);
    let safe: HashSet<String> = ["echo".into()].iter().cloned().collect();
    // Mimic the ServerPool::pick guard.
    for tool in &["echo", "echo", "echo", "fetch_http"] {
        let is_safe = safe.contains(*tool);
        let idx = next_peer_index(3, is_safe, &counter);
        if *tool == "echo" {
            assert!(idx < 3, "echo should round-robin");
        } else {
            assert_eq!(idx, 0, "fetch_http (not in safe set) pins to 0");
        }
    }
}

fn local_http_mcp_def(url: String, auth: Option<McpAuthConfig>) -> McpServerDef {
    let def = McpServerDef {
        name: "localtest".to_string(),
        url,
        headers: HashMap::new(),
        auth,
        enabled: true,
        source: "test".to_string(),
        command: None,
        args: vec![],
        env: HashMap::new(),
        pool_size: None,
        pool_safe_tools: Vec::new(),
    };
    assert!(!def.is_stdio());
    def
}

#[tokio::test]
async fn duplicate_tool_definitions_are_advertised_once() {
    let harness = crate::test_support::spawn_duplicate_tool_mcp_server().await.unwrap();
    let def = local_http_mcp_def(harness.url.clone(), None);
    let mut mgr = McpServerManager::new(vec![def.clone()], reqwest::Client::new());

    mgr.connect_and_initialize(&def).await.unwrap();

    let tools: Vec<_> = mgr
        .tool_catalog()
        .iter()
        .filter(|tool| tool.namespaced_name == "localtest__echo")
        .collect();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].description.as_deref(), Some("first definition"));
}

#[tokio::test]
async fn local_http_mcp_e2e_uses_brokered_oauth_and_records_tool_call() {
    let _lock = TEST_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let _store_guard = EnvVarGuard::set("CAPSEM_CREDENTIAL_STORE_PATH", dir.path().join("store.json"));
    let harness = crate::test_support::spawn_recording_mcp_server().await.unwrap();
    let credential_ref = format!("credential:blake3:{}", "d".repeat(64));
    capsem_credentials::CredentialStore::global().clear_for_test();
    capsem_credentials::CredentialStore::global()
        .capture(
            capsem_credentials::CredentialProvider::Mcp,
            &credential_ref,
            "local-mcp-oauth-token",
        )
        .expect("test credential should store");
    let def = local_http_mcp_def(
        harness.url.clone(),
        Some(McpAuthConfig {
            kind: McpAuthKind::OAuth,
            credential_ref,
        }),
    );
    let mut mgr = McpServerManager::new(vec![def.clone()], reqwest::Client::new());

    mgr.connect_and_initialize(&def)
        .await
        .expect("local MCP server should initialize");

    assert!(
        mgr.is_running("localtest"),
        "local server should be running after successful init"
    );
    assert!(
        mgr.tool_catalog()
            .iter()
            .any(|tool| tool.namespaced_name == "localtest__echo"),
        "local MCP should expose echo, got catalog: {:?}",
        mgr.tool_catalog()
    );

    let result = mgr
        .call_tool("localtest__echo", serde_json::json!({ "message": "winter" }))
        .await
        .expect("local echo tool should dispatch");
    let result_json = serde_json::to_string(&result).unwrap();
    assert!(
        result_json.contains("echo:winter"),
        "tool result should include echo output: {result_json}"
    );

    let tool_calls = harness.state.tool_calls();
    assert_eq!(
        tool_calls,
        vec![crate::test_support::RecordedMcpToolCall {
            tool: "echo".to_string(),
            arguments: serde_json::json!({ "message": "winter" }),
        }]
    );

    let requests = harness.state.http_requests();
    assert!(
        requests.iter().any(|request| request
            .header("authorization")
            .is_some_and(|value| value == "Bearer local-mcp-oauth-token")),
        "local MCP server should receive the broker-resolved bearer token: {requests:?}"
    );
    assert!(
        requests.iter().all(|request| !request
            .header("authorization")
            .unwrap_or_default()
            .contains("credential:blake3:")),
        "broker references must not be sent as auth material: {requests:?}"
    );
}

#[tokio::test]
async fn local_http_mcp_unresolved_broker_ref_fails_before_network_dispatch() {
    let _lock = TEST_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let _store_guard = EnvVarGuard::set("CAPSEM_CREDENTIAL_STORE_PATH", dir.path().join("store.json"));
    let harness = crate::test_support::spawn_recording_mcp_server().await.unwrap();
    let def = local_http_mcp_def(
        harness.url.clone(),
        Some(McpAuthConfig {
            kind: McpAuthKind::Bearer,
            credential_ref: "credential:blake3:missing-local-mcp-token".to_string(),
        }),
    );
    let mut mgr = McpServerManager::new(vec![def.clone()], reqwest::Client::new());

    let err = mgr
        .connect_and_initialize(&def)
        .await
        .expect_err("unresolved broker ref must fail closed");

    assert!(
        err.to_string().contains("could not be resolved"),
        "unexpected error: {err:#}"
    );
    assert!(
        harness.state.http_requests().is_empty(),
        "unresolved broker refs must fail before any remote MCP request"
    );
}

// ── Pool routing edges ─────────────────────────────────────────────
//
// Round-robin distribution is already covered above. These pin down the two
// guard conditions around it: a degenerate pool must never index out of
// bounds, and pinning a pool-unsafe tool must not consume a round-robin turn
// (if it did, safe tools would skip peers).

#[test]
fn degenerate_pools_always_pick_the_first_peer() {
    let counter = AtomicUsize::new(0);
    for safe in [true, false] {
        assert_eq!(next_peer_index(1, safe, &counter), 0, "single-peer pool");
        assert_eq!(next_peer_index(0, safe, &counter), 0, "empty pool");
    }
}

#[test]
fn pinning_a_pool_unsafe_tool_does_not_consume_a_round_robin_turn() {
    let counter = AtomicUsize::new(0);

    assert_eq!(next_peer_index(3, false, &counter), 0);
    assert_eq!(next_peer_index(3, false, &counter), 0);

    assert_eq!(
        next_peer_index(3, true, &counter),
        0,
        "the first pool-safe call still starts at peer 0"
    );
    assert_eq!(next_peer_index(3, true, &counter), 1);
}

// ── Outbound header safety ─────────────────────────────────────────
//
// Custom headers come from server definitions, not from this process. One that
// collided with `Mcp-Session-Id` would let configuration steer another
// session's stream, so the reserved set must stay rejected -- case included,
// since header names are compared case-insensitively.

fn header(name: &str) -> HeaderName {
    HeaderName::from_bytes(name.as_bytes()).expect("valid header name")
}

#[test]
fn reserved_mcp_headers_cannot_be_overridden_by_a_server_definition() {
    for name in [
        "accept",
        "Accept",
        "ACCEPT",
        "mcp-session-id",
        "Mcp-Session-Id",
        "last-event-id",
        "Last-Event-Id",
    ] {
        assert!(
            validate_mcp_custom_header(&header(name)).is_err(),
            "{name} is reserved and must be refused"
        );
    }
}

#[test]
fn protocol_version_is_the_one_reserved_header_a_definition_may_set() {
    for name in ["mcp-protocol-version", "MCP-Protocol-Version"] {
        assert!(
            validate_mcp_custom_header(&header(name)).is_ok(),
            "{name} is explicitly carved out"
        );
    }
}

#[test]
fn ordinary_headers_pass_validation() {
    for name in ["authorization", "x-api-key", "user-agent", "content-type"] {
        assert!(validate_mcp_custom_header(&header(name)).is_ok(), "{name}");
    }
}

#[test]
fn rejected_header_error_names_the_offending_header() {
    let err = validate_mcp_custom_header(&header("mcp-session-id")).unwrap_err();
    assert!(err.contains("mcp-session-id"), "unexpected error: {err}");
}

// ── WWW-Authenticate scope parsing ─────────────────────────────────

#[test]
fn scope_is_read_from_a_quoted_challenge() {
    let scope = extract_mcp_scope_from_header(r#"Bearer realm="https://idp.example.com", scope="mcp:read mcp:write""#);
    assert_eq!(scope.as_deref(), Some("mcp:read mcp:write"));
}

#[test]
fn unquoted_scope_stops_at_the_first_delimiter() {
    for (header, want) in [
        ("Bearer scope=mcp:read, realm=x", "mcp:read"),
        ("Bearer scope=mcp:read;realm=x", "mcp:read"),
        ("Bearer scope=mcp:read realm=x", "mcp:read"),
        ("Bearer scope=mcp:read", "mcp:read"),
    ] {
        assert_eq!(extract_mcp_scope_from_header(header).as_deref(), Some(want), "{header}");
    }
}

#[test]
fn scope_key_matches_case_insensitively_but_keeps_the_original_value_case() {
    let scope = extract_mcp_scope_from_header(r#"Bearer SCOPE="MCP:Read""#);
    assert_eq!(scope.as_deref(), Some("MCP:Read"));
}

#[test]
fn a_challenge_without_a_scope_yields_none() {
    for header in [
        "Bearer realm=\"https://idp.example.com\"",
        "Basic",
        "",
        "Bearer scope=",
        "Bearer scope=,realm=x",
    ] {
        assert_eq!(
            extract_mcp_scope_from_header(header),
            None,
            "{header:?} carries no usable scope"
        );
    }
}

// ── JSON-RPC error sniffing ────────────────────────────────────────

#[test]
fn only_a_json_rpc_error_body_parses_as_an_error() {
    let error = parse_mcp_json_rpc_error(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"bad request"}}"#);
    assert!(error.is_some(), "an error envelope must be recognized");
}

#[test]
fn success_and_malformed_bodies_are_not_treated_as_errors() {
    for body in [
        r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"error":"not a json-rpc envelope"}"#,
        "not json at all",
        "",
    ] {
        assert!(
            parse_mcp_json_rpc_error(body).is_none(),
            "{body:?} must not be reported as a JSON-RPC error"
        );
    }
}
