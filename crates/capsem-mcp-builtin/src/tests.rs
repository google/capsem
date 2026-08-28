use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
    resp.error = Some(capsem_core::mcp::types::JsonRpcError {
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
    resp.error = Some(capsem_core::mcp::types::JsonRpcError {
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
    for weird in [
        serde_json::json!("true"),
        serde_json::json!(1),
        serde_json::json!(null),
    ] {
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
