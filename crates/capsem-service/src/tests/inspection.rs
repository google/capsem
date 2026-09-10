use super::*;

#[test]
fn file_event_enum_matches_the_ledger_writer_vocabulary() {
    use capsem_logger::FileAction;
    for action in [
        FileAction::Created,
        FileAction::Modified,
        FileAction::Deleted,
        FileAction::Restored,
        FileAction::Read,
        FileAction::Imported,
        FileAction::Exported,
    ] {
        let decoded: api::FileEventAction = serde_json::from_value(json!(action.as_str())).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), action.as_str());
    }
}

#[tokio::test]
async fn inspection_filters_fail_explicitly_before_querying_a_session() {
    let app = build_service_router(make_test_state());
    for suffix in [
        "timeline?layers=exec,invented",
        "timeline?layers=",
        "timeline?since=nonsense",
        "timeline?limit=-1",
        "history?layer=net",
        "history?offset=-1",
    ] {
        let (status, _) = route_request(
            app.clone(),
            axum::http::Method::GET,
            &format!("/vms/missing/{suffix}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{suffix}");
    }
}

#[tokio::test]
async fn inspection_never_reports_a_broken_ledger_as_empty_activity() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session = state.run_dir.join("sessions/broken-inspection");
    std::fs::create_dir_all(&session).unwrap();
    let db_path = session.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 8).unwrap();
    writer.shutdown_blocking();
    let connection = rusqlite::Connection::open(&db_path).unwrap();
    connection.execute("DROP TABLE exec_events", []).unwrap();
    connection
        .execute("CREATE TABLE exec_events (id INTEGER PRIMARY KEY)", [])
        .unwrap();
    drop(connection);
    insert_fake_instance_with_session_dir(&state, "broken-inspection", std::process::id(), session);
    let app = build_service_router(state);
    for route in ["history", "timeline", "stats/detail"] {
        let (status, body) = route_request(
            app.clone(),
            axum::http::Method::GET,
            &format!("/vms/broken-inspection/{route}"),
            None,
        )
        .await;
        assert!(status.is_server_error(), "{route}: {status} {body}");
        assert!(body.to_string().contains("exec_events"), "{body}");
    }
}

