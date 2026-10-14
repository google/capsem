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
