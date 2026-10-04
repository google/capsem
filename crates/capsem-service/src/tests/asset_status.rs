use super::*;

#[tokio::test]
async fn ensure_route_starts_one_reconciliation_and_settles_ready() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    install_test_runtime_assets(&state);
    let app = build_service_router(Arc::clone(&state));

    let (status, ensured) = route_request(app.clone(), axum::http::Method::POST, "/assets/ensure", None).await;

    assert_eq!(status, StatusCode::OK, "{ensured}");
    assert_eq!(ensured["started"], true);
    let settled = asset_wait::wait_for_assets(&app).await;
    assert_eq!(settled["ready"], true, "{settled}");
    assert_eq!(settled["downloaded"], 0, "a verified set downloads nothing");
}

#[test]
fn a_second_ensure_while_one_runs_starts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    state.asset_reconcile_inflight.store(true, Ordering::Release);

    assert!(!asset_background::start_asset_ensure(&state));
    assert!(state.asset_reconcile_inflight.load(Ordering::Acquire));
}

#[cfg(unix)]
#[tokio::test]
async fn does_not_read_asset_contents_on_hot_path() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    install_test_runtime_assets(&state);
    let rootfs = state.runtime_asset_set().expect("runtime asset set").resolved.rootfs;
    let app = build_service_router(state);

    std::fs::set_permissions(&rootfs, std::fs::Permissions::from_mode(0o000)).unwrap();
    let (status, hot_status) = route_request(app, axum::http::Method::GET, "/assets/status", None).await;
    std::fs::set_permissions(&rootfs, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(status, StatusCode::OK, "{hot_status}");
    assert_eq!(
        hot_status["ready"], true,
        "asset status is a polled readiness route and must not hash or read asset contents"
    );
}
