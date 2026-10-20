//! `GET /vms/{id}/export/rows.ndjson.gz`: streamed rows-only ledger export tests.

use std::collections::BTreeMap;

use super::*;
use capsem_logger::ledger_protocol::{DEFAULT_EXPORT_LEDGER_TABLES, MAX_LEDGER_QUERY_ROWS};

fn parse_ndjson_gz(bytes: &[u8]) -> Vec<serde_json::Value> {
    let raw = capsem_logger::gunzip_bytes(bytes).expect("decompress streamed rows.ndjson.gz");
    let text = String::from_utf8(raw).expect("rows.ndjson.gz is UTF-8");
    text.lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("each line is valid JSON"))
        .collect()
}

async fn seed_session_for_rows_export(state: &Arc<ServiceState>, vm_id: &str, session_dir: &StdPath) {
    std::fs::create_dir_all(session_dir).unwrap();
    insert_fake_instance_with_session_dir(state, vm_id, std::process::id(), session_dir.to_path_buf());
    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::ExecEvent(capsem_logger::ExecEvent {
            event_id: Some("aa11bb22cc33".to_string()),
            timestamp: std::time::SystemTime::now(),
            exec_id: 101,
            command: "echo rows-export".to_string(),
            source: "api".to_string(),
            target: capsem_proto::ipc::ExecTarget::Vm,
            trace_id: Some("trace-rows".to_string()),
            process_name: Some("bash".to_string()),
            credential_ref: None,
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEventComplete(
            capsem_logger::ExecEventComplete {
                exec_id: 101,
                exit_code: 0,
                duration_ms: 5,
                stdout: b"rows-export\n".to_vec(),
                stderr: Vec::new(),
                stdout_bytes: 12,
                stderr_bytes: 0,
                pid: Some(42),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::ModelCall(capsem_logger::ModelCall {
            event_id: Some("dd44ee55ff66".to_string()),
            timestamp: std::time::SystemTime::now(),
            provider: "openai".to_string(),
            protocol: Some("openai".to_string()),
            model: Some("gpt-5-nano".to_string()),
            process_name: Some("codex".to_string()),
            pid: Some(42),
            method: "POST".to_string(),
            path: "/v1/responses".to_string(),
            stream: false,
            system_prompt_preview: None,
            messages_count: 1,
            tools_count: 1,
            request_bytes: 32,
            request_body: Some(br#"{"input":"hello"}"#.to_vec()),
            message_id: Some("msg-1".to_string()),
            status_code: Some(200),
            text_content: Some("world".to_string()),
            thinking_content: None,
            response_body: Some(br#"{"output_text":"world"}"#.to_vec()),
            stop_reason: Some("end_turn".to_string()),
            input_tokens: Some(4),
            output_tokens: Some(2),
            usage_details: BTreeMap::new(),
            duration_ms: 10,
            response_bytes: 23,
            estimated_cost_usd: 0.0001,
            trace_id: Some("trace-rows".to_string()),
            credential_ref: None,
            tool_calls: vec![capsem_logger::ToolCallEntry {
                event_id: Some("112233445566".to_string()),
                call_index: 0,
                call_id: "call-1".to_string(),
                tool_name: "bash".to_string(),
                arguments: Some(r#"{"cmd":"ls"}"#.to_string()),
                origin: "model".to_string(),
                trace_id: Some("trace-rows".to_string()),
            }],
            tool_responses: vec![capsem_logger::ToolResponseEntry {
                event_id: Some("778899aabbcc".to_string()),
                call_id: "call-1".to_string(),
                content_preview: Some("ok".to_string()),
                is_error: false,
                trace_id: Some("trace-rows".to_string()),
                credential_ref: None,
            }],
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_000_000,
                "998877665544",
                "model.call",
                "profiles.rules.test_rule",
                r#"{"name":"test_rule"}"#,
                r#"{"event_type":"model.call"}"#,
            )
            .with_rule_action(capsem_logger::SecurityRuleAction::Allow)
            .with_detection_level(capsem_logger::SecurityDetectionLevel::High),
        ))
        .await;
    writer.flush().await;
    tokio::task::spawn_blocking(move || writer.shutdown_blocking())
        .await
        .unwrap();
}

#[tokio::test]
async fn default_rows_export_streams_header_and_non_security_tables() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("rows-vm");
    seed_session_for_rows_export(&state, "rows-vm", &session_dir).await;

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::GET)
                .uri("/vms/rows-vm/export/rows.ndjson.gz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(headers[axum::http::header::CONTENT_TYPE], "application/gzip");
    assert_eq!(
        headers[axum::http::header::CONTENT_DISPOSITION],
        "attachment; filename=\"capsem-session-rows-vm.rows.ndjson.gz\""
    );
    assert!(!headers.contains_key(axum::http::header::CONTENT_ENCODING));

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let lines = parse_ndjson_gz(&bytes);
    assert!(lines.len() >= 5, "expected header plus exported rows: {lines:?}");

    let header = &lines[0];
    assert_eq!(header["type"], "header");
    assert_eq!(header["format"], "capsem.rows.v1");
    assert_eq!(header["format_version"], 1);
    assert_eq!(header["vm_id"], "rows-vm");
    assert!(header["exported_at"].as_str().is_some_and(|s| !s.is_empty()));
    let expected_tables: Vec<serde_json::Value> = DEFAULT_EXPORT_LEDGER_TABLES
        .iter()
        .map(|table| json!(table))
        .collect();
    assert_eq!(header["tables"], serde_json::Value::Array(expected_tables));

    let rows = &lines[1..];
    assert!(rows
        .iter()
        .any(|line| line["table"] == "model_calls" && line["row"]["event_id"] == "dd44ee55ff66"));
    assert!(rows
        .iter()
        .any(|line| line["table"] == "tool_calls" && line["row"]["call_id"] == "call-1"));
    assert!(rows
        .iter()
        .any(|line| line["table"] == "tool_responses" && line["row"]["event_id"] == "778899aabbcc"));
    assert!(rows
        .iter()
        .any(|line| line["table"] == "exec_events" && line["row"]["event_id"] == "aa11bb22cc33"));
    assert!(rows
        .iter()
        .any(|line| line["table"] == "event_body_blobs" && line["row"]["event_id"] == "dd44ee55ff66"));
    // Default export excludes security_rule_events and security_decision_events.
    assert!(rows.iter().all(|line| line["table"] != "security_rule_events"));
    assert!(rows.iter().all(|line| line["table"] != "security_decision_events"));
}

#[tokio::test]
async fn rows_export_supports_explicit_tables_filter_and_rejects_invalid_tables() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("filter-vm");
    seed_session_for_rows_export(&state, "filter-vm", &session_dir).await;

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::GET)
                .uri("/vms/filter-vm/export/rows.ndjson.gz?tables=model_calls,security_rule_events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let lines = parse_ndjson_gz(&bytes);
    assert_eq!(
        lines[0]["tables"],
        json!(["model_calls", "security_rule_events"])
    );
    let tables_seen: Vec<&str> = lines[1..]
        .iter()
        .map(|line| line["table"].as_str().unwrap())
        .collect();
    assert_eq!(tables_seen, vec!["model_calls", "security_rule_events"]);
    assert_eq!(lines[2]["row"]["event_id"], "998877665544");
    assert_eq!(lines[2]["row"]["rule_json"], r#"{"name":"test_rule"}"#);

    for bad_query in [
        "tables=sqlite_master",
        "tables=ledger_commitments",
        "tables=",
        "tables=model_calls,unknown_table",
    ] {
        let (status, body) = route_request(
            app.clone(),
            axum::http::Method::GET,
            &format!("/vms/filter-vm/export/rows.ndjson.gz?{bad_query}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_query}: {body}");
    }

    let (status, _) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/missing-vm/export/rows.ndjson.gz",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rows_export_pages_across_multiple_ledger_query_batches_in_id_order() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("paged-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "paged-vm", std::process::id(), session_dir.clone());

    let total_rows = MAX_LEDGER_QUERY_ROWS + 5;
    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 64).unwrap();
    for index in 0..total_rows {
        writer
            .write(capsem_logger::WriteOp::ExecEvent(capsem_logger::ExecEvent {
                event_id: Some(format!("{index:012x}")),
                timestamp: std::time::SystemTime::UNIX_EPOCH,
                exec_id: (1000 + index) as u64,
                command: format!("echo row-{index}"),
                source: "api".to_string(),
                target: capsem_proto::ipc::ExecTarget::Vm,
                trace_id: None,
                process_name: Some("bash".to_string()),
                credential_ref: None,
            }))
            .await;
    }
    writer.flush().await;
    tokio::task::spawn_blocking(move || writer.shutdown_blocking())
        .await
        .unwrap();

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::GET)
                .uri("/vms/paged-vm/export/rows.ndjson.gz?tables=exec_events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let lines = parse_ndjson_gz(&bytes);
    assert_eq!(lines.len(), total_rows + 1);
    for (idx, entry) in lines[1..].iter().enumerate() {
        assert_eq!(entry["table"], "exec_events");
        assert_eq!(entry["row"]["id"], (idx + 1) as i64);
        assert_eq!(entry["row"]["command"], format!("echo row-{idx}"));
    }
}
