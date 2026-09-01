use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn handler_without_snapshots() -> BuiltinHandler {
    BuiltinHandler {
        http_client: reqwest::Client::new(),
        db: Arc::new(DbWriter::open_in_memory(8).expect("in-memory DB")),
        security_rules: Arc::new(SecurityRuleSet::new(Vec::new())),
        plugin_policy: Arc::new(BTreeMap::new()),
        scheduler: None,
        workspace_dir: None,
    }
}

fn handler_with_snapshots(root: &std::path::Path) -> BuiltinHandler {
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    BuiltinHandler {
        scheduler: Some(Arc::new(Mutex::new(AutoSnapshotScheduler::new(
            root.to_path_buf(),
            2,
            2,
            Duration::from_secs(60),
        )))),
        workspace_dir: Some(workspace),
        ..handler_without_snapshots()
    }
}

#[test]
fn snapshot_pagination_params_preserve_include_changes() {
    let params: SnapshotPaginationParams = serde_json::from_value(serde_json::json!({
        "format": "json",
        "include_changes": true
    }))
    .expect("snapshot pagination params should deserialize");

    let args = to_args(&params);
    assert_eq!(args["format"], "json");
    assert_eq!(args["include_changes"], true);
}

#[test]
fn router_and_server_info_expose_the_complete_builtin_surface() {
    let tools = BuiltinHandler::tool_router();
    let names = tools
        .list_all()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();
    for expected in [
        "echo",
        "fetch_http",
        "grep_http",
        "http_headers",
        "snapshots_changes",
        "snapshots_list",
        "snapshots_revert",
        "snapshots_create",
        "snapshots_delete",
        "snapshots_history",
        "snapshots_compact",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing builtin tool {expected}"
        );
    }

    let info = handler_without_snapshots().get_info();
    assert_eq!(info.server_info.name, "capsem-local");
    assert!(!info.server_info.version.is_empty());
}

#[tokio::test]
async fn echo_handler_returns_input_without_touching_io() {
    let handler = handler_without_snapshots();
    let value = handler
        .echo(Parameters(EchoParams {
            text: "transport fixture".to_string(),
        }))
        .await
        .unwrap();
    assert_eq!(value, "transport fixture");
}

#[tokio::test]
async fn snapshot_handlers_operate_on_real_scheduler_state() {
    let root = tempfile::tempdir().unwrap();
    let handler = handler_with_snapshots(root.path());
    let pagination = || SnapshotPaginationParams {
        start_index: None,
        max_length: None,
        format: Some("json".to_string()),
        include_changes: Some(true),
    };

    assert!(!handler
        .snapshots_changes(Parameters(pagination()))
        .await
        .unwrap()
        .is_empty());
    assert!(!handler
        .snapshots_list(Parameters(pagination()))
        .await
        .unwrap()
        .is_empty());
    assert!(handler
        .snapshots_history(Parameters(SnapshotHistoryParams {
            path: "missing.txt".to_string(),
            start_index: None,
            max_length: None,
            format: Some("json".to_string()),
        }))
        .await
        .is_ok());

    let created = handler
        .snapshots_create(Parameters(SnapshotNameParams {
            name: "fixture".to_string(),
        }))
        .await;
    assert!(created.is_ok(), "manual snapshot failed: {created:?}");
    assert!(handler
        .snapshots_revert(Parameters(SnapshotRevertParams {
            path: "missing.txt".to_string(),
            checkpoint: None,
        }))
        .await
        .is_err());
    assert!(handler
        .snapshots_delete(Parameters(SnapshotDeleteParams {
            checkpoint: "cp-missing".to_string(),
        }))
        .await
        .is_err());
    assert!(handler
        .snapshots_compact(Parameters(SnapshotCompactParams {
            checkpoints: vec!["cp-missing".to_string()],
            name: Some("compacted".to_string()),
        }))
        .await
        .is_err());
}

#[test]
fn snapshot_tools_fail_closed_when_session_state_is_absent() {
    let handler = handler_without_snapshots();
    let error = match handler.snapshot_state() {
        Ok(_) => panic!("snapshot state unexpectedly available"),
        Err(error) => error,
    };
    assert!(error.contains("no session directory"));

    let session = tempfile::tempdir().unwrap();
    let handler = BuiltinHandler {
        scheduler: Some(Arc::new(Mutex::new(AutoSnapshotScheduler::new(
            session.path().to_path_buf(),
            1,
            1,
            Duration::from_secs(60),
        )))),
        ..handler_without_snapshots()
    };
    let error = match handler.snapshot_state() {
        Ok(_) => panic!("snapshot state unexpectedly available"),
        Err(error) => error,
    };
    assert!(error.contains("no workspace directory"));
}

