//! Real builtin tool calls after the binary installs its kernel sandbox.

use capsem_mcp_transport::native_worker::grant_worker;
use capsem_proto::mcp_contracts::builtin_ledger::{self, BuiltinLedgerRecord, HttpDecision, BUILTIN_LEDGER_META_KEY};
use capsem_proto::mcp_worker::{McpHttpAdmission, McpHttpRole, McpWorkerBootstrap, MCP_ADMISSION_HEADER};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rmcp::{model::CallToolRequestParams, ServiceExt};
use std::convert::Infallible;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::Stdio;
use std::time::Duration;

#[tokio::test]
async fn native_builtin_uses_only_its_inherited_http_and_returns_policy_ledger_records() {
    let (control, child_control) = UnixStream::pair().unwrap();
    let (coordinator, worker) = UnixStream::pair().unwrap();
    coordinator.set_nonblocking(true).unwrap();
    let coordinator = tokio::net::UnixStream::from_std(coordinator).unwrap();
    let broker = tokio::spawn(async move {
        let service = hyper::service::service_fn(|request: http::Request<Incoming>| async move {
            assert_eq!(request.uri().host(), Some("unresolvable.invalid"));
            let admission = if request.uri().path() == "/denied" {
                McpHttpAdmission::Denied {
                    reason: "live service policy denies this".into(),
                }
            } else {
                McpHttpAdmission::Allowed {
                    rule_id: Some("coordinator-rule".into()),
                    reason: Some("live allow".into()),
                }
            };
            let body = if request.method() == http::Method::HEAD {
                ""
            } else {
                "hello confined builtin"
            };
            let response = http::Response::builder()
                .header(http::header::CONTENT_TYPE, "text/plain")
                .header(MCP_ADMISSION_HEADER, serde_json::to_string(&admission).unwrap())
                .body(Full::new(Bytes::from_static(body.as_bytes())))
                .unwrap();
            Ok::<_, Infallible>(response)
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(coordinator), service)
            .await;
    });
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_capsem-mcp-builtin"));
    command
        .env("CAPSEM_ACTIVE_POLICY", "/must-not-open-a-policy-file")
        .env("CAPSEM_TEST_INHERITED_SECRET", "discard-builtin-inherited-secret")
        .stdin(Stdio::from(OwnedFd::from(child_control)))
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    let mut child = command.spawn().unwrap();
    drop(command);
    grant_worker(
        &control,
        McpWorkerBootstrap {
            role: McpHttpRole::Builtin,
            generation: [8; 16],
        },
        &[worker.as_fd()],
    )
    .await
    .unwrap();
    drop(worker);
    #[cfg(target_os = "linux")]
    assert!(!std::fs::read(format!("/proc/{}/environ", child.id().unwrap()))
        .unwrap()
        .windows(b"discard-builtin-inherited-secret".len())
        .any(|window| window == b"discard-builtin-inherited-secret"));
    control.set_nonblocking(true).unwrap();
    let control = tokio::net::UnixStream::from_std(control).unwrap();
    let client = tokio::time::timeout(Duration::from_secs(2), ().serve(control.into_split()))
        .await
        .unwrap()
        .unwrap();
    for (name, arguments, denied, method) in [
        ("echo", serde_json::json!({"text": "confined echo"}), false, None),
        (
            "fetch_http",
            serde_json::json!({"url": "http://unresolvable.invalid/allowed", "format": "raw"}),
            false,
            Some("GET"),
        ),
        (
            "http_headers",
            serde_json::json!({"url": "http://unresolvable.invalid/denied", "method": "POST"}),
            true,
            Some("HEAD"),
        ),
    ] {
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            client.call_tool(CallToolRequestParams::new(name).with_arguments(arguments.as_object().unwrap().clone())),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.is_error.unwrap_or(false), denied);
        let value = serde_json::to_value(&result).unwrap();
        if let Some(method) = method {
            let records = builtin_ledger::decode(result.meta.unwrap()[BUILTIN_LEDGER_META_KEY].clone()).unwrap();
            let [BuiltinLedgerRecord::HttpRequest(record)] = records.as_slice() else {
                panic!("missing HTTP evidence")
            };
            assert_eq!(record.method, method);
            assert_eq!(record.domain, "unresolvable.invalid");
            if denied {
                assert_eq!(record.decision, HttpDecision::Denied);
                assert_eq!(record.policy_reason.as_deref(), Some("live service policy denies this"));
            } else {
                assert_eq!(record.decision, HttpDecision::Allowed);
                assert_eq!(record.policy_rule.as_deref(), Some("coordinator-rule"));
                assert!(value["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("hello confined builtin"));
            }
        } else {
            assert_eq!(value["content"][0]["text"], "confined echo");
        }
    }
    client.cancel().await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
    broker.abort();
}
