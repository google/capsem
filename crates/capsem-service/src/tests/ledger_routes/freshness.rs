use super::*;

/// The service must not infer "unchanged" from `session.db` metadata: a
/// commit that lands only in the WAL leaves the main file's size and mtime
/// exactly as they were, so the writer-owned checkpoint cadence decided when
/// security routes noticed new rows. Routes read through the logger handle,
/// whose external reader re-syncs on SQLite `data_version` for every query.
#[tokio::test]
async fn security_routes_observe_wal_commits_after_an_empty_read() {
    let state = make_test_state();
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session");
    std::fs::create_dir_all(&session).unwrap();
    insert_fake_instance_with_session_dir(&state, "wal-vm", std::process::id(), session.clone());
    let path = session.join("session.db");
    let writer = capsem_logger::DbWriter::open(&path, 16).unwrap();
    let app = build_service_router(state);
    let routes = ["security/latest", "detection/latest", "security/status"];
    for route in routes {
        let value = read(&app, route).await;
        if route.ends_with("status") {
            assert_eq!(value["total"], 0);
        } else {
            assert_eq!(value.as_array().unwrap().len(), 0);
        }
    }
    let before = main_file_fingerprint(&path);
    writer
        .write_checked(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_123_456,
                "abcdef123456",
                "network.connect",
                "profiles.rules.expose",
                "{}",
                "{}",
            )
            .with_rule_action(capsem_logger::SecurityRuleAction::Block)
            .with_detection_level(capsem_logger::SecurityDetectionLevel::High),
        ))
        .await
        .unwrap();
    writer.flush_checked().await.unwrap();
    assert_eq!(main_file_fingerprint(&path), before, "test requires a WAL-only commit");
    for route in routes {
        let value = read(&app, route).await;
        if route.ends_with("status") {
            assert_eq!(value["total"], 1);
        } else {
            assert_eq!(value.as_array().unwrap().len(), 1, "stale {route}");
            assert_eq!(value[0]["event_id"], "abcdef123456");
        }
    }
    writer.shutdown_blocking();
}

/// Size and mtime of the main database file, the two facts a WAL-only
/// commit leaves untouched.
fn main_file_fingerprint(path: &StdPath) -> (u64, std::time::SystemTime) {
    let metadata = std::fs::metadata(path).unwrap();
    (metadata.len(), metadata.modified().unwrap())
}

async fn read(app: &axum::Router, route: &str) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/vms/wal-vm/{route}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
