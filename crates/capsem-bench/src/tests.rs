use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[test]
fn selected_scenarios_are_strict() {
    let selected = select_scenarios(Some("tiny_http,model_json_response")).unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|scenario| scenario.name)
            .collect::<Vec<_>>(),
        vec!["tiny_http", "model_json_response"]
    );
    assert!(select_scenarios(Some("bogus")).is_err());
}

#[test]
fn deterministic_protocol_rail_owns_ten_megabyte_throughput() {
    let scenario = SCENARIOS
        .iter()
        .find(|scenario| scenario.name == "http_10mb")
        .expect("the protocol benchmark must retain its 10 MB throughput scenario");

    assert_eq!(scenario.path, "/bytes/10mb");
    assert_eq!(scenario.expected_bytes, Some(10 * 1024 * 1024));
    assert_eq!(scenario.body_kind, "10mb");
}

#[test]
fn latency_percentiles_are_interpolated() {
    let summary = latency_summary(vec![1.0, 2.0, 3.0, 4.0, 100.0]);
    assert_eq!(summary.min, 1.0);
    assert_eq!(summary.p50, 3.0);
    assert_eq!(summary.p95, 80.8);
    assert_eq!(summary.p99, 96.2);
    assert_eq!(summary.max, 100.0);
}

#[test]
fn result_ok_checks_status_size_and_required_text() {
    let scenario = SCENARIOS
        .iter()
        .copied()
        .find(|scenario| scenario.name == "tiny_http")
        .unwrap();
    assert!(result_ok(
        &RequestSample {
            status: 200,
            size: 24,
            latency_ms: 1.0,
            attempts: 1,
            error: None,
            required_text_present: true,
            secret_shaped_fixture_seen: false,
        },
        scenario
    ));
    assert!(!result_ok(
        &RequestSample {
            status: 200,
            size: 23,
            latency_ms: 1.0,
            attempts: 1,
            error: None,
            required_text_present: true,
            secret_shaped_fixture_seen: false,
        },
        scenario
    ));
}

#[test]
fn dns_query_builder_and_rcode_parser_are_strict() {
    let query = build_dns_query("load-test.capsem-bogus", 1, 0xCAFE).unwrap();
    assert_eq!(&query[..2], b"\xCA\xFE");
    assert!(query
        .windows("capsem-bogus".len())
        .any(|w| w == b"capsem-bogus"));
    let mut response = vec![0xCA, 0xFE, 0x81, 0x83];
    response.extend_from_slice(&query[4..]);
    assert_eq!(parse_dns_rcode(&response), Some(3));
    assert!(build_dns_query(&format!("{}.test", "x".repeat(64)), 1, 1).is_err());
}

#[test]
fn delta_computes_abstraction_cost() {
    let host = ScenarioResult {
        name: "tiny_http".to_string(),
        path: "/tiny".to_string(),
        body_kind: "tiny".to_string(),
        total_requests: 100,
        concurrency: 10,
        successful: 100,
        failed: 0,
        transport_retries: 0,
        total_duration_ms: 10.0,
        requests_per_sec: 10_000.0,
        transfer_bytes: 2400,
        bytes_per_sec: 240_000.0,
        latency_ms: LatencySummary {
            min: 0.1,
            max: 2.0,
            mean: 0.5,
            p50: 0.4,
            p95: 1.0,
            p99: 1.5,
        },
        latency_samples: Vec::new(),
        errors: BTreeMap::new(),
        secret_shaped_fixture_seen: None,
        raw_secret_stored_in_result: None,
    };
    let guest = ScenarioResult {
        requests_per_sec: 2_500.0,
        bytes_per_sec: 60_000.0,
        latency_ms: LatencySummary {
            p50: 1.4,
            p95: 5.0,
            p99: 9.5,
            ..host.latency_ms.clone()
        },
        latency_samples: Vec::new(),
        ..host.clone()
    };
    let host_values = [host];
    let guest_values = [guest];
    let host_rows = rows_by_name(&host_values);
    let guest_rows = rows_by_name(&guest_values);
    let host_row = host_rows["tiny_http"];
    let guest_row = guest_rows["tiny_http"];
    assert_eq!(
        ratio(guest_row.requests_per_sec, host_row.requests_per_sec),
        0.25
    );
    assert_eq!(
        round1(guest_row.latency_ms.p95 - host_row.latency_ms.p95),
        4.0
    );
    assert_eq!(guest_row.failed as isize - host_row.failed as isize, 0);
}

