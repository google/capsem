use super::*;
mod credentials;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Semaphore;

#[tokio::test]
async fn obsolete_policy_failure_cannot_cancel_current_validated_owner() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let owner = controller(&path);
    let fixture = Fixture::new();
    fixture.create_release.add_permits(1);
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([97; 32]);
    owner
        .create(
            request,
            cap.clone(),
            LeasePolicy::new(Duration::from_secs(10)).unwrap(),
            Arc::new(fixture.clone()),
        )
        .await
        .unwrap();
    let obsolete = owner
        .0
        .registry
        .publish_lease_limit(LeasePolicy::new(Duration::from_secs(5)).unwrap())
        .unwrap();
    owner
        .0
        .registry
        .publish_lease_limit(LeasePolicy::new(Duration::from_secs(10)).unwrap())
        .unwrap();
    let corrupt = path.join(format!("{}.json", capsem_crypto::uuid_v4()));
    std::fs::write(&corrupt, b"corrupt").unwrap();
    assert!(owner.limit_worker(obsolete).await.is_err());
    let work = owner.0.works.lock().await.get(&request).unwrap().clone();
    assert!(!work.cancel.is_cancelled());
    std::fs::remove_file(corrupt).unwrap();
    fixture.cleanup_release.add_permits(1);
    assert_eq!(owner.close(request, cap).await.unwrap().state(), State::Closed);
}

#[tokio::test]
async fn invalid_host_policy_closes_original_owned_effects_without_client_authority() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    fixture.create_release.add_permits(1);
    fixture.cleanup_release.add_permits(1);
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([96; 32]);
    let created = owner
        .create(
            request,
            cap.clone(),
            LeasePolicy::new(Duration::from_secs(10)).unwrap(),
            Arc::new(fixture.clone()),
        )
        .await
        .unwrap();
    owner.deny_lease_admission().await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let snapshot = owner.inspect(request, cap.clone()).await.unwrap().unwrap();
            if snapshot.state() == State::Closed {
                assert_eq!(snapshot.generation(), created.generation());
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn tighter_policy_updates_expiry_worker_and_original_cleanup_without_a_claim() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    fixture.create_release.add_permits(1);
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([93; 32]);
    owner
        .create(
            request,
            cap.clone(),
            LeasePolicy::new(Duration::from_secs(10)).unwrap(),
            Arc::new(fixture.clone()),
        )
        .await
        .unwrap();
    assert!(
        owner
            .set_lease_limit(LeasePolicy::new(Duration::from_millis(40)).unwrap())
            .now_or_never()
            .is_none(),
        "drop the polled reload waiter; its owned policy worker must continue"
    );
    tokio::time::timeout(Duration::from_secs(1), fixture.cleanup_entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert_eq!(
        owner.inspect(request, cap.clone()).await.unwrap().unwrap().state(),
        State::Closing
    );
    fixture.cleanup_release.add_permits(1);
    assert_eq!(owner.close(request, cap).await.unwrap().state(), State::Closed);
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stale_admission_or_claim_completion_cannot_overwrite_newer_tighter_timer() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    fixture.create_release.add_permits(1);
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([94; 32]);
    owner
        .create(
            request,
            cap.clone(),
            LeasePolicy::new(Duration::from_secs(10)).unwrap(),
            Arc::new(fixture.clone()),
        )
        .await
        .unwrap();
    let work = owner.0.works.lock().await.get(&request).unwrap().clone();
    let before = work.deadline.borrow().unwrap();
    owner
        .set_lease_limit(LeasePolicy::new(Duration::from_secs(2)).unwrap())
        .await
        .unwrap();
    let after = work.deadline.borrow().unwrap();
    assert!(after.at < before.at);
    assert!(after.revision > before.revision);
    work.update_deadline(before);
    assert_eq!(work.deadline.borrow().unwrap().at, after.at);
    assert_eq!(work.deadline.borrow().unwrap().revision, after.revision);
    fixture.cleanup_release.add_permits(1);
    owner.close(request, cap).await.unwrap();
}

#[tokio::test]
async fn ownership_lookup_keeps_live_intent_when_storage_disappears_and_checks_generation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let owner = controller(&path);
    let fixture = Fixture::new();
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([83; 32]);
    let creating = {
        let owner = owner.clone();
        let effects = fixture.clone();
        let cap = cap.clone();
        tokio::spawn(async move {
            owner
                .create(
                    request,
                    cap,
                    LeasePolicy::new(Duration::from_secs(10)).unwrap(),
                    Arc::new(effects),
                )
                .await
        })
    };
    fixture.create_entered.acquire().await.unwrap().forget();
    assert!(owner.owns_vm(&fixture.binding).await.unwrap());
    let reopened = controller(&path);
    assert!(reopened.owns_vm(&fixture.binding).await.unwrap());
    let other_generation = VmBinding::new(fixture.binding.id().into(), capsem_crypto::uuid_v4()).unwrap();
    assert!(!owner.owns_vm(&other_generation).await.unwrap());
    std::fs::remove_file(path.join(format!("{request}.json"))).unwrap();
    assert!(
        owner.owns_vm(&fixture.binding).await.unwrap(),
        "trusted live authority survives missing durable metadata"
    );
    std::fs::write(path.join(format!("{request}.json")), b"invalid ownership record").unwrap();
    assert!(owner.owns_vm(&fixture.binding).await.unwrap());
    assert!(matches!(
        reopened.owns_vm(&fixture.binding).await,
        Err(Failure::StoreUnavailable)
    ));
    fixture.create_release.add_permits(1);
    let _ = creating.await;
    fixture.cleanup_release.add_permits(1);
    let _ = owner.close(request, cap).await;
}

