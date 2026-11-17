//! Actual binary readiness and tool dispatch through inherited capabilities.

use capsem_mcp_transport::native_worker::grant_worker;
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
            "capabilities": { "tools": {}, "resources": {}, "prompts": {} },
            "serverInfo": { "name": "fixture", "version": "1" }
        }),
        "resources/list" => serde_json::json!({"resources":[]}),
        "prompts/list" => serde_json::json!({"prompts":[]}),
        "tools/list" => serde_json::json!({"tools": [{
            "name": "echo", "description": "fixture echo",
            "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}
        }]}),
        "tools/call" => {
            let args = &message["params"]["arguments"];
            let text = if args["boundary"] == true {
                let empty =
                    serde_json::json!({"jsonrpc":"2.0", "id":id, "result":{"content":[{"type":"text","text":""}]}});
                let overhead = serde_json::to_vec(&empty).unwrap().len()
                    - serde_json::to_vec(id).unwrap().len()
                    - b",\"id\":".len();
                "r".repeat(capsem_proto::MCP_PAYLOAD_MAX_BYTES - overhead + usize::from(args["oversized"] == true))
            } else {
                args["text"].as_str().unwrap().to_owned()
            };
            serde_json::json!({"content": [{"type":"text", "text":text}]})
        }
        method => panic!("unexpected upstream method {method}"),
    };
    let bytes = serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result})).unwrap();
    let (content_type, bytes) = if message["params"]["arguments"]["sse"] == true {
        (
            "text/event-stream",
            format!("data: {}\r\n\r\n", String::from_utf8(bytes).unwrap()).into_bytes(),
        )
    } else {
        ("application/json", bytes)
    };
    http::Response::builder()
        .status(if message["params"]["arguments"]["http_error"] == true {
            http::StatusCode::INTERNAL_SERVER_ERROR
        } else {
            http::StatusCode::OK
        })
        .header(http::header::CONTENT_TYPE, content_type)
        .body(Full::new(Bytes::from(bytes)))
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
        .stdout(Stdio::null())
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
    let (mut output, mut control) = tokio::net::UnixStream::from_std(control).unwrap().into_split();
    write_frame(&mut control, &vec![definition()]).await.unwrap();
    let unexpected_file = directory.path().join("unapproved-command-ran");
    let mut unapproved = definition();
    unapproved.name = "unapproved".into();
    unapproved.command = Some("/usr/bin/touch".into());
    unapproved.args = vec![unexpected_file.to_string_lossy().into_owned()];
    let large_echo = "r".repeat(512 * 1024);
    let mut cases = vec![
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
                arguments: serde_json::json!({"text": large_echo}),
                timeout_ms: Some(1000),
            },
        ),
    ];
    for (index, sse) in [false, true].into_iter().enumerate() {
        for (offset, oversized, boundary) in [(0, false, true), (1, true, true), (2, false, false)] {
            cases.push((10 + index as u64 * 3 + offset, AggregatorMethod::CallTool {
                name: "assigned__echo".into(),
                arguments: serde_json::json!({"boundary":boundary, "oversized":oversized, "sse":sse, "text":"after refusal"}),
                timeout_ms: Some(2000),
            }));
        }
    }
    cases.extend([
        (
            16,
            AggregatorMethod::CallTool {
                name: "assigned__echo".into(),
                arguments: serde_json::json!({"boundary":true, "oversized":true, "http_error":true}),
                timeout_ms: Some(2000),
            },
        ),
        (
            17,
            AggregatorMethod::CallTool {
                name: "assigned__echo".into(),
                arguments: serde_json::json!({"text":"after refusal"}),
                timeout_ms: Some(2000),
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
    ]);
    for (id, method) in cases {
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
                assert_eq!(result["content"][0]["text"], large_echo);
            }
            (10 | 13, AggregatorResult::CallResult { result }) => {
                let text = result["content"][0]["text"].as_str().unwrap();
                assert!(text.len() > capsem_proto::MCP_PAYLOAD_MAX_BYTES - 256);
                assert!(text.bytes().all(|byte| byte == b'r'));
                let response = capsem_proto::mcp_contracts::JsonRpcResponse::ok(
                    Some(serde_json::json!("\0".repeat(capsem_proto::MCP_REQUEST_ID_MAX_BYTES))),
                    result,
                );
                let bytes = capsem_proto::mcp_contracts::json_rpc_wire::encode_response(&response).unwrap();
                assert_eq!(bytes.len(), capsem_proto::MCP_JSON_RPC_MAX_BYTES);
                capsem_proto::encode_mcp_frame(9, 0, &"x".repeat(capsem_proto::MCP_FRAME_MAX_PROCESS_NAME_LEN), &bytes)
                    .unwrap();
            }
            (11 | 14 | 16, AggregatorResult::Error { error }) => {
                assert!(error.contains("tool may already have run"), "{error}");
            }
            (12 | 15 | 17, AggregatorResult::CallResult { result }) => {
                assert_eq!(result["content"][0]["text"], "after refusal")
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
