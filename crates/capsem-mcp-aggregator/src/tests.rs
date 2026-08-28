use std::collections::HashMap;

use super::*;

fn http_def(name: &str) -> McpServerDef {
    McpServerDef {
        name: name.to_string(),
        url: format!("https://{name}.example.com/v1"),
        command: None,
        args: vec![],
        env: HashMap::new(),
        headers: HashMap::new(),
        auth: None,
        enabled: true,
        source: "test".to_string(),
        pool_size: None,
        pool_safe_tools: Vec::new(),
    }
}

/// A manager holding definitions but never initialized, so no test here opens a
/// socket. `handle_request` only reaches the network through `Refresh`, and that
/// test uses disabled definitions.
fn manager(defs: Vec<McpServerDef>) -> Arc<RwLock<McpServerManager>> {
    Arc::new(RwLock::new(McpServerManager::new(
        defs,
        reqwest::Client::new(),
    )))
}

#[tokio::test]
async fn framing_error_stops_before_following_bytes_are_reinterpreted() {
    use tokio::io::AsyncWriteExt;

    let (mut writer, mut reader) = tokio::io::duplex(1024);
    tokio::spawn(async move {
        writer
            .write_all(&u32::MAX.to_be_bytes())
            .await
            .expect("write oversized prefix");
        write_frame(
            &mut writer,
            &AggregatorRequest {
                id: 42,
                method: AggregatorMethod::ListServers,
            },
        )
        .await
        .expect("write bytes following corrupt prefix");
    });

    let request = read_next_request(&mut reader).await;

    assert!(
        request.is_none(),
        "bytes after a framing error have unknowable alignment and must not be parsed"
    );
}

async fn dispatch(
    mgr: &Arc<RwLock<McpServerManager>>,
    id: u64,
    method: AggregatorMethod,
) -> AggregatorResponse {
    handle_request(mgr, AggregatorRequest { id, method }).await
}

// ── Correlation ────────────────────────────────────────────────────
//
// Responses travel back out of order -- handlers run concurrently and a single
// writer task drains them -- so capsem-process matches them to requests purely
// by `id`. A handler that dropped or rewrote the id would silently misroute a
// reply into another caller's result.

#[tokio::test]
async fn every_method_echoes_the_request_id() {
    let mgr = manager(vec![http_def("github")]);
    let methods = vec![
        AggregatorMethod::ListServers,
        AggregatorMethod::ListTools,
        AggregatorMethod::ListResources,
        AggregatorMethod::ListPrompts,
        AggregatorMethod::CallTool {
            name: "github__search".to_string(),
            arguments: serde_json::json!({}),
        },
        AggregatorMethod::ReadResource {
            uri: "github__file:///x".to_string(),
        },
        AggregatorMethod::GetPrompt {
            name: "github__review".to_string(),
            arguments: serde_json::json!({}),
        },
        AggregatorMethod::Shutdown,
    ];

    for (offset, method) in methods.into_iter().enumerate() {
        let id = 9_000 + offset as u64;
        assert_eq!(dispatch(&mgr, id, method).await.id, id);
    }
}

// ── Status projection ──────────────────────────────────────────────

#[tokio::test]
async fn list_servers_reports_definitions_as_disconnected_before_initialize() {
    let mgr = manager(vec![http_def("github")]);

    let resp = dispatch(&mgr, 1, AggregatorMethod::ListServers).await;
    let AggregatorResult::Servers { servers } = resp.body else {
        panic!("expected Servers, got {:?}", resp.body);
    };

    assert_eq!(servers.len(), 1);
    let s = &servers[0];
    assert_eq!(s.name, "github");
    assert_eq!(s.url, "https://github.example.com/v1");
    assert_eq!(s.source, "test");
    assert!(s.enabled);
    assert!(!s.is_stdio);
    assert!(
        !s.connected,
        "never initialized, so it must not read connected"
    );
    assert_eq!(s.tool_count, 0);
    assert_eq!(s.resource_count, 0);
    assert_eq!(s.prompt_count, 0);
}

#[tokio::test]
async fn list_servers_distinguishes_stdio_and_disabled_definitions() {
    let mut stdio = http_def("local");
    stdio.url = String::new();
    stdio.command = Some("/usr/bin/local-mcp".to_string());
    let mut off = http_def("off");
    off.enabled = false;

    let mgr = manager(vec![stdio, off]);
    let resp = dispatch(&mgr, 2, AggregatorMethod::ListServers).await;
    let AggregatorResult::Servers { servers } = resp.body else {
        panic!("expected Servers, got {:?}", resp.body);
    };

    assert!(servers[0].is_stdio, "command-backed server is stdio");
    assert!(!servers[1].enabled);
    assert!(!servers[1].is_stdio, "url-backed server is not stdio");
}

