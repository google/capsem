use super::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::oneshot;
mod runtime;

#[derive(Default)]
struct Provider {
    queued: std::sync::Mutex<VecDeque<oneshot::Receiver<GoogleConnection>>>,
    connections: Arc<std::sync::Mutex<BTreeMap<String, GoogleConnection>>>,
    begins: AtomicUsize,
    dropped: Arc<AtomicBool>,
}
struct Scoped(Arc<AtomicBool>);
impl Drop for Scoped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
impl Provider {
    fn queue(&self) -> oneshot::Sender<GoogleConnection> {
        let (sent, received) = oneshot::channel();
        self.queued.lock().unwrap().push_back(received);
        sent
    }
}
impl EnrollmentProvider for Provider {
    fn begin(&self, id: String, _reconnect: Option<GoogleConnection>) -> EnrollmentFuture<Enrollment> {
        self.begins.fetch_add(1, Ordering::AcqRel);
        let received = self.queued.lock().unwrap().pop_front();
        let dropped = Arc::clone(&self.dropped);
        let connections = Arc::clone(&self.connections);
        Box::pin(async move {
            let received = received.ok_or(api::CredentialErrorCode::ConfigurationRequired)?;
            let guard = Scoped(dropped);
            Ok(Enrollment {
                authorization_url: Some(Zeroizing::new(
                    "https://accounts.google.com/consent?state=private-browser-state".into(),
                )),
                completion: Box::pin(async move {
                    let _guard = guard;
                    let connection = received.await.map_err(|_| api::CredentialErrorCode::ProviderRejected)?;
                    connections.lock().unwrap().insert(id, connection.clone());
                    Ok(connection)
                }),
            })
        })
    }
    fn restore(&self, id: String) -> EnrollmentFuture<GoogleConnection> {
        let connection = self.connections.lock().unwrap().get(&id).cloned();
        Box::pin(async move { connection.ok_or(api::CredentialErrorCode::ConnectionUnavailable) })
    }
}

fn profile(provider: Arc<Provider>) -> BrokerProfile {
    let capability = crate::GrantCapability::new(
        "probe".into(),
        BTreeSet::from(["openid".into()]),
        vec![crate::GrantTarget::new("GET", "https://example.test/probe").unwrap()],
    )
    .unwrap();
    BrokerProfile::configured(
        api::ScopeProfileSummary {
            id: "fixture".into(),
            enrollment_kind: api::EnrollmentKind::OwnedOauthClient,
            label: "Fixture".into(),
            application_label: "Test only".into(),
            requested_scopes: vec!["openid".into()],
            capabilities: vec![api::CapabilitySummary {
                id: "probe".into(),
                label: "Probe".into(),
                consumer: "fixture".into(),
                required_scopes: vec!["openid".into()],
                supported: true,
            }],
            enabled: true,
        },
        vec![capability],
        provider,
    )
    .unwrap()
}

fn consent() -> api::AuthorizeGoogleRequest {
    let mut value = request();
    value.scope_profile_id = "fixture".into();
    value
}

