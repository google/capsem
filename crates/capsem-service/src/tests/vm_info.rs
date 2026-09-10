use super::*;

#[tokio::test]
async fn vm_list_stays_lightweight_and_info_reads_file_activity_through_db_owner() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions/list-hot-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let file_event = capsem_logger::FileEvent {
        event_id: Some("abcdef123456".into()),
        timestamp: std::time::SystemTime::now(),
        action: capsem_logger::FileAction::Created,
        path: "/root/list-hot-proof.txt".into(),
        size: Some(12),
        trace_id: Some("tracelisthot".into()),
        credential_ref: None,
    };
    let db_path = session_dir.join("session.db");
    tokio::task::spawn_blocking(move || {
        let writer = capsem_logger::DbWriter::open(&db_path, 8).unwrap();
        writer.write_blocking(capsem_logger::WriteOp::FileEvent(file_event));
        writer.shutdown_blocking();
    })
    .await
    .unwrap();
    insert_fake_instance_with_session_dir(&state, "list-hot-vm", 4242, session_dir);

    let list: ListResponse = decode_response_json(handle_list(State(Arc::clone(&state))).await).await;
    let listed = list
        .sandboxes
        .iter()
        .find(|vm| vm.id == "list-hot-vm")
        .expect("running VM listed");
    assert!(
        listed.total_input_tokens.is_none(),
        "/vms/list is a hot route and must not read session.db telemetry"
    );
    assert!(listed.model_call_count.is_none());

    let Json(info) = handle_info(State(state), Path("list-hot-vm".into()))
        .await
        .expect("detail route includes typed activity summaries");
    let body = serde_json::to_value(&info).unwrap();
    assert_eq!(body["files"]["total_events"], 1);
    assert_eq!(body["files"]["actions"][0]["action"], "created");
    assert!(
        info.total_file_events.is_none(),
        "/vms/{{id}}/info must not inline raw telemetry SQL; use ledger DB APIs"
    );
    assert!(info.model_call_count.is_none());
}

