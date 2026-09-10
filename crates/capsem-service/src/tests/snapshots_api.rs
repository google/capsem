use super::*;

#[tokio::test]
async fn changes_route_requires_one_checkpoint_and_returns_typed_paginated_differences() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session = state.run_dir.join("persistent/snapshot-test");
    let workspace = session.join("guest/workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(session.join("guest/system")).unwrap();
    std::fs::write(workspace.join("modified"), "old").unwrap();
    std::fs::write(workspace.join("deleted"), "gone").unwrap();
    let mut scheduler = capsem_core::auto_snapshot::AutoSnapshotScheduler::new(
        session.clone(),
        10,
        12,
        std::time::Duration::from_secs(300),
    );
    scheduler.take_snapshot().unwrap();
    std::fs::write(workspace.join("modified"), "new").unwrap();
    std::fs::remove_file(workspace.join("deleted")).unwrap();
    std::fs::write(workspace.join("created"), "new").unwrap();
    scheduler.take_snapshot().unwrap();
    let entry = test_persistent_entry("snapshot-test", session);
    let id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(entry.name.clone(), entry);
    let app = build_service_router(state);
    let (status, body) = route_request(
        app.clone(),
        axum::http::Method::GET,
        &format!("/vms/{id}/changes?checkpoint=cp-0&limit=1&offset=1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let changes: capsem_api::ChangesResponse = serde_json::from_value(body).unwrap();
    assert_eq!(changes.total, 3);
    assert!(changes.has_more);
    assert_eq!(changes.changes[0].kind, capsem_api::FileChangeKind::Deleted);
    assert_eq!(changes.changes[0].path, "deleted");
    let (status, body) = route_request(
        app.clone(),
        axum::http::Method::GET,
        &format!("/vms/{id}/snapshots/list"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let snapshots: capsem_api::SnapshotsList = serde_json::from_value(body).unwrap();
    assert_eq!(snapshots.snapshots[0].origin, capsem_api::SnapshotOrigin::Auto);
    assert_eq!(snapshots.total, 2);
    let (status, body) = route_request(
        app.clone(),
        axum::http::Method::GET,
        &format!("/vms/{id}/changes?checkpoint=cp-1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 0);
    assert_eq!(body["changes"], json!([]));
    let (status, body) = route_request(
        app.clone(),
        axum::http::Method::GET,
        &format!("/vms/{id}/changes?checkpoint=cp-0&offset=18446744073709551615"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["changes"], json!([]));
    assert_eq!(body["has_more"], false);
    for (query, expected) in [
        ("", StatusCode::BAD_REQUEST),
        ("?checkpoint=../secret", StatusCode::BAD_REQUEST),
        ("?checkpoint=cp-99", StatusCode::NOT_FOUND),
        ("?checkpoint=cp-0&limit=-1", StatusCode::BAD_REQUEST),
    ] {
        let (status, _) = route_request(
            app.clone(),
            axum::http::Method::GET,
            &format!("/vms/{id}/changes{query}"),
            None,
        )
        .await;
        assert_eq!(status, expected, "{query}");
    }
}
