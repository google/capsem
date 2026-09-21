use super::*;
use crate::tests::{insert_fake_instance_with_session_dir, route_request, spawn_fake_process};

fn fixture() -> (Arc<ServiceState>, PathBuf, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::tests::make_test_state();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(session_dir.join("guest/workspace")).unwrap();
    std::fs::write(state.run_dir.join("preview.port"), "19444").unwrap();
    insert_fake_instance_with_session_dir(&state, "box", 1, session_dir);
    let uds_path = state.instances.lock().unwrap()["box"].uds_path.clone();
    (state, uds_path, dir)
}

#[tokio::test]
async fn create_preview_declares_an_owner_resource_without_a_bypass_port() {
    let (state, uds_path, _dir) = fixture();
    let exposure_id = "0199df26-d0f2-74f2-a304-ef67b79d1217";
    let owner = spawn_fake_process(&uds_path, 1, move |message| {
        let ServiceToProcess::DeclarePreview {
            id,
            listener_port: 19444,
            guest_port: 8080,
            target: PublicationTarget::Container,
        } = message
        else {
            panic!("unexpected {message:?}")
        };
        let reply = ProcessToService::PortPublished {
            id: *id,
            publication: Some(capsem_proto::ipc::PublicationInfo {
                id: exposure_id.into(),
                host_port: None,
                guest_port: 8080,
                target: PublicationTarget::Container,
                access: PublicationAccess::HttpPreview,
                router_pid: 7,
            }),
            error: None,
            policy_refused: false,
        };
        Box::pin(async move { Some(reply) })
    });
    let (status, body) = call(
        &state,
        axum::http::Method::POST,
        "/vms/box/exposures",
        Some(json!({"guest_port": 8080, "access": "http_preview"})),
    )
    .await;
    owner.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({
            "id": exposure_id,
            "host_port": null,
            "guest_port": 8080,
            "target": "container",
            "access": "http_preview"
        })
    );
}

#[tokio::test]
async fn preview_session_material_is_bound_to_the_current_owner_generation() {
    let (state, uds_path, _dir) = fixture();
    let exposure_id = "0199df26-d0f2-74f2-a304-ef67b79d1217";
    let owner = spawn_fake_process(&uds_path, 2, move |message| {
        let reply = match message {
            ServiceToProcess::ListPublications { id } => ProcessToService::PublicationList {
                id: *id,
                generation: 42,
                publications: vec![capsem_proto::ipc::PublicationInfo {
                    id: exposure_id.into(),
                    host_port: None,
                    guest_port: 8080,
                    target: PublicationTarget::Container,
                    access: PublicationAccess::HttpPreview,
                    router_pid: 7,
                }],
            },
            ServiceToProcess::CreatePreviewSession {
                id,
                exposure_id: requested,
            } if requested == exposure_id => ProcessToService::PreviewSessionCreated {
                id: *id,
                bootstrap_token: Some("single-use-bootstrap".into()),
                expires_in_seconds: 30,
                error: None,
            },
            other => panic!("unexpected {other:?}"),
        };
        Box::pin(async move { Some(reply) })
    });
    let (status, body) = call(
        &state,
        axum::http::Method::POST,
        &format!("/internal/vms/box/exposures/{exposure_id}/preview-session"),
        Some(json!({})),
    )
    .await;
    owner.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["owner_generation"], "42");
    assert_eq!(body["bootstrap_token"], "single-use-bootstrap");
    assert_eq!(body["exposure"]["id"], exposure_id);
}

async fn call(
    state: &Arc<ServiceState>,
    method: axum::http::Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    route_request(build_service_router(Arc::clone(state)), method, uri, body).await
}

#[tokio::test]
async fn create_relays_the_target_to_the_owner_and_returns_the_bound_port() {
    let (state, uds_path, _dir) = fixture();
    let owner = spawn_fake_process(&uds_path, 1, |message| {
        let reply = match message {
            ServiceToProcess::PublishPort {
                id,
                host_port: 0,
                guest_port: 8080,
                target: PublicationTarget::Vm,
            } => Some(ProcessToService::PortPublished {
                id: *id,
                publication: Some(capsem_proto::ipc::PublicationInfo {
                    id: "49152".into(),
                    host_port: Some(49152),
                    guest_port: 8080,
                    target: PublicationTarget::Vm,
                    access: PublicationAccess::LoopbackTcp,
                    router_pid: 7,
                }),
                error: None,
                policy_refused: false,
            }),
            other => panic!("unexpected owner request: {other:?}"),
        };
        Box::pin(async move { reply })
    });
    let (status, body) = call(
        &state,
        axum::http::Method::POST,
        "/vms/box/exposures",
        Some(json!({"guest_port": 8080, "target": "vm"})),
    )
    .await;
    owner.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"id": "49152", "host_port": 49152, "guest_port": 8080, "target": "vm", "access": "loopback_tcp"})
    );
}

