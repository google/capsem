//! Exported session metrics are the API's numbers, not a second count.

use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};

use super::*;
use crate::service_runtime::telemetry_export::{collect, grant_metric_endpoint, register, SessionTable};

fn net(decision: capsem_logger::Decision, bytes_sent: u64) -> capsem_logger::WriteOp {
    capsem_logger::WriteOp::NetEvent(
        serde_json::from_value(serde_json::json!({
            "timestamp": 1_789_000_000.0, "domain": "example.com", "port": 443,
            "decision": decision.as_str(), "bytes_sent": bytes_sent, "bytes_received": 0, "duration_ms": 1,
        }))
        .unwrap(),
    )
}

fn model(input: u64, output: u64, cost: f64) -> capsem_logger::WriteOp {
    capsem_logger::WriteOp::ModelCall(
        serde_json::from_value(serde_json::json!({
            "timestamp": 1_789_000_000.0, "provider": "anthropic", "model": "m", "method": "POST",
            "path": "/v1/messages", "stream": false, "messages_count": 1, "tools_count": 0,
            "request_bytes": 1, "input_tokens": input, "output_tokens": output, "usage_details": {},
            "duration_ms": 1, "response_bytes": 1, "estimated_cost_usd": cost,
            "tool_calls": [{"call_index": 0, "call_id": "c", "tool_name": "bash", "origin": "native"}],
            "tool_responses": [],
        }))
        .unwrap(),
    )
}

fn session(state: &ServiceState, id: &str, ops: Vec<capsem_logger::WriteOp>) {
    let session_dir = state.run_dir.join("sessions").join(id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    for op in ops {
        writer.write_blocking(op);
    }
    writer.shutdown_blocking();
    insert_fake_instance_with_session_dir(state, id, std::process::id(), session_dir);
}

/// Every exported point, as (metric, session.id, extra attribute, value).
fn exported(exporter: &InMemoryMetricExporter) -> Vec<(String, String, String, f64)> {
    let mut points = Vec::new();
    for resource in exporter.get_finished_metrics().unwrap() {
        for metric in resource.scope_metrics().flat_map(|scope| scope.metrics()) {
            let mut push = |attributes: Vec<&opentelemetry::KeyValue>, value: f64| {
                let mut keys: Vec<String> = attributes.iter().map(|kv| kv.key.to_string()).collect();
                keys.sort();
                let session = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == "session.id")
                    .map(|kv| kv.value.to_string())
                    .unwrap();
                let extra = attributes
                    .iter()
                    .find(|kv| !["session.id", "profile.id", "persistent"].contains(&kv.key.as_str()))
                    .map(|kv| kv.value.to_string())
                    .unwrap_or_default();
                let base = ["persistent", "profile.id", "session.id"];
                assert!(
                    base.iter().all(|key| keys.iter().any(|k| k == key)) && keys.len() <= 4,
                    "{}: attributes are exactly session.id, profile.id, persistent (+ one breakdown): {keys:?}",
                    metric.name()
                );
                points.push((metric.name().to_string(), session, extra, value));
            };
            match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) => {
                    for point in sum.data_points() {
                        push(point.attributes().collect(), point.value() as f64);
                    }
                }
                AggregatedMetrics::F64(MetricData::Sum(sum)) => {
                    for point in sum.data_points() {
                        push(point.attributes().collect(), point.value());
                    }
                }
                other => panic!("{} exported as {other:?}", metric.name()),
            }
        }
    }
    points
}

#[tokio::test]
async fn exported_session_metrics_equal_the_api_totals() {
    let (state, _dir) = make_test_state_with_tempdir();
    session(
        &state,
        "export-a",
        vec![
            net(capsem_logger::Decision::Allowed, 1),
            net(capsem_logger::Decision::Denied, 1),
            model(100, 40, 0.25),
        ],
    );
    session(&state, "export-b", vec![model(7, 3, 0.000_5), model(1, 1, 0.1)]);

    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter.clone())
        .build();
    let table = SessionTable::default();
    let _instruments = register(
        &opentelemetry::metrics::MeterProvider::meter(&provider, "capsem"),
        &table,
    );
    *table.write().unwrap() = collect(&state).await.unwrap();
    provider.force_flush().unwrap();
    let points = exported(&exporter);
    let value = |metric: &str, session: &str, extra: &str| {
        points
            .iter()
            .find(|(m, s, e, _)| m == metric && s == session && e == extra)
            .map(|(_, _, _, value)| *value)
            .unwrap_or_else(|| panic!("no {metric} point for {session} {extra}: {points:?}"))
    };

    let app = build_service_router(Arc::clone(&state));
    for id in ["export-a", "export-b"] {
        let (status, info) =
            route_request(app.clone(), axum::http::Method::GET, &format!("/vms/{id}/info"), None).await;
        assert_eq!(status, StatusCode::OK, "{info}");
        let api = |field: &str| info[field].as_f64().unwrap_or_else(|| panic!("{field}: {info}"));
        use capsem_telemetry::session as n;
        assert_eq!(value(n::SESSION_REQUESTS_TOTAL, id, "allowed"), api("allowed_requests"));
        assert_eq!(value(n::SESSION_REQUESTS_TOTAL, id, "denied"), api("denied_requests"));
        assert_eq!(value(n::SESSION_TOKENS_TOTAL, id, "input"), api("total_input_tokens"));
        assert_eq!(value(n::SESSION_TOKENS_TOTAL, id, "output"), api("total_output_tokens"));
        assert_eq!(value(n::SESSION_MODEL_CALLS_TOTAL, id, ""), api("model_call_count"));
        assert_eq!(value(n::SESSION_TOOL_CALLS_TOTAL, id, ""), api("total_tool_calls"));
        assert_eq!(value(n::SESSION_FILE_EVENTS_TOTAL, id, ""), api("total_file_events"));
        assert_eq!(value(n::SESSION_COST_USD_TOTAL, id, ""), api("total_estimated_cost"));
    }
    assert_eq!(
        value(capsem_telemetry::session::SESSION_TOKENS_TOTAL, "export-b", "input"),
        8.0
    );
}

fn granted(open_telemetry: Option<&str>) -> Vec<String> {
    let mut corp = capsem_core::net::policy_config::SettingsFile::default();
    corp.corp_rule_files.open_telemetry = open_telemetry.map(str::to_string);
    let mut command = tokio::process::Command::new("capsem-process");
    grant_metric_endpoint(&mut command, &corp);
    command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// A VM process learns its export endpoint only from its launch arguments.
#[test]
fn a_vm_process_is_granted_the_corp_metric_endpoint_at_launch() {
    assert_eq!(
        granted(Some("https://otel.example/")),
        ["--metric-endpoint", "https://otel.example"]
    );
    assert!(granted(None).is_empty(), "no corp endpoint, no export");
    assert!(granted(Some("  ")).is_empty(), "a blank endpoint is no endpoint");
}