struct RecoveryEffects(Arc<Fixture>);
impl Effects for RecoveryEffects {
    fn is_live(&self, binding: &VmBinding) -> Result<bool> {
        self.0.is_live(binding)
    }
    fn create(
        &self,
        _ticket: Ticket,
        _cancel: CancellationToken,
        _reporter: BindingReporter,
    ) -> EffectFuture<VmBinding> {
        panic!("recovery must never invoke creation")
    }
    fn cleanup(&self, ticket: Ticket, binding: VmBinding) -> EffectFuture<()> {
        self.0.cleanup(ticket, binding)
    }
    fn reconcile(&self, _ticket: Ticket, intent: Option<VmBinding>) -> EffectFuture<Option<VmBinding>> {
        assert_eq!(intent.as_ref(), Some(&self.0.binding));
        self.0.reconciles.fetch_add(1, Ordering::SeqCst);
        let binding = self.0.binding.clone();
        Box::pin(async move { Ok(Some(binding)) })
    }
}

#[tokio::test]
async fn reopened_controller_recovers_original_intent_only_for_cleanup_and_can_retry() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let store = Registry::open(&path).unwrap();
    let fixture = Fixture::new();
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([52; 32]);
    let Reservation::New(ticket) = store.reserve(request, &cap).unwrap() else {
        panic!()
    };
    store
        .start_lease(
            &ticket,
            LeasePolicy::new(Duration::from_secs(10)).unwrap(),
            LeaseClock::now().unwrap(),
        )
        .unwrap();
    store.begin_create(&ticket, LeaseClock::now().unwrap()).unwrap();
    store
        .prepare_spawn(&ticket, fixture.binding.clone(), LeaseClock::now().unwrap())
        .unwrap();
    drop(store);
    let owner = controller(&path);
    fixture.fail_cleanup.store(true, Ordering::SeqCst);
    let effects: Arc<dyn Effects> = Arc::new(RecoveryEffects(Arc::clone(&fixture)));
    assert!(owner.recover(request, Arc::clone(&effects)).await.is_err());
    let failed = owner.inspect(request, cap.clone()).await.unwrap().unwrap();
    assert_eq!(failed.generation(), ticket.generation());
    assert_eq!(failed.state(), State::Closing);
    assert_eq!(owner.claim(request, cap.clone()).await.unwrap().state(), State::Closing);
    let recovering = {
        let owner = owner.clone();
        tokio::spawn(async move { owner.recover(request, effects).await })
    };
    tokio::time::timeout(Duration::from_secs(1), fixture.cleanup_entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    recovering.abort();
    let _cancelled = recovering.await;
    let closing = {
        let owner = owner.clone();
        tokio::spawn(async move { owner.close(request, cap).await })
    };
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 2);
    fixture.cleanup_release.add_permits(1);
    let recovered = closing.await.unwrap().unwrap();
    assert_eq!(recovered.state(), State::Closed);
    assert_eq!(recovered.generation(), ticket.generation());
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 2);
    assert!(!owner.0.works.lock().await.contains_key(&request));
}