#[tokio::test]
async fn create_refuses_capsem_service_ports_in_the_vm_namespace_without_asking_the_owner() {
    let (state, _uds_path, _dir) = fixture();
    for port in capsem_proto::CAPSEM_GUEST_LOOPBACK_PORTS {
        let (status, body) = call(
            &state,
            axum::http::Method::POST,
            "/vms/box/exposures",
            Some(json!({"guest_port": port, "target": "vm"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "port {port}: {body}");
    }
}

#[tokio::test]
async fn owner_refusal_is_a_conflict_carrying_its_reason() {
    let (state, uds_path, _dir) = fixture();
    let owner = spawn_fake_process(&uds_path, 1, |message| {
        let ServiceToProcess::PublishPort { id, .. } = message else {
            panic!("unexpected {message:?}")
        };
        let reply = ProcessToService::PortPublished {
            id: *id,
            publication: None,
            error: Some("VM publication limit reached".into()),
            policy_refused: false,
        };
        Box::pin(async move { Some(reply) })
    });
    let (status, body) = call(
        &state,
        axum::http::Method::POST,
        "/vms/box/exposures",
        Some(json!({"guest_port": 6379})),
    )
    .await;
    owner.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body.to_string().contains("VM publication limit reached"), "{body}");
}

#[tokio::test]
async fn a_policy_refusal_is_forbidden_carrying_its_reason() {
    let (state, uds_path, _dir) = fixture();
    let owner = spawn_fake_process(&uds_path, 1, |message| {
        let ServiceToProcess::PublishPort { id, .. } = message else {
            panic!("unexpected {message:?}")
        };
        let reply = ProcessToService::PortPublished {
            id: *id,
            publication: None,
            error: Some("exposure of guest port 22 is blocked by policy".into()),
            policy_refused: true,
        };
        Box::pin(async move { Some(reply) })
    });
    let (status, body) = call(
        &state,
        axum::http::Method::POST,
        "/vms/box/exposures",
        Some(json!({"guest_port": 22})),
    )
    .await;
    owner.await.unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.to_string().contains("blocked by policy"), "{body}");
}

#[tokio::test]
async fn list_reports_the_owner_registry_and_its_generation() {
    let (state, uds_path, _dir) = fixture();
    let owner = spawn_fake_process(&uds_path, 1, |message| {
        let ServiceToProcess::ListPublications { id } = message else {
            panic!("unexpected {message:?}")
        };
        let reply = ProcessToService::PublicationList {
            id: *id,
            generation: u64::MAX,
            publications: vec![capsem_proto::ipc::PublicationInfo {
                id: "16379".into(),
                host_port: Some(16379),
                guest_port: 6379,
                target: PublicationTarget::Container,
                access: PublicationAccess::LoopbackTcp,
                router_pid: 9,
            }],
        };
        Box::pin(async move { Some(reply) })
    });
    let (status, body) = call(&state, axum::http::Method::GET, "/vms/box/exposures", None).await;
    owner.await.unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"owner_generation": u64::MAX.to_string(), "exposures": [{"id": "16379", "host_port": 16379, "guest_port": 6379, "target": "container", "access": "loopback_tcp"}]})
    );
    assert!(
        body.to_string().find("router").is_none(),
        "router internals stay private"
    );
}

#[tokio::test]
async fn delete_revokes_by_id_and_unknown_ids_are_not_found() {
    let (state, uds_path, _dir) = fixture();
    let owner = spawn_fake_process(&uds_path, 2, |message| {
        let ServiceToProcess::RevokeExposure { id, exposure_id } = message else {
            panic!("unexpected {message:?}")
        };
        let reply = ProcessToService::ExposureRevoked {
            id: *id,
            revoked: exposure_id == "16379",
            error: None,
        };
        Box::pin(async move { Some(reply) })
    });
    let (status, _) = call(&state, axum::http::Method::DELETE, "/vms/box/exposures/16379", None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&state, axum::http::Method::DELETE, "/vms/box/exposures/18080", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    owner.await.unwrap();
}

#[tokio::test]
async fn exposures_on_a_vm_that_is_not_running_do_not_reach_any_owner() {
    let state = crate::tests::make_test_state();
    let (status, _) = call(&state, axum::http::Method::GET, "/vms/ghost/exposures", None).await;
    assert!(status.is_client_error(), "{status}");
}

/// Finding 11 (google/capsem#222): a leaked preview session could only be cut
/// by deleting the whole exposure. Revoking its sessions keeps the exposure
/// and ends every flow they admitted.
#[tokio::test]
async fn revoking_preview_sessions_asks_the_owner_and_reports_the_count() {
    let (state, uds_path, _dir) = fixture();
    let owner = spawn_fake_process(&uds_path, 2, |message| {
        let ServiceToProcess::RevokePreviewSessions { id, exposure_id } = message else {
            panic!("unexpected {message:?}")
        };
        let reply = if exposure_id == "preview-1" {
            ProcessToService::PreviewSessionsRevoked {
                id: *id,
                revoked: 2,
                error: None,
            }
        } else {
            ProcessToService::PreviewSessionsRevoked {
                id: *id,
                revoked: 0,
                error: Some("preview exposure not found".into()),
            }
        };
        Box::pin(async move { Some(reply) })
    });
    let (status, body) = call(
        &state,
        axum::http::Method::DELETE,
        "/vms/box/exposures/preview-1/preview-session",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revoked"], 2);
    let (status, _) = call(
        &state,
        axum::http::Method::DELETE,
        "/vms/box/exposures/ghost/preview-session",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    owner.await.unwrap();
}
