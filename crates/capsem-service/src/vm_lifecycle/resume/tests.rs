use super::*;
use crate::tests::{make_test_state_with_tempdir, route_request, test_persistent_entry};

#[tokio::test]
async fn resume_failure_does_not_report_an_existing_session_as_missing() {
    let (state, _directory) = make_test_state_with_tempdir();
    let entry = test_persistent_entry("unresumable", state.run_dir.join("missing-session-directory"));
    let id = entry.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(entry.name.clone(), entry);
    let router = crate::build_service_router(state);
    let (status, error) = route_request(
        router.clone(),
        axum::http::Method::POST,
        &format!("/vms/{id}/resume"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{error}");
    assert!(error["error"].as_str().unwrap().contains("resume failed"));
    assert_ne!(error["code"], serde_json::to_value(ErrorCode::VmNotFound).unwrap());
    let (listed, inventory) = route_request(router.clone(), axum::http::Method::GET, "/vms/list", None).await;
    assert_eq!(listed, StatusCode::OK, "{inventory}");
    assert!(inventory["sandboxes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["id"] == id));

    let missing = capsem_crypto::uuid_v4();
    let (status, error) = route_request(
        router,
        axum::http::Method::POST,
        &format!("/vms/{missing}/resume"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
    assert_eq!(error["code"], serde_json::to_value(ErrorCode::VmNotFound).unwrap());
}
