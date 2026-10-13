use super::*;
use capsem_core::{managed_sessions::VmBinding, session::write_spawn_identity};
use capsem_foundation::unix::{contained::ContainedDir, process::parent_process_id};

const GENERATION: &str = "2f2dceea-1a8b-41fd-b58a-9129d2540181";

#[test]
fn vm_owner_admission_holds_generation_lock_and_refuses_fenced_replay() {
    let root = tempfile::tempdir().unwrap();
    let parent = parent_process_id().unwrap().get();
    let binding = VmBinding::new("vm".into(), GENERATION.parse().unwrap()).unwrap();
    capsem_core::session::spawn_owner::prepare(root.path(), &binding, parent).unwrap();
    let owner = admit("vm", Some(GENERATION), Some(parent), root.path()).unwrap();
    assert!(admit("vm", Some(GENERATION), Some(parent), root.path()).is_err());
    assert!(capsem_core::session::spawn_owner::fence(root.path(), &binding).is_err());
    drop(owner);
    let mut fenced = capsem_core::session::spawn_owner::fence(root.path(), &binding).unwrap();
    assert!(!fenced.confirm_exit().unwrap());
    drop(fenced);
    assert!(admit("vm", Some(GENERATION), Some(parent), root.path()).is_err());
}

#[test]
fn vm_owner_admission_requires_actual_parent_and_exact_trusted_spawn_metadata() {
    let root = tempfile::tempdir().unwrap();
    let parent = parent_process_id().unwrap().get();
    assert!(validate("vm", Some(GENERATION), None, root.path()).is_err());
    assert!(validate("vm", Some(GENERATION), Some(std::process::id()), root.path()).is_err());
    assert!(validate("vm", None, Some(parent), root.path()).is_err());
    assert!(validate("vm", Some("bad"), Some(parent), root.path()).is_err());
    assert!(validate("vm", Some(GENERATION), Some(parent), root.path()).is_err());
    let directory = ContainedDir::open_root(root.path()).unwrap();
    let binding = VmBinding::new("vm".into(), GENERATION.parse().unwrap()).unwrap();
    write_spawn_identity(&directory, &binding).unwrap();
    validate("vm", Some(GENERATION), Some(parent), root.path()).unwrap();
    assert!(validate("different", Some(GENERATION), Some(parent), root.path()).is_err());
    assert!(validate(
        "vm",
        Some("2f2dceea-1a8b-41fd-b58a-9129d2540182"),
        Some(parent),
        root.path()
    )
    .is_err());
    assert!(validate(
        "vm",
        Some("2F2DCEEA-1A8B-41FD-B58A-9129D2540181"),
        Some(parent),
        root.path()
    )
    .is_err());
    std::fs::write(root.path().join("spawn-identity.json"), b"corrupt").unwrap();
    assert!(validate("vm", Some(GENERATION), Some(parent), root.path()).is_err());
}
