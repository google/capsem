//! Persistent (named) VM registry backed by a JSON file.
//!
//! [`PersistentRegistry`] is the on-disk record of named VMs that survive
//! daemon restarts. It is decoupled from `ServiceState`: register / unregister
//! operations each atomically rewrite the JSON file, so a crash between
//! operations leaves the registry in a consistent state.

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::{LockResult, Mutex, MutexGuard, PoisonError};

use anyhow::{anyhow, Context, Result};
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
    /// Load the registry. A missing file is an empty registry; a file that
    /// exists but cannot be read or parsed is an error, never an empty
    /// registry: the next `register` would have saved the empty one over it
    /// and silently forgotten every persistent VM (their directories left
    /// orphaned under `persistent/`), which is what a schema change without a
    /// serde default used to do on upgrade.
    pub fn load(path: PathBuf) -> Result<Self> {
        let data = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).with_context(|| {
                format!(
                    "persistent VM registry {} is unreadable; refusing to overwrite it",
                    path.display()
                )
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => PersistentRegistryData::default(),
            Err(error) => {
                return Err(error).with_context(|| format!("read persistent VM registry {}", path.display()));
            }
        };
        let mut registry = Self { path, data };
        if registry.ensure_entry_ids() {
            registry.save()?;
        }
        Ok(registry)
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
        write_registry_file(&self.path, &self.serialized()?)
    }

    fn serialized(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.data)?)
    }

    pub fn register(&mut self, entry: PersistentVmEntry) -> Result<()> {
        self.insert(entry)?;
        self.save()
    }

    /// Validate and add an entry to the in-memory table without saving.
    fn insert(&mut self, mut entry: PersistentVmEntry) -> Result<()> {
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
        Ok(())
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

/// Atomic write: write to temp file, fsync, then rename. Prevents torn
/// writes on crash from losing all persistent VM state.
fn write_registry_file(path: &std::path::Path, json: &str) -> Result<()> {
    let tmp_path = path.with_extension("json.tmp");
    let mut f = std::fs::File::create(&tmp_path)?;
    std::io::Write::write_all(&mut f, json.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// The registry as the service shares it: an in-memory table behind a short
/// lock, and a file write that never happens under that lock.
///
/// `save` used to fsync the file while the data mutex was held, and every
/// list, info and status route takes that mutex to read. Each lifecycle
/// event (persist, fork, suspend, exit, purge) therefore stalled every poll
/// for the length of an fsync -- 2 to 4 ms on an idle SSD here, far more on
/// a busy disk. A guard's `commit` serializes under the data lock, takes the
/// write lock, releases the data lock, and only then writes: readers wait on
/// serialization alone, and writers still land on disk in the order they
/// changed the table.
pub struct SharedRegistry {
    data: Mutex<PersistentRegistry>,
    write: Mutex<()>,
}

impl SharedRegistry {
    pub fn new(registry: PersistentRegistry) -> Self {
        Self {
            data: Mutex::new(registry),
            write: Mutex::new(()),
        }
    }

    /// The in-memory table. Reads and in-memory edits are cheap; persisting
    /// them is `RegistryGuard::commit` (or `register`, `unregister`, `save`).
    pub fn lock(&self) -> LockResult<RegistryGuard<'_>> {
        match self.data.lock() {
            Ok(registry) => Ok(RegistryGuard {
                registry,
                write: &self.write,
            }),
            Err(poisoned) => Err(PoisonError::new(RegistryGuard {
                registry: poisoned.into_inner(),
                write: &self.write,
            })),
        }
    }
}

pub struct RegistryGuard<'a> {
    registry: MutexGuard<'a, PersistentRegistry>,
    write: &'a Mutex<()>,
}

impl Deref for RegistryGuard<'_> {
    type Target = PersistentRegistry;
    fn deref(&self) -> &PersistentRegistry {
        &self.registry
    }
}

impl DerefMut for RegistryGuard<'_> {
    fn deref_mut(&mut self) -> &mut PersistentRegistry {
        &mut self.registry
    }
}

impl RegistryGuard<'_> {
    /// Persist the table this guard changed. Consumes the guard: the data
    /// lock is released before the disk is touched.
    pub fn commit(self) -> Result<()> {
        let write = self.write.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let path = self.registry.path.clone();
        let json = self.registry.serialized()?;
        drop(self.registry);
        let written = write_registry_file(&path, &json);
        drop(write);
        written
    }

    /// Add an entry and persist it.
    pub fn register(mut self, entry: PersistentVmEntry) -> Result<()> {
        self.registry.insert(entry)?;
        self.commit()
    }

    /// Remove an entry and persist the table.
    pub fn unregister(mut self, name: &str) -> Result<()> {
        self.registry.data.vms.remove(name);
        self.commit()
    }

    /// Persist in-memory edits made through `get_mut`.
    pub fn save(self) -> Result<()> {
        self.commit()
    }
}

pub fn new_persistent_vm_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests;
