//! One running VM as the service sees it: the owner process, its sockets,
//! the boot assets it booted, and how its records are removed.
use super::*;
use tokio_util::sync::CancellationToken;

/// Cancellation owned by one registered worker generation.
pub(crate) struct WorkerAuthority(CancellationToken);

/// A non-owning grant canceled when its registered generation is revoked.
#[derive(Clone)]
pub(crate) struct WorkerGrant(CancellationToken);

impl Default for WorkerAuthority {
    fn default() -> Self {
        Self(CancellationToken::new())
    }
}

impl WorkerAuthority {
    pub(crate) fn grant(&self) -> WorkerGrant {
        WorkerGrant(self.0.clone())
    }

    pub(crate) fn revoke(&self) {
        self.0.cancel();
    }
}

impl Drop for WorkerAuthority {
    fn drop(&mut self) {
        self.revoke();
    }
}

impl WorkerGrant {
    pub(crate) fn is_revoked(&self) -> bool {
        self.0.is_cancelled()
    }

    pub(crate) async fn revoked(&self) {
        self.0.cancelled().await;
    }
}

pub(crate) struct InstanceInfo {
    pub(crate) browser_request: Option<capsem_api::BrowserRequest>,
    /// Includes cleared requests, so an older observer cannot resurrect a URL.
    pub(crate) browser_revision: u64,
    pub(crate) terminal: Option<capsem_api::TerminalSummary>,
    pub(crate) id: String,
    /// One actual spawn, never reused when an ID is resumed or replaced.
    pub(crate) generation: uuid::Uuid,
    pub(crate) authority: WorkerAuthority,
    pub(crate) upstream_policy: crate::upstream_broker::PolicyPublisher,
    pub(crate) name: String,
    pub(crate) asset_pins: BootAssetPins,
    pub(crate) pid: u32,
    pub(crate) uds_path: PathBuf,
    pub(crate) session_dir: PathBuf,
    pub(crate) ram_mb: u64,
    pub(crate) cpus: u32,
    #[allow(dead_code)]
    pub(crate) start_time: std::time::Instant,
    pub(crate) base_version: String,
    /// Whether this is a persistent (named) VM
    pub(crate) persistent: bool,
    /// Environment variables injected at boot
    #[allow(dead_code)]
    pub(crate) env: Option<std::collections::HashMap<String, String>>,
    /// Sandbox this VM was cloned from, if any
    pub(crate) forked_from: Option<String>,
    /// Key-value metadata labels attached at creation
    pub(crate) labels: Option<std::collections::HashMap<String, String>>,
}

impl InstanceInfo {
    pub(crate) fn observe_browser(&mut self, mut request: capsem_api::BrowserRequest) {
        if request.id > self.browser_revision {
            self.browser_revision = request.id;
            request.owner_generation = Some(self.generation.to_string());
            self.browser_request = Some(request);
        }
    }

    pub(crate) fn clear_browser(&mut self, request_id: u64) {
        self.browser_revision = self.browser_revision.max(request_id);
        if self
            .browser_request
            .as_ref()
            .is_some_and(|request| request.id <= request_id)
        {
            self.browser_request = None;
        }
    }
}

#[cfg(test)]
mod tests;

/// Host-only durable identity precedes launch. It is a matching fact for
/// recovery, never authority to signal a PID without live ownership proof.
pub(crate) fn persist_spawn_identity(session_dir: &StdPath, id: &str, generation: uuid::Uuid) -> Result<()> {
    let binding = capsem_core::managed_sessions::VmBinding::new(id.to_owned(), generation)?;
    let session = capsem_foundation::unix::contained::ContainedDir::open_root(session_dir)?;
    if let Some(previous) = capsem_core::session::read_spawn_identity(&session)? {
        anyhow::ensure!(previous.id() == id, "previous VM spawn owner changed");
        if previous != binding {
            let mut fence = capsem_core::session::spawn_owner::fence(session_dir, &previous)?;
            anyhow::ensure!(fence.confirm_exit()?, "previous native VM exit is unconfirmed");
        }
    }
    capsem_core::session::spawn_owner::prepare(session_dir, &binding, std::process::id())
}

impl ServiceState {
    pub(crate) fn native_retirements_path(&self) -> PathBuf {
        self.run_dir.join("managed-retirements")
    }

    /// Preserve exact native exit before directory moves, deletion or reboot.
    /// `never_spawned` is the joined original producer's no-child authority.
    pub(crate) fn persist_native_exit(
        &self,
        session_dir: &StdPath,
        binding: &capsem_core::managed_sessions::VmBinding,
        never_spawned: bool,
    ) -> Result<()> {
        use capsem_core::session::spawn_owner::{fence, prepare, RetirementStore};
        let receipts = RetirementStore::open(&self.native_retirements_path())?;
        if receipts.contains(binding)? {
            return Ok(());
        }
        if never_spawned {
            let run = capsem_foundation::unix::contained::ContainedDir::open_root(&self.run_dir)?;
            let sessions = run.descend_or_create("sessions".as_ref(), 0o700)?;
            let root = sessions.descend_or_create(binding.id().as_ref(), 0o700)?;
            if capsem_core::session::read_spawn_identity(&root)?.is_none() {
                prepare(session_dir, binding, std::process::id())?;
            }
        }
        let mut fence = fence(session_dir, binding)?;
        anyhow::ensure!(fence.confirm_exit()?, "original native VM exit is unconfirmed");
        receipts.retain(&fence)
    }

    /// Remove a running instance record. The removal is the ownership token
    /// for teardown: of two callers racing to evict one VM, only one gets
    /// the record back.
    pub(crate) fn evict_instance(&self, id: &str, generation: uuid::Uuid) -> Option<InstanceInfo> {
        let mut instances = self.instances.lock().unwrap();
        if instances.get(id)?.generation != generation {
            return None;
        }
        let removed = instances.remove(id)?;
        drop(instances);
        removed.authority.revoke();
        self.approvals
            .retire(&capsem_core::security_engine::approval::ApprovalBinding {
                session_id: id.into(),
                generation,
            });
        self.remove_proxy_worker(id, generation);
        Some(removed)
    }

    /// Unregister a persistent VM. An absent entry is already forgotten, so
    /// nothing is written for it. Saves the registry file: call off the
    /// async worker.
    pub(crate) fn forget_persistent_entry(&self, name: &str) -> Result<()> {
        let registry = self.persistent_registry.lock().unwrap();
        if !registry.contains(name) {
            return Ok(());
        }
        registry.unregister(name)
    }
}
