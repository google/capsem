use super::*;
use crate::mcp::http_transport::BrokerHttpClient;
use capsem_proto::mcp_worker::MCP_ADMISSION_HEADER;
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::convert::Infallible;
use std::time::Duration;

async fn client() -> (BuiltinHttpClient, tokio::task::JoinHandle<()>) {
    let (coordinator, worker) = std::os::unix::net::UnixStream::pair().unwrap();
    coordinator.set_nonblocking(true).unwrap();
    let coordinator = tokio::net::UnixStream::from_std(coordinator).unwrap();
    let broker = tokio::spawn(async move {
        let handler = hyper::service::service_fn(|request: http::Request<Incoming>| async move {
            let receipt = match request.uri().path() {
                "/missing" => None,
                "/malformed" => Some("invalid receipt"),
                "/unknown" => Some(r#"{"status":"future"}"#),
                "/extra" => Some(r#"{"status":"allowed","rule_id":null,"reason":null,"extra":true}"#),
                "/denied" => Some(r#"{"status":"denied","reason":"current service policy blocks this"}"#),
                _ => Some(r#"{"status":"allowed","rule_id":"service-rule","reason":"current service allow"}"#),
            };
            let body = if request.method() == http::Method::HEAD {
                ""
            } else {
                "hello needle"
            };
            let mut response = http::Response::builder()
                .header(http::header::CONTENT_TYPE, "text/plain")
                .body(Full::new(Bytes::from_static(body.as_bytes())))
                .unwrap();
            if let Some(receipt) = receipt {
                response
                    .headers_mut()
                    .insert(MCP_ADMISSION_HEADER, receipt.parse().unwrap());
                if request.uri().path() == "/duplicate" {
                    response
                        .headers_mut()
                        .append(MCP_ADMISSION_HEADER, receipt.parse().unwrap());
                }
            }
            Ok::<_, Infallible>(response)
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(coordinator), handler)
            .await;
    });
    let http = BrokerHttpClient::connect(worker, Duration::from_secs(1)).await.unwrap();
    (BuiltinHttpClient::brokered(http), broker)
}

#[tokio::test]
async fn brokered_tools_use_service_admission_without_resolving_names_and_keep_ledger_fields() {
    let (client, broker) = client().await;
    for (tool, args, method) in [
        (
            "fetch_http",
            serde_json::json!({"url": "http://unresolvable.invalid/allowed", "format":"raw"}),
            "GET",
        ),
        (
            "grep_http",
            serde_json::json!({"url": "http://unresolvable.invalid/allowed", "pattern":"needle"}),
            "GET",
        ),
        (
            "http_headers",
            serde_json::json!({"url": "http://unresolvable.invalid/allowed", "method":"POST"}),
            "HEAD",
        ),
        (
            "http_headers",
            serde_json::json!({"url": "http://unresolvable.invalid/allowed", "method":"GET"}),
            "GET",
        ),
    ] {
        let mut ledger = Vec::new();
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            call_builtin_tool(
                tool,
                &args,
                &client,
                &default_dev_security_rules(),
                &BTreeMap::new(),
                None,
                &mut ledger,
            ),
        )
        .await
        .unwrap();
        let result = response.result.unwrap();
        assert_ne!(result["isError"], true, "{tool}: {result}");
        assert!(
            !result.to_string().contains(MCP_ADMISSION_HEADER),
            "private receipt exposed"
        );
        let [BuiltinLedgerRecord::HttpRequest(record)] = ledger.as_slice() else {
            panic!("missing HTTP record")
        };
        assert!(matches!(record.decision, HttpDecision::Allowed));
        assert_eq!(record.domain, "unresolvable.invalid");
        assert_eq!(record.method, method);
        assert_eq!(record.path, "/allowed");
        assert_eq!(record.status_code, Some(200));
        assert_eq!(record.policy_action, "allow");
        assert_eq!(record.policy_rule.as_deref(), Some("service-rule"));
        assert_eq!(record.policy_reason.as_deref(), Some("current service allow"));
    }
    broker.abort();
}

#[tokio::test]
async fn brokered_tools_fail_closed_without_one_valid_allowed_receipt() {
    let (client, broker) = client().await;
    for path in ["denied", "missing", "malformed", "unknown", "extra", "duplicate"] {
        let mut ledger = Vec::new();
        let response = call_builtin_tool(
            "http_headers",
            &serde_json::json!({"url": format!("http://unresolvable.invalid/{path}")}),
            &client,
            &default_dev_security_rules(),
            &BTreeMap::new(),
            None,
            &mut ledger,
        )
        .await;
        assert_eq!(response.result.unwrap()["isError"], true, "{path} failed open");
        let [BuiltinLedgerRecord::HttpRequest(record)] = ledger.as_slice() else {
            panic!("missing denial record")
        };
        assert!(matches!(record.decision, HttpDecision::Denied));
        assert_eq!(record.method, "HEAD");
        assert_eq!(record.status_code, None);
        assert_eq!(record.policy_action, "block");
        if path == "denied" {
            assert_eq!(
                record.policy_reason.as_deref(),
                Some("current service policy blocks this")
            );
        }
    }
    broker.abort();
}