#[test]
fn native_profiles_cannot_be_enabled_by_the_registered_google_token_lane() {
    let provider = Arc::new(Provider::default());
    let configured = profile(Arc::clone(&provider));
    let mut summary = configured.summary;
    summary.enrollment_kind = api::EnrollmentKind::NativeConsumerSignIn;
    assert!(matches!(
        BrokerProfile::configured(summary, configured.capabilities.into_values().collect(), provider),
        Err(api::CredentialErrorCode::UnsupportedCapability)
    ));
}
fn binding() -> SessionBinding {
    SessionBinding::new(
        crate::GrantSession::new(
            *capsem_crypto::uuid_v4().as_bytes(),
            *capsem_crypto::uuid_v4().as_bytes(),
            *capsem_crypto::uuid_v4().as_bytes(),
        )
        .unwrap(),
        SystemTime::now() + Duration::from_secs(60),
    )
    .unwrap()
}
fn grant_request(binding: SessionBinding, connection_id: String) -> api::CreateSessionGrantRequest {
    api::CreateSessionGrantRequest {
        request_id: capsem_crypto::uuid_v4().to_string(),
        expected_owner_generation: binding.ids().1,
        connection_id,
        capability_ids: vec!["probe".into()],
        expires_at: None,
    }
}
async fn verified() -> (impl Drop, GoogleConnection) {
    let (fixture, client, tokens) = crate::oauth::test_connection(vec![crate::oauth::TestReply::json("")]).await;
    let connection = client
        .open_connection(tokens, crate::OAuthConnectionStorage::memory(65536).unwrap())
        .await
        .unwrap();
    (fixture, connection)
}
async fn joined(broker: &ConnectionBroker, id: &str) {
    let job = broker.0.state.lock().await.jobs.remove(id);
    if let Some(job) = job {
        // Take the handle before awaiting: the job itself owns completion of
        // the durable transition and does not need to retain its own handle.
        tokio::time::timeout(Duration::from_secs(2), job)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn cancel_is_terminal_idempotent_and_awaits_scoped_callback_drop() {
    let provider = Arc::new(Provider::default());
    let sender = provider.queue();
    let broker = ConnectionBroker::open(
        BrokerStorage::Memory,
        vec![profile(Arc::clone(&provider))],
        authority(),
        limits(),
    )
    .await
    .unwrap();
    let request = consent();
    let started = broker.authorize(request.clone()).await.unwrap();
    assert!(started.authorization_url.unwrap().contains("private-browser-state"));
    let repeated = broker.authorize(request.clone()).await.unwrap();
    assert_eq!(repeated.transaction.id, started.transaction.id);
    let mut conflict = request.clone();
    conflict.reconnect_connection_id = Some(capsem_crypto::uuid_v4().to_string());
    assert_eq!(
        broker.authorize(conflict).await.unwrap_err(),
        api::CredentialErrorCode::RequestConflict
    );
    let cancelled = broker.cancel_authorization(&started.transaction.id).await.unwrap();
    assert_eq!(cancelled.state, api::TransactionState::Cancelled);
    assert!(provider.dropped.load(Ordering::Acquire));
    assert!(sender.is_closed());
    assert_eq!(
        broker.cancel_authorization(&started.transaction.id).await.unwrap(),
        cancelled
    );
    let repeated = broker.authorize(request).await.unwrap();
    assert_eq!(repeated.transaction, cancelled);
    assert!(repeated.authorization_url.is_none());
    assert_eq!(provider.begins.load(Ordering::Acquire), 1);
    assert!(broker.list().await.unwrap().connections.is_empty());
}

#[tokio::test]
async fn expiration_closes_callback_and_cannot_replay_enrollment() {
    let provider = Arc::new(Provider::default());
    let sender = provider.queue();
    let mut bounds = limits();
    bounds.enrollment_lifetime = Duration::from_millis(30);
    let broker = ConnectionBroker::open(
        BrokerStorage::Memory,
        vec![profile(Arc::clone(&provider))],
        authority(),
        bounds,
    )
    .await
    .unwrap();
    let request = consent();
    let started = broker.authorize(request.clone()).await.unwrap();
    joined(&broker, &started.transaction.id).await;
    assert_eq!(
        broker.authorization(&started.transaction.id).await.unwrap().state,
        api::TransactionState::Expired
    );
    assert!(sender.is_closed());
    assert!(provider.dropped.load(Ordering::Acquire));
    assert_eq!(
        broker.authorize(request).await.unwrap().transaction.id,
        started.transaction.id
    );
    assert_eq!(provider.begins.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn terminal_retention_keeps_recent_retries_and_reclaims_old_slots_without_touching_pending() {
    let provider = Arc::new(Provider::default());
    let _first = provider.queue();
    let _second = provider.queue();
    let mut bounds = limits();
    bounds.max_transactions = 2;
    let broker = ConnectionBroker::open(
        BrokerStorage::Memory,
        vec![profile(Arc::clone(&provider))],
        authority(),
        bounds,
    )
    .await
    .unwrap();
    let request = consent();
    let first = broker.authorize(request.clone()).await.unwrap();
    broker.cancel_authorization(&first.transaction.id).await.unwrap();
    let second = broker.authorize(consent()).await.unwrap();
    assert_eq!(broker.prune_at(SystemTime::now()).await.unwrap().transactions, 0);
    assert_eq!(
        broker.authorize(request).await.unwrap().transaction.state,
        api::TransactionState::Cancelled
    );
    let future = SystemTime::now() + bounds.retention_lifetime + Duration::from_secs(1);
    assert_eq!(broker.prune_at(future).await.unwrap().transactions, 1);
    assert_eq!(
        broker.authorization(&second.transaction.id).await.unwrap().state,
        api::TransactionState::Pending
    );
    assert_eq!(
        broker.authorization(&first.transaction.id).await.unwrap_err(),
        api::CredentialErrorCode::TransactionExpired
    );
    let _third = provider.queue();
    broker.authorize(consent()).await.unwrap();
    assert_eq!(provider.begins.load(Ordering::Acquire), 3);
}

#[tokio::test]
async fn durable_restart_cancels_pending_and_never_persists_browser_material() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private/broker.json");
    let provider = Arc::new(Provider::default());
    let _sender = provider.queue();
    let request = consent();
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
    let started = broker.authorize(request.clone()).await.unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("private-browser-state"));
    drop(broker);
    let restored = ConnectionBroker::open(
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
    assert_eq!(
        restored.authorization(&started.transaction.id).await.unwrap().state,
        api::TransactionState::Cancelled
    );
    let replay = restored.authorize(request).await.unwrap();
    assert_eq!(replay.transaction.id, started.transaction.id);
    assert!(replay.authorization_url.is_none());
    assert_eq!(provider.begins.load(Ordering::Acquire), 1);
    drop(restored);
    std::fs::write(
        &path,
        b"{\"version\":99,\"connections\":[],\"transactions\":[],\"grants\":[]}",
    )
    .unwrap();
    assert!(matches!(
        ConnectionBroker::open(
            BrokerStorage::File { path, max_bytes: 65536 },
            vec![],
            authority(),
            limits()
        )
        .await,
        Err(api::CredentialErrorCode::StorageUnavailable)
    ));
}

#[tokio::test]
async fn explicit_grants_are_atomic_owned_and_revoked_before_disconnect() {
    let (_fixture, connection) = verified().await;
    let provider = Arc::new(Provider::default());
    let sender = provider.queue();
    let authority = authority();
    let broker = ConnectionBroker::open(
        BrokerStorage::Memory,
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
    let session = binding();
    let request = grant_request(session, id.clone());
    assert_eq!(
        broker
            .create_grant(session, request.clone(), |_, _| false)
            .await
            .unwrap_err(),
        api::CredentialErrorCode::PolicyDenied
    );
    assert!(broker.grants(session, |_, _| true).await.unwrap().grants.is_empty());
    let grant = broker
        .create_grant(session, request.clone(), |_, _| true)
        .await
        .unwrap();
    assert_eq!(
        broker
            .create_grant(session, request.clone(), |_, _| true)
            .await
            .unwrap()
            .id,
        grant.id
    );
    let mut conflict = request.clone();
    conflict.capability_ids.push("other".into());
    let consumer_target = crate::GrantTarget::new("GET", "https://example.test/probe").unwrap();
    assert!(broker
        .with_grant(session, &grant.id, &consumer_target, |_, _| true, |_| true)
        .await
        .unwrap());
    assert!(broker
        .with_grant(
            binding(),
            &grant.id,
            &consumer_target,
            |_, _| true,
            |_| panic!("wrong owner")
        )
        .await
        .is_err());
    assert_eq!(
        broker.create_grant(session, conflict, |_, _| true).await.unwrap_err(),
        api::CredentialErrorCode::RequestConflict
    );
    assert_eq!(
        broker
            .create_grant(binding(), request.clone(), |_, _| true)
            .await
            .unwrap_err(),
        api::CredentialErrorCode::OwnerChanged
    );
    let denied = broker.grants(session, |_, _| false).await.unwrap();
    assert_eq!(denied.denied_capability_ids, vec!["probe"]);
    assert!(!denied.grants[0].runtime_bound);
    let handle = broker
        .0
        .state
        .lock()
        .await
        .grants
        .get(&grant.id)
        .unwrap()
        .handle
        .unwrap();
    let target = crate::GrantTarget::new("GET", "https://example.test/probe").unwrap();
    assert!(authority
        .with_token(handle, session.session, &target, Instant::now(), |_, _| true, |_| true)
        .await
        .unwrap());
    let result = broker.disconnect(&id).await.unwrap();
    assert_eq!(result.local_authority, api::LocalCredentialAuthority::Revoked);
    assert_eq!(result.revoked_grant_count, 1);
    assert!(broker
        .with_grant(
            session,
            &grant.id,
            &consumer_target,
            |_, _| true,
            |_| panic!("revoked broker admission")
        )
        .await
        .is_err());
    assert!(authority
        .with_token(
            handle,
            session.session,
            &target,
            Instant::now(),
            |_, _| true,
            |_| panic!("retired grant must not expose material")
        )
        .await
        .is_err());
    let replay = broker.create_grant(session, request, |_, _| true).await.unwrap();
    assert_eq!(replay.state, api::GrantState::Revoked);
    assert!(!replay.runtime_bound);
    joined(&broker, &id).await;
    let disconnected = broker.connection(&id).await.unwrap();
    assert_eq!(disconnected.state, api::ConnectionState::Disconnected);
    assert_eq!(
        disconnected.provider_revocation,
        api::ProviderRevocationState::Succeeded
    );
    assert!(
        !broker
            .0
            .state
            .lock()
            .await
            .connections
            .get(&id)
            .unwrap()
            .record
            .retiring,
        "completed retirement permits explicit native reconnect"
    );
    let text = serde_json::to_string(&broker.list().await.unwrap()).unwrap();
    for secret in [
        "private-access",
        "private-refresh",
        "private-connection-subject",
        "private-browser-state",
    ] {
        assert!(!text.contains(secret));
    }
}

#[tokio::test]
async fn managed_owner_retirement_is_visible_and_cannot_reissue_the_old_request() {
    let (_fixture, connection) = verified().await;
    let provider = Arc::new(Provider::default());
    let sender = provider.queue();
    let authority = authority();
    let broker = ConnectionBroker::open(
        BrokerStorage::Memory,
        vec![profile(provider)],
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
    let session = binding();
    let request = grant_request(session, id);
    let grant = broker
        .create_grant(session, request.clone(), |_, _| true)
        .await
        .unwrap();
    let (id, owner, _) = session.session.identities();
    assert_eq!(authority.revoke_owner(id, owner).unwrap(), 1);
    assert_eq!(
        broker.grants(session, |_, _| true).await.unwrap().grants[0].state,
        api::GrantState::Revoked
    );
    assert_eq!(
        broker.create_grant(session, request, |_, _| true).await.unwrap().id,
        grant.id
    );
    assert!(!broker.grants(session, |_, _| true).await.unwrap().grants[0].runtime_bound);
}

#[tokio::test]
async fn preserved_approval_rebinds_only_the_same_owner_and_fences_old_runtime() {
    let (_fixture, connection) = verified().await;
    let provider = Arc::new(Provider::default());
    let sender = provider.queue();
    let authority = authority();
    let broker = ConnectionBroker::open(
        BrokerStorage::Memory,
        vec![profile(provider)],
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
    let old = broker
        .0
        .state
        .lock()
        .await
        .grants
        .get(&grant.id)
        .unwrap()
        .handle
        .unwrap();
    let (session, owner, _) = original.session.identities();
    let (_, _, runtime) = original.session.identities();
    broker
        .stop_runtime(session, runtime, crate::GrantStopPolicy::Preserve)
        .await
        .unwrap();
    let stopped = broker.0.state.lock().await.grants[&grant.id].record.summary.clone();
    assert!(!stopped.runtime_bound);
    assert_eq!(stopped.state, api::GrantState::Active);
    let resumed = SessionBinding::new(
        GrantSession::new(session, owner, *capsem_crypto::uuid_v4().as_bytes()).unwrap(),
        original.expires,
    )
    .unwrap();
    assert!(broker.grants(binding(), |_, _| true).await.unwrap().grants.is_empty());
    assert!(!broker.grants(resumed, |_, _| false).await.unwrap().grants[0].runtime_bound);
    let bound = broker.grants(resumed, |_, _| true).await.unwrap();
    assert!(bound.grants[0].runtime_bound);
    let new = broker
        .0
        .state
        .lock()
        .await
        .grants
        .get(&grant.id)
        .unwrap()
        .handle
        .unwrap();
    assert_ne!(old, new);
    broker
        .stop_runtime(session, runtime, crate::GrantStopPolicy::Revoke)
        .await
        .unwrap();
    assert!(broker.grants(resumed, |_, _| true).await.unwrap().grants[0].runtime_bound);
    let target = crate::GrantTarget::new("GET", "https://example.test/probe").unwrap();
    assert!(authority
        .with_token(
            old,
            original.session,
            &target,
            Instant::now(),
            |_, _| true,
            |_| panic!("old runtime token")
        )
        .await
        .is_err());
    assert!(authority
        .with_token(new, resumed.session, &target, Instant::now(), |_, _| true, |_| true)
        .await
        .unwrap());
    let revoked = broker.revoke_grant(resumed, &grant.id).await.unwrap();
    assert_eq!(revoked.grant.state, api::GrantState::Revoked);
    assert!(!broker.grants(resumed, |_, _| true).await.unwrap().grants[0].runtime_bound);
}

#[tokio::test]
async fn metadata_recovery_requires_original_owner_consent_and_current_policy() {
    let (_fixture, connection) = verified().await;
    let provider = Arc::new(Provider::default());
    let sender = provider.queue();
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
    let started = broker.authorize(consent()).await.unwrap();
    sender.send(connection).unwrap();
    joined(&broker, &started.transaction.id).await;
    let id = broker
        .authorization(&started.transaction.id)
        .await
        .unwrap()
        .connection_id
        .unwrap();
    let session = binding();
    let request = grant_request(session, id);
    let granted = broker
        .create_grant(session, request.clone(), |_, _| true)
        .await
        .unwrap();
    drop(broker);
    assert!(
        matches!(
            ConnectionBroker::open(
                BrokerStorage::File {
                    path: path.clone(),
                    max_bytes: 65536
                },
                vec![],
                authority(),
                limits()
            )
            .await,
            Err(api::CredentialErrorCode::ConfigurationRequired)
        ),
        "missing custody provider cannot invent connection revisions or consent metadata"
    );
    // This test proves controller metadata recovery. The private connection
    // owner's actual file restore and native runtime retirement have separate
    // tests; this fixture keeps its already-verified owner across reconstruction.
    let restored = ConnectionBroker::open(
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
    assert!(restored.grants(binding(), |_, _| true).await.unwrap().grants.is_empty());
    assert!(!restored.grants(session, |_, _| false).await.unwrap().grants[0].runtime_bound);
    assert!(restored.grants(session, |_, _| true).await.unwrap().grants[0].runtime_bound);
    assert_eq!(
        restored
            .create_grant(session, request.clone(), |_, _| true)
            .await
            .unwrap()
            .id,
        granted.id
    );
    restored.revoke_grant(session, &granted.id).await.unwrap();
    drop(restored);
    let restored = ConnectionBroker::open(
        BrokerStorage::File { path, max_bytes: 65536 },
        vec![profile(provider)],
        authority(),
        limits(),
    )
    .await
    .unwrap();
    assert_eq!(
        restored
            .create_grant(session, request, |_, _| true)
            .await
            .unwrap()
            .state,
        api::GrantState::Revoked
    );
    assert!(!restored.grants(session, |_, _| true).await.unwrap().grants[0].runtime_bound);
}

#[tokio::test]
async fn failed_durable_grant_publication_retires_authority_and_cannot_replay() {
    let (_fixture, connection) = verified().await;
    let provider = Arc::new(Provider::default());
    let sender = provider.queue();
    let authority = authority();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private/broker.json");
    let broker = ConnectionBroker::open(
        BrokerStorage::File {
            path: path.clone(),
            max_bytes: 65536,
        },
        vec![profile(provider)],
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
    let session = binding();
    let request = grant_request(session, id);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert_eq!(
        broker
            .create_grant(session, request.clone(), |_, _| true)
            .await
            .unwrap_err(),
        api::CredentialErrorCode::StorageCommitFailed
    );
    let replay = broker.create_grant(session, request, |_, _| true).await.unwrap();
    assert_eq!(replay.state, api::GrantState::Revoked);
    assert!(!replay.runtime_bound);
    assert!(broker
        .0
        .state
        .lock()
        .await
        .grants
        .get(&replay.id)
        .unwrap()
        .handle
        .is_none());
    let (id, owner, _) = session.session.identities();
    assert_eq!(authority.revoke_owner(id, owner).unwrap(), 0);
}

#[tokio::test]
async fn terminal_cleanup_is_durable_and_never_removes_live_connections_or_unexpired_grants() {
    let (_fixture, connection) = verified().await;
    let provider = Arc::new(Provider::default());
    let sender = provider.queue();
    let authority = authority();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private/broker.json");
    let broker = ConnectionBroker::open(
        BrokerStorage::File {
            path: path.clone(),
            max_bytes: 65536,
        },
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
    let session = binding();
    let grant = broker
        .create_grant(session, grant_request(session, id.clone()), |_, _| true)
        .await
        .unwrap();
    assert_eq!(broker.prune().await.unwrap(), BrokerPruned::default());
    assert_eq!(
        broker.connection(&id).await.unwrap().state,
        api::ConnectionState::Connected
    );
    assert!(broker.grants(session, |_, _| true).await.unwrap().grants[0].runtime_bound);
    broker.disconnect(&id).await.unwrap();
    joined(&broker, &id).await;
    let future = SystemTime::now() + limits().retention_lifetime + Duration::from_secs(31);
    assert_eq!(
        broker.prune_at(future).await.unwrap(),
        BrokerPruned {
            transactions: 1,
            grants: 1,
            connections: 1
        }
    );
    assert_eq!(
        broker.connection(&id).await.unwrap_err(),
        api::CredentialErrorCode::ConnectionUnavailable
    );
    assert_eq!(
        broker.revoke_grant(session, &grant.id).await.unwrap_err(),
        api::CredentialErrorCode::GrantUnavailable
    );
    drop(broker);
    let restored = ConnectionBroker::open(
        BrokerStorage::File { path, max_bytes: 65536 },
        vec![],
        super::authority(),
        limits(),
    )
    .await
    .unwrap();
    assert!(restored.list().await.unwrap().connections.is_empty());
}

#[tokio::test]
async fn unavailable_consent_retires_grants_and_expiry_never_publishes_an_unsafe_revision() {
    let (_fixture, connection) = verified().await;
    let provider = Arc::new(Provider::default());
    let sender = provider.queue();
    let broker = ConnectionBroker::open(BrokerStorage::Memory, vec![profile(provider)], authority(), limits())
        .await
        .unwrap();
    let started = broker.authorize(consent()).await.unwrap();
    sender.send(connection.clone()).unwrap();
    joined(&broker, &started.transaction.id).await;
    let id = broker
        .authorization(&started.transaction.id)
        .await
        .unwrap()
        .connection_id
        .unwrap();
    let session = binding();
    let grant = broker
        .create_grant(session, grant_request(session, id), |_, _| true)
        .await
        .unwrap();
    {
        let mut state = broker.0.state.lock().await;
        let entry = state.grants.get_mut(&grant.id).unwrap();
        entry.record.summary.expires_at = timestamp(SystemTime::now() - Duration::from_secs(1)).unwrap();
        entry.record.summary.revision = (1_u64 << 53) - 1;
        drop(state);
    }
    assert_eq!(
        broker.grants(session, |_, _| true).await.unwrap_err(),
        api::CredentialErrorCode::StorageUnavailable
    );
    {
        let mut state = broker.0.state.lock().await;
        let entry = state.grants.get_mut(&grant.id).unwrap();
        entry.record.summary.state = api::GrantState::Active;
        entry.record.summary.revision = 1;
        entry.record.summary.expires_at = timestamp(SystemTime::now() + Duration::from_secs(30)).unwrap();
        drop(state);
    }
    connection.disconnect().await.unwrap();
    let result = broker.grants(session, |_, _| true).await.unwrap();
    assert_eq!(result.grants[0].state, api::GrantState::Revoked);
    assert!(!result.grants[0].runtime_bound);
}
