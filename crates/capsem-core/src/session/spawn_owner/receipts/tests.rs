use super::*;
use crate::session::spawn_owner::{enter, fence, prepare};
use capsem_foundation::unix::process::parent_process_id;

fn binding() -> VmBinding {
    VmBinding::new("receipt-vm".into(), capsem_crypto::uuid_v4()).unwrap()
}

#[test]
fn retained_exit_survives_real_session_deletion_and_store_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let session = directory.path().join("session");
    std::fs::create_dir(&session).unwrap();
    let original = binding();
    prepare(&session, &original, std::process::id()).unwrap();
    let mut retired = fence(&session, &original).unwrap();
    let path = directory.path().join("retired");
    let receipts = RetirementStore::open(&path).unwrap();
    assert!(!receipts.contains(&original).unwrap());
    assert!(receipts.retain(&retired).is_err(), "a fence alone is not exit proof");
    retired.confirm_exit().unwrap();
    receipts.retain(&retired).unwrap();
    receipts.retain(&retired).unwrap();
    drop(retired);
    drop(receipts);
    std::fs::remove_dir_all(&session).unwrap();
    let receipts = RetirementStore::open(&path).unwrap();
    assert!(receipts.contains(&original).unwrap());
    assert!(!receipts.contains(&binding()).unwrap());
    let other_id = VmBinding::new("foreign-vm".into(), original.generation()).unwrap();
    assert!(receipts.contains(&other_id).is_err());
}

#[test]
fn retained_exit_refuses_live_native_owner_and_incomplete_proof() {
    let directory = tempfile::tempdir().unwrap();
    let session = directory.path().join("session");
    std::fs::create_dir(&session).unwrap();
    let original = binding();
    let parent = parent_process_id().unwrap().get();
    prepare(&session, &original, parent).unwrap();
    let owner = enter(&session, &original, parent).unwrap();
    assert!(fence(&session, &original).is_err());
    drop(owner);
    let mut unconfirmed = fence(&session, &original).unwrap();
    assert!(!unconfirmed.confirm_exit().unwrap());
    let receipts = RetirementStore::open(&directory.path().join("retired")).unwrap();
    assert!(receipts.retain(&unconfirmed).is_err());
    assert!(!receipts.contains(&original).unwrap());
}

#[test]
fn retained_exit_refuses_linked_corrupt_or_forged_incomplete_inventory_before_pruning() {
    let directory = tempfile::tempdir().unwrap();
    let original = binding();
    prepare(directory.path(), &original, std::process::id()).unwrap();
    let mut retired = fence(directory.path(), &original).unwrap();
    retired.confirm_exit().unwrap();
    let path = directory.path().join("retired");
    let receipts = RetirementStore::open(&path).unwrap();
    receipts.retain(&retired).unwrap();
    let file = path.join(format!("{}.json", original.generation()));
    let bytes = std::fs::read(&file).unwrap();
    let held = directory.path().join("held");
    std::fs::hard_link(&file, &held).unwrap();
    assert!(receipts.contains(&original).is_err());
    assert!(receipts.prune_unreferenced(&[]).is_err());
    std::fs::remove_file(&held).unwrap();
    std::fs::remove_file(&file).unwrap();
    std::os::unix::fs::symlink(&held, &file).unwrap();
    assert!(receipts.contains(&original).is_err());
    assert!(receipts.prune_unreferenced(&[]).is_err());
    std::fs::remove_file(&file).unwrap();
    drop(
        receipts
            .root
            .create_new_private_file(file.file_name().unwrap())
            .unwrap(),
    );
    let record: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for field in ["fenced", "retired"] {
        let mut incomplete = record.clone();
        incomplete[field] = false.into();
        std::fs::write(&file, serde_json::to_vec(&incomplete).unwrap()).unwrap();
        assert!(receipts.contains(&original).is_err());
        assert!(receipts.prune_unreferenced(&[]).is_err());
    }
    std::fs::write(&file, b"corrupt").unwrap();
    assert!(receipts.retain(&retired).is_err());
    assert!(receipts.prune_unreferenced(&[]).is_err());
    assert_eq!(std::fs::read(file).unwrap(), b"corrupt");
}

#[test]
fn retained_exit_pruning_keeps_exact_references_and_validates_all_entries_first() {
    let directory = tempfile::tempdir().unwrap();
    let receipts = RetirementStore::open(&directory.path().join("retired")).unwrap();
    let mut references = Vec::new();
    for _ in 0..2 {
        let original = binding();
        let session = tempfile::tempdir().unwrap();
        prepare(session.path(), &original, std::process::id()).unwrap();
        let mut retired = fence(session.path(), &original).unwrap();
        retired.confirm_exit().unwrap();
        receipts.retain(&retired).unwrap();
        references.push(original);
    }
    assert_eq!(receipts.prune_unreferenced(&references).unwrap(), 0);
    assert_eq!(receipts.prune_unreferenced(&references[..1]).unwrap(), 1);
    assert!(receipts.contains(&references[0]).unwrap());
    assert!(!receipts.contains(&references[1]).unwrap());
    let corrupt = receipts.root.path().join("bad.json");
    std::fs::write(&corrupt, b"corrupt").unwrap();
    assert!(receipts.prune_unreferenced(&[]).is_err());
    assert!(receipts.contains(&references[0]).unwrap());
    std::fs::remove_file(corrupt).unwrap();
    assert_eq!(receipts.prune_unreferenced(&[]).unwrap(), 1);
}
