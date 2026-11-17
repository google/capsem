use super::*;
use crate::unix::lock::LockMode;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::PermissionsExt;

fn acquired(attempt: DescriptorLockAttempt) -> DescriptorLock {
    match attempt {
        DescriptorLockAttempt::Acquired(held) => held,
        DescriptorLockAttempt::Contended(_) => panic!("unexpected contention"),
    }
}

#[test]
fn independent_descriptor_readers_coexist_and_hold_exclusive_reclamation_until_drop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("generation.bodylog");
    std::fs::write(&path, b"original archive bytes").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let mut first = acquired(try_lock_descriptor(File::open(&path).unwrap(), LockMode::Shared).unwrap());
    let second = acquired(try_lock_descriptor(File::open(&path).unwrap(), LockMode::Shared).unwrap());
    let observer = match try_lock_descriptor(File::open(&path).unwrap(), LockMode::Exclusive).unwrap() {
        DescriptorLockAttempt::Contended(file) => file,
        _ => panic!("active readers must prevent reclamation"),
    };
    first.seek(SeekFrom::Start(9)).unwrap();
    let mut bytes = Vec::new();
    first.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"archive bytes");
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
    drop(first);
    let observer = match try_lock_descriptor(observer, LockMode::Exclusive).unwrap() {
        DescriptorLockAttempt::Contended(file) => file,
        _ => panic!("second reader still pins generation"),
    };
    drop(second);
    let exclusive = acquired(try_lock_descriptor(observer, LockMode::Exclusive).unwrap());
    assert!(matches!(
        try_lock_descriptor(File::open(&path).unwrap(), LockMode::Shared).unwrap(),
        DescriptorLockAttempt::Contended(_)
    ));
    drop(exclusive);
    acquired(try_lock_descriptor(File::open(&path).unwrap(), LockMode::Shared).unwrap());
}

#[test]
fn descriptor_lock_follows_selected_inode_across_replacement_and_unlink() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("generation.bodylog");
    std::fs::write(&path, b"old generation").unwrap();
    let held = acquired(try_lock_descriptor(File::open(&path).unwrap(), LockMode::Shared).unwrap());
    let old = File::open(&path).unwrap();
    let replacement = dir.path().join("replacement");
    std::fs::write(&replacement, b"new generation").unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    let mut selected = acquired(try_lock_descriptor(File::open(&path).unwrap(), LockMode::Exclusive).unwrap());
    let mut bytes = Vec::new();
    selected.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"new generation");
    std::fs::remove_file(&path).unwrap();
    let old = match try_lock_descriptor(old, LockMode::Exclusive).unwrap() {
        DescriptorLockAttempt::Contended(file) => file,
        _ => panic!("replacement must not release old generation lease"),
    };
    drop(held);
    let mut old = acquired(try_lock_descriptor(old, LockMode::Exclusive).unwrap());
    let mut bytes = Vec::new();
    old.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"old generation");
}

#[test]
fn descriptor_leases_interoperate_with_another_process_kernel_lock() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("generation.bodylog");
    std::fs::write(&path, b"archive").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "unix::lock::tests::subprocess_lock_holder", "--nocapture"])
        .env("CAPSEM_FOUNDATION_LOCK_HELPER", &path)
        .env("CAPSEM_FOUNDATION_LOCK_MODE", "shared")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while output.read_line(&mut line).unwrap() != 0 {
        if line.contains("LOCKED") {
            break;
        }
        line.clear();
    }
    assert!(line.contains("LOCKED"));
    let local = acquired(try_lock_descriptor(File::open(&path).unwrap(), LockMode::Shared).unwrap());
    drop(local);
    let observer = match try_lock_descriptor(File::open(&path).unwrap(), LockMode::Exclusive).unwrap() {
        DescriptorLockAttempt::Contended(file) => file,
        _ => panic!("remote reader must prevent reclamation"),
    };
    child.kill().unwrap();
    child.wait().unwrap();
    acquired(try_lock_descriptor(observer, LockMode::Exclusive).unwrap());
}
