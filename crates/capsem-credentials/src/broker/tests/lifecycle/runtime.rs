use super::*;

async fn granted(
    storage: BrokerStorage,
) -> (
    impl Drop,
    ConnectionBroker,
    Arc<Provider>,
    Arc<crate::GrantAuthority>,
    SessionBinding,
    api::SessionGrantSummary,
) {
    let (fixture, connection) = verified().await;
    let provider = Arc::new(Provider::default());
    let sender = provider.queue();
    let authority = authority();
    let broker = ConnectionBroker::open(
        storage,
        vec![profile(Arc::clone(&provider))],
        Arc::clone(&authority),
        limits(),
    )
    .await
    .unwrap();
    let started = broker.authorize(consent()).await.unwrap();
    sender.send(connection).unwrap();
    joined(&broker, &started.transaction.id).await;
    let id = broker
        .authorization(&started.transaction.id)
        .await
        .unwrap()
        .connection_id
        .unwrap();
    let original = binding();
    let grant = broker
        .create_grant(original, grant_request(original, id), |_, _| true)
        .await
        .unwrap();
    (fixture, broker, provider, authority, original, grant)
}

#[tokio::test]
async fn lifecycle_runtime_fence_precedes_contended_metadata() {
    let (_fixture, broker, _provider, authority, original, grant) = granted(BrokerStorage::Memory).await;
    let held = broker.0.state.lock().await;
    let handle = held.grants[&grant.id].handle.unwrap();
    let (session, _, runtime) = original.session.identities();
    let mut pending = Box::pin(broker.stop_runtime(session, runtime, crate::GrantStopPolicy::Preserve));
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut pending)
        .await
        .is_err());
    assert_eq!(
        authority.visibility(handle, original.session, Instant::now()),
        Some(crate::grants::GrantVisibility::Preserved)
    );
    drop(held);
    pending.await.unwrap();
    let state = broker.0.state.lock().await;
    let summary = state.grants[&grant.id].record.summary.clone();
    drop(state);
    assert_eq!(summary.state, api::GrantState::Active);
    assert!(!summary.runtime_bound);
    assert!(summary.revision > grant.revision);
}

#[tokio::test]
async fn lifecycle_runtime_maintenance_observes_synchronous_fences() {
    let (_fixture, broker, _provider, authority, original, grant) = granted(BrokerStorage::Memory).await;
    let (session, _, runtime) = original.session.identities();
    authority
        .detach_runtime(session, runtime, crate::GrantStopPolicy::Preserve)
        .unwrap();
    broker.prune().await.unwrap();
    assert!(
        !broker.0.state.lock().await.grants[&grant.id]
            .record
            .summary
            .runtime_bound
    );
    authority.revoke_session(session).unwrap();
    broker.prune().await.unwrap();
    assert_eq!(
        broker.0.state.lock().await.grants[&grant.id].record.summary.state,
        api::GrantState::Revoked
    );
}

#[tokio::test]
async fn lifecycle_runtime_noop_avoids_metadata_io_and_failed_commit_still_fences() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private/broker.json");
    let (_fixture, broker, _provider, authority, original, grant) = granted(BrokerStorage::File {
        path: path.clone(),
        max_bytes: 65536,
    })
    .await;
    let handle = broker.0.state.lock().await.grants[&grant.id].handle.unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let unrelated = *capsem_crypto::uuid_v4().as_bytes();
    broker
        .stop_runtime(unrelated, unrelated, crate::GrantStopPolicy::Revoke)
        .await
        .unwrap();
    broker.retire_session(unrelated).await.unwrap();
    let (session, _, runtime) = original.session.identities();
    assert_eq!(
        broker
            .stop_runtime(session, runtime, crate::GrantStopPolicy::Revoke)
            .await,
        Err(api::CredentialErrorCode::StorageCommitFailed)
    );
    assert_eq!(authority.visibility(handle, original.session, Instant::now()), None);
    assert_eq!(
        broker.grants(original, |_, _| true).await.unwrap().grants[0].state,
        api::GrantState::Revoked
    );
}

