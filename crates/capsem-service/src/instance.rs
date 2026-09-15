//! One running VM as the service sees it: the owner process, its sockets,
//! the profile it booted from, and how its records are removed.
use super::*;

pub(crate) struct InstanceInfo {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) profile_id: String,
    pub(crate) profile_revision: String,
    pub(crate) profile_payload_hash: String,
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
    /// What the VM owner shows when it asks the service about private names
    /// on the VM's behalf: minted at spawn, written to the session directory
    /// for the owner alone, matched here. Never reaches the guest.
    pub(crate) owner_secret: String,
}

impl ServiceState {
    /// Remove a running instance record. The removal is the ownership token
    /// for teardown: of two callers racing to evict one VM, only one gets
    /// the record back.
    pub(crate) fn evict_instance(&self, id: &str) -> Option<InstanceInfo> {
        self.instances.lock().unwrap().remove(id)
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
