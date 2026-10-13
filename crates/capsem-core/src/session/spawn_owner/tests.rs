use super::*;
use capsem_foundation::unix::process::parent_process_id;

fn new_binding() -> VmBinding {
    VmBinding::new("owner-vm".into(), capsem_crypto::uuid_v4()).unwrap()
}

#[test]
fn spawn_owner_records_original_pid_and_excludes_duplicate_boot_or_recovery() {
    let root = tempfile::tempdir().unwrap();
    let binding = new_binding();
    let parent = parent_process_id().unwrap().get();
    prepare(root.path(), &binding, parent).unwrap();
    let owner = enter(root.path(), &binding, parent).unwrap();
    assert!(enter(root.path(), &binding, parent).is_err());
    assert!(fence(root.path(), &binding).is_err());
    assert!(prepare(root.path(), &new_binding(), parent).is_err());
    drop(owner);
    let retired = fence(root.path(), &binding).unwrap();
    assert_eq!(retired.pid().unwrap().get(), std::process::id());
    assert_eq!(retired.parent().get(), parent);
    assert!(enter(root.path(), &binding, parent).is_err());
    drop(retired);
    assert!(enter(root.path(), &binding, parent).is_err());
    assert!(prepare(root.path(), &binding, parent).is_err());
}

#[test]
fn spawn_owner_preboot_fence_is_durable_and_refuses_late_admission() {
    let root = tempfile::tempdir().unwrap();
    let binding = new_binding();
    let parent = parent_process_id().unwrap().get();
    prepare(root.path(), &binding, parent).unwrap();
    let retired = fence(root.path(), &binding).unwrap();
    assert!(retired.pid().is_none());
    drop(retired);
    let reopened = fence(root.path(), &binding).unwrap();
    assert!(reopened.pid().is_none());
    drop(reopened);
    assert!(enter(root.path(), &binding, parent).is_err());
    assert!(prepare(root.path(), &binding, parent).is_err());
}

#[test]
fn spawn_owner_unknown_changed_or_corrupt_metadata_is_never_an_absence_receipt() {
    let root = tempfile::tempdir().unwrap();
    let original = new_binding();
    let parent = parent_process_id().unwrap().get();
    assert!(fence(root.path(), &original).is_err());
    assert!(enter(root.path(), &original, parent).is_err());
    let directory = ContainedDir::open_root(root.path()).unwrap();
    super::super::write_spawn_identity(&directory, &original).unwrap();
    assert!(fence(root.path(), &original).is_err(), "legacy metadata is not absence");
    assert!(
        prepare(root.path(), &new_binding(), parent).is_err(),
        "legacy ownership cannot be overwritten"
    );
    std::fs::remove_file(root.path().join("spawn-identity.json")).unwrap();
    prepare(root.path(), &original, parent).unwrap();
    assert!(fence(root.path(), &new_binding()).is_err());
    assert!(enter(root.path(), &original, std::process::id()).is_err());
    let record = root.path().join(RECORD);
    std::fs::write(&record, b"corrupt").unwrap();
    assert!(fence(root.path(), &original).is_err());
    assert!(enter(root.path(), &original, parent).is_err());
    assert!(prepare(root.path(), &new_binding(), parent).is_err());
    assert_eq!(std::fs::read(&record).unwrap(), b"corrupt");
}

#[test]
fn spawn_owner_refuses_linked_or_symlinked_private_witnesses() {
    let root = tempfile::tempdir().unwrap();
    let original = new_binding();
    let parent = parent_process_id().unwrap().get();
    prepare(root.path(), &original, parent).unwrap();
    let record = root.path().join(RECORD);
    let held = root.path().join("outside-held");
    std::fs::hard_link(&record, &held).unwrap();
    assert!(fence(root.path(), &original).is_err());
    assert!(enter(root.path(), &original, parent).is_err());
    let bytes = std::fs::read(&held).unwrap();
    std::fs::remove_file(&record).unwrap();
    std::os::unix::fs::symlink(&held, &record).unwrap();
    assert!(fence(root.path(), &original).is_err());
    assert!(enter(root.path(), &original, parent).is_err());
    assert_eq!(std::fs::read(&held).unwrap(), bytes);
}

#[test]
fn spawn_owner_subprocess_fixture() {
    let Ok(path) = std::env::var("CAPSEM_TEST_OWNER_ROOT") else {
        return;
    };
    let binding: VmBinding = serde_json::from_str(&std::env::var("CAPSEM_TEST_OWNER_BINDING").unwrap()).unwrap();
    let parent = parent_process_id().unwrap().get();
    let _owner = enter(Path::new(&path), &binding, parent).unwrap();
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
}

#[test]
fn spawn_owner_subprocess_holds_native_lifetime_until_actual_exit() {
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let binding = new_binding();
    prepare(directory.path(), &binding, std::process::id()).unwrap();
    let root = ContainedDir::open_root(directory.path()).unwrap();
    let mut child = Child(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "session::spawn_owner::tests::spawn_owner_subprocess_fixture",
                "--nocapture",
            ])
            .env_clear()
            .env("CAPSEM_TEST_OWNER_ROOT", directory.path())
            .env("CAPSEM_TEST_OWNER_BINDING", serde_json::to_string(&binding).unwrap())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while read(&root).unwrap().unwrap().pid != Some(child.0.id()) {
        assert!(
            std::time::Instant::now() < deadline,
            "native child did not publish its witness"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        fence(directory.path(), &binding).is_err(),
        "cross-process lifetime lock was bypassed"
    );
    drop(child.0.stdin.take());
    assert!(child.0.wait().unwrap().success());
    let fenced = fence(directory.path(), &binding).unwrap();
    assert_eq!(fenced.pid().unwrap().get(), child.0.id());
    assert!(enter(directory.path(), &binding, std::process::id()).is_err());
}