struct Fixture {
    live: AtomicBool,
    creates: AtomicUsize,
    cleanups: AtomicUsize,
    reconciles: AtomicUsize,
    fail_cleanup: AtomicBool,
    report_pending: AtomicBool,
    reporter: std::sync::Mutex<Option<BindingReporter>>,
    create_entered: Semaphore,
    create_release: Semaphore,
    cancelled: Semaphore,
    cleanup_entered: Semaphore,
    cleanup_release: Semaphore,
    binding: VmBinding,
}

impl Fixture {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            live: AtomicBool::new(true),
            creates: AtomicUsize::new(0),
            cleanups: AtomicUsize::new(0),
            reconciles: AtomicUsize::new(0),
            fail_cleanup: AtomicBool::new(false),
            report_pending: AtomicBool::new(false),
            reporter: std::sync::Mutex::new(None),
            create_entered: Semaphore::new(0),
            create_release: Semaphore::new(0),
            cancelled: Semaphore::new(0),
            cleanup_entered: Semaphore::new(0),
            cleanup_release: Semaphore::new(0),
            binding: VmBinding::new("actual-canonical-id".into(), capsem_crypto::uuid_v4()).unwrap(),
        })
    }
}

#[tokio::test]
async fn ended_managed_spawn_cannot_renew_and_claim_joins_original_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([85; 32]);
    fixture.create_release.add_permits(1);
    let active = owner
        .create(
            request,
            cap.clone(),
            LeasePolicy::new(Duration::from_secs(10)).unwrap(),
            Arc::new(fixture.clone()),
        )
        .await
        .unwrap();
    assert_eq!(active.state(), State::Active);
    fixture.live.store(false, Ordering::SeqCst);
    assert!(matches!(
        owner.claim(request, Capability::from_bytes([86; 32])).await,
        Err(Failure::Forbidden)
    ));
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 0);
    let claiming = {
        let owner = owner.clone();
        let cap = cap.clone();
        tokio::spawn(async move { owner.claim(request, cap).await })
    };
    tokio::time::timeout(Duration::from_secs(1), fixture.cleanup_entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert!(!claiming.is_finished(), "claim must join the original cleanup barrier");
    claiming.abort();
    assert!(claiming.await.unwrap_err().is_cancelled());
    let closing = owner.claim(request, cap.clone()).await.unwrap();
    assert_eq!(closing.state(), State::Closing);
    fixture.cleanup_release.add_permits(1);
    let ended = owner.close(request, cap.clone()).await.unwrap();
    assert_eq!(ended.state(), State::Closed, "ended spawn remained renewable");
    assert_eq!(ended.generation(), active.generation());
    assert_eq!(
        ended.expires_wall_ms(),
        active.expires_wall_ms(),
        "absence must not renew the original lease"
    );
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
    assert_eq!(owner.claim(request, cap).await.unwrap().state(), State::Closed);
}

#[tokio::test]
async fn ended_spawn_with_failed_cleanup_cannot_regain_active_renewal() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([87; 32]);
    fixture.create_release.add_permits(1);
    let active = owner
        .create(
            request,
            cap.clone(),
            LeasePolicy::new(Duration::from_secs(10)).unwrap(),
            Arc::new(fixture.clone()),
        )
        .await
        .unwrap();
    fixture.live.store(false, Ordering::SeqCst);
    fixture.fail_cleanup.store(true, Ordering::SeqCst);
    assert!(matches!(
        owner.claim(request, cap.clone()).await,
        Err(Failure::CleanupFailed)
    ));
    let closing = owner.claim(request, cap.clone()).await.unwrap();
    assert_eq!(closing.state(), State::Closing);
    assert_eq!(closing.expires_wall_ms(), active.expires_wall_ms());
    fixture.cleanup_release.add_permits(1);
    assert_eq!(owner.close(request, cap).await.unwrap().state(), State::Closed);
}