#[tokio::test]
async fn catalogs_are_empty_before_any_server_is_initialized() {
    let mgr = manager(vec![http_def("github")]);

    let tools = dispatch(&mgr, 3, AggregatorMethod::ListTools).await;
    let AggregatorResult::Tools { tools } = tools.body else {
        panic!("expected Tools");
    };
    assert!(tools.is_empty());

    let resources = dispatch(&mgr, 4, AggregatorMethod::ListResources).await;
    let AggregatorResult::Resources { resources } = resources.body else {
        panic!("expected Resources");
    };
    assert!(resources.is_empty());

    let prompts = dispatch(&mgr, 5, AggregatorMethod::ListPrompts).await;
    let AggregatorResult::Prompts { prompts } = prompts.body else {
        panic!("expected Prompts");
    };
    assert!(prompts.is_empty());
}

// ── Adversarial dispatch ───────────────────────────────────────────
//
// Tool names, resource URIs, and prompt names originate outside this process.
// An unresolvable one must come back as a structured Error the driver can
// forward, never a panic that takes the whole aggregator (and every in-flight
// request on it) down.

fn error_text(body: &AggregatorResult) -> &str {
    match body {
        AggregatorResult::Error { error } => error,
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn unresolvable_tool_call_returns_an_error_result() {
    let mgr = manager(vec![http_def("github")]);
    let resp = dispatch(
        &mgr,
        6,
        AggregatorMethod::CallTool {
            name: "nosuch__tool".to_string(),
            arguments: serde_json::json!({"q": 1}),
        },
    )
    .await;
    assert!(!error_text(&resp.body).is_empty());
}

#[tokio::test]
async fn unresolvable_resource_read_returns_an_error_result() {
    let mgr = manager(vec![http_def("github")]);
    let resp = dispatch(
        &mgr,
        7,
        AggregatorMethod::ReadResource {
            uri: "nosuch__file:///etc/passwd".to_string(),
        },
    )
    .await;
    assert!(!error_text(&resp.body).is_empty());
}

#[tokio::test]
async fn unresolvable_prompt_get_returns_an_error_result() {
    let mgr = manager(vec![http_def("github")]);
    let resp = dispatch(
        &mgr,
        8,
        AggregatorMethod::GetPrompt {
            name: "nosuch__prompt".to_string(),
            arguments: serde_json::json!({}),
        },
    )
    .await;
    assert!(!error_text(&resp.body).is_empty());
}

#[tokio::test]
async fn malformed_namespaced_names_are_rejected_without_panicking() {
    let mgr = manager(vec![http_def("github")]);
    for name in ["", "__", "no_separator", "github__", "__tool", "a__b__c"] {
        let resp = dispatch(
            &mgr,
            10,
            AggregatorMethod::CallTool {
                name: name.to_string(),
                arguments: serde_json::Value::Null,
            },
        )
        .await;
        assert!(
            matches!(resp.body, AggregatorResult::Error { .. }),
            "{name:?} should resolve to an Error, got {:?}",
            resp.body
        );
    }
}

// ── Lifecycle ──────────────────────────────────────────────────────

#[tokio::test]
async fn refresh_replaces_the_definition_set() {
    let mgr = manager(vec![http_def("old")]);

    // Disabled definitions keep `initialize_all` off the network.
    let mut replacement = http_def("new");
    replacement.enabled = false;
    let resp = dispatch(
        &mgr,
        11,
        AggregatorMethod::Refresh {
            servers: vec![replacement],
        },
    )
    .await;
    assert!(matches!(resp.body, AggregatorResult::Ok { ok: true }));

    let listed = dispatch(&mgr, 12, AggregatorMethod::ListServers).await;
    let AggregatorResult::Servers { servers } = listed.body else {
        panic!("expected Servers");
    };
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "new");
    assert!(!servers[0].enabled);
}

#[tokio::test]
async fn refresh_to_an_empty_set_drops_every_definition() {
    let mgr = manager(vec![http_def("github"), http_def("gitlab")]);
    let resp = dispatch(&mgr, 13, AggregatorMethod::Refresh { servers: vec![] }).await;
    assert!(matches!(resp.body, AggregatorResult::Ok { ok: true }));

    let listed = dispatch(&mgr, 14, AggregatorMethod::ListServers).await;
    let AggregatorResult::Servers { servers } = listed.body else {
        panic!("expected Servers");
    };
    assert!(servers.is_empty());
}

#[tokio::test]
async fn shutdown_acknowledges_even_on_the_spawned_handler_path() {
    let mgr = manager(vec![http_def("github")]);
    let resp = dispatch(&mgr, 15, AggregatorMethod::Shutdown).await;
    assert!(matches!(resp.body, AggregatorResult::Ok { ok: true }));
}

// ── CLI contract ───────────────────────────────────────────────────
//
// capsem-process spawns this binary with both guard flags; `capsem_guard::install`
// only runs when the pair is present, so the pair has to survive parsing.

#[test]
fn guard_flags_parse_as_a_pair() {
    let args = Args::parse_from([
        "capsem-mcp-aggregator",
        "--parent-pid",
        "4242",
        "--lock-path",
        "/tmp/agg.lock",
    ]);
    assert_eq!(args.parent_pid, Some(4242));
    assert_eq!(args.lock_path.unwrap().to_str().unwrap(), "/tmp/agg.lock");
}

#[test]
fn guard_flags_are_optional_for_standalone_runs() {
    let args = Args::parse_from(["capsem-mcp-aggregator"]);
    assert_eq!(args.parent_pid, None);
    assert_eq!(args.lock_path, None);
}
