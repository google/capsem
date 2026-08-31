//! Persistent (named) VM registry backed by a JSON file.
//!
//! [`PersistentRegistry`] is the on-disk record of named VMs that survive
//! daemon restarts. It is decoupled from `ServiceState`: register / unregister
//! operations each atomically rewrite the JSON file, so a crash between
//! operations leaves the registry in a consistent state.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PersistentVmEntry {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub profile_id: String,
    pub profile_revision: String,
    pub profile_payload_hash: String,
    pub asset_pins: BootAssetPins,
    pub ram_mb: u64,
    pub cpus: u32,
    pub base_version: String,
    pub created_at: String,
    pub session_dir: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none", default, alias = "source_image")]
    pub forked_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    #[serde(default)]
    pub suspended: bool,
    /// `true` when the most recent boot of this VM died before reaching
    /// ready (e.g. signed-manifest mismatch, asset hash drift, Apple VZ
    /// entitlement missing). Cleared on the next successful boot. Used
    /// by `capsem list` / `capsem status` to mark the VM `Defunct`
    /// instead of the misleading `Stopped`, and by `capsem logs <id>`
    /// to surface the preserved process.log. A crashed ephemeral VM has
    /// no registry entry; for those the `-failed-*` session dir is the
    /// only signal.
    #[serde(default)]
    pub defunct: bool,
    /// Short tail of `process.log` from the last crash. Cached here so
    /// `capsem list` / `capsem status` can describe the failure without
    /// each tool re-reading the session dir on every call.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub checkpoint_path: Option<String>,
    /// User-provided env vars from /vms/create -- replayed on every resume so the
    /// guest sees the same environment after stop+resume cycles.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub env: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BootAssetPins {
    pub kernel: BootAssetPin,
    pub initrd: BootAssetPin,
    pub rootfs: BootAssetPin,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BootAssetPin {
    pub name: String,
    pub hash: String,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct PersistentRegistryData {
    pub vms: HashMap<String, PersistentVmEntry>,
}

pub struct PersistentRegistry {
    path: PathBuf,
    pub data: PersistentRegistryData,
}

impl PersistentRegistry {
    pub fn load(path: PathBuf) -> Self {
        let data = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut registry = Self { path, data };
        if registry.ensure_entry_ids() {
            let _ = registry.save();
        }
        registry
    }

    fn ensure_entry_ids(&mut self) -> bool {
        let mut changed = false;
        for entry in self.data.vms.values_mut() {
            if entry.id.trim().is_empty() {
                entry.id = new_persistent_vm_id();
                changed = true;
            }
        }
        changed
    }

    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.data)?;
        // Atomic write: write to temp file, fsync, then rename.
        // Prevents torn writes on crash from losing all persistent VM state.
        let tmp_path = self.path.with_extension("json.tmp");
        let mut f = std::fs::File::create(&tmp_path)?;
        std::io::Write::write_all(&mut f, json.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }

    pub fn register(&mut self, mut entry: PersistentVmEntry) -> Result<()> {
        if self.data.vms.contains_key(&entry.name) {
            return Err(anyhow!(
                "persistent VM \"{}\" already exists. Use resume to reconnect.",
                entry.name
            ));
        }
        if entry.id.trim().is_empty() {
            entry.id = new_persistent_vm_id();
        }
        if self.data.vms.values().any(|existing| existing.id == entry.id) {
            return Err(anyhow!("persistent VM id \"{}\" already exists", entry.id));
        }
        self.data.vms.insert(entry.name.clone(), entry);
        self.save()
    }

    pub fn unregister(&mut self, name: &str) -> Result<()> {
        self.data.vms.remove(name);
        self.save()
    }

    pub fn get(&self, name: &str) -> Option<&PersistentVmEntry> {
        self.data.vms.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut PersistentVmEntry> {
        self.data.vms.get_mut(name)
    }

    pub fn list(&self) -> impl Iterator<Item = &PersistentVmEntry> {
        self.data.vms.values()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.data.vms.contains_key(name)
    }
}

pub fn new_persistent_vm_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests;