impl Effects for Arc<Fixture> {
    fn is_live(&self, binding: &VmBinding) -> Result<bool> {
        anyhow::ensure!(*binding == self.binding, "fixture ownership changed");
        Ok(self.live.load(Ordering::SeqCst))
    }
    fn create(&self, _ticket: Ticket, cancel: CancellationToken, reporter: BindingReporter) -> EffectFuture<VmBinding> {
        let fixture = Arc::clone(self);
        Box::pin(async move {
            reporter.prepare(fixture.binding.clone()).await?;
            fixture.creates.fetch_add(1, Ordering::SeqCst);
            if fixture.report_pending.load(Ordering::SeqCst) {
                assert_eq!(
                    reporter.registered(fixture.binding.clone()).await?.state(),
                    State::Creating
                );
                *fixture.reporter.lock().unwrap() = Some(reporter);
            }
            fixture.create_entered.add_permits(1);
            tokio::select! {
                permit = fixture.create_release.acquire() => permit.unwrap().forget(),
                () = cancel.cancelled() => {
                    fixture.cancelled.add_permits(1);
                    fixture.create_release.acquire().await.unwrap().forget();
                }
            }
            Ok(fixture.binding.clone())
        })
    }
    fn cleanup(&self, _ticket: Ticket, binding: VmBinding) -> EffectFuture<()> {
        let fixture = Arc::clone(self);
        Box::pin(async move {
            assert_eq!(binding, fixture.binding);
            fixture.cleanups.fetch_add(1, Ordering::SeqCst);
            if fixture.fail_cleanup.swap(false, Ordering::SeqCst) {
                anyhow::bail!("injected cleanup failure");
            }
            fixture.cleanup_entered.add_permits(1);
            fixture.cleanup_release.acquire().await.unwrap().forget();
            Ok(())
        })
    }
    fn reconcile(&self, _ticket: Ticket, intent: Option<VmBinding>) -> EffectFuture<Option<VmBinding>> {
        // A create timeout may cancel immediately after prepare_spawn durably
        // records the intent but before this fixture increments `creates`.
        // Reconciliation must accept that exact durable intent in either
        // scheduling order and still reject a substituted binding.
        if let Some(intent) = intent {
            assert_eq!(intent, self.binding);
        }
        self.reconciles.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(None) })
    }
}