#[test]
fn failed_scenarios_are_poisoned_benchmark_numbers() {
    let failed = ScenarioResult {
        name: "tiny_http".to_string(),
        path: "/tiny".to_string(),
        body_kind: "tiny".to_string(),
        total_requests: 100,
        concurrency: 10,
        successful: 0,
        failed: 100,
        transport_retries: 0,
        total_duration_ms: 10.0,
        requests_per_sec: 10_000.0,
        transfer_bytes: 0,
        bytes_per_sec: 0.0,
        latency_ms: LatencySummary {
            min: 0.1,
            max: 2.0,
            mean: 0.5,
            p50: 0.4,
            p95: 1.0,
            p99: 1.5,
        },
        latency_samples: Vec::new(),
        errors: BTreeMap::from([("request:connection refused".to_string(), 100)]),
        secret_shaped_fixture_seen: None,
        raw_secret_stored_in_result: None,
    };
    let err = validate_successful_scenarios(&[failed], "host_direct").unwrap_err();
    let message = err.to_string();
    assert!(message.contains("poisoned numbers"), "{message}");
    assert!(message.contains("tiny_http failed=100/100"), "{message}");
}

#[tokio::test]
async fn http_transport_retries_reconnect_until_bounded_budget_is_exhausted() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let accepted_for_server = Arc::clone(&accepted);
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let attempt = accepted_for_server.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt < HTTP_REQUEST_ATTEMPTS {
                drop(stream);
                continue;
            }
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf).await.unwrap();
            let body =
                br#"{"output":[{"content":[{"type":"output_text","text":"tool_calls"}]}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                std::str::from_utf8(body).unwrap()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            break;
        }
    });

    let scenario = SCENARIOS
        .iter()
        .copied()
        .find(|scenario| scenario.name == "model_json_response")
        .unwrap();
    let clients = HttpClients::build(1, Duration::from_secs(1)).unwrap();
    let sample = run_one_request(
        &clients,
        &format!("http://{addr}{}", scenario.path),
        scenario,
        Duration::from_secs(1),
    )
    .await;
    server.await.unwrap();

    assert_eq!(sample.status, 200);
    assert_eq!(sample.attempts, HTTP_REQUEST_ATTEMPTS);
    assert!(sample.error.is_none(), "{sample:?}");
    assert!(sample.required_text_present);
    assert_eq!(accepted.load(Ordering::SeqCst), HTTP_REQUEST_ATTEMPTS);
}

#[test]
fn guest_protocol_command_uses_one_capsem_bench_invocation() {
    let command = guest_protocol_command(
        "http://127.0.0.1:3713",
        Some("127.0.0.1:3713"),
        50_000,
        64,
        30_000,
        "tiny_http,mcp_tool_call,dns_local_nxdomain",
    );
    assert!(
        command.starts_with("capsem-bench-rs protocol "),
        "{command}"
    );
    assert!(command.contains("--base-url http://127.0.0.1:3713"));
    assert!(command.contains("--dns-udp-addr 127.0.0.1:3713"));
    assert!(command.contains("--requests 50000"));
    assert!(command.contains("--concurrency 64"));
    assert!(command.contains("--timeout-ms 30000"));
    assert!(command.contains("--scenarios tiny_http,mcp_tool_call,dns_local_nxdomain"));
    assert!(command
        .ends_with("--json-out /tmp/capsem-benchmark.json && cat /tmp/capsem-benchmark.json"));
}

