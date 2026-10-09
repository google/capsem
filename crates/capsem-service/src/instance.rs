//! One running VM as the service sees it: the owner process, its sockets,
//! the boot assets it booted, and how its records are removed.
use super::*;
use tokio_util::sync::CancellationToken;

/// Cancellation owned by one registered worker generation.
pub(crate) struct WorkerAuthority(CancellationToken);

/// A non-owning grant canceled when its registered generation is revoked.
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
    pub(crate) async fn revoked(&self) {
        self.0.cancelled().await;
    }
}

pub(crate) struct InstanceInfo {
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
}

#[cfg(test)]
mod tests;

/// Host-only durable identity precedes launch. It is a matching fact for
/// recovery, never authority to signal a PID without live ownership proof.
pub(crate) fn persist_spawn_identity(session_dir: &StdPath, id: &str, generation: uuid::Uuid) -> Result<()> {
    let binding = capsem_core::managed_sessions::VmBinding::new(id.to_owned(), generation)?;
    let session = capsem_foundation::unix::contained::ContainedDir::open_root(session_dir)?;
    capsem_core::session::write_spawn_identity(&session, &binding)
}

impl ServiceState {
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
