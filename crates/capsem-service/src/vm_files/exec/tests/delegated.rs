use super::*;
use crate::tests::OperatorRequestExt;
use capsem_api::{AgentAccessScope, AgentOperation, AgentSessionScope};
use capsem_core::{
    agent_access::{Grant, Reservation},
    managed_sessions::{Capability, LeaseClock},
};

fn issue(
    state: &ServiceState,
    session: &str,
    generation: uuid::Uuid,
    operation: AgentOperation,
    duration: std::time::Duration,
) -> (uuid::Uuid, Arc<Grant>) {
    let access = capsem_crypto::uuid_v4();
    let scope = AgentAccessScope::new(
        vec![operation],
        vec![],
        vec![AgentSessionScope::new(session.into(), generation.to_string()).unwrap()],
    )
    .unwrap();
    let Reservation::New(admission) = state
        .caller_authority
        .agents
        .reserve(access, &Capability::from_bytes([0x11; 32]), scope, duration, || {
            LeaseClock::now().ok()
        })
        .unwrap()
    else {
        panic!("new grant");
    };
    (access, admission.commit(|| LeaseClock::now().ok()).unwrap())
}

fn request(session: &str, access: uuid::Uuid) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::post(format!("/vms/{session}/exec"))
        .header("authorization", format!("Bearer {access}.{}", "11".repeat(32)))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(r#"{"command":"scoped-exec","timeout_secs":1}"#))
        .unwrap()
}

#[tokio::test]
async fn delegated_execute_reaches_its_exact_generation_through_the_private_seat() {
    let state = crate::tests::make_test_state();
    let tmp = tempfile::tempdir().unwrap();
    let session = capsem_crypto::uuid_v4().to_string();
    crate::tests::insert_fake_instance_with_session_dir(&state, &session, std::process::id(), tmp.path().to_owned());
    let (generation, path) = {
        let instances = state.instances.lock().unwrap();
        let instance = &instances[&session];
        let result = (instance.generation, instance.uds_path.clone());
        drop(instances);
        result
    };
    let owner = crate::tests::spawn_fake_process(&path, 1, |message| {
        let ServiceToProcess::Exec { id, command, .. } = message else {
            panic!("expected exec");
        };
        assert_eq!(command, "scoped-exec");
        let id = *id;
        Box::pin(async move {
            Some(ProcessToService::ExecResult {
                id,
                stdout: b"scoped".to_vec(),
                stderr: vec![],
                exit_code: 0,
                truncated: false,
            })
        })
    });
    let (access, _) = issue(
        &state,
        &session,
        generation,
        AgentOperation::Execute,
        std::time::Duration::from_secs(60),
    );
    let response = crate::router_runtime::build_service_router(state)
        .operator_request(request(&session, access))
        .await
        .unwrap();
    if response.status() != StatusCode::OK {
        owner.abort();
    }
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(owner.await.unwrap().len(), 1);
}

fn owned_session() -> (Arc<ServiceState>, tempfile::TempDir, String, uuid::Uuid, PathBuf) {
    let state = crate::tests::make_test_state();
    let tmp = tempfile::tempdir().unwrap();
    let session = capsem_crypto::uuid_v4().to_string();
    crate::tests::insert_fake_instance_with_session_dir(&state, &session, std::process::id(), tmp.path().to_owned());
    let instances = state.instances.lock().unwrap();
    let instance = &instances[&session];
    let generation = instance.generation;
    let path = instance.uds_path.clone();
    drop(instances);
    (state, tmp, session, generation, path)
}

fn admitted_actor(state: &ServiceState, session: &str, access: uuid::Uuid) -> crate::caller_auth::AuthenticatedActor {
    let headers = request(session, access).into_parts().0.headers;
    let mut actor = state.caller_authority.authenticate_gateway(&headers).unwrap();
    assert!(actor.bind_execution(state, session));
    actor
}

