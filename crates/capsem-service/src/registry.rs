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
    pub name: String,
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub checkpoint_path: Option<String>,
    /// User-provided env vars from /provision -- replayed on every resume so the
    /// guest sees the same environment after stop+resume cycles.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub env: Option<HashMap<String, String>>,
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
        Self { path, data }
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

    pub fn register(&mut self, entry: PersistentVmEntry) -> Result<()> {
        if self.data.vms.contains_key(&entry.name) {
            return Err(anyhow!(
                "persistent VM \"{}\" already exists. Use resume to reconnect.",
                entry.name
            ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_entry(name: &str, session_dir: PathBuf) -> PersistentVmEntry {
        PersistentVmEntry {
            name: name.into(),
            ram_mb: 2048,
            cpus: 2,
            base_version: "0.1.0".into(),
            created_at: "12345".into(),
            session_dir,
            forked_from: None,
            description: None,
            suspended: false,
            checkpoint_path: None,
            env: None,
        }
    }

    #[test]
    fn persistent_registry_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_registry.json");

        let mut registry = PersistentRegistry::load(path.clone());
        assert_eq!(registry.data.vms.len(), 0);

        let mut entry = make_entry("mydev", dir.path().join("mydev"));
        entry.ram_mb = 4096;
        entry.cpus = 4;
        registry.register(entry).unwrap();

        assert!(registry.contains("mydev"));
        assert_eq!(registry.get("mydev").unwrap().ram_mb, 4096);

        // Reload from disk
        let registry2 = PersistentRegistry::load(path);
        assert!(registry2.contains("mydev"));
        assert_eq!(registry2.get("mydev").unwrap().cpus, 4);
    }

    #[test]
    fn persistent_registry_rejects_duplicate() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_registry.json");

        let mut registry = PersistentRegistry::load(path);
        let entry = make_entry("dup", dir.path().join("dup"));
        registry.register(entry.clone()).unwrap();
        let err = registry.register(entry).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn persistent_registry_unregister() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_registry.json");

        let mut registry = PersistentRegistry::load(path);
        registry
            .register(make_entry("tmp", dir.path().join("tmp")))
            .unwrap();
        assert!(registry.contains("tmp"));
        registry.unregister("tmp").unwrap();
        assert!(!registry.contains("tmp"));
    }

    #[test]
    fn persistent_registry_get_mut() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_registry.json");

        let mut registry = PersistentRegistry::load(path);
        registry
            .register(make_entry("mutvm", dir.path().join("mutvm")))
            .unwrap();

        if let Some(entry) = registry.get_mut("mutvm") {
            entry.ram_mb = 8192;
        }
        assert_eq!(registry.get("mutvm").unwrap().ram_mb, 8192);
    }

    #[test]
    fn resume_clears_suspended_flag_in_registry() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_registry.json");

        let mut registry = PersistentRegistry::load(path.clone());
        let mut entry = make_entry("resumevm", dir.path().join("resumevm"));
        entry.suspended = true;
        entry.checkpoint_path = Some("checkpoint.vzsave".into());
        registry.register(entry).unwrap();

        // Verify suspended initially
        assert!(registry.get("resumevm").unwrap().suspended);
        assert!(registry.get("resumevm").unwrap().checkpoint_path.is_some());

        // Simulate what resume_sandbox does after spawning the process
        if let Some(entry) = registry.get_mut("resumevm") {
            entry.suspended = false;
            entry.checkpoint_path = None;
        }
        registry.save().unwrap();

        // Verify cleared
        assert!(!registry.get("resumevm").unwrap().suspended);
        assert!(registry.get("resumevm").unwrap().checkpoint_path.is_none());

        // Verify persists to disk
        let registry2 = PersistentRegistry::load(path);
        assert!(!registry2.get("resumevm").unwrap().suspended);
    }

    #[test]
    fn suspended_flag_roundtrips_through_json() {
        let mut entry = make_entry("jsonvm", PathBuf::from("/tmp/jsonvm"));
        entry.suspended = true;
        entry.checkpoint_path = Some("checkpoint.vzsave".into());
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: PersistentVmEntry = serde_json::from_str(&json).unwrap();
        assert!(parsed.suspended);
        assert_eq!(parsed.checkpoint_path.as_deref(), Some("checkpoint.vzsave"));
    }

    #[test]
    fn suspended_flag_defaults_to_false_when_missing() {
        // Old registry entries won't have the suspended field
        let json = r#"{"name":"old","ram_mb":2048,"cpus":2,"base_version":"0.1.0","created_at":"0","session_dir":"/tmp/old"}"#;
        let entry: PersistentVmEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.suspended, "suspended should default to false");
        assert!(
            entry.checkpoint_path.is_none(),
            "checkpoint_path should default to None"
        );
    }

    // -----------------------------------------------------------------------
    // Coverage additions (sprint plan: >= 90% on registry.rs)
    // -----------------------------------------------------------------------

    #[test]
    fn load_returns_empty_on_corrupt_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("corrupt.json");
        std::fs::write(&path, "not json").unwrap();

        let registry = PersistentRegistry::load(path);
        assert_eq!(registry.list().count(), 0);
    }

    #[test]
    fn load_returns_empty_on_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("does-not-exist.json");

        let registry = PersistentRegistry::load(path);
        assert_eq!(registry.list().count(), 0);
    }

    #[test]
    fn get_returns_none_for_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("reg.json");
        let mut registry = PersistentRegistry::load(path);
        registry
            .register(make_entry("present", dir.path().join("present")))
            .unwrap();
        assert!(registry.get("absent").is_none());
    }

    #[test]
    fn get_mut_returns_none_for_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("reg.json");
        let mut registry = PersistentRegistry::load(path);
        registry
            .register(make_entry("present", dir.path().join("present")))
            .unwrap();
        assert!(registry.get_mut("absent").is_none());
    }

    #[test]
    fn contains_false_for_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("reg.json");
        let registry = PersistentRegistry::load(path);
        assert!(!registry.contains("never-registered"));
    }

    #[test]
    fn list_iterates_all_registered() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("reg.json");
        let mut registry = PersistentRegistry::load(path);
        registry
            .register(make_entry("a", dir.path().join("a")))
            .unwrap();
        registry
            .register(make_entry("b", dir.path().join("b")))
            .unwrap();

        let names: std::collections::HashSet<&str> =
            registry.list().map(|e| e.name.as_str()).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains("a"));
        assert!(names.contains("b"));
    }

    #[test]
    fn save_writes_atomically_via_temp_rename() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("atomic.json");
        let tmp_path = path.with_extension("json.tmp");

        let mut registry = PersistentRegistry::load(path.clone());
        registry
            .register(make_entry("one", dir.path().join("one")))
            .unwrap();

        // Final file present, temp sibling gone (rename completed).
        assert!(path.exists(), "registry json should exist after save");
        assert!(
            !tmp_path.exists(),
            "temp file should be renamed, not left behind"
        );
    }
}
