use super::*;

#[tokio::test]
async fn failed_retirement_commit_retries_metadata_without_repeating_provider_revocation() {
    let (fixture, client, tokens) = crate::oauth::test_connection(vec![crate::oauth::TestReply::json("")]).await;
    let connection = client
        .open_connection(tokens, crate::OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let provider = Arc::new(Provider::default());
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private/broker.json");
    let broker = ConnectionBroker::open(
        BrokerStorage::File {
            path: path.clone(),
            max_bytes: 65536,
        },
        vec![profile(Arc::clone(&provider))],
        authority(),
        limits(),
    )
    .await
    .unwrap();
    let id = connected(&broker, &provider, connection).await;
    broker.disconnect(&id).await.unwrap();
    let saved = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    joined(&broker, &id).await;
    assert_eq!(
        broker.connection(&id).await.unwrap().error_code,
        Some(api::CredentialErrorCode::StorageCommitFailed)
    );
    assert!(
        broker.0.state.lock().await.connections[&id].record.retiring,
        "failed commit must retain pending retirement for maintenance"
    );
    let requests = fixture.records.lock().unwrap().len();
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, saved).unwrap();
    broker.prune().await.unwrap();
    joined(&broker, &id).await;
    assert_eq!(broker.connection(&id).await.unwrap().error_code, None);
    assert_eq!(fixture.records.lock().unwrap().len(), requests);
    let saved: Snapshot = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(!saved.connections[0].retiring);
    assert_eq!(saved.connections[0].error_code, None);
}