#[test]
fn parse_guest_protocol_artifact_rejects_legacy_guest_wrapper_json() {
    let stdout = r#"mock-server-protocol base_url=http://127.0.0.1:3713 requests=100 concurrency=10
JSON results saved to /tmp/capsem-benchmark.json
{
          "version": "0.3.0",
          "timestamp": 1782339183.0,
          "hostname": "capsem",
          "mock_server_protocol": {
            "version": "1.0",
            "base_url": "http://127.0.0.1:3713",
            "total_requests": 100,
            "concurrency": 10,
            "timeout_s": 30.0,
            "selected_scenarios": ["tiny_http"],
            "scenarios": [{
              "name": "tiny_http",
              "path": "/tiny",
              "body_kind": "tiny",
              "method": "GET",
              "expected_status": 200,
              "total_requests": 100,
              "concurrency": 10,
              "successful": 100,
              "failed": 0,
              "total_duration_ms": 100.0,
              "requests_per_sec": 1000.0,
              "transfer_bytes": 2100,
              "bytes_per_sec": 21000.0,
              "latency_ms": {
                "min": 0.1,
                "max": 0.5,
                "mean": 0.2,
                "p50": 0.2,
                "p95": 0.4,
                "p99": 0.5
              },
              "errors": {},
              "secret_shaped_fixture_seen": null,
              "raw_secret_stored_in_result": null
            }],
            "websocket": []
          }
        }"#;
    let err = parse_guest_protocol_artifact(stdout).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("parse guest capsem-bench-rs artifact")
            || message.contains("must be produced by capsem-bench-rs"),
        "{message}"
    );
}

#[test]
fn shell_quote_preserves_single_argument_boundaries() {
    assert_eq!(
        shell_quote("http://127.0.0.1:3713"),
        "http://127.0.0.1:3713"
    );
    assert_eq!(
        shell_quote("tiny_http,mcp_tool_call"),
        "tiny_http,mcp_tool_call"
    );
    assert_eq!(shell_quote("weird value"), "'weird value'");
    assert_eq!(shell_quote("can't"), "'can'\"'\"'t'");
}

#[test]
fn build_delta_report_keeps_inline_artifact_identity() {
    let row = ScenarioResult {
        name: "tiny_http".to_string(),
        path: "/tiny".to_string(),
        body_kind: "tiny".to_string(),
        total_requests: 100,
        concurrency: 10,
        successful: 100,
        failed: 0,
        transport_retries: 0,
        total_duration_ms: 10.0,
        requests_per_sec: 10_000.0,
        transfer_bytes: 2400,
        bytes_per_sec: 240_000.0,
        latency_ms: LatencySummary {
            min: 0.1,
            max: 2.0,
            mean: 0.5,
            p50: 0.4,
            p95: 1.0,
            p99: 1.5,
        },
        latency_samples: Vec::new(),
        errors: BTreeMap::new(),
        secret_shaped_fixture_seen: None,
        raw_secret_stored_in_result: None,
    };
    let host = Artifact {
        version: VERSION.to_string(),
        timestamp: 1.0,
        hostname: "host".to_string(),
        benchmark: "capsem-bench-rs".to_string(),
        mock_server_protocol: ProtocolReport {
            version: "1.1-rust".to_string(),
            lane: "host_direct".to_string(),
            base_url: "http://127.0.0.1:3713".to_string(),
            dns_udp_addr: None,
            total_requests: 100,
            concurrency: 10,
            timeout_ms: 30_000,
            selected_scenarios: vec!["tiny_http".to_string()],
            scenarios: vec![row.clone()],
        },
    };
    let mut guest = Artifact {
        mock_server_protocol: ProtocolReport {
            lane: "guest_capsem".to_string(),
            scenarios: vec![ScenarioResult {
                requests_per_sec: 5_000.0,
                latency_ms: LatencySummary {
                    p50: 1.4,
                    p95: 3.0,
                    p99: 4.5,
                    ..row.latency_ms.clone()
                },
                latency_samples: Vec::new(),
                ..row
            }],
            ..host.mock_server_protocol.clone()
        },
        ..host.clone()
    };
    guest.hostname = "guest".to_string();
    let delta = build_delta_report(
        "host:inline".to_string(),
        "guest:inline".to_string(),
        &host,
        &guest,
    )
    .unwrap();
    assert_eq!(delta.host_artifact, "host:inline");
    assert_eq!(delta.guest_artifact, "guest:inline");
    assert_eq!(delta.host_lane, "host_direct");
    assert_eq!(delta.guest_lane, "guest_capsem");
}

// ── JSON extraction from guest command output ──────────────────────
//
// extract_first_json_value scrapes a JSON document out of whatever a guest
// command printed on stdout: banners, shell noise, progress lines. The bench
// harness then treats that value as the measurement. If it picks the wrong
// object or silently finds none, a benchmark reports a number that was never
// measured, so the scan's exact behaviour matters.

