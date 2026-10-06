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
    ticket: std::sync::Mutex<Option<Ticket>>,
    binding: std::sync::Mutex<Option<VmBinding>>,
    effects: Arc<dyn Effects>,
    cancel: CancellationToken,
    created: watch::Sender<Option<ControlResult<Snapshot>>>,
    deadline: watch::Sender<tokio::time::Instant>,
    cleanup: Mutex<Option<watch::Sender<Option<ControlResult<Snapshot>>>>>,
}

impl Work {
    fn remember_binding(&self, binding: VmBinding) -> ControlResult<()> {
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
        self.work.upgrade().ok_or(Failure::RecoveryPending)?;
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
            ticket: std::sync::Mutex::new(None),
            binding: std::sync::Mutex::new(None),
            effects,
            cancel: CancellationToken::new(),
            created: watch::channel(None).0,
            deadline: watch::channel(tokio::time::Instant::now()).0,
            cleanup: Mutex::new(None),
        });
        {
            let mut works = self.0.works.lock().await;
            if works.contains_key(&request) {
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
            .ok_or(Failure::RecoveryPending)?;
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
            work.deadline.send_replace(tokio::time::Instant::from_std(deadline));
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
        .map_err(|_| Failure::StoreUnavailable)
    }

    async fn create_worker(&self, request: Uuid, capability: Capability, policy: LeasePolicy, work: Arc<Work>) {
        let admission = self
            .io(move |registry| {
                let Reservation::New(ticket) = registry.reserve(request, &capability)? else {
                    anyhow::bail!("managed duplicate reservation");
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
        work.deadline.send_replace(tokio::time::Instant::from_std(deadline));
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
        let snapshot = self.io(move |registry| registry.close(request, &capability)).await?;
        if snapshot.state == State::Closed {
            return Ok(snapshot);
        }
        let ticket = Ticket {
            request,
            generation: snapshot.generation,
        };
        let work = self.0.works.lock().await.get(&request).cloned();
        if let Some(work) = work {
            work.cancel.cancel();
            self.cleanup_work(ticket, work, true).await
        } else {
            let latest = self.snapshot(ticket).await?;
            if latest.state == State::Closed {
                Ok(latest)
            } else {
                Err(Failure::RecoveryPending)
            }
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
        let snapshot = self.snapshot(ticket.clone()).await?;
        if snapshot.state == State::Closed {
            return Ok(snapshot);
        }
        let original = work
            .ticket
            .lock()
            .map_err(|_| Failure::RecoveryPending)?
            .clone()
            .ok_or(Failure::RecoveryPending)?;
        if original.generation != ticket.generation {
            return Err(Failure::RecoveryPending);
        }
        let effects = Arc::clone(&work.effects);
        let cleanup_ticket = ticket.clone();
        let known = work
            .binding
            .lock()
            .map_err(|_| Failure::RecoveryPending)?
            .clone()
            .or(snapshot.vm);
        let registry_owner = self.clone();
        let intent = snapshot.spawn_intent;
        let cleaned = bounded(self.0.bounds.cleanup, async move {
            let binding = match known {
                Some(binding) => Some(binding),
                None => effects.reconcile(cleanup_ticket.clone(), intent).await?,
            };
            if let Some(binding) = binding {
                let bind_ticket = cleanup_ticket.clone();
                let verified = binding.clone();
                registry_owner
                    .io(move |registry| registry.reconcile_vm(&bind_ticket, verified))
                    .await?;
                effects.cleanup(cleanup_ticket.clone(), binding.clone()).await?;
                registry_owner
                    .io(move |registry| registry.complete_cleanup(&cleanup_ticket, &binding))
                    .await
                    .map_err(anyhow::Error::from)
            } else {
                registry_owner
                    .io(move |registry| registry.confirm_absent(&cleanup_ticket))
                    .await
                    .map_err(anyhow::Error::from)
            }
        })
        .await
        .map_err(|()| Failure::CleanupFailed)?;
        self.remove_work(ticket.request, &work).await;
        Ok(cleaned)
    }

    async fn expiry_worker(&self, ticket: Ticket, work: Arc<Work>) {
        let mut deadline = work.deadline.subscribe();
        let mut retry = false;
        loop {
            let next = *deadline.borrow_and_update();
            tokio::select! {
                () = work.cancel.cancelled() => {},
                () = tokio::time::sleep_until(next) => {},
                changed = deadline.changed() => { if changed.is_err() { return; } continue; }
            }
            let expire_ticket = ticket.clone();
            let snapshot = self
                .io(move |registry| registry.expire_lease(&expire_ticket, LeaseClock::now()?))
                .await;
            let closing = matches!(snapshot, Ok(ref snapshot) if matches!(snapshot.state, State::Closing | State::Closed | State::Unknown));
            if closing || work.cancel.is_cancelled() {
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
