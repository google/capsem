use super::*;
use crate::scenarios::{Artifact, LatencySummary, ProtocolReport, ScenarioResult};
use std::collections::BTreeMap;

#[test]
fn collector_keeps_raw_protocol_latencies() {
    let artifact = Artifact {
        version: "test".into(),
        timestamp: 1.0,
        hostname: "guest".into(),
        benchmark: "capsem-bench-rs".into(),
        mock_server_protocol: ProtocolReport {
            version: "test".into(),
            lane: "guest_capsem".into(),
            base_url: "http://127.0.0.1:3713".into(),
            dns_udp_addr: None,
            total_requests: 3,
            concurrency: 2,
            timeout_ms: 1_000,
            selected_scenarios: vec!["model_json_response".into()],
            scenarios: vec![ScenarioResult {
                name: "model_json_response".into(),
                path: "/model/response".into(),
                body_kind: "model_json".into(),
                total_requests: 3,
                concurrency: 2,
                successful: 3,
                failed: 0,
                transport_retries: 0,
                total_duration_ms: 4.0,
                requests_per_sec: 750.0,
                transfer_bytes: 120,
                bytes_per_sec: 30_000.0,
                latency_ms: LatencySummary {
                    min: 0.5,
                    max: 2.5,
                    mean: 1.5,
                    p50: 1.5,
                    p95: 2.4,
                    p99: 2.5,
                },
                latency_samples: vec![0.5, 1.5, 2.5],
                errors: BTreeMap::new(),
                secret_shaped_fixture_seen: None,
                raw_secret_stored_in_result: None,
            }],
        },
    };

    let document = build(&artifact).unwrap();
    let latency = &document.metrics["guest_capsem.model_json_response.latency_ms"];
    assert_eq!(latency.unit, "milliseconds");
    assert_eq!(latency.samples, [0.5, 1.5, 2.5]);
    assert!(document.sidecar.contains("model_json_response"));
    assert!(!document.sidecar.contains("latency_samples"));
}
