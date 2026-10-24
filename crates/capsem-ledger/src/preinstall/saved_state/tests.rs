use super::*;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};

fn check(home: &Path, state: HostSchemaAdmission, require_current: bool) -> Result<()> {
    admit(
        &home.canonicalize().unwrap(),
        state,
        require_current,
        Instant::now() + std::time::Duration::from_secs(1),
    )
}

#[test]
fn genuinely_empty_default_namespace_is_admitted_without_creation() {
    let home = tempfile::tempdir().unwrap();
    assert!(check(home.path(), HostSchemaAdmission::Absent, false).is_ok());
    assert!(!home.path().join(".capsem").exists());
    for name in ["instances", "sessions", "persistent"] {
        fs::create_dir_all(home.path().join(".capsem/run").join(name)).unwrap();
    }
    assert!(check(home.path(), HostSchemaAdmission::Absent, false).is_ok());
}

#[test]
fn any_retained_child_or_registry_refuses_even_without_a_legacy_ledger() {
    for (namespace, child) in [
        ("instances", "vm"),
        ("sessions", ".unknown"),
        ("persistent", "named"),
        ("persistent_registry.json", ""),
    ] {
        let home = tempfile::tempdir().unwrap();
        let mut entry = home.path().join(".capsem/run").join(namespace);
        if !child.is_empty() {
            entry.push(child);
        }
        fs::create_dir_all(entry.parent().unwrap()).unwrap();
        fs::write(&entry, b"unparsed retained state").unwrap();
        let before = fs::read(&entry).unwrap();
        assert!(
            check(home.path(), HostSchemaAdmission::Absent, false).is_err(),
            "{namespace}"
        );
        assert_eq!(fs::read(entry).unwrap(), before);
    }
}

#[test]
fn hostile_or_unknown_namespace_is_not_absence() {
    for name in [
        ".capsem",
        ".capsem/run",
        ".capsem/run/instances",
        ".capsem/run/persistent_registry.json",
    ] {
        let home = tempfile::tempdir().unwrap();
        let entry = home.path().join(name);
        fs::create_dir_all(entry.parent().unwrap()).unwrap();
        symlink(home.path().join("missing"), &entry).unwrap();
        assert!(
            check(home.path(), HostSchemaAdmission::Absent, false).is_err(),
            "{name}"
        );
    }
    let home = tempfile::tempdir().unwrap();
    let run = home.path().join(".capsem/run");
    fs::create_dir_all(&run).unwrap();
    fs::set_permissions(&run, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(check(home.path(), HostSchemaAdmission::Absent, false).is_err());
    fs::set_permissions(&run, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(run.join("sessions"), b"not a directory").unwrap();
    assert!(check(home.path(), HostSchemaAdmission::Absent, false).is_err());
}

#[test]
fn current_host_uses_existing_host_validation_and_legacy_in_service_is_refused() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".capsem/run/persistent/named")).unwrap();
    assert!(check(home.path(), HostSchemaAdmission::Current, true).is_ok());
    assert!(check(home.path(), HostSchemaAdmission::Absent, true).is_err());
    assert!(admit(home.path(), HostSchemaAdmission::Absent, false, Instant::now()).is_err());
}

#[test]
fn empty_corrupt_or_directory_registry_and_dangling_saved_entry_all_refuse() {
    for bytes in [b"".as_slice(), b"{}", b"corrupt"] {
        let home = tempfile::tempdir().unwrap();
        let run = home.path().join(".capsem/run");
        fs::create_dir_all(&run).unwrap();
        fs::write(run.join("persistent_registry.json"), bytes).unwrap();
        assert!(check(home.path(), HostSchemaAdmission::Absent, false).is_err());
    }
    let home = tempfile::tempdir().unwrap();
    let run = home.path().join(".capsem/run");
    fs::create_dir_all(run.join("persistent_registry.json")).unwrap();
    assert!(check(home.path(), HostSchemaAdmission::Absent, false).is_err());
    fs::remove_dir(run.join("persistent_registry.json")).unwrap();
    fs::create_dir(run.join("persistent")).unwrap();
    symlink("missing-target", run.join("persistent/.orphan")).unwrap();
    assert!(check(home.path(), HostSchemaAdmission::Absent, false).is_err());
}
