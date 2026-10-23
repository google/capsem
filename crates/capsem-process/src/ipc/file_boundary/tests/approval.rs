use super::*;
use capsem_core::net::policy_config::{SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource};
use capsem_proto::approval::{ApprovalClientMessage, ApprovalOutcome, ApprovalServerMessage};

fn ask_import(dispatcher: &mut Dispatcher, peer: &crate::tests::ApprovalPeer) {
    let runtime = Arc::get_mut(&mut dispatcher.mcp_runtime).unwrap();
    runtime.approvals = Some(Arc::clone(&peer.capability));
    let profile = SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.ask_import]
        name = "ask_import"
        action = "ask"
        match = 'file.import.path.endsWith("options.json") || file.export.path.endsWith("options.json")'
    "#,
    )
    .unwrap();
    *runtime.security_rules.write().unwrap() =
        Arc::new(SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap());
}

#[tokio::test]
async fn original_import_waits_for_explicit_decision_without_blocking_ipc() {
    for action in [
        capsem_proto::ipc::FileBoundaryAction::Import,
        capsem_proto::ipc::FileBoundaryAction::Export,
    ] {
        for outcome in [
            ApprovalOutcome::Approved,
            ApprovalOutcome::Denied,
            ApprovalOutcome::Expired,
        ] {
            let temp = tempfile::tempdir().unwrap();
            let (mut dispatcher, mut control) = Dispatcher::new(temp.path());
            let peer = crate::tests::ApprovalPeer::new().await;
            ask_import(&mut dispatcher, &peer);
            let (request, response, handler) = channel(&dispatcher).await;
            let mut original = import_request();
            if let ServiceToProcess::LogFileBoundary {
                action: original_action,
                ..
            } = &mut original
            {
                *original_action = action;
            }
            request.send(original).await.unwrap();
            let ApprovalClientMessage::Begin {
                request_id,
                request: ask,
            } = tokio::time::timeout(Duration::from_secs(2), peer.receiver.recv())
                .await
                .expect("original import must be offered before a result is returned")
                .unwrap()
            else {
                panic!("begin")
            };
            assert!(!ask.summary.contains("options.json"));
            assert_eq!(ask.held_bytes, 8);
            assert!(tokio::time::timeout(Duration::from_millis(20), response.recv())
                .await
                .is_err());
            request.send(ServiceToProcess::Ping).await.unwrap();
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(1), response.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                ProcessToService::Pong
            ));
            peer.sender
                .send(ApprovalServerMessage { request_id, outcome })
                .await
                .unwrap();
            let result = tokio::time::timeout(Duration::from_secs(1), response.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(
                matches!(
                    result,
                    ProcessToService::LogFileBoundaryResult { id: 91, success, .. } if success == outcome.is_allowed()
                ),
                "{result:?}"
            );
            assert!(control.try_recv().is_err(), "host import has no guest side effect");
            drop(request);
            drop(response);
            tokio::time::timeout(Duration::from_secs(1), handler)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
    }
}

#[tokio::test]
async fn disconnected_import_cancels_its_original_approval() {
    let temp = tempfile::tempdir().unwrap();
    let (mut dispatcher, _control) = Dispatcher::new(temp.path());
    let peer = crate::tests::ApprovalPeer::new().await;
    ask_import(&mut dispatcher, &peer);
    let (request, response, handler) = channel(&dispatcher).await;
    request.send(import_request()).await.unwrap();
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
    tokio::time::timeout(Duration::from_secs(1), handler)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), peer.receiver.recv()).await.unwrap().unwrap(),
        ApprovalClientMessage::Cancel { request_id: id } if id == request_id
    ));
}

#[tokio::test]
async fn changed_policy_refuses_a_late_import_approval() {
    let temp = tempfile::tempdir().unwrap();
    let (mut dispatcher, _control) = Dispatcher::new(temp.path());
    let peer = crate::tests::ApprovalPeer::new().await;
    ask_import(&mut dispatcher, &peer);
    let (request, response, handler) = channel(&dispatcher).await;
    request.send(import_request()).await.unwrap();
    let ApprovalClientMessage::Begin { request_id, .. } =
        tokio::time::timeout(Duration::from_secs(1), peer.receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("begin")
    };
    *dispatcher.mcp_runtime.security_rules.write().unwrap() = Arc::new(SecurityRuleSet::new(Vec::new()));
    peer.sender
        .send(ApprovalServerMessage {
            request_id,
            outcome: ApprovalOutcome::Approved,
        })
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(1), response.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(result, ProcessToService::LogFileBoundaryResult { success: false, .. }),
        "{result:?}"
    );
    drop(request);
    drop(response);
    tokio::time::timeout(Duration::from_secs(1), handler)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn import_admission_counts_the_original_size_instead_of_a_small_preview() {
    let temp = tempfile::tempdir().unwrap();
    let (mut dispatcher, _control) = Dispatcher::new(temp.path());
    let peer = crate::tests::ApprovalPeer::new().await;
    ask_import(&mut dispatcher, &peer);
    let (request, response, handler) = channel(&dispatcher).await;
    let mut original = import_request();
    if let ServiceToProcess::LogFileBoundary { size, .. } = &mut original {
        *size = capsem_proto::APPROVAL_HELD_BYTES_PER_REQUEST as u64;
    }
    request.send(original).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(1), response.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(result, ProcessToService::LogFileBoundaryResult { success: false, .. }),
        "{result:?}"
    );
    assert!(tokio::time::timeout(Duration::from_millis(20), peer.receiver.recv())
        .await
        .is_err());
    drop(request);
    drop(response);
    tokio::time::timeout(Duration::from_secs(1), handler)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