#[tokio::test]
async fn retirement_timeout_remains_fenced_and_maintenance_keeps_one_worker() {
    let mut delayed = crate::oauth::TestReply::json("");
    delayed.delay = Duration::from_millis(200);
    let (_fixture, client, tokens) =
        crate::oauth::test_connection(vec![delayed, crate::oauth::TestReply::json("")]).await;
    let connection = client
        .open_connection(tokens, crate::OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let provider = Arc::new(Provider::default());
    let mut limits = limits();
    limits.retirement_timeout = Duration::from_millis(20);
    let broker = ConnectionBroker::open(
        BrokerStorage::Memory,
        vec![profile(Arc::clone(&provider))],
        authority(),
        limits,
    )
    .await
    .unwrap();
    let id = connected(&broker, &provider, connection.clone()).await;
    broker.disconnect(&id).await.unwrap();
    let job = broker.0.state.lock().await.jobs[&id].id();
    broker.prune().await.unwrap();
    broker.prune().await.unwrap();
    assert_eq!(broker.0.state.lock().await.jobs[&id].id(), job);
    joined(&broker, &id).await;
    assert_eq!(
        broker.connection(&id).await.unwrap().error_code,
        Some(api::CredentialErrorCode::ProviderTimeout)
    );
    assert!(
        connection.access(Instant::now()).await.is_err(),
        "timeout never restores local authority"
    );
    broker.prune().await.unwrap();
    joined(&broker, &id).await;
    assert_eq!(
        broker.connection(&id).await.unwrap().provider_revocation,
        api::ProviderRevocationState::Succeeded
    );
}

async fn connected(broker: &ConnectionBroker, provider: &Provider, connection: GoogleConnection) -> String {
    let sender = provider.queue();
    let started = broker.authorize(consent()).await.unwrap();
    sender.send(connection).unwrap();
    joined(broker, &started.transaction.id).await;
    broker
        .authorization(&started.transaction.id)
        .await
        .unwrap()
        .connection_id
        .unwrap()
}

#[tokio::test]
async fn maintenance_retries_failed_provider_retirement_and_stops_after_acknowledgement() {
    let mut failed = crate::oauth::TestReply::json("private-provider-detail");
    failed.status = 503;
    let (fixture, client, tokens) =
        crate::oauth::test_connection(vec![failed, crate::oauth::TestReply::json("")]).await;
    let connection = client
        .open_connection(tokens, crate::OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    let provider = Arc::new(Provider::default());
    let broker = ConnectionBroker::open(
        BrokerStorage::Memory,
        vec![profile(Arc::clone(&provider))],
        authority(),
        limits(),
    )
    .await
    .unwrap();
    let id = connected(&broker, &provider, connection).await;
    broker.disconnect(&id).await.unwrap();
    joined(&broker, &id).await;
    let failed = broker.connection(&id).await.unwrap();
    assert_eq!(failed.state, api::ConnectionState::Disconnected);
    assert_eq!(failed.provider_revocation, api::ProviderRevocationState::Failed);
    assert_eq!(failed.error_code, Some(api::CredentialErrorCode::ProviderRejected));
    let before = fixture.records.lock().unwrap().len();
    broker.prune().await.unwrap();
    joined(&broker, &id).await;
    let completed = broker.connection(&id).await.unwrap();
    assert_eq!(completed.provider_revocation, api::ProviderRevocationState::Succeeded);
    assert_eq!(completed.error_code, None);
    assert_eq!(fixture.records.lock().unwrap().len(), before + 1);
    broker.prune().await.unwrap();
    assert!(broker.0.state.lock().await.jobs.is_empty());
    assert_eq!(fixture.records.lock().unwrap().len(), before + 1);
    assert!(!serde_json::to_string(&broker.list().await.unwrap())
        .unwrap()
        .contains("private-provider-detail"));
}

#[tokio::test]
async fn restart_resumes_interrupted_durable_disconnect_without_rebinding_grants() {
    let (_fixture, connection) = verified().await;
    let provider = Arc::new(Provider::default());
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private/broker.json");
    let storage = || BrokerStorage::File {
        path: path.clone(),
        max_bytes: 65536,
    };
    let broker = ConnectionBroker::open(storage(), vec![profile(Arc::clone(&provider))], authority(), limits())
        .await
        .unwrap();
    let id = connected(&broker, &provider, connection.clone()).await;
    let session = binding();
    let request = grant_request(session, id.clone());
    let grant = broker
        .create_grant(session, request.clone(), |_, _| true)
        .await
        .unwrap();
    broker.disconnect(&id).await.unwrap();
    // Current-thread execution has not polled the newly spawned worker yet.
    // Abort its exact handle after the real durable intent has been published.
    let job = broker.0.state.try_lock().unwrap().jobs.remove(&id).unwrap();
    job.abort();
    assert!(job.await.unwrap_err().is_cancelled());
    assert_eq!(connection.status().state, crate::GoogleConnectionState::Connected);
    drop(broker);
    let broker = ConnectionBroker::open(storage(), vec![profile(Arc::clone(&provider))], authority(), limits())
        .await
        .unwrap();
    assert_eq!(
        broker.create_grant(session, request, |_, _| true).await.unwrap().state,
        api::GrantState::Revoked
    );
    joined(&broker, &id).await;
    let summary = broker.connection(&id).await.unwrap();
    assert_eq!(summary.state, api::ConnectionState::Disconnected);
    assert_eq!(summary.provider_revocation, api::ProviderRevocationState::Succeeded);
    assert!(!broker.0.state.lock().await.connections[&id].record.retiring);
    assert!(
        !broker
            .approvals(session.owner())
            .await
            .unwrap()
            .grants
            .iter()
            .find(|item| item.id == grant.id)
            .unwrap()
            .runtime_bound
    );
    drop(broker);
    let broker = ConnectionBroker::open(storage(), vec![profile(provider)], authority(), limits())
        .await
        .unwrap();
    assert_eq!(
        broker.connection(&id).await.unwrap().provider_revocation,
        api::ProviderRevocationState::Succeeded
    );
    assert!(
        broker.0.state.lock().await.jobs.is_empty(),
        "completed retirement must not launch another worker"
    );
}
