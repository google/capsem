use super::*;
use crate::client::tests::fake_service::FakeService;
use serde_json::json;

#[tokio::test]
async fn ordinary_requests_use_gateway_auth_and_never_the_service_socket() {
    let service = FakeService::start();
    service.route("GET", "/vms/list", 200, json!({"sandboxes": []}));
    let response: ApiResponse<ListResponse> = service.client.get("/vms/list").await.unwrap();
    assert!(response.into_result().unwrap().sandboxes.is_empty());
    let recorded = service.recorded();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].via_gateway);
}

#[tokio::test]
async fn missing_token_sends_no_vm_request_on_either_transport() {
    let service = FakeService::start();
    std::fs::remove_file(service.socket.parent().unwrap().join("gateway.token")).unwrap();
    let error = service.client.get::<serde_json::Value>("/vms/list").await.unwrap_err();
    assert!(format!("{error:#}").contains("gateway token unavailable"));
    assert!(service.recorded().is_empty());
}

#[tokio::test]
async fn rejected_token_never_falls_back_or_replays_a_mutation() {
    let service = FakeService::start();
    std::fs::write(service.socket.parent().unwrap().join("gateway.token"), "invalid-token").unwrap();
    service.route("POST", "/vms/create", 200, json!({"id": "must-not-create"}));
    let error = service
        .client
        .post::<_, serde_json::Value>("/vms/create", json!({}))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("gateway HTTP 401"));
    assert!(!error.to_string().contains("invalid-token"));
    let recorded = service.recorded();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].via_gateway);
}

#[tokio::test]
async fn binary_files_and_encoded_queries_survive_the_sdk_path() {
    let service = FakeService::start();
    service.route("POST", "/vms/vm-1/files/content", 200, json!({"ok": true}));
    let bytes = vec![0, 255, 13, 10];
    service
        .client
        .request_bytes(
            "POST",
            "/vms/vm-1/files/content?path=%2Fspace%20here%2F%25.bin",
            Some(bytes.clone()),
            Some("application/octet-stream"),
        )
        .await
        .unwrap();
    let recorded = service.recorded();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].via_gateway);
    assert_eq!(recorded[0].body, bytes);
    assert_eq!(recorded[0].path, "/vms/vm-1/files/content?path=%2Fspace+here%2F%25.bin");
}

#[tokio::test]
async fn direct_administration_cannot_issue_vm_requests() {
    let service = FakeService::start();
    let admin = ServiceAdminClient::new(service.socket.clone(), false);
    let error = admin.get::<serde_json::Value>("/vms/list").await.unwrap_err();
    assert!(error
        .to_string()
        .contains("restricted to local status/asset administration"));
    assert!(service.recorded().is_empty());
    service.route("GET", "/system/status", 200, json!({"ready": true}));
    assert_eq!(
        admin.get::<serde_json::Value>("/system/status").await.unwrap(),
        json!({"ready": true})
    );
    assert!(!service.recorded()[0].via_gateway);
}
