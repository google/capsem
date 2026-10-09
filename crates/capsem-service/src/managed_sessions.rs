//! Internal adapter; public managed HTTP routes and policy remain separately
//! reviewed. Core owns the continuation/lease protocol, service owns effects.

use super::*;
use capsem_core::managed_sessions::{
    controller::{BindingReporter, EffectFuture, Effects},
    Snapshot, Ticket, VmBinding,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

mod grants;

/// Mandatory broker authority supplied by the service, never inferred from
/// the ownership capability. Failure leaves managed close pending.
pub trait GrantRetirement: Send + Sync {
    fn revoke(&self, ticket: Ticket, binding: VmBinding) -> EffectFuture<()>;
}

#[derive(Clone)]
pub struct ManagedLifecycle {
    state: Arc<ServiceState>,
    request: Option<ProvisionRequest>,
    binding: VmBinding,
    grants: Arc<dyn GrantRetirement>,
    work: Arc<Work>,
}

struct Work {
    scope: std::sync::OnceLock<(uuid::Uuid, uuid::Uuid)>,
    continuation: tokio::sync::Mutex<Option<watch::Sender<Option<bool>>>>,
    cancel: CancellationToken,
}

struct CreationCompletion(watch::Sender<Option<bool>>);
impl Drop for CreationCompletion {
    fn drop(&mut self) {
        let pending = self.0.borrow().is_none();
        if pending {
            self.0.send_replace(Some(false));
        }
    }
}

impl ManagedLifecycle {
    pub fn new(state: Arc<ServiceState>, request: ProvisionRequest, grants: Arc<dyn GrantRetirement>) -> Result<Self> {
        Self::with_grants(state, request, |_, _| grants)
    }

    pub fn new_with_broker(
        state: Arc<ServiceState>,
        request: ProvisionRequest,
        authority: Arc<capsem_credentials::GrantAuthority>,
    ) -> Result<Self> {
        Self::with_grants(state, request, |binding, work| {
            grants::retirement(authority, binding, work)
        })
    }

    fn with_grants(
        state: Arc<ServiceState>,
        request: ProvisionRequest,
        factory: impl FnOnce(&VmBinding, &Arc<Work>) -> Arc<dyn GrantRetirement>,
    ) -> Result<Self> {
        anyhow::ensure!(
            request.name.is_none() && !request.persistent,
            "managed sessions must be unnamed and ephemeral"
        );
        let binding = VmBinding::new(new_persistent_vm_id(), uuid::Uuid::new_v4())?;
        let work = Arc::new(Work {
            scope: Default::default(),
            continuation: tokio::sync::Mutex::new(None),
            cancel: CancellationToken::new(),
        });
        let grants = factory(&binding, &work);
        Ok(Self {
            state,
            request: Some(request),
            binding,
            grants,
            work,
        })
    }

    /// Cleanup-only adoption of the durable original target. Process metadata
    /// remains evidence of intent, never proof that an untracked child exited.
    pub fn for_recovery(
        state: Arc<ServiceState>,
        snapshot: &Snapshot,
        grants: Arc<dyn GrantRetirement>,
    ) -> Result<Self> {
        Self::recover_with_grants(state, snapshot, |_, _| grants)
    }

    pub fn for_recovery_with_broker(
        state: Arc<ServiceState>,
        snapshot: &Snapshot,
        authority: Arc<capsem_credentials::GrantAuthority>,
    ) -> Result<Self> {
        Self::recover_with_grants(state, snapshot, |binding, work| {
            grants::retirement(authority, binding, work)
        })
    }

    fn recover_with_grants(
        state: Arc<ServiceState>,
        snapshot: &Snapshot,
        factory: impl FnOnce(&VmBinding, &Arc<Work>) -> Arc<dyn GrantRetirement>,
    ) -> Result<Self> {
        let binding = snapshot
            .vm()
            .or_else(|| snapshot.spawn_intent())
            .context("managed recovery has no prepared service target")?
            .clone();
        let id = uuid::Uuid::parse_str(binding.id())?;
        anyhow::ensure!(
            !id.is_nil() && id.to_string() == binding.id(),
            "managed recovery requires a canonical service VM id"
        );
        let work = Arc::new(Work {
            scope: std::sync::OnceLock::from((snapshot.request(), snapshot.generation())),
            continuation: tokio::sync::Mutex::new(None),
            cancel: CancellationToken::new(),
        });
        let grants = factory(&binding, &work);
        Ok(Self {
            state,
            request: None,
            binding,
            grants,
            work,
        })
    }

    /// Grant admission requires this continuation's original registry ticket
    /// and a service-tracked instance with the actual immutable spawn UUID.
    pub fn credential_binding(&self, ticket: &Ticket) -> Result<capsem_credentials::GrantSession> {
        let binding = grants::bound_session(ticket, &self.binding, &self.work)?;
        anyhow::ensure!(
            self.matching_instance(&self.binding)?,
            "managed credential owner is unavailable"
        );
        Ok(binding)
    }

    fn scope(&self, ticket: &Ticket) -> Result<()> {
        let scope = (ticket.request(), ticket.generation());
        let original = self.work.scope.get_or_init(|| scope);
        anyhow::ensure!(*original == scope, "managed adapter cannot change request generation");
        Ok(())
    }

    fn matching_instance(&self, binding: &VmBinding) -> Result<bool> {
        let instances = self.state.instances.lock().unwrap();
        if let Some(instance) = instances.get(binding.id()) {
            anyhow::ensure!(
                instance.generation == binding.generation() && !instance.persistent,
                "managed VM ownership changed"
            );
            return Ok(true);
        }
        drop(instances);
        Ok(false)
    }

    async fn join_creation(&self) -> Result<bool> {
        self.work.cancel.cancel();
        let continuation = self
            .work
            .continuation
            .lock()
            .await
            .as_ref()
            .map(watch::Sender::subscribe);
        if let Some(mut result) = continuation {
            loop {
                if result.borrow_and_update().is_some() {
                    break;
                }
                result.changed().await?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    async fn provision(
        &self,
        ticket: Ticket,
        cancel: CancellationToken,
        reporter: BindingReporter,
    ) -> Result<VmBinding> {
        let request = self.request.as_ref().context("managed recovery cannot create")?;
        self.scope(&ticket)?;
        reporter.prepare(self.binding.clone()).await?;
        anyhow::ensure!(
            !cancel.is_cancelled() && !self.work.cancel.is_cancelled(),
            "managed creation cancelled"
        );
        let _launch = self.state.lifecycle.admit()?;
        if self
            .state
            .off_worker(|state| vm_asset_block_reason(&state))
            .await
            .map_err(app_error)?
            .is_some()
        {
            anyhow::bail!("managed runtime assets are unavailable");
        }
        let networks = network_routes::resolve_network_names(&*self.state.networks.lock().await, &request.networks)
            .map_err(app_error)?;
        let binding = self.binding.clone();
        let container = request.container.clone();
        let request = request.clone();
        let _vz = self.state.lifecycle.vz.read().await;
        let _host = vm_lifecycle::acquire_vz_host_lock(startup::VzHostLockMode::Shared)
            .await
            .map_err(app_error)?;
        reporter.prepare(self.binding.clone()).await?;
        anyhow::ensure!(
            !cancel.is_cancelled() && !self.work.cancel.is_cancelled(),
            "managed creation cancelled before spawn"
        );
        let caller_cancel = cancel.clone();
        let owned_cancel = self.work.cancel.clone();
        self.state
            .off_worker(move |state| {
                anyhow::ensure!(
                    !caller_cancel.is_cancelled() && !owned_cancel.is_cancelled(),
                    "managed creation cancelled before provision"
                );
                let resources = resolve_vm_resources(request.ram_mb, request.cpus);
                state.provision_sandbox_generation(
                    ProvisionOptions {
                        id: binding.id(),
                        name: binding.id(),
                        ram_mb: resources.ram_mb,
                        cpus: resources.cpus,
                        scratch_disk_size_gb: resources.scratch_disk_size_gb,
                        version_override: None,
                        persistent: false,
                        env: capsem_core::container::session_env(request.env, request.container.is_some()),
                        labels: crate::non_empty_labels(request.labels),
                        from: request.from.map(|source| CloneFrom {
                            source,
                            replace_image: request.container.is_some(),
                        }),
                        description: None,
                    },
                    binding.generation(),
                )
            })
            .await
            .map_err(app_error)??;
        anyhow::ensure!(
            self.matching_instance(&self.binding)?,
            "managed VM exited before registration"
        );
        reporter.registered(self.binding.clone()).await?;
        drop(_host);
        drop(_vz);
        let socket = running_uds_path(&self.state, self.binding.id()).map_err(app_error)?;
        tokio::select! {
            ready = wait_for_vm_ready(&socket, 30, Some(&self.state), Some(self.binding.id())) => ready.map_err(anyhow::Error::msg)?,
            () = cancel.cancelled() => anyhow::bail!("managed creation cancelled"),
            () = self.work.cancel.cancelled() => anyhow::bail!("managed creation cancelled"),
        }
        tokio::select! {
            created = vm_files::complete_create(&self.state, self.binding.id(), &networks, container) => { created.map_err(app_error)?; },
            () = cancel.cancelled() => anyhow::bail!("managed workload setup cancelled"),
            () = self.work.cancel.cancelled() => anyhow::bail!("managed workload setup cancelled"),
        }
        Ok(self.binding.clone())
    }

    async fn retire(&self, ticket: Ticket, binding: VmBinding) -> Result<()> {
        self.scope(&ticket)?;
        anyhow::ensure!(binding == self.binding, "managed cleanup target changed");
        self.matching_instance(&binding)?;
        self.join_creation().await?;
        self.matching_instance(&binding)?;
        let stopped = vm_lifecycle::shutdown_vm_process(
            &self.state,
            binding.id(),
            ShutdownMode::Discard,
            Some(binding.generation()),
        )
        .await
        .map_err(app_error)?;
        if stopped.is_none() {
            anyhow::ensure!(
                self.state.retirements.wait(binding.id(), binding.generation()).await?,
                "managed child retirement is unknown"
            );
        }
        self.state.containers.cancel_and_wait(binding.id()).await;
        self.grants.revoke(ticket, binding.clone()).await?;
        network_routes::vm_deleted_checked(&self.state, binding.id()).await?;
        self.remove_session(binding, true).await
    }

    async fn remove_session(&self, binding: VmBinding, require_identity: bool) -> Result<()> {
        let state = Arc::clone(&self.state);
        let target = binding;
        self.state
            .off_worker(move |_| {
                let directory = state.run_dir.join("sessions").join(target.id());
                match capsem_foundation::unix::contained::ContainedDir::open_root(&directory) {
                    Ok(root) => {
                        let identity = capsem_core::session::read_spawn_identity(&root)?;
                        anyhow::ensure!(
                            identity.as_ref() == Some(&target) || (!require_identity && identity.is_none()),
                            "managed session identity changed"
                        );
                        state.delete_session_dir(&directory)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                Ok(())
            })
            .await
            .map_err(app_error)?
    }
}

impl Effects for ManagedLifecycle {
    fn create(&self, ticket: Ticket, cancel: CancellationToken, reporter: BindingReporter) -> EffectFuture<VmBinding> {
        let owner = self.clone();
        Box::pin(async move {
            anyhow::ensure!(owner.request.is_some(), "managed recovery cannot create");
            owner.scope(&ticket)?;
            let mut continuation = owner.work.continuation.lock().await;
            anyhow::ensure!(continuation.is_none(), "managed creation cannot replay");
            let (done, mut result) = watch::channel(None);
            *continuation = Some(done.clone());
            drop(continuation);
            let worker = owner.clone();
            tokio::spawn(async move {
                let completion = CreationCompletion(done);
                let result = worker.provision(ticket, cancel, reporter).await;
                completion.0.send_replace(Some(result.is_ok()));
            });
            loop {
                let outcome = *result.borrow_and_update();
                if let Some(success) = outcome {
                    anyhow::ensure!(success, "managed creation failed");
                    return Ok(owner.binding.clone());
                }
                result.changed().await?;
            }
        })
    }
    fn cleanup(&self, ticket: Ticket, binding: VmBinding) -> EffectFuture<()> {
        let owner = self.clone();
        Box::pin(async move { owner.retire(ticket, binding).await })
    }
    fn reconcile(&self, ticket: Ticket, intent: Option<VmBinding>) -> EffectFuture<Option<VmBinding>> {
        let owner = self.clone();
        Box::pin(async move {
            owner.scope(&ticket)?;
            let owned_continuation = owner.join_creation().await?;
            if let Some(intent) = intent {
                anyhow::ensure!(intent == owner.binding, "managed recovery target changed");
                if owner.matching_instance(&intent)? {
                    return Ok(Some(intent));
                }
                let retired = owner.state.retirements.wait(intent.id(), intent.generation()).await?;
                if !retired {
                    // The service producer registers retirement before spawn.
                    // With its owned continuation finished, no registration
                    // proves that this exact new target never spawned a child.
                    anyhow::ensure!(owned_continuation, "managed absence requires original child authority");
                    owner.grants.revoke(ticket, intent.clone()).await?;
                    network_routes::vm_deleted_checked(&owner.state, intent.id()).await?;
                    owner.remove_session(intent, false).await?;
                    return Ok(None);
                }
                owner.retire(ticket, intent).await?;
            }
            Ok(None)
        })
    }
}

fn app_error(error: AppError) -> anyhow::Error {
    anyhow::anyhow!("service lifecycle operation failed ({})", error.status)
}

#[cfg(test)]
mod tests;