async fn spawn_one_response_http_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local HTTP fixture");
    let addr = listener.local_addr().expect("fixture local addr");
    tokio::spawn(async move {
        let Ok((mut socket, _peer)) = listener.accept().await else {
            return;
        };
        let mut buf = [0_u8; 1024];
        let _ = socket.read(&mut buf).await;
        let response = concat!(
            "HTTP/1.1 200 OK\r\n",
            "content-type: text/plain; charset=utf-8\r\n",
            "x-capsem-fixture: builtin-flush\r\n",
            "content-length: 0\r\n",
            "\r\n"
        );
        let _ = socket.write_all(response.as_bytes()).await;
    });
    format!("http://{addr}/")
}

async fn spawn_stalled_body_http_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stalled HTTP fixture");
    let addr = listener.local_addr().expect("fixture local addr");
    tokio::spawn(async move {
        let Ok((mut socket, _peer)) = listener.accept().await else {
            return;
        };
        let mut buf = [0_u8; 1024];
        let _ = socket.read(&mut buf).await;
        let response = concat!(
            "HTTP/1.1 200 OK\r\n",
            "content-type: text/plain\r\n",
            "content-length: 1\r\n",
            "\r\n"
        );
        let _ = socket.write_all(response.as_bytes()).await;
        std::future::pending::<()>().await;
    });
    format!("http://{addr}/")
}

#[tokio::test]
async fn builtin_http_client_times_out_while_reading_a_stalled_body() {
    let client = build_http_client(
        std::time::Duration::from_millis(50),
        std::time::Duration::from_millis(50),
    )
    .expect("build test client");
    let url = spawn_stalled_body_http_server().await;

    let result = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        client.get(url).send().await?.text().await
    })
    .await;

    match result {
        Ok(Err(error)) => assert!(error.is_timeout(), "unexpected HTTP error: {error}"),
        Ok(Ok(body)) => panic!("stalled body unexpectedly completed: {body:?}"),
        Err(_) => panic!("the request outlived the client-owned deadline"),
    }
}

#[tokio::test]
async fn http_builtin_flushes_net_event_before_tool_response_returns() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 16).expect("open test db"));
    let handler = BuiltinHandler {
        http_client: reqwest::Client::new(),
        db: Arc::clone(&db),
        security_rules: Arc::new(SecurityRuleSet::new(Vec::new())),
        plugin_policy: Arc::new(BTreeMap::new()),
        scheduler: None,
        workspace_dir: None,
    };
    let url = spawn_one_response_http_server().await;

    let text = call_builtin(
        &handler,
        "http_headers",
        serde_json::json!({"url": url, "method": "HEAD"}),
    )
    .await
    .expect("builtin call succeeds");
    assert!(text.contains("Status: 200"), "{text}");

    let rows = db
        .reader()
        .expect("reader")
        .recent_net_events(10)
        .expect("recent net events");
    assert!(
        rows.iter().any(|row| row.domain == "127.0.0.1"
            && row.method.as_deref() == Some("HEAD")
            && row.decision == capsem_logger::Decision::Allowed),
        "net event must be durable before returning tool response: {rows:?}"
    );
}

// ── Tool-failure propagation ───────────────────────────────────────
//
// extract_text decides whether a builtin tool failure reaches the agent as a
// failure or as a successful result whose body happens to contain error prose.
// The `isError` branch exists because it once did the latter: a blocked domain
// came back as Ok(text) and the agent read it as a successful fetch. These
// pin both refusal channels -- transport-level `error`, and the logical
// `isError: true` the builtin sets for a policy refusal.

fn response(body: serde_json::Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(1)),
        result: Some(body),
        error: None,
        meta: None,
    }
}

#[test]
fn transport_error_becomes_err_with_its_message() {
    let mut resp = response(serde_json::json!({"content": [{"text": "ignored"}]}));
    resp.error = Some(capsem_proto::mcp_contracts::JsonRpcError {
        code: -32000,
        message: "vsock closed".to_string(),
        data: None,
    });

    assert_eq!(extract_text(resp), Err("vsock closed".to_string()));
}

