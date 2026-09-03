use std::io::{BufRead, BufReader};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::process::{Command, Stdio};

use nix::fcntl::{fcntl, FcntlArg, FdFlag};

use super::{try_acquire, try_acquire_after_open, LockAttempt, LockMode};

fn acquired(attempt: LockAttempt) -> super::FileLock {
    match attempt {
        LockAttempt::Acquired(lock) => lock,
        LockAttempt::Contended => panic!("lock unexpectedly contended"),
    }
}

#[test]
fn exclusive_lock_is_cloexec_owner_only_and_reacquirable() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nested/lock");
    let lock = acquired(try_acquire(&path, LockMode::Exclusive).unwrap());
    let metadata = std::fs::symlink_metadata(&path).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(lock.path(), path);
    let flags = FdFlag::from_bits_truncate(fcntl(lock.as_raw_fd(), FcntlArg::F_GETFD).unwrap());
    assert!(flags.contains(FdFlag::FD_CLOEXEC));
    assert!(matches!(
        try_acquire(lock.path(), LockMode::Exclusive).unwrap(),
        LockAttempt::Contended
    ));

    drop(lock);
    assert!(matches!(
        try_acquire(&path, LockMode::Exclusive).unwrap(),
        LockAttempt::Acquired(_)
    ));
}

#[test]
fn shared_locks_are_compatible_but_exclude_a_writer() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("lock");
    let first = acquired(try_acquire(&path, LockMode::Shared).unwrap());
    let second = acquired(try_acquire(&path, LockMode::Shared).unwrap());
    assert!(matches!(
        try_acquire(&path, LockMode::Exclusive).unwrap(),
        LockAttempt::Contended
    ));
    drop(first);
    assert!(matches!(
        try_acquire(&path, LockMode::Exclusive).unwrap(),
        LockAttempt::Contended
    ));
    drop(second);
    assert!(matches!(
        try_acquire(&path, LockMode::Exclusive).unwrap(),
        LockAttempt::Acquired(_)
    ));
}

#[test]
fn lock_refuses_a_symlink() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    std::fs::write(&target, b"").unwrap();
    let link = root.path().join("lock");
    symlink(target, &link).unwrap();
    assert!(try_acquire(&link, LockMode::Exclusive).is_err());
}

#[test]
fn lock_refuses_a_path_replaced_after_open() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("lock");
    let replacement = root.path().join("replacement");
    std::fs::write(&replacement, b"").unwrap();
    let result = try_acquire_after_open(&path, LockMode::Exclusive, || {
        std::fs::rename(&replacement, &path).unwrap();
    });
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    assert!(matches!(
        try_acquire(&path, LockMode::Exclusive).unwrap(),
        LockAttempt::Acquired(_)
    ));
}

#[test]
fn another_process_contends_and_release_is_observed() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("lock");
    let executable = std::env::current_exe().unwrap();
    let mut child = Command::new(executable)
        .args(["--exact", "unix::lock::tests::subprocess_lock_holder", "--nocapture"])
        .env("CAPSEM_FOUNDATION_LOCK_HELPER", &path)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while stdout.read_line(&mut line).unwrap() != 0 {
        if line.contains("LOCKED") {
            break;
        }
        line.clear();
    }
    assert!(line.contains("LOCKED"));
    assert!(matches!(
        try_acquire(&path, LockMode::Exclusive).unwrap(),
        LockAttempt::Contended
    ));
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(matches!(
        try_acquire(&path, LockMode::Exclusive).unwrap(),
        LockAttempt::Acquired(_)
    ));
}

#[test]
fn subprocess_lock_holder() {
    let Some(path) = std::env::var_os("CAPSEM_FOUNDATION_LOCK_HELPER") else {
        return;
    };
    let _lock = acquired(try_acquire(std::path::Path::new(&path), LockMode::Exclusive).unwrap());
    println!("LOCKED");
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    std::thread::park();
}