#[test]
fn json_is_extracted_from_surrounding_shell_noise() {
    let output = "warning: locale unset\n{\"requests\":10,\"ok\":true}\ndone\n";

    let value = extract_first_json_value(output).expect("object found");

    assert_eq!(value["requests"], 10);
    assert_eq!(value["ok"], true);
}

#[test]
fn the_first_parseable_object_wins() {
    // Two candidates: the earlier one is the result, the later one must not
    // overwrite it.
    let output = r#"{"round":1} then later {"round":2}"#;

    assert_eq!(extract_first_json_value(output).unwrap()["round"], 1);
}

#[test]
fn a_brace_inside_prose_falls_through_to_the_real_object() {
    // The scan is not string-aware: it tries every `{` in order, including one
    // sitting inside quoted prose. That candidate simply fails to parse, so
    // the next one wins. Worth stating, because it means correctness here
    // rests on the parse failing rather than on the brace being skipped.
    let output = r#"note: "a { brace" then {"real":true}"#;

    let value = extract_first_json_value(output).expect("object found");

    assert_eq!(value["real"], true);
}

#[test]
fn nested_objects_are_returned_whole() {
    let output = r#"prefix {"outer":{"inner":[1,2,3]}} suffix"#;

    let value = extract_first_json_value(output).expect("object found");

    assert_eq!(value["outer"]["inner"][2], 3);
}

#[test]
fn output_without_a_parseable_object_yields_none() {
    for output in [
        "",
        "no braces here",
        "{ unterminated",
        "{not: valid json}",
    ] {
        assert_eq!(
            extract_first_json_value(output),
            None,
            "{output:?} should not parse"
        );
    }
}

#[test]
fn trailing_output_after_the_object_is_tolerated() {
    // The guest command's own exit banner follows the payload; the scan uses a
    // streaming deserializer precisely so this still parses.
    let output = "{\"ok\":true}\nexit code 0\n";

    assert_eq!(extract_first_json_value(output).unwrap()["ok"], true);
}

// ── Latency summary edges ──────────────────────────────────────────
//
// These numbers get published as benchmark results, so a degenerate sample set
// must not produce a plausible-looking figure.

#[test]
fn an_empty_sample_set_summarises_to_zeros() {
    let summary = latency_summary(Vec::new());

    assert_eq!(summary.min, 0.0);
    assert_eq!(summary.max, 0.0);
    assert_eq!(summary.mean, 0.0);
    assert_eq!(summary.p50, 0.0);
    assert_eq!(summary.p99, 0.0);
}

#[test]
fn a_single_sample_is_every_percentile() {
    let summary = latency_summary(vec![7.5]);

    assert_eq!(summary.min, 7.5);
    assert_eq!(summary.max, 7.5);
    assert_eq!(summary.mean, 7.5);
    assert_eq!(summary.p50, 7.5);
    assert_eq!(summary.p95, 7.5);
    assert_eq!(summary.p99, 7.5);
}

#[test]
fn samples_are_sorted_before_percentiles_are_taken() {
    // Arrival order is not latency order; summarising unsorted input would
    // report whichever sample happened to land at the percentile index.
    let ascending = latency_summary(vec![1.0, 2.0, 3.0, 4.0, 100.0]);
    let shuffled = latency_summary(vec![100.0, 3.0, 1.0, 4.0, 2.0]);

    assert_eq!(ascending.p50, shuffled.p50);
    assert_eq!(ascending.p95, shuffled.p95);
    assert_eq!(ascending.min, shuffled.min);
    assert_eq!(ascending.max, shuffled.max);
}

#[test]
fn percentiles_never_index_past_the_sample_set() {
    // p99 of two samples lands between them, not past the end.
    let summary = latency_summary(vec![10.0, 20.0]);

    assert!(summary.p99 <= summary.max, "{summary:?}");
    assert!(summary.p50 >= summary.min, "{summary:?}");
}

#[test]
fn round3_keeps_three_decimals_without_drifting() {
    assert_eq!(round3(1.23456), 1.235);
    assert_eq!(round3(1.0), 1.0);
    assert_eq!(round3(0.0005), 0.001);
    assert_eq!(round3(-1.23456), -1.235);
}