async fn assert_no_connection(listener: &tokio::net::UnixListener) {
    std::future::poll_fn(|cx| {
        assert!(listener.poll_accept(cx).is_pending(), "refused exec contacted an owner");
        std::task::Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn wrong_operation_session_generation_and_public_transport_are_audited_without_ipc() {
    use tower::ServiceExt;
    let (state, _tmp, session, generation, path) = owned_session();
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let app = crate::router_runtime::build_service_router(Arc::clone(&state));
    let mut accesses = vec![];
    for (target, owner_generation, operation) in [
        (session.clone(), generation, AgentOperation::Inspect),
        (
            capsem_crypto::uuid_v4().to_string(),
            generation,
            AgentOperation::Execute,
        ),
        (session.clone(), capsem_crypto::uuid_v4(), AgentOperation::Execute),
    ] {
        let (access, _) = issue(
            &state,
            &target,
            owner_generation,
            operation,
            std::time::Duration::from_secs(60),
        );
        let response = app.clone().operator_request(request(&session, access)).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        accesses.push(access);
    }
    let (access, _) = issue(
        &state,
        &session,
        generation,
        AgentOperation::Execute,
        std::time::Duration::from_secs(60),
    );
    let response = app.clone().oneshot(request(&session, access)).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "public UDS cannot confer delegated transport"
    );
    accesses.push(access);
    let response = app
        .operator_request(request(&capsem_crypto::uuid_v4().to_string(), access))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    accesses.push(access);
    assert_no_connection(&listener).await;
    let events = state.host_ledger.host_events().await.unwrap();
    assert_eq!(events.len(), accesses.len());
    for (event, access) in events.iter().zip(accesses) {
        assert_eq!(event.kind, capsem_logger::HostEventKind::AccessDenied);
        assert_eq!(event.actor, format!("agent:{access}"));
        assert!(!event
            .detail
            .windows(64)
            .any(|value| value == "11".repeat(32).as_bytes()));
    }
}

#[tokio::test]
async fn delegated_authority_ending_during_preparation_is_audited_without_dispatch() {
    for action in ["revoke", "expire", "replace"] {
        let (state, _tmp, session, generation, path) = owned_session();
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let lease = if action == "expire" {
            std::time::Duration::from_millis(500)
        } else {
            std::time::Duration::from_secs(60)
        };
        let (access, grant) = issue(&state, &session, generation, AgentOperation::Execute, lease);
        let actor = admitted_actor(&state, &session, access);
        let mut execution = Box::pin(handle_exec(
            actor,
            State(Arc::clone(&state)),
            Path(session.clone()),
            Json(ExecRequest {
                command: "never".into(),
                timeout_secs: Some(1),
                target: None,
            }),
        ));
        std::future::poll_fn(|cx| {
            assert!(execution.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        match action {
            "revoke" => {
                state.caller_authority.agents.revoke(access).unwrap();
            }
            "expire" => tokio::time::sleep_until(grant.deadline().into()).await,
            "replace" => {
                state.instances.lock().unwrap().get_mut(&session).unwrap().generation = capsem_crypto::uuid_v4();
                std::fs::write(path.with_extension("ready"), "ready").unwrap();
            }
            _ => unreachable!(),
        }
        let error = tokio::time::timeout(std::time::Duration::from_secs(2), execution)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(error.status, StatusCode::FORBIDDEN, "{action}: {}", error.body.error);
        assert_no_connection(&listener).await;
        let events = state.host_ledger.host_events().await.unwrap();
        assert_eq!(events.len(), 1, "{action}");
        assert_eq!(events[0].kind, capsem_logger::HostEventKind::AccessDenied);
        assert_eq!(events[0].actor, format!("agent:{access}"));
    }
}

#[tokio::test]
async fn delegated_authority_is_rechecked_after_hello_before_sending_any_command() {
    use std::os::fd::AsFd;
    for action in ["revoke", "expire", "replace"] {
        let (state, _tmp, session, generation, path) = owned_session();
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let lease = if action == "expire" {
            std::time::Duration::from_millis(500)
        } else {
            std::time::Duration::from_secs(60)
        };
        let (access, grant) = issue(&state, &session, generation, AgentOperation::Execute, lease);
        let actor = admitted_actor(&state, &session, access);
        let owner = actor.execution_owner(&state, &session).unwrap();
        let server_state = Arc::clone(&state);
        let server = tokio::spawn(async move {
            let (socket, _) = tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut socket = socket.into_std().unwrap();
            tokio::task::spawn_blocking(move || {
                assert!(
                    capsem_foundation::unix::fd::wait_readable(socket.as_fd(), std::time::Duration::from_secs(5))
                        .unwrap()
                );
                match action {
                    "revoke" => {
                        server_state.caller_authority.agents.revoke(access).unwrap();
                    }
                    "expire" => std::thread::sleep(
                        grant
                            .deadline()
                            .checked_duration_since(std::time::Instant::now())
                            .unwrap_or_default(),
                    ),
                    "replace" => {
                        server_state
                            .instances
                            .lock()
                            .unwrap()
                            .get_mut(&session)
                            .unwrap()
                            .generation = capsem_crypto::uuid_v4()
                    }
                    _ => unreachable!(),
                }
                capsem_foundation::ipc_handshake::negotiate_responder(&mut socket, "capsem-process", "").unwrap();
                socket
            })
            .await
            .unwrap()
        });
        let error = ipc_command::send_owner_command_guarded(&state, &owner, ServiceToProcess::Ping, Some(1), || {
            actor
                .check_execution(&state, &owner)
                .map_err(|_| ipc_command::IpcCommandError::AccessDenied)
        })
        .await
        .unwrap_err();
        assert_eq!(error, ipc_command::IpcCommandError::AccessDenied, "{action}");
        let mut socket = server.await.unwrap();
        tokio::task::spawn_blocking(move || {
            use std::io::Read;
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            assert_eq!(
                socket.read(&mut [0; 1]).unwrap(),
                0,
                "no command may follow denied hello"
            );
        })
        .await
        .unwrap();
    }
}
