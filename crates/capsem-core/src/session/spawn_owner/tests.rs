use super::*;
use capsem_foundation::unix::process::parent_process_id;
use std::io::{BufRead, Write};

fn new_binding() -> VmBinding {
    VmBinding::new("owner-vm".into(), capsem_crypto::uuid_v4()).unwrap()
}

fn isolated_owner_command(
    program: impl AsRef<std::ffi::OsStr>,
    coverage_profile: Option<&std::ffi::OsStr>,
) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command.env_clear();
    if let Some(profile) = coverage_profile {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command
}

#[test]
fn isolated_owner_child_preserves_only_the_coverage_output_path() {
    let profile = std::ffi::OsStr::new("/tmp/capsem-profile-%p-%m.profraw");
    let command = isolated_owner_command("true", Some(profile));
    assert_eq!(
        command.get_envs().collect::<Vec<_>>(),
        [(std::ffi::OsStr::new("LLVM_PROFILE_FILE"), Some(profile))]
    );
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
    let mut retired = retired;
    assert!(!retired.confirm_exit().unwrap(), "a live PID is not an exit receipt");
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
    let mut reopened = fence(root.path(), &binding).unwrap();
    assert!(reopened.pid().is_none());
    assert!(reopened.confirm_exit().unwrap());
    drop(reopened);
    assert!(enter(root.path(), &binding, parent).is_err());
    assert!(prepare(root.path(), &binding, parent).is_err());
    prepare(root.path(), &new_binding(), parent).unwrap();
}

#[test]
fn spawn_owner_refuses_other_kernel_scope_before_admission_or_retirement() {
    for field in ["boot", "namespace"] {
        let directory = tempfile::tempdir().unwrap();
        let binding = new_binding();
        let parent = parent_process_id().unwrap().get();
        prepare(directory.path(), &binding, parent).unwrap();
        let file = directory.path().join(RECORD);
        let mut record: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        record["scope"][field] = match field {
            "boot" => serde_json::json!(capsem_crypto::uuid_v4().to_string()),
            _ => serde_json::json!([0, 1]),
        };
        std::fs::write(&file, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(enter(directory.path(), &binding, parent).is_err());
        assert!(fence(directory.path(), &binding).is_err());
        assert!(prepare(directory.path(), &new_binding(), parent).is_err());
    }
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
    println!("CAPSEM_OWNER_READY");
    std::io::stdout().flush().unwrap();
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
    let coverage_profile = std::env::var_os("LLVM_PROFILE_FILE");
    let mut child = Child(
        isolated_owner_command(std::env::current_exe().unwrap(), coverage_profile.as_deref())
            .args([
                "--exact",
                "session::spawn_owner::tests::spawn_owner_subprocess_fixture",
                "--nocapture",
            ])
            .env("CAPSEM_TEST_OWNER_ROOT", directory.path())
            .env("CAPSEM_TEST_OWNER_BINDING", serde_json::to_string(&binding).unwrap())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    // `read` verifies a single-linked private witness. Polling it during the
    // child's atomic rename can open the old inode just before it is unlinked.
    // Observe publication through the child, then read its stable witness.
    let output = child.0.stdout.take().unwrap();
    let (ready, published) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(output).lines() {
            if line.unwrap() == "CAPSEM_OWNER_READY" {
                ready.send(()).unwrap();
            }
        }
    });
    published.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    assert_eq!(read(&root).unwrap().unwrap().pid, Some(child.0.id()));
    assert!(
        fence(directory.path(), &binding).is_err(),
        "cross-process lifetime lock was bypassed"
    );
    let handle = ProcessHandle::open(ProcessId::try_from(child.0.id()).unwrap()).unwrap();
    drop(child.0.stdin.take());
    assert!(handle
        .wait_until_exit(std::time::Instant::now() + std::time::Duration::from_secs(2))
        .unwrap());
    let mut fenced = fence(directory.path(), &binding).unwrap();
    assert_eq!(fenced.pid().unwrap().get(), child.0.id());
    assert!(fenced.confirm_exit().unwrap());
    assert!(child.0.wait().unwrap().success());
    reader.join().unwrap();
    assert!(enter(directory.path(), &binding, std::process::id()).is_err());
    drop(fenced);
    assert!(fence(directory.path(), &binding).unwrap().confirm_exit().unwrap());
    assert!(prepare(directory.path(), &binding, std::process::id()).is_err());
    prepare(directory.path(), &new_binding(), std::process::id()).unwrap();
}
