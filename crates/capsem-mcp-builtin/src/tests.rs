use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A non-public address is reachable only through an explicit allow rule,
/// so a test that talks to a loopback fixture carries one.
fn loopback_allowed_rules() -> SecurityRuleSet {
    capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.allow_local_fixture]
        name = "allow_local_fixture"
        action = "allow"
        reason = "local test fixture"
        match = 'http.host == "127.0.0.1"'
        "#,
    )
    .and_then(|profile| {
        SecurityRuleSet::compile_profile(&profile, capsem_core::net::policy_config::SecurityRuleSource::User)
    })
    .expect("test security rules compile")
}

fn handler() -> BuiltinHandler {
    BuiltinHandler {
        http_client: BuiltinHttpClient::new(HTTP_REQUEST_TIMEOUT, HTTP_CONNECT_TIMEOUT),
        security_rules: Arc::new(SecurityRuleSet::new(Vec::new())),
        plugin_policy: Arc::new(BTreeMap::new()),
    }
}

#[test]
fn router_and_server_info_expose_the_complete_builtin_surface() {
    let tools = BuiltinHandler::tool_router();
    let mut names = tools
        .list_all()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();
    names.sort();
    // Exact: the builtin surface is guest-callable, so any tool added or
    // brought back (the retired `snapshots_*` family wrote and reverted host
    // session state) must be a deliberate change to this list.
    assert_eq!(names, ["echo", "fetch_http", "grep_http", "http_headers"]);

    let info = handler().get_info();
    assert_eq!(info.server_info.name, "capsem-local");
    assert!(!info.server_info.version.is_empty());
}

#[tokio::test]
async fn echo_handler_returns_input_without_touching_io() {
    let handler = handler();
    let value = handler
        .echo(Parameters(EchoParams {
            text: "transport fixture".to_string(),
        }))
        .await
        .unwrap();
    assert_eq!(value, "transport fixture");
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
    let client = BuiltinHttpClient::new(
        std::time::Duration::from_millis(50),
        std::time::Duration::from_millis(50),
    )
    .pinned("127.0.0.1", &[])
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

/// The records a tool result carries for capsem-process to write.
fn ledger_records(result: &CallToolResult) -> Vec<BuiltinLedgerRecord> {
    let value = result
        .meta
        .as_ref()
        .and_then(|meta| meta.get(BUILTIN_LEDGER_META_KEY))
        .cloned()
        .expect("the result carries ledger records");
    builtin_ledger::decode(value).expect("the records decode")
}

fn text_of(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| content.as_text().map(|text| text.text.clone()))
        .collect()
}

/// The builtin writes no ledger: the request it made goes back to
/// capsem-process on the result itself.
#[tokio::test]
async fn an_http_tool_hands_its_request_back_on_the_result() {
    let handler = BuiltinHandler {
        security_rules: Arc::new(loopback_allowed_rules()),
        ..handler()
    };
    let url = spawn_one_response_http_server().await;

    let result = call_builtin(
        &handler,
        "http_headers",
        serde_json::json!({"url": url, "method": "HEAD"}),
    )
    .await;
    assert_eq!(result.is_error, Some(false), "{result:?}");
    assert!(text_of(&result).contains("Status: 200"), "{result:?}");

    let records = ledger_records(&result);
    let [BuiltinLedgerRecord::HttpRequest(request)] = records.as_slice() else {
        panic!("one request, one record: {result:?}");
    };
    assert_eq!(request.domain, "127.0.0.1");
    assert_eq!(request.method, "HEAD");
    assert_eq!(request.decision, builtin_ledger::HttpDecision::Allowed);
    assert_eq!(request.status_code, Some(200));
}

/// A refusal is the row an investigator most wants, so it rides on the error
/// result exactly as a success rides on a success.
#[tokio::test]
async fn a_refused_request_is_recorded_on_the_error_result() {
    let result = handler()
        .fetch_http(Parameters(FetchHttpParams {
            url: "http://127.0.0.1:1/".to_string(),
            format: None,
            start_index: None,
            max_length: None,
        }))
        .await;
    assert_eq!(result.is_error, Some(true), "{result:?}");

    let records = ledger_records(&result);
    let [BuiltinLedgerRecord::HttpRequest(request)] = records.as_slice() else {
        panic!("one refusal, one record: {result:?}");
    };
    assert_eq!(request.decision, builtin_ledger::HttpDecision::Denied);
    assert_eq!(request.policy_action, "block");
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
    let client = BuiltinHttpClient::new(HTTP_REQUEST_TIMEOUT, HTTP_CONNECT_TIMEOUT)
        .pinned("127.0.0.1", &[])
        .expect("build client");
    let url = spawn_redirecting_http_server().await;

    let resp = client
        .get(url)
        .send()
        .await
        .expect("request completes without following redirect");
    assert_eq!(
        resp.status().as_u16(),
        302,
        "redirects must not be followed -- a 3xx to another host would bypass the domain policy check"
    );
}