#[tokio::test]
async fn status_reports_db_readiness() {
    let (state, _dir) = make_test_state_with_tempdir();
    let app = build_service_router(Arc::clone(&state));
    let session_dir = state.run_dir.join("sessions").join("status-db-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.shutdown_blocking();
    insert_fake_instance_with_session_dir(&state, "status-db-vm", std::process::id(), session_dir);

    let (status, body) = route_request(app, axum::http::Method::GET, "/vms/status-db-vm/info", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["session_db"]["ready"], true,
        "session status must expose DB readiness from the service-owned DbHandle: {body}"
    );
    assert!(
        body["session_db"].get("error").is_none(),
        "ready session DB status must not invent an error: {body}"
    );
}

#[tokio::test]
async fn info_route_reports_nested_ai_activity_through_db_owner() {
    let (state, _dir) = make_test_state_with_tempdir();
    let app = build_service_router(Arc::clone(&state));
    let session_dir = state.run_dir.join("sessions").join("toolbar-stats-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let mut usage_details = BTreeMap::new();
    usage_details.insert("thinking".to_string(), 3);
    usage_details.insert("reasoning".to_string(), 4);
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.write_blocking(capsem_logger::WriteOp::ModelCall(capsem_logger::ModelCall {
        event_id: Some("abc123abc123".to_string()),
        timestamp: std::time::SystemTime::now(),
        provider: "openai".to_string(),
        protocol: Some("openai".to_string()),
        model: Some("gpt-5-demo".to_string()),
        process_name: Some("codex".to_string()),
        pid: Some(42),
        method: "POST".to_string(),
        path: "/v1/responses".to_string(),
        stream: false,
        system_prompt_preview: None,
        messages_count: 1,
        tools_count: 1,
        request_bytes: 32,
        request_body_preview: None,
        request_body_full: None,
        message_id: Some("msg-toolbar".to_string()),
        status_code: Some(200),
        text_content: Some("done".to_string()),
        thinking_content: Some("checking stats".to_string()),
        response_body_full: None,
        stop_reason: Some("end_turn".to_string()),
        input_tokens: Some(12),
        output_tokens: Some(7),
        usage_details,
        duration_ms: 25,
        response_bytes: 64,
        estimated_cost_usd: 0.001,
        trace_id: Some("trace-toolbar".to_string()),
        credential_ref: None,
        tool_calls: vec![capsem_logger::ToolCallEntry {
            call_index: 0,
            call_id: "tool-toolbar".to_string(),
            tool_name: "Read".to_string(),
            arguments: Some(r#"{"path":"/root/demo.md"}"#.to_string()),
            origin: "model".to_string(),
            trace_id: Some("trace-toolbar".to_string()),
        }],
        tool_responses: vec![],
    }));
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
    writer.flush_checked().await.unwrap();
    writer.shutdown_blocking();
    insert_fake_instance_with_session_dir(&state, "toolbar-stats-vm", std::process::id(), session_dir);

    let (status, body) = route_request(app, axum::http::Method::GET, "/vms/toolbar-stats-vm/info", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["session_db"]["ready"], true);
    assert_eq!(body["ai"]["model_call_count"], 1);
    assert_eq!(body["ai"]["total_input_tokens"], 12);
    assert_eq!(body["ai"]["total_output_tokens"], 7);
    assert_eq!(body["ai"]["total_thinking_tokens"], 3);
    assert_eq!(body["ai"]["models"][0]["model"], "gpt-5-demo");
    assert_eq!(body["ai"]["models"][0]["provider"], "openai");
    assert_eq!(body["network"]["total_requests"], 1);
    assert_eq!(body["network"]["denied_requests"], 1);
    assert_eq!(body["network"]["bytes_sent"], 3);
    assert_eq!(body["files"]["total_events"], 0);
    assert_eq!(body["ai"]["mcp"][0]["server_name"], "local");
    assert_eq!(body["ai"]["mcp"][0]["tool_name"], "fetch_http");
    assert_eq!(body["ai"]["mcp"][0]["call_count"], 1);
    assert_eq!(body["ai"]["mcp"][0]["bytes_sent"], 33);
    assert_eq!(body["ai"]["mcp"][0]["duration_ms"], 17);
    let typed: capsem_api::SandboxInfo = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(typed.profile_id, "code");
    assert!(typed.ai.is_some());
    assert!(
        body.get("model_call_count").is_none(),
        "/vms/{{id}}/info must not inline ledger counters; use /vms/{{id}}/stats/detail"
    );
    assert!(body.get("total_input_tokens").is_none());
    assert!(body.get("total_thinking_tokens").is_none());
    assert!(body.get("total_output_tokens").is_none());
    assert!(body.get("total_tool_calls").is_none());
    assert!(body.get("total_estimated_cost").is_none());
}

#[tokio::test]
async fn broken_session_db_schema_is_explicit_error_for_session_status() {
    let (state, _dir) = make_test_state_with_tempdir();
    let app = build_service_router(Arc::clone(&state));
    let session_dir = state.run_dir.join("sessions").join("status-broken-db-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer.shutdown_blocking();
    let conn = rusqlite::Connection::open(session_dir.join("session.db")).unwrap();
    conn.execute("DROP TABLE net_events", []).unwrap();
    conn.execute("CREATE TABLE net_events (id INTEGER PRIMARY KEY)", [])
        .unwrap();
    drop(conn);
    let entry = test_persistent_entry("status-broken-db-vm", session_dir);
    let vm_id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("status-broken-db-vm".to_string(), entry);
    state.hydrate_session_db_handles();
    let handle = state
        .session_db_handle(&vm_id)
        .expect("startup hydration installs the handle so routes surface the schema error explicitly");
    assert!(
        handle.ready().await.is_err(),
        "a malformed session schema must fail readiness instead of being treated as ready"
    );

    let (status, body) = route_request(app, axum::http::Method::GET, &format!("/vms/{vm_id}/info"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("ai").is_none());
    assert!(body.get("network").is_none());
    assert!(body.get("files").is_none());
    assert_eq!(
        body["session_db"]["ready"], false,
        "broken session schemas must be visible in status instead of being treated as ready: {body}"
    );
    let error = body["session_db"]["error"]
        .as_str()
        .expect("broken DB status must carry the explicit DB readiness error");
    assert!(
        error.contains("not ready") || error.contains("missing required column") || error.contains("no such column"),
        "broken DB status must expose the schema failure, got: {error}"
    );
}

#[tokio::test]
async fn info_rejects_unknown_file_activity_instead_of_silently_dropping_it() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session = state.run_dir.join("sessions/invalid-action-vm");
    std::fs::create_dir_all(&session).unwrap();
    let db_path = session.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 8).unwrap();
    writer.write_blocking(capsem_logger::WriteOp::FileEvent(capsem_logger::FileEvent {
        event_id: Some("abcdef123456".into()),
        timestamp: std::time::SystemTime::now(),
        action: capsem_logger::FileAction::Created,
        path: "/root/example.txt".into(),
        size: Some(1),
        trace_id: None,
        credential_ref: None,
    }));
    writer.shutdown_blocking();
    let connection = rusqlite::Connection::open(&db_path).unwrap();
    connection
        .execute("UPDATE fs_events SET action = 'invented_action'", [])
        .unwrap();
    drop(connection);
    insert_fake_instance_with_session_dir(&state, "invalid-action-vm", std::process::id(), session);
    let (status, body) = route_request(
        build_service_router(state),
        axum::http::Method::GET,
        "/vms/invalid-action-vm/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(body.to_string().contains("invented_action"), "{body}");
}
