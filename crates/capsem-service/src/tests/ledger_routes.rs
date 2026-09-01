use super::*;

#[tokio::test]
async fn security_routes_read_security_ledger_from_session_db() {
    let state = make_test_state();
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("vm-ledger");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "vm-ledger", std::process::id(), session_dir.clone());

    let db_path = session_dir.join("session.db");
    let db_path_for_writer = db_path.clone();
    tokio::task::spawn_blocking(move || {
        let writer = capsem_logger::DbWriter::open(&db_path_for_writer, 16).unwrap();
        writer.write_blocking(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_123_456,
                "abcdef123456",
                "model.call",
                "profiles.rules.ai_ollama_model_api",
                r#"{"name":"ollama_model_api_observed","match":"model.provider == \"ollama\""}"#,
                r#"{"model":{"provider":"ollama","name":"llama3.2"}}"#,
            )
            .with_rule_action(capsem_logger::SecurityRuleAction::Allow)
            .with_detection_level(capsem_logger::SecurityDetectionLevel::Informational)
            .with_trace_id("trace_ollama"),
        ));
        writer.shutdown_blocking();
    })
    .await
    .unwrap();
    let response = handle_security_latest(
        State(Arc::clone(&state)),
        Path("vm-ledger".to_string()),
        Query(SecurityLedgerQuery { limit: Some(10) }),
    )
    .await
    .expect("security latest reads session ledger");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let events: Vec<capsem_logger::SecurityRuleEvent> = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.event_id, "abcdef123456");
    assert_eq!(event.event_type, "model.call");
    assert_eq!(event.rule_id, "profiles.rules.ai_ollama_model_api");
    assert_eq!(event.rule_action, capsem_logger::SecurityRuleAction::Allow);
    assert_eq!(
        event.detection_level,
        capsem_logger::SecurityDetectionLevel::Informational
    );
    assert!(event.rule_json.contains("ollama_model_api_observed"));
    assert!(event.event_json.contains(r#""provider":"ollama""#));
    assert_eq!(event.trace_id.as_deref(), Some("trace_ollama"));

    let response = handle_security_info(State(state), Path("vm-ledger".to_string()))
        .await
        .expect("security status reads session ledger");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let stats: capsem_logger::SecurityRuleStats = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stats.total, 1);
    assert_eq!(stats.by_action[0].rule_action, "allow");
    assert_eq!(stats.by_action[0].count, 1);
    assert_eq!(stats.by_event_type[0].event_type, "model.call");
    assert_eq!(stats.by_event_type[0].count, 1);
    assert_eq!(stats.by_level[0].detection_level, "informational");
    assert_eq!(stats.by_level[0].count, 1);
    assert_eq!(stats.by_rule[0].rule_id, "profiles.rules.ai_ollama_model_api");
    assert_eq!(stats.by_rule[0].rule_action, "allow");
    assert_eq!(stats.by_rule[0].detection_level, "informational");
    assert_eq!(stats.by_rule[0].count, 1);
    assert_eq!(stats.by_rule[0].latest_event_id, "abcdef123456");
}

