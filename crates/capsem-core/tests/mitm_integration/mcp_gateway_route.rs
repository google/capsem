//! A workload reaches Capsem's MCP gateway over HTTP at `mcp.capsem.internal`.
//!
//! Containers cannot open vsock, so the in-guest relay is out of their
//! reach; their port-80 connections reach this proxy like any other flow.
//! The exact name is answered by this VM's MCP endpoint -- the same policy
//! and ledger the framed path uses, attributed to this VM's session and the
//! connection's process -- and never dialed. Every lookalike is either
//! refused as a reserved name or handled as the ordinary host it is; none
//! reaches the gateway.

use std::sync::Mutex;

use capsem_core::net::mitm_proxy::{McpEndpointState, McpTimeouts};
use capsem_proto::mcp_aggregator::{AggregatorClient, AggregatorMethod, AggregatorResponse, AggregatorResult};

use super::*;

/// Every HTTP request is blocked; MCP calls to `local__secret` are blocked.
/// The gateway must answer under MCP policy alone.
const RULES: &str = r#"
[profiles.rules.block_all_http]
name = "block_all_http"
action = "block"
reason = "test: no HTTP leaves this VM"
match = 'has(http.host)'

[profiles.rules.block_secret_tool]
name = "block_secret_tool"
action = "block"
reason = "test: the secret tool is off limits"
match = 'mcp.tool_call.name == "local__secret"'
"#;

/// The tools the aggregator was asked to call, in order.
type Called = Arc<Mutex<Vec<String>>>;

/// This VM's proxy with an MCP endpoint whose aggregator records each call.
fn proxy_with_gateway() -> (Arc<MitmProxyConfig>, Arc<DbWriter>, Called) {
    let (config, db) = make_proxy_config_with_security_rules(security_rules_from_toml(RULES), &[80]);
    let mut config = Arc::try_unwrap(config).unwrap_or_else(|_| panic!("a fresh config has one owner"));
    let called: Called = Arc::default();
    let (aggregator, mut rx) = AggregatorClient::channel(8);
    let seen = Arc::clone(&called);
    tokio::spawn(async move {
        while let Some((req, resp_tx)) = rx.recv().await {
            let body = match req.method {
                AggregatorMethod::CallTool { name, .. } => {
                    seen.lock().unwrap().push(name.clone());
                    AggregatorResult::CallResult {
                        result: serde_json::json!({"content": [{"type": "text", "text": format!("pong:{name}")}]}),
                    }
                }
                _ => AggregatorResult::Error {
                    error: "unexpected aggregator method".to_string(),
                },
            };
            let _ = resp_tx.send(AggregatorResponse { id: req.id, body });
        }
    });
    config.mcp_endpoint = Some(Arc::new(McpEndpointState::new(
        aggregator,
        Arc::new(std::sync::RwLock::new(Arc::clone(
            config.engine.policy().snapshot().security_rules(),
        ))),
        Arc::new(std::sync::RwLock::new(Arc::clone(
            config.engine.policy().snapshot().plugins(),
        ))),
        Arc::new(tokio::sync::Semaphore::new(4)),
        McpTimeouts::default(),
    )));
    (Arc::new(config), db, called)
}

/// One keep-alive connection into the proxy, attributed to `process` the
/// way the guest's net-proxy attributes every flow it relays.
async fn workload_connection(
    config: Arc<MitmProxyConfig>,
    process: &str,
) -> (
    hyper::client::conn::http1::SendRequest<Full<Bytes>>,
    tokio::task::JoinHandle<()>,
) {
    let (proxy_task, addr) = spawn_proxy(config).await;
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(format!("\0CAPSEM_META:{process}\n").as_bytes())
        .await
        .unwrap();
    let (sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tcp)).await.unwrap();
    tokio::spawn(conn);
    (sender, proxy_task)
}

async fn send(
    sender: &mut hyper::client::conn::http1::SendRequest<Full<Bytes>>,
    method: &str,
    host: &str,
    path: &str,
    body: &str,
) -> (u16, String) {
    let request = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("host", host)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn tool_call(id: u32, name: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": name, "arguments": {"text": "hi"}},
    })
    .to_string()
}

