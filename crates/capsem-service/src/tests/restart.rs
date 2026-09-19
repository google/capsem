use super::*;
use crate::router_runtime::restart::accept_restart;

#[tokio::test]
async fn restart_acknowledges_acceptance_and_requires_new_gateway_credentials() {
    let state = make_test_state();
    let (status, Json(response)) = accept_restart(&state, Some(api::ServiceManager::Launchd)).unwrap();
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(response.status, api::RestartStatus::Accepted);
    assert_eq!(response.manager, api::ServiceManager::Launchd);
    assert_eq!(response.authentication, api::RestartAuthentication::NewTokenRequired);
    tokio::time::timeout(std::time::Duration::from_secs(1), state.update_restart.notified())
        .await
        .unwrap();
    assert!(state.lifecycle.admit().is_err());
    assert_eq!(
        accept_restart(&state, Some(api::ServiceManager::Launchd))
            .unwrap_err()
            .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn restart_refuses_unmanaged_service_or_active_launch_without_notifying_shutdown() {
    let state = make_test_state();
    assert_eq!(
        accept_restart(&state, None).unwrap_err().0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let launch = state.lifecycle.admit().unwrap();
    assert_eq!(
        accept_restart(&state, Some(api::ServiceManager::Systemd))
            .unwrap_err()
            .0,
        StatusCode::CONFLICT
    );
    drop(launch);
    assert!(state.lifecycle.admit().is_ok());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), state.update_restart.notified())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn restart_refuses_active_vms_without_changing_their_registry() {
    let state = make_test_state();
    insert_fake_instance_with_session_dir(
        &state,
        "active",
        std::process::id(),
        state.run_dir.join("sessions/active"),
    );
    let before = state.instances.lock().unwrap()["active"].pid;
    assert_eq!(
        accept_restart(&state, Some(api::ServiceManager::Systemd))
            .unwrap_err()
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(state.instances.lock().unwrap()["active"].pid, before);
    assert!(state.lifecycle.admit().is_ok());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), state.update_restart.notified())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn restart_route_refuses_unmanaged_process_and_concurrent_update() {
    let state = make_test_state();
    let app = build_service_router(state.clone());
    let (status, _) = route_request(app.clone(), axum::http::Method::POST, "/restart", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let _update = state.update_lock.lock().await;
    let (status, _) = route_request(app, axum::http::Method::POST, "/restart", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn accepted_restart_rejects_lifecycle_mutations_with_conflict() {
    let state = make_test_state();
    let _ = accept_restart(&state, Some(api::ServiceManager::Systemd)).unwrap();
    let app = build_service_router(state);
    for (path, body) in [
        ("/vms/create", Some(json!({"profile_id": "code"}))),
        ("/run", Some(json!({"profile_id": "code", "command": "true"}))),
        ("/vms/missing/start", None),
        ("/vms/missing/resume", None),
        ("/vms/missing/fork", Some(json!({"name": "fork"}))),
    ] {
        let (status, _) = route_request(app.clone(), axum::http::Method::POST, path, body).await;
        assert_eq!(status, StatusCode::CONFLICT, "{path}");
    }
}