#[tokio::test]
async fn history_routes_read_history_ledger_from_session_db() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("history-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "history-vm", std::process::id(), session_dir.clone());

    let db_path = session_dir.join("session.db");
    let marker = "history-ledger-marker";
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::ExecEvent(capsem_logger::ExecEvent {
            event_id: None,
            timestamp: std::time::SystemTime::now(),
            exec_id: 41,
            command: format!("echo {marker}"),
            source: "api".to_string(),
            trace_id: Some("trace-history".to_string()),
            process_name: Some("bash".to_string()),
            credential_ref: None,
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEventComplete(
            capsem_logger::ExecEventComplete {
                exec_id: 41,
                exit_code: 0,
                duration_ms: 17,
                stdout_preview: Some(format!("{marker}\n")),
                stderr_preview: None,
                stdout_bytes: (marker.len() + 1) as u64,
                stderr_bytes: 0,
                pid: Some(123),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::AuditEvent(capsem_logger::AuditEvent {
            event_id: None,
            timestamp: std::time::SystemTime::now(),
            pid: 123,
            ppid: 1,
            uid: 0,
            exe: "/usr/bin/bash".to_string(),
            comm: Some("bash".to_string()),
            argv: format!("bash -lc 'echo {marker}'"),
            cwd: Some("/root".to_string()),
            tty: None,
            session_id: Some(1),
            audit_id: Some("audit-history".to_string()),
            exec_event_id: Some(41),
            parent_exe: Some("/usr/bin/sh".to_string()),
            trace_id: Some("trace-history".to_string()),
            credential_ref: None,
        }))
        .await;
    writer.shutdown_blocking();

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let direct_counts = reader.history_counts().unwrap();
    assert_eq!(direct_counts.exec_count, 1);
    assert_eq!(direct_counts.audit_count, 1);
    let (status, counts) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/history-vm/history/counts",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{counts}");
    assert_eq!(counts["exec_count"], 1);
    assert_eq!(counts["audit_count"], 1);

    let (status, history) = route_request(
        app.clone(),
        axum::http::Method::GET,
        &format!("/vms/history-vm/history?search={marker}&limit=10"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["total"], 2);
    let commands = history["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 2, "{history}");
    assert!(commands.iter().any(|entry| entry["layer"] == "exec"
        && entry["command"].as_str().unwrap().contains(marker)
        && entry["stdout_preview"].as_str().unwrap().contains(marker)));
    assert!(commands.iter().any(|entry| entry["layer"] == "audit"
        && entry["command"].as_str().unwrap().contains(marker)
        && entry["details"]["exe"] == "/usr/bin/bash"));

    let (status, processes) =
        route_request(app, axum::http::Method::GET, "/vms/history-vm/history/processes", None).await;
    assert_eq!(status, StatusCode::OK, "{processes}");
    assert_eq!(processes["processes"][0]["exe"], "/usr/bin/bash");
    assert_eq!(processes["processes"][0]["command_count"], 1);
}

#[tokio::test]
async fn detection_latest_route_filters_non_detection_rule_rows() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("detect-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "detect-vm", std::process::id(), session_dir.clone());

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_123_456,
                "aaaaaa000000",
                "http.request",
                "profiles.rules.default_http",
                r#"{"name":"default_http"}"#,
                r#"{"event_type":"http.request"}"#,
            )
            .with_rule_action(capsem_logger::SecurityRuleAction::Allow),
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_123_457,
                "bbbbbb000000",
                "model.call",
                "profiles.rules.ai_unknown_provider",
                r#"{"name":"ai_unknown_provider"}"#,
                r#"{"event_type":"model.call","model":{"provider":"unknown"}}"#,
            )
            .with_rule_action(capsem_logger::SecurityRuleAction::Allow)
            .with_detection_level(capsem_logger::SecurityDetectionLevel::High),
        ))
        .await;
    writer.shutdown_blocking();
    let direct_rows = capsem_logger::DbReader::open(&db_path)
        .unwrap()
        .recent_security_rule_events(10)
        .unwrap();
    assert_eq!(direct_rows.len(), 2);
    assert!(direct_rows.iter().any(
        |row| row.event_id == "bbbbbb000000" && row.detection_level == capsem_logger::SecurityDetectionLevel::High
    ));
    let (status, detection) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/detect-vm/detection/latest?limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detection}");
    let detection_rows = detection.as_array().unwrap();
    assert_eq!(detection_rows.len(), 1, "{detection}");
    assert_eq!(detection_rows[0]["event_id"], "bbbbbb000000");
    assert_eq!(detection_rows[0]["detection_level"], "high");

    let (status, enforcement) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/detect-vm/enforcement/latest?limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{enforcement}");
    let enforcement_rows = enforcement.as_array().unwrap();
    assert_eq!(enforcement_rows.len(), 2, "{enforcement}");
    assert!(enforcement_rows.iter().any(|row| row["event_id"] == "aaaaaa000000"));
    assert!(enforcement_rows.iter().any(|row| row["event_id"] == "bbbbbb000000"));
}