#[tokio::test]
async fn pending_vm_is_discoverable_immutable_and_closed_only_after_setup_retires() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    fixture.report_pending.store(true, Ordering::SeqCst);
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([29; 32]);
    let create = {
        let owner = owner.clone();
        let cap = cap.clone();
        let effects = effects(&fixture);
        tokio::spawn(async move {
            owner
                .create(
                    request,
                    cap,
                    LeasePolicy::new(Duration::from_secs(10)).unwrap(),
                    effects,
                )
                .await
        })
    };
    fixture.create_entered.acquire().await.unwrap().forget();
    let pending = owner.claim(request, cap.clone()).await.unwrap();
    assert_eq!(pending.state(), State::Creating);
    assert_eq!(pending.vm(), Some(&fixture.binding));
    let reporter = fixture.reporter.lock().unwrap().clone().unwrap();
    let different = VmBinding::new(fixture.binding.id().into(), capsem_crypto::uuid_v4()).unwrap();
    assert!(reporter.registered(different).await.is_err());
    assert_eq!(
        owner.inspect(request, cap.clone()).await.unwrap().unwrap().vm(),
        Some(&fixture.binding)
    );
    let close = {
        let owner = owner.clone();
        let cap = cap.clone();
        tokio::spawn(async move { owner.close(request, cap).await })
    };
    fixture.cancelled.acquire().await.unwrap().forget();
    assert_eq!(
        reporter.registered(fixture.binding.clone()).await.unwrap().state(),
        State::Closing
    );
    assert!(!close.is_finished());
    fixture.create_release.add_permits(1);
    fixture.cleanup_entered.acquire().await.unwrap().forget();
    assert!(!close.is_finished());
    fixture.cleanup_release.add_permits(1);
    let closed = close.await.unwrap().unwrap();
    let created = create.await.unwrap().unwrap();
    assert_eq!(created.state(), State::Closing);
    assert_eq!(closed.state(), State::Closed);
    assert_eq!(closed.generation(), pending.generation());
    assert!(reporter.registered(fixture.binding.clone()).await.is_err());
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn close_before_create_never_polls_effects_or_reuses_ownership() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([25; 32]);
    let tombstone = owner.close(request, cap.clone()).await.unwrap();
    assert!(owner
        .create(
            request,
            cap.clone(),
            LeasePolicy::new(Duration::from_secs(10)).unwrap(),
            effects(&fixture)
        )
        .await
        .is_err());
    let unchanged = owner.inspect(request, cap).await.unwrap().unwrap();
    assert_eq!(unchanged.state(), State::Closed);
    assert_eq!(unchanged.generation(), tombstone.generation());
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn renewal_reschedules_expiry_without_duplicate_creation() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    fixture.create_release.add_permits(1);
    fixture.cleanup_release.add_permits(1);
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([26; 32]);
    let policy = LeasePolicy::new(Duration::from_millis(400)).unwrap();
    let original = owner
        .create(request, cap.clone(), policy, effects(&fixture))
        .await
        .unwrap();
    assert!(owner
        .create(request, cap.clone(), policy, effects(&fixture))
        .await
        .is_err());
    tokio::time::sleep(Duration::from_millis(140)).await;
    assert!(owner.claim(request, Capability::from_bytes([27; 32])).await.is_err());
    let renewed = owner.claim(request, cap.clone()).await.unwrap();
    assert_eq!(renewed.generation(), original.generation());
    assert!(renewed.expires_wall_ms().unwrap() > original.expires_wall_ms().unwrap());
    tokio::time::sleep(Duration::from_millis(280)).await;
    assert_eq!(
        owner.inspect(request, cap.clone()).await.unwrap().unwrap().state(),
        State::Active
    );
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 0);
    owner.close(request, cap).await.unwrap();
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn interrupted_create_requires_an_absence_barrier_and_never_replays() {
    let root = tempfile::tempdir().unwrap();
    let owner = Controller::new(
        Registry::open(&root.path().join("ownership")).unwrap(),
        EffectBounds::new(Duration::from_millis(30), Duration::from_secs(2)).unwrap(),
    );
    let fixture = Fixture::new();
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([28; 32]);
    let policy = LeasePolicy::new(Duration::from_secs(10)).unwrap();
    assert!(matches!(
        owner.create(request, cap.clone(), policy, effects(&fixture)).await,
        Err(Failure::CreationFailed)
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if owner.inspect(request, cap.clone()).await.unwrap().unwrap().state() == State::Closed {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.reconciles.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 0);
    // The deadline may expire just before the effect increments `creates`.
    // What matters is that a closed request cannot start another effect.
    let creates_after_close = fixture.creates.load(Ordering::SeqCst);
    assert!(creates_after_close <= 1);
    assert!(owner.create(request, cap, policy, effects(&fixture)).await.is_err());
    assert_eq!(fixture.creates.load(Ordering::SeqCst), creates_after_close);
}

fn controller(root: &std::path::Path) -> Controller {
    Controller::new(
        Registry::open(root).unwrap(),
        EffectBounds::new(Duration::from_secs(2), Duration::from_secs(2)).unwrap(),
    )
}

fn effects(fixture: &Arc<Fixture>) -> Arc<dyn Effects> {
    Arc::new(Arc::clone(fixture))
}

#[tokio::test]
async fn corrupt_store_cannot_extend_a_live_managed_vm_past_its_finite_lease() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let owner = controller(&path);
    let fixture = Fixture::new();
    let request = capsem_crypto::uuid_v4();
    fixture.create_release.add_permits(1);
    let snapshot = owner
        .create(
            request,
            Capability::from_bytes([94; 32]),
            LeasePolicy::new(Duration::from_secs(30)).unwrap(),
            effects(&fixture),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.state(), State::Active);
    // Start the short expiry window only after activation: slow filesystem
    // admission must not consume the lease this corruption probe exercises.
    owner
        .set_lease_limit(LeasePolicy::new(Duration::from_millis(100)).unwrap())
        .await
        .unwrap();
    // Inject between owned I/O operations: an expiry transaction already
    // writing its temporary file must not replace the corruption afterward.
    let corrupt_path = path.join(format!("{request}.json"));
    owner
        .io(move |_| {
            std::fs::write(corrupt_path, b"corrupt ownership record")?;
            Ok(())
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), fixture.cleanup_entered.acquire())
        .await
        .expect("a store failure bypassed finite VM retirement")
        .unwrap()
        .forget();
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
    assert!(
        matches!(
            owner.inspect(request, Capability::from_bytes([94; 32])).await,
            Err(Failure::StoreUnavailable)
        ),
        "failed persistence must not manufacture a Closed acknowledgement"
    );
    fixture.cleanup_release.add_permits(1);
    let work = owner.0.works.lock().await.get(&request).unwrap().clone();
    let ticket = Ticket {
        request,
        generation: snapshot.generation(),
    };
    assert!(
        matches!(
            owner.cleanup_work(ticket, work, false).await,
            Err(Failure::StoreUnavailable)
        ),
        "the completed cleanup must report the failed durable acknowledgement"
    );
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn missing_store_close_authenticates_memory_and_retires_only_the_original_vm() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let owner = controller(&path);
    let fixture = Fixture::new();
    let request = capsem_crypto::uuid_v4();
    let capability = Capability::from_bytes([95; 32]);
    fixture.create_release.add_permits(1);
    owner
        .create(
            request,
            capability.clone(),
            LeasePolicy::new(Duration::from_secs(10)).unwrap(),
            effects(&fixture),
        )
        .await
        .unwrap();
    std::fs::remove_file(path.join(format!("{request}.json"))).unwrap();
    assert!(
        matches!(
            owner.close(request, Capability::from_bytes([96; 32])).await,
            Err(Failure::Forbidden)
        ),
        "missing storage let an unauthenticated caller overwrite live ownership"
    );
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 0);
    let closing = {
        let owner = owner.clone();
        tokio::spawn(async move { owner.close(request, capability).await })
    };
    tokio::time::timeout(Duration::from_secs(1), fixture.cleanup_entered.acquire())
        .await
        .expect("authenticated close did not retire its original VM")
        .unwrap()
        .forget();
    assert!(
        !closing.is_finished(),
        "close returned before the trusted effect's cleanup barrier"
    );
    fixture.cleanup_release.add_permits(1);
    assert!(
        matches!(closing.await.unwrap(), Err(Failure::StoreUnavailable)),
        "missing original persistence must not acknowledge a replacement generation as Closed"
    );
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ownership_denials_are_typed_and_cannot_retire_a_live_generation() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    let request = capsem_crypto::uuid_v4();
    let capability = Capability::from_bytes([91; 32]);
    let wrong = Capability::from_bytes([92; 32]);
    let policy = LeasePolicy::new(Duration::from_secs(10)).unwrap();
    fixture.create_release.add_permits(1);
    let created = owner
        .create(request, capability.clone(), policy, effects(&fixture))
        .await
        .unwrap();
    for denial in [
        owner.claim(request, wrong.clone()).await.unwrap_err(),
        owner.close(request, wrong.clone()).await.unwrap_err(),
        owner
            .create(request, wrong.clone(), policy, effects(&fixture))
            .await
            .unwrap_err(),
    ] {
        assert!(matches!(denial, Failure::Forbidden), "{denial:?}");
    }
    assert!(matches!(
        owner
            .create(request, capability.clone(), policy, effects(&fixture))
            .await,
        Err(Failure::DuplicateRequest)
    ));
    assert_eq!(
        owner.claim(request, capability.clone()).await.unwrap().generation(),
        created.generation()
    );
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.cancelled.available_permits(), 0);
    fixture.cleanup_release.add_permits(1);
    assert_eq!(
        owner.close(request, capability.clone()).await.unwrap().state(),
        State::Closed
    );
    // Terminal records still authenticate and reject late replay after the
    // in-memory continuation has retired.
    assert!(matches!(
        owner.create(request, wrong, policy, effects(&fixture)).await,
        Err(Failure::Forbidden)
    ));
    assert!(matches!(
        owner.create(request, capability, policy, effects(&fixture)).await,
        Err(Failure::DuplicateRequest)
    ));
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unknown_claim_and_broken_store_have_distinct_nonsecret_errors() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ownership");
    let owner = controller(&path);
    let request = capsem_crypto::uuid_v4();
    let capability = Capability::from_bytes([93; 32]);
    assert!(matches!(
        owner.claim(request, capability.clone()).await,
        Err(Failure::NotFound)
    ));
    std::fs::write(path.join(format!("{request}.json")), b"corrupt-private-record-secret").unwrap();
    let failure = owner.claim(request, capability).await.unwrap_err();
    assert!(matches!(failure, Failure::StoreUnavailable));
    assert_eq!(failure.to_string(), "managed ownership store unavailable");
}