#[tokio::test]
async fn lifecycle_runtime_terminal_delete_revokes_recovered_unbound_approval_durably() {
    let root = tempfile::tempdir().unwrap();
    let storage = || BrokerStorage::File {
        path: root.path().join("private/broker.json"),
        max_bytes: 65536,
    };
    let (_fixture, broker, provider, _authority, original, grant) = granted(storage()).await;
    drop(broker);
    let restored = ConnectionBroker::open(storage(), vec![profile(Arc::clone(&provider))], authority(), limits())
        .await
        .unwrap();
    assert!(
        !restored.0.state.lock().await.grants[&grant.id]
            .record
            .summary
            .runtime_bound
    );
    let (session, _, _) = original.session.identities();
    assert!(restored.has_session_approval(session).await);
    restored.retire_session(session).await.unwrap();
    assert!(!restored.has_session_approval(session).await);
    drop(restored);
    let restored = ConnectionBroker::open(storage(), vec![profile(provider)], authority(), limits())
        .await
        .unwrap();
    let result = restored.grants(original, |_, _| true).await.unwrap();
    assert_eq!(result.grants[0].state, api::GrantState::Revoked);
    assert!(!result.grants[0].runtime_bound);
    assert!(restored.grants(binding(), |_, _| true).await.unwrap().grants.is_empty());
    assert_eq!(
        restored.list().await.unwrap().connections[0].state,
        api::ConnectionState::Connected
    );
}

#[tokio::test]
async fn stopped_approval_inventory_cannot_rebind_and_revoke_is_owner_scoped_and_durable() {
    let root = tempfile::tempdir().unwrap();
    let storage = || BrokerStorage::File {
        path: root.path().join("private/broker.json"),
        max_bytes: 65536,
    };
    let (_fixture, broker, provider, authority, original, grant) = granted(storage()).await;
    let (session, owner, runtime) = original.session.identities();
    let approval = crate::broker::ApprovalOwner::new(session, owner).unwrap();
    let old = broker.0.state.lock().await.grants[&grant.id].handle.unwrap();
    broker
        .stop_runtime(session, runtime, crate::GrantStopPolicy::Preserve)
        .await
        .unwrap();
    let result = broker.approvals(approval).await.unwrap();
    assert_eq!(result.owner_generation, uuid::Uuid::from_bytes(owner).to_string());
    assert_eq!(result.grants[0].state, api::GrantState::Active);
    assert!(!result.grants[0].runtime_bound);
    assert_eq!(
        authority.visibility(old, original.session, Instant::now()),
        Some(crate::grants::GrantVisibility::Preserved)
    );
    let wrong = crate::broker::ApprovalOwner::new(session, *capsem_crypto::uuid_v4().as_bytes()).unwrap();
    assert!(broker.approvals(wrong).await.unwrap().grants.is_empty());
    assert_eq!(
        broker.revoke_approval(wrong, &grant.id).await.unwrap_err(),
        api::CredentialErrorCode::OwnerChanged
    );
    let other_session = crate::broker::ApprovalOwner::new(*capsem_crypto::uuid_v4().as_bytes(), owner).unwrap();
    assert_eq!(
        broker.revoke_approval(other_session, &grant.id).await.unwrap_err(),
        api::CredentialErrorCode::OwnerChanged
    );
    let result = broker.revoke_approval(approval, &grant.id).await.unwrap();
    assert_eq!(result.grant.state, api::GrantState::Revoked);
    assert_eq!(authority.visibility(old, original.session, Instant::now()), None);
    drop(broker);
    let restored = ConnectionBroker::open(storage(), vec![profile(provider)], super::super::authority(), limits())
        .await
        .unwrap();
    let result = restored.approvals(approval).await.unwrap();
    assert_eq!(result.grants[0].state, api::GrantState::Revoked);
    assert!(!result.grants[0].runtime_bound);
}

#[tokio::test]
async fn stopped_approval_inventory_expires_finite_approval_without_provider_fetch() {
    let (_fixture, broker, _provider, authority, original, grant) = granted(BrokerStorage::Memory).await;
    let (session, owner, runtime) = original.session.identities();
    let handle = broker.0.state.lock().await.grants[&grant.id].handle.unwrap();
    broker
        .stop_runtime(session, runtime, crate::GrantStopPolicy::Preserve)
        .await
        .unwrap();
    broker
        .0
        .state
        .lock()
        .await
        .grants
        .get_mut(&grant.id)
        .unwrap()
        .record
        .summary
        .expires_at = timestamp(SystemTime::UNIX_EPOCH).unwrap();
    let inventory = broker
        .approvals(crate::broker::ApprovalOwner::new(session, owner).unwrap())
        .await
        .unwrap();
    assert_eq!(inventory.grants[0].state, api::GrantState::Expired);
    assert!(!inventory.grants[0].runtime_bound);
    assert_eq!(authority.visibility(handle, original.session, Instant::now()), None);
    assert!(crate::broker::ApprovalOwner::new([0; 16], owner).is_err());
    assert!(crate::broker::ApprovalOwner::new(session, [0; 16]).is_err());
}
