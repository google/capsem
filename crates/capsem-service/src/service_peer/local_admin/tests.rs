use super::*;
use capsem_proto::gateway_grant::auth::GatewayAuthSecret;

#[tokio::test]
async fn doctor_administration_requires_live_operator_and_pinned_cli_peer() {
    let state = crate::tests::make_test_state();
    let mut epoch = state.caller_authority.begin();
    epoch.register(GatewayAuthSecret::from_bytes([b'A'; 64])).unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("authorization", format!("Bearer {}", "A".repeat(64)).parse().unwrap());
    let peer = capsem_foundation::unix::peer::PeerIdentity {
        pid: capsem_foundation::unix::process::ProcessId::try_from(std::process::id()).unwrap(),
        uid: capsem_foundation::unix::process::current_uid(),
    };
    let executable = std::env::current_exe().unwrap();
    assert!(admit_expected(&state, Some(peer), &headers, executable.clone())
        .await
        .is_some());
    assert!(admit_expected(&state, None, &headers, executable.clone())
        .await
        .is_none());
    assert!(
        admit_expected(&state, Some(peer), &headers, executable.with_file_name("other-client"))
            .await
            .is_none()
    );
    assert!(
        admit_expected(&state, Some(peer), &Default::default(), executable.clone())
            .await
            .is_none()
    );
    headers.insert("authorization", format!("Bearer {}", "B".repeat(64)).parse().unwrap());
    assert!(admit_expected(&state, Some(peer), &headers, executable.clone())
        .await
        .is_none());
    headers.insert("authorization", format!("Bearer {}", "A".repeat(64)).parse().unwrap());
    use capsem_core::{
        agent_access::Reservation,
        managed_sessions::{Capability, LeaseClock},
    };
    let id = capsem_crypto::uuid_v4();
    let scope = capsem_api::AgentAccessScope::new(vec![capsem_api::AgentOperation::Execute], vec![], vec![]).unwrap();
    let Reservation::New(admission) = state
        .caller_authority
        .agents
        .reserve(
            id,
            &Capability::from_bytes([0x11; 32]),
            scope,
            std::time::Duration::from_secs(60),
            || LeaseClock::now().ok(),
        )
        .unwrap()
    else {
        panic!("new admission")
    };
    admission.commit(|| LeaseClock::now().ok()).unwrap();
    headers.insert(
        "authorization",
        format!("Bearer {id}.{}", "11".repeat(32)).parse().unwrap(),
    );
    assert!(admit_expected(&state, Some(peer), &headers, executable.clone())
        .await
        .is_none());
    headers.insert("authorization", format!("Bearer {}", "A".repeat(64)).parse().unwrap());
    drop(epoch);
    assert!(admit_expected(&state, Some(peer), &headers, executable).await.is_none());
}

#[tokio::test]
async fn public_doctor_cleanup_denies_untrusted_clients_and_registered_vm_owners() {
    use tower::ServiceExt;
    let state = crate::tests::make_test_state();
    let mut epoch = state.caller_authority.begin();
    epoch.register(GatewayAuthSecret::from_bytes([b'A'; 64])).unwrap();
    let peer = super::super::ServicePeer(Some(capsem_foundation::unix::peer::PeerIdentity {
        pid: capsem_foundation::unix::process::ProcessId::try_from(std::process::id()).unwrap(),
        uid: capsem_foundation::unix::process::current_uid(),
    }));
    let app = crate::router_runtime::build_service_router(state.clone());
    for bearer in [None, Some("forged".to_owned()), Some("A".repeat(64))] {
        let mut request =
            axum::http::Request::post("/vms/doctor-test/preserve-failure").extension(axum::extract::ConnectInfo(peer));
        if let Some(bearer) = bearer {
            request = request.header("authorization", format!("Bearer {bearer}"));
        }
        assert_eq!(
            app.clone()
                .oneshot(request.body(axum::body::Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            axum::http::StatusCode::FORBIDDEN
        );
    }
    crate::tests::insert_fake_instance_with_session_dir(
        &state,
        "doctor-test",
        std::process::id(),
        state.run_dir.join("sessions/doctor-test"),
    );
    let owner = {
        let instances = state.instances.lock().unwrap();
        crate::owner_connection::OwnerConnection::capture(instances.get("doctor-test").unwrap()).unwrap()
    };
    let result = crate::session_cleanup::handle_preserve_failure(
        AuthenticatedActor::vm_owner(&owner),
        axum::extract::State(state.clone()),
        axum::extract::Path("doctor-test".into()),
    )
    .await;
    assert_eq!(result.unwrap_err().status, axum::http::StatusCode::FORBIDDEN);
    let request = axum::http::Request::post("/vms/doctor-test/preserve-failure")
        .extension(axum::extract::ConnectInfo(peer))
        .header("authorization", format!("Bearer {}", "A".repeat(64)))
        .body(axum::body::Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        axum::http::StatusCode::FORBIDDEN
    );
    assert!(state.instances.lock().unwrap().contains_key("doctor-test"));
}
