use super::*;
use capsem_core::net::policy_config::{SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource};
use capsem_proto::approval::{ApprovalClientMessage, ApprovalOutcome, ApprovalServerMessage};

#[tokio::test]
async fn received_export_stops_transport_retries_while_original_bytes_wait_for_approval() {
    let temp = tempfile::tempdir().unwrap();
    let (mut dispatcher, mut control) = Dispatcher::new(temp.path());
    let peer = crate::tests::ApprovalPeer::new().await;
    let runtime = Arc::get_mut(&mut dispatcher.mcp_runtime).unwrap();
    runtime.approvals = Some(Arc::clone(&peer.capability));
    let profile = SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.ask_export]
        name = "ask_export"
        action = "ask"
        match = 'file.export.path == "/workspace/private.txt"'
    "#,
    )
    .unwrap();
    *runtime.security_rules.write().unwrap() =
        Arc::new(SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap());
    let (process, service) = tokio::net::UnixStream::pair().unwrap();
    let owner = dispatcher.connection(process);
    let mut service = service.into_std().unwrap();
    let (request, response) = tokio::task::spawn_blocking(move || {
        capsem_foundation::ipc_handshake::negotiate_initiator(&mut service, "capsem-service-test", "").unwrap();
        channel_from_std::<ServiceToProcess, ProcessToService>(service).unwrap()
    })
    .await
    .unwrap();
    request
        .send(ServiceToProcess::ReadFile {
            id: 9,
            path: "/workspace/private.txt".into(),
        })
        .await
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), control.recv())
            .await
            .unwrap(),
        Some(ServiceToProcess::ReadFile { id: 9, .. })
    ));
    let mut file_calls = tokio::task::JoinSet::new();
    crate::vsock::handle_guest_msg_with_tasks(
        capsem_proto::GuestToHost::FileContent {
            id: 9,
            path: "/guest/untrusted-name".into(),
            data: b"held export".to_vec(),
        },
        &dispatcher.job_store,
        &dispatcher.net_state.db,
        &dispatcher.mcp_runtime.security_rules,
        &dispatcher.mcp_runtime.plugin_policy,
        &mut file_calls,
    )
    .await;
    let ApprovalClientMessage::Begin { request_id, .. } =
        tokio::time::timeout(Duration::from_secs(2), peer.receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("begin")
    };
    assert!(
        tokio::time::timeout(GUEST_PAYLOAD_TIMEOUT + Duration::from_millis(200), control.recv())
            .await
            .is_err(),
        "physical FileContent receipt must stop the transport retry watchdog before human wait"
    );
    assert!(tokio::time::timeout(Duration::from_millis(20), response.recv())
        .await
        .is_err());
    peer.sender
        .send(ApprovalServerMessage {
            request_id,
            outcome: ApprovalOutcome::Approved,
        })
        .await
        .unwrap();
    assert!(
        matches!(tokio::time::timeout(Duration::from_secs(1), response.recv()).await.unwrap().unwrap(),
        ProcessToService::ReadFileResult { id: 9, data: Some(data), error: None } if data == b"held export")
    );
    drop(request);
    drop(response);
    tokio::time::timeout(Duration::from_secs(1), owner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    while file_calls.join_next().await.is_some() {}
}

