use super::*;
use tempfile::TempDir;

fn make_entry(name: &str, session_dir: PathBuf) -> PersistentVmEntry {
    PersistentVmEntry {
        id: new_persistent_vm_id(),
        name: name.into(),
        profile_id: "code".into(),
        profile_revision: "2026.06.08.7".into(),
        profile_payload_hash: "blake3:1111111111111111111111111111111111111111111111111111111111111111".into(),
        asset_pins: test_asset_pins(),
        ram_mb: 2048,
        cpus: 2,
        base_version: "0.1.0".into(),
        created_at: "12345".into(),
        session_dir,
        forked_from: None,
        description: None,
        suspended: false,
        defunct: false,
        last_error: None,
        checkpoint_path: None,
        env: None,
    }
}

fn test_asset_pins() -> BootAssetPins {
    BootAssetPins {
        kernel: BootAssetPin {
            name: "vmlinuz".into(),
            hash: "blake3:aa933a569fe27ed014ae76b58eb278d72fbde8a3cbd4c06a23da2987e70d0bd1".into(),
        },
        initrd: BootAssetPin {
            name: "initrd.img".into(),
            hash: "blake3:ad31b76e82d487b207302109396b6dfa9bca97cb624c576dd3ccb6f59946cc96".into(),
        },
        rootfs: BootAssetPin {
            name: "rootfs.erofs".into(),
            hash: "blake3:dd32949abf690412c611f1a558d1bb6462089f98e585009d70fb70e8ad6a6620".into(),
        },
    }
}

#[test]
fn persistent_registry_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_registry.json");

    let mut registry = PersistentRegistry::load(path.clone()).expect("registry loads");
    assert_eq!(registry.data.vms.len(), 0);

    let mut entry = make_entry("mydev", dir.path().join("mydev"));
    entry.ram_mb = 4096;
    entry.cpus = 4;
    registry.register(entry).unwrap();

    assert!(registry.contains("mydev"));
    assert_eq!(registry.get("mydev").unwrap().ram_mb, 4096);

    // Reload from disk
    let registry2 = PersistentRegistry::load(path).expect("registry loads");
    assert!(registry2.contains("mydev"));
    assert_eq!(registry2.get("mydev").unwrap().cpus, 4);
}

#[test]
fn persistent_registry_backfills_missing_ids() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_registry.json");
    std::fs::write(
        &path,
        r#"{
  "vms": {
    "legacy": {
      "name": "legacy",
      "profile_id": "code",
      "profile_revision": "2026.06.08.7",
      "profile_payload_hash": "blake3:1111111111111111111111111111111111111111111111111111111111111111",
      "asset_pins": {
        "kernel": {"name": "vmlinuz", "hash": "blake3:aa933a569fe27ed014ae76b58eb278d72fbde8a3cbd4c06a23da2987e70d0bd1"},
        "initrd": {"name": "initrd.img", "hash": "blake3:ad31b76e82d487b207302109396b6dfa9bca97cb624c576dd3ccb6f59946cc96"},
        "rootfs": {"name": "rootfs.erofs", "hash": "blake3:dd32949abf690412c611f1a558d1bb6462089f98e585009d70fb70e8ad6a6620"}
      },
      "ram_mb": 2048,
      "cpus": 2,
      "base_version": "0.1.0",
      "created_at": "12345",
      "session_dir": "/tmp/legacy"
    }
  }
}"#,
    )
    .unwrap();

    let registry = PersistentRegistry::load(path.clone()).expect("registry loads");
    let id = &registry.get("legacy").unwrap().id;
    assert!(!id.is_empty(), "legacy registry entries must get durable ids");

    let reloaded = PersistentRegistry::load(path).expect("registry loads");
    assert_eq!(
        reloaded.get("legacy").unwrap().id,
        *id,
        "backfilled ids must be saved instead of regenerated on each load"
    );
}

#[test]
fn persistent_registry_rejects_duplicate() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_registry.json");

    let mut registry = PersistentRegistry::load(path).expect("registry loads");
    let entry = make_entry("dup", dir.path().join("dup"));
    registry.register(entry.clone()).unwrap();
    let err = registry.register(entry).unwrap_err();
    assert!(err.to_string().contains("already exists"));
}