#[tokio::test]
async fn stats_detail_has_typed_nullable_events_and_captured_bodies() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session = state.run_dir.join("sessions/detail-types");
    std::fs::create_dir_all(&session).unwrap();
    let db_path = session.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 8).unwrap();
    writer.shutdown_blocking();
    let connection = rusqlite::Connection::open(&db_path).unwrap();
    connection
        .execute_batch(
            r#"
        INSERT INTO model_calls(event_id,timestamp,provider,method,path,input_tokens,estimated_cost_usd)
        VALUES('abcdef000001','2026-09-10T00:00:00Z','custom-provider','POST','/model',5000000000,0.125);
        INSERT INTO tool_calls(call_index,call_id,tool_name,origin)
        VALUES(0,'tool-1','custom-tool','native');
        INSERT INTO net_events(timestamp,domain,decision,bytes_received)
        VALUES('2026-09-10T00:00:00Z','example.test','denied',6000000000);
        INSERT INTO dns_events(timestamp,qname,qtype,qclass,rcode,decision,source_proto)
        VALUES('2026-09-10T00:00:00Z','example.test',28,1,0,'redirected','udp');
        INSERT INTO fs_events(timestamp,action,path,size)
        VALUES('2026-09-10T00:00:00Z','import','/workspace/large',7000000000);
        INSERT INTO exec_events(timestamp,exec_id,command,exit_code)
        VALUES('2026-09-10T00:00:00Z',4294967296,'false',-9);
        INSERT INTO audit_events(timestamp,pid,ppid,uid,exe,argv,exit_code)
        VALUES('2026-09-10T00:00:00Z',42,1,4294967295,'/bin/false','["false"]',1);
        INSERT INTO substitution_events(timestamp,material_class,source,event_type,algorithm,substitution_ref,outcome)
        VALUES('2026-09-10T00:00:00Z','credential','http.body.response.custom','http.response','blake3',
            'credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','captured');
        INSERT INTO event_body_blobs(event_id,event_type,source_table,direction,original_bytes,
            stored_bytes,truncated,body_hash,body,created_at)
        VALUES('abcdef000001','model.call','model_calls','response',100,4,1,
            'blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
            X'74657374','2026-09-10T00:00:00Z');
    "#,
        )
        .unwrap();
    drop(connection);
    insert_fake_instance_with_session_dir(&state, "detail-types", std::process::id(), session);
    let app = build_service_router(Arc::clone(&state));
    let (status, value) = route_request(app, axum::http::Method::GET, "/vms/detail-types/stats/detail", None).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let detail: api::VmStatsDetailResponse = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(detail.model_stats[0].input_tokens, 5_000_000_000);
    assert_eq!(detail.model_stats[0].estimated_cost_usd, 0.125);
    assert_eq!(detail.model_stats[0].model, "unknown");
    assert!(detail.model_events[0].model.is_none());
    assert!(detail.model_events[0].output_tokens.is_none());
    assert_eq!(detail.tool_events[0].source, api::ToolOrigin::Native);
    assert!(!detail.tool_events[0].model_parent_missing);
    assert!(detail.tool_events[0].timestamp.is_none());
    assert_eq!(detail.http_events[0].decision, api::NetworkDecision::Denied);
    assert_eq!(detail.http_events[0].bytes_received, Some(6_000_000_000));
    assert!(detail.http_events[0].status_code.is_none());
    assert_eq!(detail.dns_events[0].decision, api::NetworkDecision::Redirected);
    assert_eq!(detail.dns_events[0].source_proto, Some(api::NetworkProtocol::Udp));
    assert_eq!(detail.file_events[0].action, api::FileEventAction::Imported);
    assert_eq!(detail.file_events[0].size, Some(7_000_000_000));
    assert_eq!(detail.process_events[0].exec_id, 4_294_967_296);
    assert_eq!(detail.process_events[0].exit_code, Some(-9));
    assert_eq!(detail.audit_events[0].uid, u32::MAX);
    assert_eq!(detail.audit_events[0].argv, r#"["false"]"#);
    assert_eq!(detail.credential_events[0].verb, api::CredentialOutcome::Captured);
    assert_eq!(
        detail.credential_events[0].event_type,
        Some(api::CredentialEventType::HttpResponse)
    );
    assert_eq!(detail.credential_events[0].source, "http.body.response.custom");
    let body = &detail.body_blobs["abcdef000001"][0];
    assert_eq!(body.direction, api::BodyDirection::Response);
    assert!(body.truncated);
    assert_eq!(body.body, "test");
    assert_eq!(body.stored_bytes, 4);
    assert_eq!(value["tool_events"][0]["model_parent_missing"], false);
    assert_eq!(value["body_blobs"]["abcdef000001"][0]["truncated"], true);
    assert!(
        state.stats_detail_response_cache.lock().unwrap().is_empty(),
        "stats detail must use DB-owned reads without a service projection cache"
    );
}

#[tokio::test]
async fn stats_detail_rejects_invalid_event_categories_and_negative_counts() {
    for sql in [
        "INSERT INTO net_events(timestamp,domain,decision) VALUES('now','example.test','invented')",
        "INSERT INTO net_events(timestamp,domain,decision,bytes_sent) VALUES('now','example.test','allowed',-1)",
        "INSERT INTO fs_events(timestamp,action,path) VALUES('now','invented','/workspace/file')",
        "INSERT INTO dns_events(timestamp,qname,qtype,qclass,rcode,decision,source_proto)
            VALUES('now','example.test',1,1,0,'allowed','invented')",
    ] {
        let (state, _dir) = make_test_state_with_tempdir();
        let session = state.run_dir.join("sessions/invalid-detail");
        std::fs::create_dir_all(&session).unwrap();
        let db_path = session.join("session.db");
        let writer = capsem_logger::DbWriter::open(&db_path, 8).unwrap();
        writer.shutdown_blocking();
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        connection.execute_batch(sql).unwrap();
        drop(connection);
        insert_fake_instance_with_session_dir(&state, "invalid-detail", std::process::id(), session);
        let (status, body) = route_request(
            build_service_router(state),
            axum::http::Method::GET,
            "/vms/invalid-detail/stats/detail",
            None,
        )
        .await;
        assert!(status.is_server_error(), "{sql}: {status} {body}");
        assert!(body.to_string().contains("stats_detail"), "{body}");
    }
}