#[test]
fn logical_tool_failure_becomes_err_not_ok_text() {
    // A policy refusal from the builtin: the transport succeeded, the tool did
    // not. Returning Ok here is the bug this branch fixed.
    let resp = response(serde_json::json!({
        "isError": true,
        "content": [{"type": "text", "text": "domain blocked by policy"}]
    }));

    assert_eq!(
        extract_text(resp),
        Err("domain blocked by policy".to_string()),
        "a refused tool call must not look like a successful fetch"
    );
}

#[test]
fn successful_call_joins_every_text_block() {
    let resp = response(serde_json::json!({
        "content": [
            {"type": "text", "text": "first"},
            {"type": "text", "text": "second"}
        ]
    }));

    assert_eq!(extract_text(resp), Ok("first\nsecond".to_string()));
}

#[test]
fn non_text_content_blocks_are_skipped_not_rendered() {
    let resp = response(serde_json::json!({
        "content": [
            {"type": "image", "data": "base64..."},
            {"type": "text", "text": "kept"},
            {"type": "text", "text": 42}
        ]
    }));

    assert_eq!(
        extract_text(resp),
        Ok("kept".to_string()),
        "only string `text` fields render"
    );
}

#[test]
fn a_result_without_content_falls_back_to_pretty_json() {
    let resp = response(serde_json::json!({"slots": 3}));
    let text = extract_text(resp).expect("no isError, so Ok");

    assert!(text.contains("\"slots\""), "unexpected body: {text}");
    assert!(text.contains('3'));
}

#[test]
fn a_missing_result_is_rendered_rather_than_dropped() {
    let mut resp = response(serde_json::Value::Null);
    resp.result = None;

    assert_eq!(extract_text(resp), Ok("null".to_string()));
}

#[test]
fn empty_content_array_is_success_with_no_text() {
    let resp = response(serde_json::json!({"content": []}));
    assert_eq!(extract_text(resp), Ok(String::new()));
}

#[test]
fn transport_error_wins_over_a_logical_failure() {
    let mut resp = response(serde_json::json!({
        "isError": true,
        "content": [{"text": "policy refusal"}]
    }));
    resp.error = Some(capsem_proto::mcp_contracts::JsonRpcError {
        code: -32000,
        message: "connection reset".to_string(),
        data: None,
    });

    assert_eq!(
        extract_text(resp),
        Err("connection reset".to_string()),
        "the transport failure is the more specific cause"
    );
}

#[test]
fn a_non_boolean_is_error_does_not_signal_failure() {
    // Documents a sharp edge: `isError` is read with as_bool(), so a server
    // sending the string "true" or the number 1 yields Ok. Anything other than
    // a JSON boolean is not a refusal signal, and a builtin that wants to
    // refuse must send a real `true`.
    for weird in [serde_json::json!("true"), serde_json::json!(1), serde_json::json!(null)] {
        let resp = response(serde_json::json!({
            "isError": weird,
            "content": [{"text": "body"}]
        }));
        assert_eq!(
            extract_text(resp),
            Ok("body".to_string()),
            "only a JSON boolean true is a refusal"
        );
    }
}

async fn spawn_redirecting_http_server() -> String {
    // Responds 302 to an unrelated host. If the client follows redirects it
    // would leave the originally-checked domain (SSRF); a safe client returns
    // the 302 to the caller instead.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect fixture");
    let addr = listener.local_addr().expect("fixture local addr");
    tokio::spawn(async move {
        while let Ok((mut socket, _peer)) = listener.accept().await {
            let mut buf = [0_u8; 1024];
            let _ = socket.read(&mut buf).await;
            let response = concat!(
                "HTTP/1.1 302 Found\r\n",
                "location: http://blocked.invalid/secret\r\n",
                "content-length: 0\r\n",
                "\r\n"
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    format!("http://{addr}/")
}

#[tokio::test]
async fn builtin_http_client_does_not_follow_redirects() {
    let client = build_http_client(HTTP_REQUEST_TIMEOUT, HTTP_CONNECT_TIMEOUT).expect("build client");
    let url = spawn_redirecting_http_server().await;

    let resp = client.get(url).send().await.expect("request completes without following redirect");
    assert_eq!(
        resp.status().as_u16(),
        302,
        "redirects must not be followed -- a 3xx to another host would bypass the domain policy check"
    );
}
