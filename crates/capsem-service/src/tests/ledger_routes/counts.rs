//! Counts that cover the whole session, however long it ran.

use super::*;

/// Security status and history counts cover the whole session. The stats used
/// to share a 2000-match window with the plugin and credential payload scans,
/// and history counts loaded every row to count them.
#[tokio::test]
async fn security_status_and_history_count_past_the_old_window() {
    const MATCHES: usize = 2_100;
    let (state, _dir) = make_test_state_with_tempdir();
    let session_dir = state.run_dir.join("sessions/busy-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let db_path = session_dir.join("session.db");
    tokio::task::spawn_blocking(move || {
        let writer = capsem_logger::DbWriter::open(&db_path, 256).unwrap();
        for row in 0..MATCHES {
            writer.write_blocking(capsem_logger::WriteOp::SecurityRuleEvent(
                capsem_logger::SecurityRuleEvent::new(
                    1_789_000_000_000 + row as i64,
                    format!("{row:012x}"),
                    "http.request",
                    format!("profiles.rules.r{}", row % 3),
                    "{}",
                    r#"{"event_type":"http.request"}"#,
                ),
            ));
            writer.write_blocking(capsem_logger::WriteOp::AuditEvent(
                serde_json::from_value(json!({
                    "timestamp": 1_789_000_000.0 + row as f64, "pid": 1, "ppid": 0, "uid": 0,
                    "exe": "/bin/sh", "argv": "sh",
                }))
                .unwrap(),
            ));
        }
        writer.shutdown_blocking();
    })
    .await
    .unwrap();
    insert_fake_instance_with_session_dir(&state, "busy-vm", std::process::id(), session_dir);
    let app = build_service_router(Arc::clone(&state));

    let (status, security) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/busy-vm/security/status",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{security}");
    assert_eq!(security["total"], MATCHES);
    let by_rule: u64 = security["by_rule"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rule| rule["count"].as_u64().unwrap())
        .sum();
    assert_eq!(by_rule, MATCHES as u64);

    let (status, counts) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/busy-vm/history/counts",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{counts}");
    assert_eq!(counts, json!({"exec_count": 0, "audit_count": MATCHES}));
    let (status, processes) = route_request(app, axum::http::Method::GET, "/vms/busy-vm/history/processes", None).await;
    assert_eq!(status, StatusCode::OK, "{processes}");
    assert_eq!(processes["processes"][0]["exe"], "/bin/sh");
    assert_eq!(processes["processes"][0]["command_count"], MATCHES);
}
