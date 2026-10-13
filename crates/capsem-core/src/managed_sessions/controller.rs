//! Request-independent managed lifecycle ownership. Service adapters implement
//! the effects using their existing process, session and grant owners.

use super::*;
use futures::FutureExt;
use std::{collections::HashMap, future::Future, panic::AssertUnwindSafe, pin::Pin, sync::Arc, time::Duration};
use tokio::sync::{watch, Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

pub type EffectFuture<T> = Pin<Box<dyn Future<Output = Result<T>> + Send>>;

/// Effects never receive the caller's capability. Cleanup must await process
/// exit, setup retirement, session cleanup and generation-scoped grant
/// revocation. Reconcile must retire interrupted setup before reporting absence.
pub trait Effects: Send + Sync {
    /// Nonblocking observation of the exact tracked ephemeral spawn. Absence
    /// or an error requires cleanup; this is never a retirement/closure proof.
    fn is_live(&self, binding: &VmBinding) -> Result<bool>;
    fn create(&self, ticket: Ticket, cancel: CancellationToken, reporter: BindingReporter) -> EffectFuture<VmBinding>;
    fn cleanup(&self, ticket: Ticket, binding: VmBinding) -> EffectFuture<()>;
    fn reconcile(&self, ticket: Ticket, intent: Option<VmBinding>) -> EffectFuture<Option<VmBinding>>;
}

#[derive(Clone, Copy, Debug)]
pub struct EffectBounds {
    create: Duration,
    cleanup: Duration,
}

impl EffectBounds {
    pub fn new(create: Duration, cleanup: Duration) -> Result<Self> {
        ensure!(
            !create.is_zero() && !cleanup.is_zero(),
            "managed effect bounds must be positive"
        );
        let now = tokio::time::Instant::now();
        ensure!(
            now.checked_add(create).is_some() && now.checked_add(cleanup).is_some(),
            "managed effect bounds overflow"
        );
        Ok(Self { create, cleanup })
    }
}

/// Generic errors deliberately omit effect messages, which can contain
/// credential-bearing upstream diagnostics.
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum Failure {
    #[error("managed ownership capability denied")]
    Forbidden,
    #[error("managed request not found")]
    NotFound,
    #[error("managed ownership store unavailable")]
    StoreUnavailable,
    #[error("managed request already registered; recover ownership instead")]
    DuplicateRequest,
    #[error("managed creation failed; ownership requires reconciliation")]
    CreationFailed,
    #[error("managed cleanup failed; close remains pending")]
    CleanupFailed,
    #[error("managed ownership recovery is pending")]
    RecoveryPending,
}

type ControlResult<T> = std::result::Result<T, Failure>;

#[derive(Clone)]
pub struct Controller(Arc<Inner>);

struct Inner {
    registry: Arc<Registry>,
    io: Arc<Semaphore>,
    works: Mutex<HashMap<Uuid, Arc<Work>>>,
    bounds: EffectBounds,
}

struct Work {
    recovering: bool,
    capability_hash: Option<capsem_crypto::blake3::Hash>,
    ticket: std::sync::Mutex<Option<Ticket>>,
    binding: std::sync::Mutex<Option<VmBinding>>,
    intent: std::sync::Mutex<Option<VmBinding>>,
    effects: Arc<dyn Effects>,
    cancel: CancellationToken,
    created: watch::Sender<Option<ControlResult<Snapshot>>>,
    deadline: watch::Sender<Option<LeaseDeadline>>,
    cleanup: Mutex<Option<watch::Sender<Option<ControlResult<Snapshot>>>>>,
}

impl Work {
    fn update_deadline(&self, deadline: LeaseDeadline) {
        self.deadline.send_if_modified(|current| {
            if current.is_none_or(|previous| deadline.revision > previous.revision) {
                *current = Some(deadline);
                true
            } else {
                false
            }
        });
    }

    fn remember_binding(&self, binding: VmBinding) -> ControlResult<()> {
        if self.intent.lock().map_err(|_| Failure::RecoveryPending)?.as_ref() != Some(&binding) {
            return Err(Failure::CreationFailed);
        }
        let mut known = self.binding.lock().map_err(|_| Failure::RecoveryPending)?;
        if known.as_ref().is_some_and(|original| original != &binding) {
            return Err(Failure::CreationFailed);
        }
        *known = Some(binding);
        drop(known);
        Ok(())
    }
}

/// A trusted service effect reports the actual VM after registration and
/// before workload readiness. This handle contains no ownership capability.
#[derive(Clone)]
pub struct BindingReporter {
    owner: Controller,
    work: std::sync::Weak<Work>,
    ticket: Ticket,
}

impl BindingReporter {
    /// Must complete before any VM spawn side effect. The selected canonical
    /// ID and generation survive a crash before registration can be reported.
    pub async fn prepare(&self, binding: VmBinding) -> ControlResult<Snapshot> {
        let work = self.work.upgrade().ok_or(Failure::RecoveryPending)?;
        {
            let mut intent = work.intent.lock().map_err(|_| Failure::RecoveryPending)?;
            if intent.as_ref().is_some_and(|original| original != &binding) {
                return Err(Failure::CreationFailed);
            }
            *intent = Some(binding.clone());
        }
        let ticket = self.ticket.clone();
        self.owner
            .io(move |registry| registry.prepare_spawn(&ticket, binding, LeaseClock::now()?))
            .await
    }
    pub async fn registered(&self, binding: VmBinding) -> ControlResult<Snapshot> {
        let work = self.work.upgrade().ok_or(Failure::RecoveryPending)?;
        work.remember_binding(binding.clone())?;
        let ticket = self.ticket.clone();
        let snapshot = self
            .owner
            .io(move |registry| registry.bind_pending(&ticket, binding, LeaseClock::now()?))
            .await?;
        if snapshot.state == State::Closing {
            work.cancel.cancel();
        }
        Ok(snapshot)
    }
}

impl Controller {
    pub fn new(registry: Registry, bounds: EffectBounds) -> Self {
        Self(Arc::new(Inner {
            registry: Arc::new(registry),
            io: Arc::new(Semaphore::new(1)),
            works: Mutex::new(HashMap::new()),
            bounds,
        }))
    }

    /// Once polled, the service-owned worker survives cancellation of this
    /// caller. Only the worker polls create, after durable lease admission.
    pub async fn create(
        &self,
        request: Uuid,
        capability: Capability,
        policy: LeasePolicy,
        effects: Arc<dyn Effects>,
    ) -> ControlResult<Snapshot> {
        let work = Arc::new(Work {
            recovering: false,
            capability_hash: Some(capability.hash(request)),
            ticket: std::sync::Mutex::new(None),
            binding: std::sync::Mutex::new(None),
            intent: std::sync::Mutex::new(None),
            effects,
            cancel: CancellationToken::new(),
            created: watch::channel(None).0,
            deadline: watch::channel(None).0,
            cleanup: Mutex::new(None),
        });
        {
            let mut works = self.0.works.lock().await;
            if let Some(existing) = works.get(&request) {
                let existing_hash = existing.capability_hash;
                drop(works);
                if let Some(hash) = existing_hash {
                    if hash != capability.hash(request) {
                        return Err(Failure::Forbidden);
                    }
                } else {
                    self.inspect(request, capability).await?.ok_or(Failure::NotFound)?;
                }
                return Err(Failure::DuplicateRequest);
            }
            works.insert(request, Arc::clone(&work));
            drop(works);
            let owner = self.clone();
            let worker = Arc::clone(&work);
            tokio::spawn(async move {
                owner.create_worker(request, capability, policy, worker).await;
            });
        }
        wait_created(&work).await
    }

    pub async fn inspect(&self, request: Uuid, capability: Capability) -> ControlResult<Option<Snapshot>> {
        self.io(move |registry| registry.inspect(request, &capability)).await
    }

    /// Reload never renews a lease. Timer revisions prevent delayed admission
    /// or claim completions from overwriting a newer tightened deadline.
    pub async fn set_lease_limit(&self, policy: LeasePolicy) -> ControlResult<usize> {
        let revision = self
            .0
            .registry
            .publish_lease_limit(policy)
            .map_err(|_| Failure::StoreUnavailable)?;
        let owner = self.clone();
        tokio::spawn(async move { owner.limit_worker(revision).await })
            .await
            .map_err(|_| Failure::StoreUnavailable)?
    }

    pub async fn deny_lease_admission(&self) -> ControlResult<()> {
        let revision = self
            .0
            .registry
            .deny_lease_admission()
            .map_err(|_| Failure::StoreUnavailable)?;
        let owner = self.clone();
        tokio::spawn(async move {
            let works = owner.0.works.lock().await;
            if owner
                .0
                .registry
                .admission_denied(revision)
                .map_err(|_| Failure::StoreUnavailable)?
            {
                for work in works.values() {
                    work.cancel.cancel();
                }
            }
            drop(works);
            Ok(())
        })
        .await
        .map_err(|_| Failure::StoreUnavailable)?
    }

    async fn limit_worker(&self, revision: u64) -> ControlResult<usize> {
        let updated = self
            .io(move |registry| {
                registry
                    .apply_lease_limit(LeaseClock::now()?)?
                    .into_iter()
                    .map(|snapshot| {
                        let ticket = Ticket {
                            request: snapshot.request,
                            generation: snapshot.generation,
                        };
                        let deadline = if matches!(snapshot.state, State::Reserved | State::Creating | State::Active)
                            && snapshot.lease_duration().is_some()
                        {
                            Some(registry.runtime_deadline(&ticket)?)
                        } else {
                            None
                        };
                        Ok((snapshot, deadline))
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .await;
        let works = self.0.works.lock().await;
        match updated {
            Ok(updated) => {
                let count = updated.len();
                for (snapshot, deadline) in updated {
                    if let Some(work) = works.get(&snapshot.request) {
                        match deadline {
                            Some(deadline) => work.update_deadline(deadline),
                            None => work.cancel.cancel(),
                        }
                    }
                }
                drop(works);
                Ok(count)
            }
            Err(error) => {
                // Persistence uncertainty cannot preserve permissive live
                // effects. The original owner still performs awaited cleanup.
                if self
                    .0
                    .registry
                    .policy_revision()
                    .map_err(|_| Failure::StoreUnavailable)?
                    == revision
                {
                    for work in works.values() {
                        work.cancel.cancel();
                    }
                }
                drop(works);
                Err(error)
            }
        }
    }

    pub async fn recovery_inventory(&self) -> ControlResult<Vec<Snapshot>> {
        self.io(Registry::recovery_inventory).await
    }

    /// Trusted host-side ownership query for persistence admission. Live
    /// prepared authority remains binding even if its durable record is lost.
    /// Failed inventory cannot establish that an unrelated VM is unowned.
    pub async fn owns_vm(&self, binding: &VmBinding) -> ControlResult<bool> {
        let works = self.0.works.lock().await;
        for work in works.values() {
            let intent = work.intent.lock().map_err(|_| Failure::RecoveryPending)?;
            let owned = intent.as_ref() == Some(binding);
            drop(intent);
            if owned {
                drop(works);
                return Ok(true);
            }
        }
        drop(works);
        Ok(self
            .recovery_inventory()
            .await?
            .iter()
            .any(|snapshot| snapshot.vm() == Some(binding) || snapshot.spawn_intent() == Some(binding)))
    }

    pub async fn prune_closed(&self, retention: Duration) -> ControlResult<usize> {
        self.io(move |registry| registry.prune_closed(retention, LeaseClock::now()?))
            .await
    }

    pub async fn prune_native_retirements(&self, path: std::path::PathBuf) -> ControlResult<usize> {
        self.io(move |registry| {
            let store = crate::session::spawn_owner::RetirementStore::open(&path)?;
            registry.prune_native_retirements(&store)
        })
        .await
    }

    /// Trusted startup recovery after prior continuations have retired. This
    /// owns cleanup only; it never polls create or renews the old lease.
    pub async fn recover(&self, request: Uuid, effects: Arc<dyn Effects>) -> ControlResult<Snapshot> {
        let mut works = self.0.works.lock().await;
        if let Some(work) = works.get(&request).cloned() {
            drop(works);
            if !work.recovering {
                return Err(Failure::RecoveryPending);
            }
            let snapshot = wait_created(&work).await?;
            let ticket = Ticket {
                request,
                generation: snapshot.generation,
            };
            return self.cleanup_work(ticket, work, true).await;
        }
        let work = Arc::new(Work {
            recovering: true,
            capability_hash: None,
            ticket: std::sync::Mutex::new(None),
            binding: std::sync::Mutex::new(None),
            intent: std::sync::Mutex::new(None),
            effects,
            cancel: CancellationToken::new(),
            created: watch::channel(None).0,
            deadline: watch::channel(None).0,
            cleanup: Mutex::new(None),
        });
        works.insert(request, Arc::clone(&work));
        drop(works);
        let (done, mut result) = watch::channel(None);
        let owner = self.clone();
        tokio::spawn(async move {
            done.send_replace(Some(owner.recovery_worker(request, work).await));
        });
        loop {
            let outcome = result.borrow_and_update().clone();
            if let Some(outcome) = outcome {
                return outcome;
            }
            result.changed().await.map_err(|_| Failure::RecoveryPending)?;
        }
    }

    async fn recovery_worker(&self, request: Uuid, work: Arc<Work>) -> ControlResult<Snapshot> {
        let recovered = self
            .io(move |registry| {
                registry
                    .recover_interrupted(request)?
                    .context("managed recovery request missing")
            })
            .await;
        let (ticket, snapshot) = match recovered {
            Ok(recovered) => recovered,
            Err(error) => {
                work.created.send_replace(Some(Err(error)));
                self.remove_work(request, &work).await;
                return Err(error);
            }
        };
        *work.ticket.lock().map_err(|_| Failure::RecoveryPending)? = Some(ticket.clone());
        *work.binding.lock().map_err(|_| Failure::RecoveryPending)? = snapshot.vm.clone();
        *work.intent.lock().map_err(|_| Failure::RecoveryPending)? = snapshot.spawn_intent.clone();
        work.cancel.cancel();
        work.created.send_replace(Some(Ok(snapshot.clone())));
        if snapshot.state == State::Closed {
            self.remove_work(request, &work).await;
            return Ok(snapshot);
        }
        self.cleanup_work(ticket, work, true).await
    }

    /// A close also outlives its HTTP waiter. Authentication precedes any
    /// cancellation; all concurrent closes join the same cleanup owner.
    pub async fn close(&self, request: Uuid, capability: Capability) -> ControlResult<Snapshot> {
        let owner = self.clone();
        tokio::spawn(async move { owner.close_worker(request, capability).await })
            .await
            .map_err(|_| Failure::CleanupFailed)?
    }

    pub async fn claim(&self, request: Uuid, capability: Capability) -> ControlResult<Snapshot> {
        let snapshot = self
            .inspect(request, capability.clone())
            .await?
            .ok_or(Failure::NotFound)?;
        if !matches!(snapshot.state, State::Reserved | State::Creating | State::Active) {
            return Ok(snapshot);
        }
        let work = self
            .0
            .works
            .lock()
            .await
            .get(&request)
            .cloned()
            .ok_or(Failure::RecoveryPending)?;
        let ticket = work
            .ticket
            .lock()
            .map_err(|_| Failure::RecoveryPending)?
            .clone()
            .ok_or(Failure::RecoveryPending)?;
        if ticket.generation != snapshot.generation {
            return Err(Failure::RecoveryPending);
        }
        if snapshot.state == State::Active {
            let binding = snapshot.vm.as_ref().ok_or(Failure::RecoveryPending)?;
            if !work.effects.is_live(binding).unwrap_or(false) {
                return self.close(request, capability).await;
            }
        }
        let (renewed, deadline) = self
            .io(move |registry| {
                let renewed = registry.renew_lease(request, &capability, LeaseClock::now()?)?;
                let deadline = if matches!(renewed.state, State::Reserved | State::Creating | State::Active) {
                    Some(registry.runtime_deadline(&ticket)?)
                } else {
                    None
                };
                Ok((renewed, deadline))
            })
            .await?;
        if let Some(deadline) = deadline {
            work.update_deadline(deadline);
        } else {
            work.cancel.cancel();
        }
        Ok(renewed)
    }

    async fn io<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&Registry) -> Result<T> + Send + 'static,
    ) -> ControlResult<T> {
        let permit = Arc::clone(&self.0.io)
            .acquire_owned()
            .await
            .map_err(|_| Failure::StoreUnavailable)?;
        let registry = Arc::clone(&self.0.registry);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation(&registry)
        })
        .await
        .map_err(|_| Failure::StoreUnavailable)?
        .map_err(|error| match error.downcast_ref::<Rejection>() {
            Some(Rejection::Forbidden) => Failure::Forbidden,
            Some(Rejection::DuplicateRequest) => Failure::DuplicateRequest,
            None => Failure::StoreUnavailable,
        })
    }

    async fn create_worker(&self, request: Uuid, capability: Capability, policy: LeasePolicy, work: Arc<Work>) {
        let admission = self
            .io(move |registry| {
                let Reservation::New(ticket) = registry.reserve(request, &capability)? else {
                    return Err(Rejection::DuplicateRequest.into());
                };
                registry.start_lease(&ticket, policy, LeaseClock::now()?)?;
                ensure!(
                    registry.begin_create(&ticket, LeaseClock::now()?)?,
                    "managed creation closed before admission"
                );
                let deadline = registry.runtime_deadline(&ticket)?;
                Ok((ticket, deadline))
            })
            .await;
        let (ticket, deadline) = match admission {
            Ok(admission) => admission,
            Err(error) => {
                work.created.send_replace(Some(Err(error)));
                self.remove_work(request, &work).await;
                return;
            }
        };
        *work.ticket.lock().expect("worker ticket lock poisoned") = Some(ticket.clone());
        work.update_deadline(deadline);
        let owner = self.clone();
        let expiry = Arc::clone(&work);
        let expiry_ticket = ticket.clone();
        tokio::spawn(async move {
            owner.expiry_worker(expiry_ticket, expiry).await;
        });
        let effects = Arc::clone(&work.effects);
        let effect_ticket = ticket.clone();
        let cancel = work.cancel.clone();
        let reporter = BindingReporter {
            owner: self.clone(),
            work: Arc::downgrade(&work),
            ticket: ticket.clone(),
        };
        let created = bounded(self.0.bounds.create, async move {
            ensure!(!cancel.is_cancelled(), "managed creation cancelled before effects");
            effects.create(effect_ticket, cancel, reporter).await
        })
        .await;
        let result = match created {
            Ok(binding) => match work.remember_binding(binding.clone()) {
                Ok(()) => {
                    let bind_ticket = ticket.clone();
                    self.io(move |registry| registry.bind_created(&bind_ticket, binding, LeaseClock::now()?))
                        .await
                }
                Err(error) => Err(error),
            },
            Err(()) => {
                let failed_ticket = ticket.clone();
                let _result = self.io(move |registry| registry.creation_failed(&failed_ticket)).await;
                Err(Failure::CreationFailed)
            }
        };
        let needs_cleanup = !result.as_ref().is_ok_and(|snapshot| snapshot.state == State::Active);
        work.created.send_replace(Some(result));
        if needs_cleanup {
            work.cancel.cancel();
        }
    }

    async fn close_worker(&self, request: Uuid, capability: Capability) -> ControlResult<Snapshot> {
        let work = self.0.works.lock().await.get(&request).cloned();
        let authenticated = work.as_ref().is_some_and(|work| work.capability_hash.is_some());
        if let Some(hash) = work.as_ref().and_then(|work| work.capability_hash) {
            if hash != capability.hash(request) {
                return Err(Failure::Forbidden);
            }
        }
        let snapshot = self.io(move |registry| registry.close(request, &capability)).await;
        if let Some(work) = work {
            // Only independently authenticated live ownership can retire under
            // a failed store. Recovery still needs its durable authority.
            if snapshot.is_ok() || authenticated {
                work.cancel.cancel();
                let _creation = wait_created(&work).await;
                let original = work.ticket.lock().map_err(|_| Failure::RecoveryPending)?.clone();
                if let Some(ticket) = original {
                    return self.cleanup_work(ticket, work, true).await;
                }
            }
        }
        let snapshot = snapshot?;
        if snapshot.state == State::Closed {
            return Ok(snapshot);
        }
        let ticket = Ticket {
            request,
            generation: snapshot.generation,
        };
        let latest = self.snapshot(ticket).await?;
        if latest.state == State::Closed {
            Ok(latest)
        } else {
            Err(Failure::RecoveryPending)
        }
    }

    async fn snapshot(&self, ticket: Ticket) -> ControlResult<Snapshot> {
        self.io(move |registry| Ok(Snapshot::from(&registry.ticket_record(&ticket)?)))
            .await
    }

    async fn cleanup_work(&self, ticket: Ticket, work: Arc<Work>, retry: bool) -> ControlResult<Snapshot> {
        let mut cleanup = work.cleanup.lock().await;
        let current = cleanup.as_ref().map(|result| result.borrow().clone());
        let start = match current {
            None => true,
            Some(Some(Err(_))) => retry,
            Some(None | Some(Ok(_))) => false,
        };
        if start {
            let result = watch::channel(None).0;
            *cleanup = Some(result.clone());
            let owner = self.clone();
            let attempt = Arc::clone(&work);
            tokio::spawn(async move {
                result.send_replace(Some(owner.cleanup_attempt(ticket, attempt).await));
            });
        }
        let mut result = cleanup.as_ref().ok_or(Failure::CleanupFailed)?.subscribe();
        drop(cleanup);
        loop {
            let value = result.borrow_and_update().clone();
            if let Some(value) = value {
                return value;
            }
            result.changed().await.map_err(|_| Failure::CleanupFailed)?;
        }
    }

    async fn cleanup_attempt(&self, ticket: Ticket, work: Arc<Work>) -> ControlResult<Snapshot> {
        let _creation = wait_created(&work).await;
        let original = work
            .ticket
            .lock()
            .map_err(|_| Failure::RecoveryPending)?
            .clone()
            .ok_or(Failure::RecoveryPending)?;
        if original.generation != ticket.generation {
            return Err(Failure::RecoveryPending);
        }
        let snapshot = self.snapshot(ticket.clone()).await;
        let effects = Arc::clone(&work.effects);
        let cleanup_ticket = ticket.clone();
        let known = work
            .binding
            .lock()
            .map_err(|_| Failure::RecoveryPending)?
            .clone()
            .or_else(|| snapshot.as_ref().ok().and_then(|snapshot| snapshot.vm.clone()));
        let intent = work
            .intent
            .lock()
            .map_err(|_| Failure::RecoveryPending)?
            .clone()
            .or_else(|| {
                snapshot
                    .as_ref()
                    .ok()
                    .and_then(|snapshot| snapshot.spawn_intent.clone())
            });
        let publish_binding = snapshot.as_ref().is_ok_and(|snapshot| snapshot.state != State::Closed);
        let registry_owner = self.clone();
        let (binding, publication_error) = bounded(self.0.bounds.cleanup, async move {
            let binding = match known {
                Some(binding) => Some(binding),
                None => effects.reconcile(cleanup_ticket.clone(), intent.clone()).await?,
            };
            let mut publication_error = None;
            if let Some(binding) = &binding {
                ensure!(
                    intent.as_ref().is_none_or(|original| original == binding),
                    "managed cleanup target differs from trusted spawn intent"
                );
                if publish_binding {
                    let bind_ticket = cleanup_ticket.clone();
                    let verified = binding.clone();
                    publication_error = registry_owner
                        .io(move |registry| registry.reconcile_vm(&bind_ticket, verified))
                        .await
                        .err();
                }
                effects.cleanup(cleanup_ticket, binding.clone()).await?;
            }
            Ok((binding, publication_error))
        })
        .await
        .map_err(|()| Failure::CleanupFailed)?;
        // Trusted live effects must retire even if disk authority is missing.
        // That grants no permission to acknowledge Closed or recreate a VM.
        let snapshot = snapshot?;
        if let Some(error) = publication_error {
            return Err(error);
        }
        let cleaned = if snapshot.state == State::Closed {
            snapshot
        } else if let Some(binding) = binding {
            let cleanup_ticket = ticket.clone();
            self.io(move |registry| registry.complete_cleanup(&cleanup_ticket, &binding))
                .await?
        } else {
            let cleanup_ticket = ticket.clone();
            self.io(move |registry| registry.confirm_absent(&cleanup_ticket))
                .await?
        };
        self.remove_work(ticket.request, &work).await;
        Ok(cleaned)
    }

    async fn expiry_worker(&self, ticket: Ticket, work: Arc<Work>) {
        let mut deadline = work.deadline.subscribe();
        let mut retry = false;
        loop {
            let next = *deadline.borrow_and_update();
            let elapsed = async {
                match next {
                    Some(next) => tokio::time::sleep_until(tokio::time::Instant::from_std(next.at)).await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                () = work.cancel.cancelled() => {},
                () = elapsed => {},
                changed = deadline.changed() => { if changed.is_err() { return; } continue; }
            }
            let expire_ticket = ticket.clone();
            let cancelled = work.cancel.is_cancelled();
            let snapshot = self
                .io(move |registry| {
                    if cancelled {
                        registry.begin_owned_cleanup(&expire_ticket)
                    } else {
                        registry.expire_lease(&expire_ticket, LeaseClock::now()?)
                    }
                })
                .await;
            let closing = matches!(snapshot, Ok(ref snapshot) if matches!(snapshot.state, State::Closing | State::Closed | State::Unknown));
            if closing || snapshot.is_err() || work.cancel.is_cancelled() {
                work.cancel.cancel();
                match self.cleanup_work(ticket.clone(), Arc::clone(&work), retry).await {
                    Ok(_) => return,
                    Err(error) => tracing::warn!(request = %ticket.request, %error, "managed cleanup remains pending"),
                }
                retry = true;
            }
            if snapshot.is_err() || work.cancel.is_cancelled() {
                tokio::time::sleep(self.0.bounds.cleanup).await;
            }
        }
    }

    async fn remove_work(&self, request: Uuid, work: &Arc<Work>) {
        let mut works = self.0.works.lock().await;
        if works.get(&request).is_some_and(|current| Arc::ptr_eq(current, work)) {
            works.remove(&request);
        }
    }
}

async fn wait_created(work: &Work) -> ControlResult<Snapshot> {
    let mut result = work.created.subscribe();
    loop {
        let value = result.borrow_and_update().clone();
        if let Some(value) = value {
            return value;
        }
        result.changed().await.map_err(|_| Failure::CreationFailed)?;
    }
}

async fn bounded<T>(timeout: Duration, effect: impl Future<Output = Result<T>> + Send) -> std::result::Result<T, ()> {
    match tokio::time::timeout(timeout, AssertUnwindSafe(effect).catch_unwind()).await {
        Ok(Ok(Ok(value))) => Ok(value),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests;
