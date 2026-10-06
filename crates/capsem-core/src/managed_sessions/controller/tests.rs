use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Semaphore;

struct RecoveryEffects(Arc<Fixture>);
impl Effects for RecoveryEffects {
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
    let request = Uuid::new_v4();
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
            binding: VmBinding::new("actual-canonical-id".into(), Uuid::new_v4()).unwrap(),
        })
    }
}

impl Effects for Arc<Fixture> {
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
        if self.creates.load(Ordering::SeqCst) > 0 {
            assert_eq!(intent.as_ref(), Some(&self.binding));
        } else {
            assert!(intent.is_none());
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
    let request = Uuid::new_v4();
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
    let different = VmBinding::new(fixture.binding.id().into(), Uuid::new_v4()).unwrap();
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
    let request = Uuid::new_v4();
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
    let request = Uuid::new_v4();
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
    let request = Uuid::new_v4();
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
    assert!(owner.create(request, cap, policy, effects(&fixture)).await.is_err());
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
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
async fn disconnected_create_and_concurrent_close_share_one_awaited_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let owner = controller(&root.path().join("ownership"));
    let fixture = Fixture::new();
    let request = Uuid::new_v4();
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
    let request = Uuid::new_v4();
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
    let request = Uuid::new_v4();
    let cap = Capability::from_bytes([24; 32]);
    owner
        .create(
            request,
            cap.clone(),
            LeasePolicy::new(Duration::from_millis(40)).unwrap(),
            effects(&fixture),
        )
        .await
        .unwrap();
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