#[tokio::test]
async fn disconnected_create_and_concurrent_close_share_one_awaited_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    let request = capsem_crypto::uuid_v4();
    let capability = Capability::from_bytes([21; 32]);
    let client = {
        let owner = owner.clone();
        let capability = capability.clone();
        let effects = effects(&fixture);
        tokio::spawn(async move {
            owner
                .create(
                    request,
                    capability,
                    LeasePolicy::new(Duration::from_secs(10)).unwrap(),
                    effects,
                )
                .await
        })
    };
    fixture.create_entered.acquire().await.unwrap().forget();
    client.abort();
    assert!(client.await.unwrap_err().is_cancelled());
    assert!(owner.close(request, Capability::from_bytes([22; 32])).await.is_err());
    assert_eq!(fixture.cancelled.available_permits(), 0);
    let close1 = {
        let owner = owner.clone();
        let cap = capability.clone();
        tokio::spawn(async move { owner.close(request, cap).await })
    };
    let close2 = {
        let owner = owner.clone();
        let cap = capability.clone();
        tokio::spawn(async move { owner.close(request, cap).await })
    };
    fixture.cancelled.acquire().await.unwrap().forget();
    assert!(!close1.is_finished() && !close2.is_finished());
    fixture.create_release.add_permits(1);
    fixture.cleanup_entered.acquire().await.unwrap().forget();
    assert!(!close1.is_finished() && !close2.is_finished());
    fixture.cleanup_release.add_permits(1);
    let first = close1.await.unwrap().unwrap();
    let second = close2.await.unwrap().unwrap();
    assert_eq!(first.state(), State::Closed);
    assert_eq!(first.generation(), second.generation());
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
    assert_eq!(
        owner.inspect(request, capability).await.unwrap().unwrap().state(),
        State::Closed
    );
}