#[tokio::test]
async fn saturated_file_memory_refuses_received_bytes_before_policy_work() {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(capsem_logger::DbWriter::open(&temp.path().join("session.db"), 64).unwrap());
    let (dispatcher, mut control) = Dispatcher::with_db(temp.path(), db);
    let _memory = Arc::clone(&dispatcher.job_store.file_memory)
        .try_acquire_many_owned(capsem_proto::APPROVAL_HELD_BYTES_TOTAL as u32)
        .unwrap();
    let (process, service) = tokio::net::UnixStream::pair().unwrap();
    let owner = dispatcher.connection(process);
    let mut service = service.into_std().unwrap();
    let (request, response) = tokio::task::spawn_blocking(move || {
        capsem_foundation::ipc_handshake::negotiate_initiator(&mut service, "capsem-service-test", "").unwrap();
        channel_from_std::<ServiceToProcess, ProcessToService>(service).unwrap()
    })
    .await
    .unwrap();
    request
        .send(ServiceToProcess::ReadFile {
            id: 8,
            path: "/workspace/private.txt".into(),
        })
        .await
        .unwrap();
    control.recv().await.unwrap();
    let mut calls = tokio::task::JoinSet::new();
    crate::vsock::handle_guest_msg_with_tasks(
        capsem_proto::GuestToHost::FileContent {
            id: 8,
            path: "untrusted".into(),
            data: b"private body".to_vec(),
        },
        &dispatcher.job_store,
        &dispatcher.net_state.db,
        &dispatcher.mcp_runtime.security_rules,
        &dispatcher.mcp_runtime.plugin_policy,
        &mut calls,
    )
    .await;
    let result = tokio::time::timeout(Duration::from_secs(1), response.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            result,
            ProcessToService::ReadFileResult {
                id: 8,
                data: None,
                error: Some(_)
            }
        ),
        "{result:?}"
    );
    dispatcher.net_state.db.flush_checked().await.unwrap();
    assert!(dispatcher
        .net_state
        .db
        .reader()
        .unwrap()
        .recent_file_events(10)
        .unwrap()
        .is_empty());
    drop(request);
    drop(response);
    tokio::time::timeout(Duration::from_secs(1), owner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    while calls.join_next().await.is_some() {}
}

#[tokio::test]
async fn ipc_disconnect_cancels_a_received_export_and_drains_its_original_reader() {
    let temp = tempfile::tempdir().unwrap();
    let (mut dispatcher, mut control) = Dispatcher::new(temp.path());
    let peer = crate::tests::ApprovalPeer::new().await;
    let runtime = Arc::get_mut(&mut dispatcher.mcp_runtime).unwrap();
    runtime.approvals = Some(Arc::clone(&peer.capability));
    let profile = SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.ask_export]
        name = "ask_export"
        action = "ask"
        match = 'file.export.path == "/workspace/private.txt"'
    "#,
    )
    .unwrap();
    *runtime.security_rules.write().unwrap() =
        Arc::new(SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap());
    let (process, service) = tokio::net::UnixStream::pair().unwrap();
    let owner = dispatcher.connection(process);
    let mut service = service.into_std().unwrap();
    let (request, response) = tokio::task::spawn_blocking(move || {
        capsem_foundation::ipc_handshake::negotiate_initiator(&mut service, "capsem-service-test", "").unwrap();
        channel_from_std::<ServiceToProcess, ProcessToService>(service).unwrap()
    })
    .await
    .unwrap();
    request
        .send(ServiceToProcess::ReadFile {
            id: 11,
            path: "/workspace/private.txt".into(),
        })
        .await
        .unwrap();
    control.recv().await.unwrap();
    let mut calls = tokio::task::JoinSet::new();
    crate::vsock::handle_guest_msg_with_tasks(
        capsem_proto::GuestToHost::FileContent {
            id: 11,
            path: "untrusted".into(),
            data: b"private body".to_vec(),
        },
        &dispatcher.job_store,
        &dispatcher.net_state.db,
        &dispatcher.mcp_runtime.security_rules,
        &dispatcher.mcp_runtime.plugin_policy,
        &mut calls,
    )
    .await;
    let ApprovalClientMessage::Begin { request_id, .. } =
        tokio::time::timeout(Duration::from_secs(1), peer.receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("begin")
    };
    drop(request);
    drop(response);
    tokio::time::timeout(Duration::from_secs(1), owner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(tokio::time::timeout(Duration::from_secs(1), peer.receiver.recv()).await.unwrap().unwrap(),
        ApprovalClientMessage::Cancel { request_id: id } if id == request_id)
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while calls.join_next().await.is_some() {}
    })
    .await
    .unwrap();
    assert!(dispatcher.job_store.active_file_ops.lock().unwrap().is_empty());
    assert!(dispatcher.job_store.jobs.lock().unwrap().is_empty());
    assert_eq!(
        dispatcher.job_store.file_memory.available_permits(),
        capsem_proto::APPROVAL_HELD_BYTES_TOTAL
    );
}