#[tokio::test]
async fn timeline_route_reads_timeline_ledger_from_session_db() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("timeline-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "timeline-vm", std::process::id(), session_dir.clone());

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::ExecEvent(capsem_logger::ExecEvent {
            event_id: Some("ccc111000000".to_string()),
            timestamp: std::time::SystemTime::now(),
            exec_id: 77,
            command: "echo timeline-marker".to_string(),
            source: "api".to_string(),
            trace_id: Some("trace-timeline".to_string()),
            process_name: Some("bash".to_string()),
            credential_ref: None,
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEventComplete(
            capsem_logger::ExecEventComplete {
                exec_id: 77,
                exit_code: 0,
                duration_ms: 11,
                stdout_preview: Some("timeline-marker\n".to_string()),
                stderr_preview: None,
                stdout_bytes: 16,
                stderr_bytes: 0,
                pid: Some(456),
            },
        ))
        .await;
    writer
        .write(capsem_logger::WriteOp::NetEvent(capsem_logger::NetEvent {
            event_id: Some("ddd222000000".to_string()),
            timestamp: std::time::SystemTime::now(),
            domain: "127.0.0.1".to_string(),
            port: 3713,
            decision: capsem_logger::Decision::Allowed,
            process_name: Some("curl".to_string()),
            pid: Some(456),
            method: Some("POST".to_string()),
            path: Some("/echo".to_string()),
            query: None,
            status_code: Some(200),
            bytes_sent: 2,
            bytes_received: 17,
            duration_ms: 9,
            matched_rule: Some("profiles.rules.default_http".to_string()),
            request_headers: None,
            response_headers: None,
            request_body_preview: Some("{}".to_string()),
            response_body_preview: Some(r#"{"ok":true}"#.to_string()),
            request_body_full: Some("{}".to_string()),
            response_body_full: Some(r#"{"ok":true}"#.to_string()),
            conn_type: Some("http".to_string()),
            policy_mode: None,
            policy_action: Some("allow".to_string()),
            policy_rule: Some("profiles.rules.default_http".to_string()),
            policy_reason: None,
            trace_id: Some("trace-timeline".to_string()),
            credential_ref: None,
        }))
        .await;
    writer.shutdown_blocking();

    let (status, timeline) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/timeline-vm/timeline?trace_id=trace-timeline&layers=exec,net&limit=20",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{timeline}");
    assert_eq!(
        timeline["columns"],
        json!([
            "timestamp",
            "layer",
            "ref",
            "summary",
            "status",
            "duration_ms",
            "trace_id"
        ])
    );
    let rows = timeline["rows"].as_array().unwrap();
    assert!(rows.iter().any(|row| row[1] == "exec"
        && row[2] == 77
        && row[3] == "echo timeline-marker"
        && row[4] == 0
        && row[5] == 11
        && row[6] == "trace-timeline"));
    assert!(rows.iter().any(|row| row[1] == "net"
        && row[2] == 1
        && row[3] == "POST 127.0.0.1/echo"
        && row[4] == 200
        && row[5] == 9
        && row[6] == "trace-timeline"));
}

#[tokio::test]
async fn triage_route_reads_triage_ledger_from_session_db() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("triage-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "triage-vm", std::process::id(), session_dir.clone());

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::NetEvent(capsem_logger::NetEvent {
            event_id: Some("aaa111000000".to_string()),
            timestamp: std::time::SystemTime::now(),
            domain: "evil.test".to_string(),
            port: 443,
            decision: capsem_logger::Decision::Denied,
            process_name: Some("curl".to_string()),
            pid: Some(789),
            method: Some("GET".to_string()),
            path: Some("/blocked".to_string()),
            query: None,
            status_code: Some(403),
            bytes_sent: 3,
            bytes_received: 0,
            duration_ms: 13,
            matched_rule: Some("corp.rules.block_evil".to_string()),
            request_headers: None,
            response_headers: None,
            request_body_preview: None,
            response_body_preview: None,
            request_body_full: None,
            response_body_full: None,
            conn_type: Some("http".to_string()),
            policy_mode: None,
            policy_action: Some("block".to_string()),
            policy_rule: Some("corp.rules.block_evil".to_string()),
            policy_reason: Some("test denied net".to_string()),
            trace_id: Some("trace-triage".to_string()),
            credential_ref: None,
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::McpCall(capsem_logger::McpCall {
            event_id: Some("bbb111000000".to_string()),
            timestamp: std::time::SystemTime::now(),
            server_name: "local".to_string(),
            method: "tools/call".to_string(),
            tool_name: Some("fetch_http".to_string()),
            transport: "vsock_frame".to_string(),
            request_id: Some("mcp-request-1".to_string()),
            request_preview: Some(r#"{"url":"https://evil.test"}"#.to_string()),
            response_preview: None,
            decision: "error".to_string(),
            duration_ms: 17,
            error_message: Some("boom".to_string()),
            process_name: Some("agent".to_string()),
            bytes_sent: 33,
            bytes_received: 0,
            policy_mode: Some("enforce".to_string()),
            policy_action: Some("block".to_string()),
            policy_rule: Some("profiles.rules.mcp_local_fetch_http".to_string()),
            policy_reason: Some("test mcp error".to_string()),
            trace_id: Some("trace-triage".to_string()),
            credential_ref: None,
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEvent(capsem_logger::ExecEvent {
            event_id: Some("ccc111000000".to_string()),
            timestamp: std::time::SystemTime::now(),
            exec_id: 91,
            command: "false".to_string(),
            source: "api".to_string(),
            trace_id: Some("trace-triage".to_string()),
            process_name: Some("bash".to_string()),
            credential_ref: None,
        }))
        .await;
    writer
        .write(capsem_logger::WriteOp::ExecEventComplete(
            capsem_logger::ExecEventComplete {
                exec_id: 91,
                exit_code: 2,
                duration_ms: 19,
                stdout_preview: None,
                stderr_preview: Some("failed\n".to_string()),
                stdout_bytes: 0,
                stderr_bytes: 7,
                pid: Some(789),
            },
        ))
        .await;
    writer.shutdown_blocking();

    let db = state
        .register_session_db_handle("triage-vm", &session_dir)
        .expect("test installs external DB reader after the process writer created session.db");
    let direct_triage = session_db_triage("triage-vm", &db, &db_path, 5).await.unwrap();
    assert_eq!(
        direct_triage["denied_net"]["rows"].as_array().unwrap().len(),
        1,
        "{direct_triage}"
    );
    assert_eq!(
        direct_triage["tool_errors"]["rows"].as_array().unwrap().len(),
        1,
        "{direct_triage}"
    );
    assert_eq!(
        direct_triage["exec_failures"]["rows"].as_array().unwrap().len(),
        1,
        "{direct_triage}"
    );

    let (status, triage) = route_request(app, axum::http::Method::GET, "/triage?id=triage-vm&limit=5", None).await;
    assert_eq!(status, StatusCode::OK, "{triage}");
    assert_eq!(triage["session_id"], "triage-vm");
    assert_eq!(
        triage["session"]["denied_net"]["columns"],
        json!(["timestamp", "domain", "decision", "status_code", "duration_ms"])
    );
    let denied_net = triage["session"]["denied_net"]["rows"].as_array().unwrap();
    assert_eq!(denied_net.len(), 1, "{triage}");
    assert_eq!(denied_net[0][1], "evil.test");
    assert_eq!(denied_net[0][2], "denied");
    assert_eq!(denied_net[0][3], 403);
    assert_eq!(denied_net[0][4], 13);

    assert_eq!(
        triage["session"]["tool_errors"]["columns"],
        json!([
            "timestamp",
            "server_name",
            "method",
            "decision",
            "policy_mode",
            "policy_action",
            "policy_rule",
            "policy_reason",
            "error_message",
            "duration_ms"
        ])
    );
    let tool_errors = triage["session"]["tool_errors"]["rows"].as_array().unwrap();
    assert_eq!(tool_errors.len(), 1, "{triage}");
    assert_eq!(tool_errors[0][1], "local");
    assert_eq!(tool_errors[0][2], "tools/call");
    assert_eq!(tool_errors[0][3], "error");
    assert_eq!(tool_errors[0][5], "block");
    assert_eq!(tool_errors[0][8], "boom");
    assert_eq!(tool_errors[0][9], 17);

    assert_eq!(
        triage["session"]["exec_failures"]["columns"],
        json!(["timestamp", "exec_id", "command", "exit_code", "duration_ms"])
    );
    let exec_failures = triage["session"]["exec_failures"]["rows"].as_array().unwrap();
    assert_eq!(exec_failures.len(), 1, "{triage}");
    assert_eq!(exec_failures[0][1], 91);
    assert_eq!(exec_failures[0][2], "false");
    assert_eq!(exec_failures[0][3], 2);
    assert_eq!(exec_failures[0][4], 19);
}

#[tokio::test]
async fn winterfell_routes_read_session_ledgers_after_startup_cache_hydration() {
    let dir = tempfile::tempdir().unwrap();
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let idx = capsem_core::session::SessionIndex::open(&sessions_dir.join("main.db")).unwrap();
    idx.create_session(&capsem_core::session::SessionRecord {
        id: "winterfell-vm".to_string(),
        mode: "virtiofs".to_string(),
        command: Some("winterfell".to_string()),
        status: "running".to_string(),
        created_at: "2026-06-24T00:00:00Z".to_string(),
        stopped_at: None,
        scratch_disk_size_gb: 16,
        ram_bytes: 4_294_967_296,
        total_requests: 0,
        allowed_requests: 0,
        denied_requests: 0,
        total_input_tokens: 0,
        total_output_tokens: 0,
        total_estimated_cost: 0.0,
        total_tool_calls: 0,
        total_file_events: 0,
        compressed_size_bytes: None,
        vacuumed_at: None,
        storage_mode: "virtiofs".to_string(),
        rootfs_hash: None,
        rootfs_version: None,
        forked_from: None,
        persistent: false,
        exec_count: 0,
        audit_event_count: 0,
    })
    .unwrap();
    drop(idx);

    let (state, _dir) = make_test_state_with_tempdir_at(dir);
    let app = build_service_router(Arc::clone(&state));
    let session_dir = sessions_dir.join("winterfell-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "winterfell-vm", std::process::id(), session_dir.clone());

    let db_path = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    writer.write_blocking(capsem_logger::WriteOp::SecurityRuleEvent(
        capsem_logger::SecurityRuleEvent::new(
            1_789_000_223_456,
            "abcdef123450",
            "model.call",
            "profiles.rules.ai_unknown_provider",
            r#"{"name":"ai_unknown_provider","match":"model.provider == \"unknown\""}"#,
            r#"{"event_type":"model.call","model":{"provider":"unknown"}}"#,
        )
        .with_rule_action(capsem_logger::SecurityRuleAction::Allow)
        .with_detection_level(capsem_logger::SecurityDetectionLevel::High)
        .with_trace_id("trace-winterfell")
        .with_turn_id("trace-winterfell")
        .with_credential_ref("credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    ));
    writer.write_blocking(capsem_logger::WriteOp::ExecEvent(capsem_logger::ExecEvent {
        event_id: Some("abcdef123451".to_string()),
        timestamp: std::time::SystemTime::now(),
        exec_id: 501,
        command: "echo winterfell".to_string(),
        source: "api".to_string(),
        trace_id: Some("trace-winterfell".to_string()),
        process_name: Some("bash".to_string()),
        credential_ref: None,
    }));
    writer.write_blocking(capsem_logger::WriteOp::ExecEventComplete(
        capsem_logger::ExecEventComplete {
            exec_id: 501,
            exit_code: 0,
            duration_ms: 12,
            stdout_preview: Some("winterfell\n".to_string()),
            stderr_preview: None,
            stdout_bytes: 11,
            stderr_bytes: 0,
            pid: Some(777),
        },
    ));
    writer.write_blocking(capsem_logger::WriteOp::NetEvent(capsem_logger::NetEvent {
        event_id: Some("abcdef123452".to_string()),
        timestamp: std::time::SystemTime::now(),
        domain: "mock.capsem.test".to_string(),
        port: 443,
        decision: capsem_logger::Decision::Allowed,
        process_name: Some("codex".to_string()),
        pid: Some(777),
        method: Some("POST".to_string()),
        path: Some("/v1/responses".to_string()),
        query: None,
        status_code: Some(200),
        bytes_sent: 44,
        bytes_received: 55,
        duration_ms: 8,
        matched_rule: Some("profiles.rules.default_http".to_string()),
        request_headers: Some("content-type: application/json".to_string()),
        response_headers: Some("content-type: application/json".to_string()),
        request_body_preview: Some(r#"{"input":"winterfell"}"#.to_string()),
        response_body_preview: Some(r#"{"output_text":"the wall holds"}"#.to_string()),
        request_body_full: Some(r#"{"input":"winterfell"}"#.to_string()),
        response_body_full: Some(r#"{"output_text":"the wall holds"}"#.to_string()),
        conn_type: Some("https".to_string()),
        policy_mode: None,
        policy_action: Some("allow".to_string()),
        policy_rule: Some("profiles.rules.default_http".to_string()),
        policy_reason: None,
        trace_id: Some("trace-winterfell".to_string()),
        credential_ref: Some(
            "credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        ),
    }));
    writer.write_blocking(capsem_logger::WriteOp::ModelCall(capsem_logger::ModelCall {
        event_id: Some("abcdef123453".to_string()),
        timestamp: std::time::SystemTime::now(),
        provider: "openai".to_string(),
        protocol: Some("openai".to_string()),
        model: Some("gpt-5-nano".to_string()),
        process_name: Some("codex".to_string()),
        pid: Some(777),
        method: "POST".to_string(),
        path: "/v1/responses".to_string(),
        stream: false,
        system_prompt_preview: None,
        messages_count: 1,
        tools_count: 1,
        request_bytes: 64,
        request_body_preview: Some(r#"{"input":"write winterfell"}"#.to_string()),
        request_body_full: Some(r#"{"input":"write winterfell"}"#.to_string()),
        message_id: Some("msg-winterfell".to_string()),
        status_code: Some(200),
        text_content: Some("the wall holds".to_string()),
        thinking_content: Some("prepare ledger proof".to_string()),
        response_body_full: Some(r#"{"output_text":"the wall holds"}"#.to_string()),
        stop_reason: Some("end_turn".to_string()),
        input_tokens: Some(9),
        output_tokens: Some(4),
        usage_details: BTreeMap::new(),
        duration_ms: 31,
        response_bytes: 33,
        estimated_cost_usd: 0.00001,
        trace_id: Some("trace-winterfell".to_string()),
        credential_ref: Some(
            "credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        ),
        tool_calls: vec![capsem_logger::ToolCallEntry {
            call_index: 0,
            call_id: "tool-winterfell".to_string(),
            tool_name: "Write".to_string(),
            arguments: Some(r#"{"path":"/root/winterfell.md"}"#.to_string()),
            origin: "model".to_string(),
            trace_id: Some("trace-winterfell".to_string()),
        }],
        tool_responses: vec![capsem_logger::ToolResponseEntry {
            call_id: "tool-winterfell".to_string(),
            content_preview: Some("Wrote winterfell.md".to_string()),
            is_error: false,
            trace_id: Some("trace-winterfell".to_string()),
            credential_ref: Some(
                "credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            ),
        }],
    }));
    writer.flush().await;
    tokio::task::spawn_blocking(move || writer.shutdown_blocking())
        .await
        .unwrap();
    let direct_rows = capsem_logger::DbReader::open(&db_path)
        .unwrap()
        .query_raw(
            "SELECT \
             (SELECT COUNT(*) FROM model_calls), \
             (SELECT COUNT(*) FROM net_events), \
             (SELECT COUNT(*) FROM exec_events), \
             (SELECT COUNT(*) FROM security_rule_events)",
        )
        .unwrap();
    assert_eq!(
        direct_rows,
        r#"{"columns":["(SELECT COUNT(*) FROM model_calls)","(SELECT COUNT(*) FROM net_events)","(SELECT COUNT(*) FROM exec_events)","(SELECT COUNT(*) FROM security_rule_events)"],"rows":[[1,1,1,1]]}"#
    );

    hydrate_startup_route_caches(&state).expect("startup hydrates profile route caches");

    let (status, stats) = route_request(app.clone(), axum::http::Method::GET, "/stats", None).await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    assert_eq!(stats["global"]["total_sessions"], 1);
    assert_eq!(stats["sessions"][0]["id"], "winterfell-vm");

    let (status, detail) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/winterfell-vm/stats/detail",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["model_events"][0]["event_id"], "abcdef123453", "{detail}");
    assert_eq!(detail["model_events"][0]["provider"], "openai", "{detail}");
    assert_eq!(detail["model_events"][0]["input_tokens"], 9, "{detail}");
    assert_eq!(detail["tool_events"][0]["call_id"], "tool-winterfell", "{detail}");
    assert_eq!(detail["tool_events"][0]["tool_name"], "Write", "{detail}");
    assert_eq!(
        detail["body_blobs"]["abcdef123453"][0]["body"],
        r#"{"input":"write winterfell"}"#
    );

    let (status, security) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/winterfell-vm/security/latest?limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{security}");
    assert_eq!(security[0]["event_id"], "abcdef123450");
    assert_eq!(security[0]["turn_id"], "trace-winterfell");
    assert_eq!(
        security[0]["credential_ref"],
        "credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );

    let (status, history) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/winterfell-vm/history/counts",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["exec_count"], 1);

    let (status, timeline) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/winterfell-vm/timeline?trace_id=trace-winterfell&layers=exec,net,model,tool&limit=20",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{timeline}");
    let rows = timeline["rows"].as_array().unwrap();
    assert!(
        rows.iter().any(|row| row[1] == "exec" && row[3] == "echo winterfell"),
        "{timeline}"
    );
    assert!(
        rows.iter()
            .any(|row| row[1] == "net" && row[3] == "POST mock.capsem.test/v1/responses"),
        "{timeline}"
    );
    assert!(
        rows.iter()
            .any(|row| row[1] == "model" && row[3] == "openai/gpt-5-nano"),
        "{timeline}"
    );
    assert!(
        rows.iter().any(|row| row[1] == "tool"
            && row[3]
                .as_str()
                .is_some_and(|summary| summary.contains("Write") && summary.contains("tool-winterfell"))),
        "{timeline}"
    );
}