#[tokio::test]
async fn cleanup_failure_is_retryable_and_never_reports_closed() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    fixture.create_release.add_permits(1);
    fixture.cleanup_release.add_permits(1);
    fixture.fail_cleanup.store(true, Ordering::SeqCst);
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([23; 32]);
    owner
        .create(
            request,
            cap.clone(),
            LeasePolicy::new(Duration::from_secs(10)).unwrap(),
            effects(&fixture),
        )
        .await
        .unwrap();
    assert!(owner.close(request, cap.clone()).await.is_err());
    assert_eq!(
        owner.inspect(request, cap.clone()).await.unwrap().unwrap().state(),
        State::Closing
    );
    assert_eq!(owner.close(request, cap).await.unwrap().state(), State::Closed);
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn finite_expiry_cleans_up_without_a_client() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    fixture.create_release.add_permits(1);
    fixture.cleanup_release.add_permits(1);
    let request = capsem_crypto::uuid_v4();
    let cap = Capability::from_bytes([24; 32]);
    let created = owner
        .create(
            request,
            cap.clone(),
            LeasePolicy::new(Duration::from_millis(400)).unwrap(),
            effects(&fixture),
        )
        .await
        .unwrap();
    assert_eq!(created.state(), State::Active);
    tokio::time::timeout(Duration::from_secs(1), fixture.cleanup_entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if owner.inspect(request, cap.clone()).await.unwrap().unwrap().state() == State::Closed {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
}
