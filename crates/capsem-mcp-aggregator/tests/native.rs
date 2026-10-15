//! Actual binary readiness and tool dispatch through inherited capabilities.

use capsem_core::mcp::native_worker::grant_worker;
use capsem_proto::mcp_aggregator::{
    read_frame, write_frame, AggregatorMethod, AggregatorRequest, AggregatorResponse, AggregatorResult,
};
use capsem_proto::mcp_contracts::{McpAuthConfig, McpAuthKind, McpServerDef};
use capsem_proto::mcp_worker::{McpHttpRole, McpWorkerBootstrap, MCP_SERVER_HEADER};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::collections::HashMap;
use std::convert::Infallible;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn definition() -> McpServerDef {
    McpServerDef {
        name: "assigned".into(),
        url: "https://configured.invalid/mcp".into(),
        headers: HashMap::from([("x-worker-secret".into(), "must-not-be-forwarded".into())]),
        auth: Some(McpAuthConfig {
            kind: McpAuthKind::Bearer,
            credential_ref: format!("credential:blake3:{}", "a".repeat(64)),
        }),
        enabled: true,
        source: "user".into(),
        command: None,
        args: Vec::new(),
        env: HashMap::new(),
        pool_size: None,
        pool_safe_tools: Vec::new(),
    }
}

async fn reply(request: http::Request<Incoming>) -> http::Response<Full<Bytes>> {
    if request.method() != http::Method::POST {
        return http::Response::builder()
            .status(405)
            .body(Full::new(Bytes::new()))
            .unwrap();
    }
    let message: serde_json::Value =
        serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let Some(id) = message.get("id") else {
        return http::Response::builder()
            .status(202)
            .body(Full::new(Bytes::new()))
            .unwrap();
    };
    let result = match message["method"].as_str().unwrap() {
        "initialize" => serde_json::json!({
            "protocolVersion": message["params"]["protocolVersion"],
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "fixture", "version": "1" }
        }),
        "tools/list" => serde_json::json!({"tools": [{
            "name": "echo", "description": "fixture echo",
            "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}
        }]}),
        "tools/call" => serde_json::json!({"content": [{
            "type": "text", "text": message["params"]["arguments"]["text"]
        }]}),
        method => panic!("unexpected upstream method {method}"),
    };
    http::Response::builder()
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(
            serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result})).unwrap(),
        )))
        .unwrap()
}

#[tokio::test]
async fn binary_confines_before_ready_and_keeps_its_capabilities_after_refresh() {
    let directory = tempfile::tempdir().unwrap();
    let (control, child_control) = UnixStream::pair().unwrap();
    let (coordinator_http, worker_http) = UnixStream::pair().unwrap();
    let (_coordinator_spawn, worker_spawn) = UnixStream::pair().unwrap();
    coordinator_http.set_nonblocking(true).unwrap();
    let coordinator_http = tokio::net::UnixStream::from_std(coordinator_http).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let broker = tokio::spawn(async move {
        let service = hyper::service::service_fn(move |request: http::Request<Incoming>| {
            captured
                .lock()
                .unwrap()
                .push((request.uri().clone(), request.headers().clone()));
            async move { Ok::<_, Infallible>(reply(request).await) }
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(coordinator_http), service)
            .await;
    });
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_capsem-mcp-aggregator"));
    command
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .arg("--lock-path")
        .arg(directory.path().join("worker.lock"))
        .env("CAPSEM_TEST_INHERITED_SECRET", "remove-this-inherited-secret")
        .stdin(Stdio::from(OwnedFd::from(child_control)))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    let mut child = command.spawn().unwrap();
    drop(command);
    grant_worker(
        &control,
        McpWorkerBootstrap {
            role: McpHttpRole::Aggregator,
            generation: [7; 16],
        },
        &[worker_http.as_fd(), worker_spawn.as_fd()],
    )
    .await
    .unwrap();
    drop((worker_http, worker_spawn));
    #[cfg(target_os = "linux")]
    assert!(
        !std::fs::read(format!("/proc/{}/environ", child.id().unwrap()))
            .unwrap()
            .windows(b"remove-this-inherited-secret".len())
            .any(|window| window == b"remove-this-inherited-secret"),
        "worker retained original inherited secret bytes"
    );
    control.set_nonblocking(true).unwrap();
    let mut control = tokio::net::UnixStream::from_std(control).unwrap();
    let mut output = child.stdout.take().unwrap();
    write_frame(&mut control, &vec![definition()]).await.unwrap();
    let unexpected_file = directory.path().join("unapproved-command-ran");
    let mut unapproved = definition();
    unapproved.name = "unapproved".into();
    unapproved.command = Some("/usr/bin/touch".into());
    unapproved.args = vec![unexpected_file.to_string_lossy().into_owned()];
    for (id, method) in [
        (1, AggregatorMethod::ListTools),
        (
            2,
            AggregatorMethod::Refresh {
                servers: vec![definition()],
            },
        ),
        (
            3,
            AggregatorMethod::CallTool {
                name: "assigned__echo".into(),
                arguments: serde_json::json!({"text": "confined echo"}),
                timeout_ms: Some(1000),
            },
        ),
        (
            4,
            AggregatorMethod::Refresh {
                servers: vec![unapproved],
            },
        ),
        (5, AggregatorMethod::ListServers),
        (6, AggregatorMethod::Shutdown),
    ] {
        write_frame(&mut control, &AggregatorRequest { id, method })
            .await
            .unwrap();
        let response: AggregatorResponse = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut output))
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(response.id, id);
        match (id, response.body) {
            (1, AggregatorResult::Tools { tools }) => assert_eq!(tools[0].namespaced_name, "assigned__echo"),
            (2 | 4 | 6, AggregatorResult::Ok { ok }) => assert!(ok),
            (3, AggregatorResult::CallResult { result }) => {
                assert_eq!(result["content"][0]["text"], "confined echo");
            }
            (5, AggregatorResult::Servers { servers }) => assert!(!servers[0].connected),
            (_, body) => panic!("unexpected response {id}: {body:?}"),
        }
    }
    assert!(tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
    assert!(!unexpected_file.exists(), "unapproved stdio executed");
    let requests = requests.lock().unwrap();
    assert!(
        requests.len() >= 5,
        "initialization, refresh and tool call must use the channel"
    );
    for (uri, headers) in requests.iter() {
        assert_eq!(uri, "https://configured.invalid/mcp");
        assert_eq!(headers[MCP_SERVER_HEADER], "assigned");
        assert!(!headers.contains_key(http::header::AUTHORIZATION));
        assert!(!headers.contains_key("x-worker-secret"));
    }
    drop(requests);
    broker.abort();
}