fn rows(db: &DbWriter, sql: &str) -> Vec<Vec<serde_json::Value>> {
    let raw: serde_json::Value = serde_json::from_str(&db.reader().unwrap().query_raw(sql).unwrap()).unwrap();
    raw["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row.as_array().unwrap().clone())
        .collect()
}

#[tokio::test]
async fn a_workload_reaches_the_mcp_gateway_by_name_under_mcp_policy_and_this_vms_ledger() {
    let (config, db, called) = proxy_with_gateway();
    let (mut sender, proxy_task) = workload_connection(config, "claude").await;
    let host = "mcp.capsem.internal";

    let initialize = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;
    let (status, body) = send(&mut sender, "POST", host, "/mcp", initialize).await;
    assert_eq!(status, 200, "{body}");
    let reply: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["result"]["serverInfo"]["name"], "capsem-mcp-mitm-endpoint");

    let initialized = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let (status, body) = send(&mut sender, "POST", host, "/mcp", initialized).await;
    assert_eq!(status, 202, "a notification is accepted with no answer: {body}");

    // The Host header's own spelling does not matter: the proxy normalizes it.
    let (status, body) = send(
        &mut sender,
        "POST",
        "MCP.Capsem.Internal.:80",
        "/mcp",
        &tool_call(2, "local__echo"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let reply: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(reply["result"]["content"][0]["text"], "pong:local__echo", "{reply}");

    let (status, body) = send(&mut sender, "POST", host, "/mcp", &tool_call(3, "local__secret")).await;
    assert_eq!(status, 200, "a policy refusal is a JSON-RPC answer: {body}");
    let reply: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        reply["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("profiles.rules.block_secret_tool")),
        "{reply}"
    );

    let (status, _) = send(&mut sender, "GET", host, "/mcp", "").await;
    assert_eq!(status, 405, "no server-initiated stream");
    let (status, _) = send(&mut sender, "POST", host, "/elsewhere", &tool_call(4, "local__echo")).await;
    assert_eq!(status, 404);
    let (status, body) = send(&mut sender, "POST", host, "/mcp", "[1,2]").await;
    assert_eq!(status, 400, "{body}");

    drop(sender);
    proxy_task.await.unwrap();
    db.flush().await;

    assert_eq!(
        *called.lock().unwrap(),
        vec!["local__echo".to_string()],
        "only the allowed call reached the aggregator"
    );
    let calls = rows(
        &db,
        "SELECT tool_name, decision, transport, process_name, policy_rule FROM tool_calls
         WHERE method = 'tools/call' ORDER BY id",
    );
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0][0], "local__echo");
    assert_eq!(calls[0][1], "allowed");
    assert_eq!(calls[1][0], "local__secret");
    assert_eq!(calls[1][1], "denied");
    assert_eq!(calls[1][4], "profiles.rules.block_secret_tool");
    for call in &calls {
        assert_eq!(call[2], "http", "recorded as the transport it arrived on: {call:?}");
        assert_eq!(call[3], "claude", "attributed to the connection's process: {call:?}");
    }
    assert!(
        db.reader().unwrap().recent_net_events(10).unwrap().is_empty(),
        "the gateway is neither dialed nor judged as HTTP"
    );
}

#[tokio::test]
async fn lookalike_names_never_reach_the_gateway() {
    let (config, db, called) = proxy_with_gateway();
    let (mut sender, proxy_task) = workload_connection(config, "claude").await;

    for host in [
        "mcp.capsem.internal:8080",
        "xmcp.capsem.internal",
        "mcp.team.capsem.internal",
        "capsem.internal",
    ] {
        let (status, body) = send(&mut sender, "POST", host, "/mcp", &tool_call(1, "local__echo")).await;
        assert_eq!(status, 403, "{host}: {body}");
        assert!(body.contains("reserved"), "{host}: {body}");
    }
    // Outside the zone a lookalike is an ordinary host, judged by HTTP policy.
    let (status, body) = send(
        &mut sender,
        "POST",
        "mcp.capsem.internal.example.com",
        "/mcp",
        &tool_call(2, "local__echo"),
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert!(body.contains("profiles.rules.block_all_http"), "{body}");

    drop(sender);
    proxy_task.await.unwrap();
    db.flush().await;

    assert!(called.lock().unwrap().is_empty(), "no lookalike reached the aggregator");
    let refused = rows(
        &db,
        "SELECT domain, port, decision, matched_rule FROM net_events
         WHERE matched_rule = 'capsem.internal.reserved' ORDER BY id",
    );
    let domains: Vec<_> = refused.iter().map(|row| (row[0].clone(), row[1].clone())).collect();
    assert_eq!(
        domains,
        vec![
            (serde_json::json!("mcp.capsem.internal"), serde_json::json!(8080)),
            (serde_json::json!("xmcp.capsem.internal"), serde_json::json!(80)),
            (serde_json::json!("mcp.team.capsem.internal"), serde_json::json!(80)),
            (serde_json::json!("capsem.internal"), serde_json::json!(80)),
        ],
        "every refusal is in the ledger: {refused:?}"
    );
}