#[test]
fn persistent_registry_unregister() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_registry.json");

    let mut registry = PersistentRegistry::load(path).expect("registry loads");
    registry.register(make_entry("tmp", dir.path().join("tmp"))).unwrap();
    assert!(registry.contains("tmp"));
    registry.unregister("tmp").unwrap();
    assert!(!registry.contains("tmp"));
}

#[test]
fn persistent_registry_get_mut() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_registry.json");

    let mut registry = PersistentRegistry::load(path).expect("registry loads");
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

    let mut registry = PersistentRegistry::load(path.clone()).expect("registry loads");
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
    let registry2 = PersistentRegistry::load(path).expect("registry loads");
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
fn persistent_vm_entry_rejects_missing_profile_contract_fields() {
    let json =
        r#"{"name":"old","ram_mb":2048,"cpus":2,"base_version":"0.1.0","created_at":"0","session_dir":"/tmp/old"}"#;
    let err = serde_json::from_str::<PersistentVmEntry>(json).unwrap_err();
    assert!(
        err.to_string().contains("profile_id"),
        "registry entries without profile contract fields must fail closed, got: {err}"
    );
}

// -----------------------------------------------------------------------
// Coverage additions (sprint plan: >= 90% on registry.rs)
// -----------------------------------------------------------------------

#[test]
fn load_returns_empty_on_missing_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("does-not-exist.json");

    let registry = PersistentRegistry::load(path).expect("registry loads");
    assert_eq!(registry.list().count(), 0);
}

#[test]
fn get_returns_none_for_missing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("reg.json");
    let mut registry = PersistentRegistry::load(path).expect("registry loads");
    registry
        .register(make_entry("present", dir.path().join("present")))
        .unwrap();
    assert!(registry.get("absent").is_none());
}

#[test]
fn get_mut_returns_none_for_missing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("reg.json");
    let mut registry = PersistentRegistry::load(path).expect("registry loads");
    registry
        .register(make_entry("present", dir.path().join("present")))
        .unwrap();
    assert!(registry.get_mut("absent").is_none());
}

#[test]
fn contains_false_for_missing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("reg.json");
    let registry = PersistentRegistry::load(path).expect("registry loads");
    assert!(!registry.contains("never-registered"));
}

#[test]
fn list_iterates_all_registered() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("reg.json");
    let mut registry = PersistentRegistry::load(path).expect("registry loads");
    registry.register(make_entry("a", dir.path().join("a"))).unwrap();
    registry.register(make_entry("b", dir.path().join("b"))).unwrap();

    let names: std::collections::HashSet<&str> = registry.list().map(|e| e.name.as_str()).collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains("a"));
    assert!(names.contains("b"));
}

#[test]
fn save_writes_atomically_via_temp_rename() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("atomic.json");
    let tmp_path = path.with_extension("json.tmp");

    let mut registry = PersistentRegistry::load(path.clone()).expect("registry loads");
    registry.register(make_entry("one", dir.path().join("one"))).unwrap();

    // Final file present, temp sibling gone (rename completed).
    assert!(path.exists(), "registry json should exist after save");
    assert!(!tmp_path.exists(), "temp file should be renamed, not left behind");
}

// A registry file that exists but cannot be parsed used to load as an empty
// registry; the next register() then saved the empty one over it and every
// persistent VM was forgotten, its directory orphaned under persistent/.
#[test]
fn persistent_registry_refuses_to_load_a_corrupt_file_and_leaves_it_alone() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("persistent_registry.json");
    std::fs::write(&path, b"{ \"vms\": { \"dev\": { \"name\": ").unwrap();

    let error = PersistentRegistry::load(path.clone())
        .err()
        .expect("corrupt registry must not load");
    assert!(error.to_string().contains("refusing to overwrite"), "{error:#}");
    assert_eq!(std::fs::read(&path).unwrap(), b"{ \"vms\": { \"dev\": { \"name\": ");
}

#[test]
fn persistent_registry_missing_file_is_an_empty_registry() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("persistent_registry.json");
    let registry = PersistentRegistry::load(path.clone()).expect("missing file is empty");
    assert!(registry.data.vms.is_empty());
    assert!(!path.exists(), "loading must not create the file");
}
