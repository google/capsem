//! Routes read a session through the DB handle, never around it.
//!
//! The service registers a session handle lazily, after capsem-process creates
//! `session.db`, and every route answer -- rows, stats, body metadata -- comes
//! back through that handle.

use super::*;

#[tokio::test]
async fn db_boundary_route_contract_db_handle_route_rewire() {
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("db-handle-route-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "db-handle-route-vm", std::process::id(), session_dir.clone());

    assert!(
        state.session_db_handle("db-handle-route-vm").is_none(),
        "session handles are registered lazily after capsem-process creates session.db"
    );
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_111_000_000,
                "abcdef123456",
                "http.request",
                "profiles.rules.default_http",
                r#"{"name":"default_http"}"#,
                r#"{"event_type":"http.request"}"#,
            )
            .with_rule_action(capsem_logger::SecurityRuleAction::Allow),
        ))
        .await;
    writer.shutdown_blocking();

    let (status, stats_detail) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/db-handle-route-vm/stats/detail",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stats_detail}");
    assert_eq!(stats_detail["model_stats"], json!([]));
    // Metadata only, and the rule match's forensic payload is one of the
    // bodies now: the route says where it is and how big it is, and the bytes
    // stay in the archive until someone asks for them.
    let payload_blob = &stats_detail["body_blobs"]["abcdef123456"][0];
    assert_eq!(payload_blob["source_table"], json!("security_rule_events"));
    assert_eq!(payload_blob["direction"], json!("payload"));
    assert_eq!(payload_blob["content_type"], json!("application/json"));
    assert_eq!(
        payload_blob["original_bytes"],
        json!(r#"{"event_type":"http.request"}"#.len())
    );
    assert!(
        payload_blob.get("body").is_none(),
        "the stats detail route must not carry body bytes: {payload_blob}"
    );

    let (status, security_status) = route_request(
        app,
        axum::http::Method::GET,
        "/vms/db-handle-route-vm/security/status",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{security_status}");
    assert_eq!(security_status["total"], 1);
    assert_eq!(security_status["by_action"][0]["rule_action"], "allow");
    assert!(
        state.session_db_handle("db-handle-route-vm").is_some(),
        "first ledger route registers the external DB reader once session.db exists"
    );
}
