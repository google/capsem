use super::*;

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
    for route in ["history", "timeline"] {
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
