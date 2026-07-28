use super::*;

#[test]
fn parse_version_body_extracts_version() {
    let resp =
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"version\":\"1.2.3\"}";
    assert_eq!(parse_version_body(resp).as_deref(), Some("1.2.3"));
}

#[test]
fn parse_version_body_missing_field_returns_none() {
    let resp = b"HTTP/1.1 200 OK\r\n\r\n{\"other\":\"x\"}";
    assert_eq!(parse_version_body(resp), None);
}

#[test]
fn parse_version_body_no_body_returns_none() {
    let resp = b"HTTP/1.1 500 OK\r\n\r\n";
    assert_eq!(parse_version_body(resp), None);
}

#[test]
fn startup_lock_is_mutually_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    let lock_path = dir.path().join("service.lock");

    let a = StartupLock::acquire(&lock_path, Duration::from_millis(50))
        .unwrap()
        .expect("first acquisition");
    let b = StartupLock::acquire(&lock_path, Duration::from_millis(50)).unwrap();
    assert!(
        b.is_none(),
        "second acquisition must fail while first is held"
    );

    drop(a);

    let c = StartupLock::acquire(&lock_path, Duration::from_millis(500))
        .unwrap()
        .expect("reacquire after drop");
    drop(c);
}

#[test]
fn vz_host_lock_is_mutually_exclusive() {
    let (dir, lock_path) = isolated_vz_host_lock_path();
    let a = VzHostLock::acquire_test_path(
        &lock_path,
        VzHostLockMode::Exclusive,
        Duration::from_millis(50),
    )
    .unwrap()
    .expect("first acquisition");
    let b = VzHostLock::acquire_test_path(
        &lock_path,
        VzHostLockMode::Exclusive,
        Duration::from_millis(50),
    )
    .unwrap();
    assert!(
        b.is_none(),
        "second VZ host lock acquisition must wait while first is held"
    );

    drop(a);

    let c = VzHostLock::acquire_test_path(
        &lock_path,
        VzHostLockMode::Exclusive,
        Duration::from_millis(500),
    )
    .unwrap()
    .expect("reacquire after drop");
    drop(c);
    drop(dir);
}

#[test]
fn vz_host_lock_allows_shared_lifecycle_starts() {
    let (dir, lock_path) = isolated_vz_host_lock_path();
    let a = VzHostLock::acquire_test_path(
        &lock_path,
        VzHostLockMode::Shared,
        Duration::from_millis(50),
    )
    .unwrap()
    .expect("first shared acquisition");
    let b = VzHostLock::acquire_test_path(
        &lock_path,
        VzHostLockMode::Shared,
        Duration::from_millis(50),
    )
    .unwrap()
    .expect("second shared acquisition");
    drop(b);
    drop(a);
    drop(dir);
}

#[test]
fn vz_host_lock_exclusive_blocks_shared() {
    let (dir, lock_path) = isolated_vz_host_lock_path();
    let a = VzHostLock::acquire_test_path(
        &lock_path,
        VzHostLockMode::Exclusive,
        Duration::from_millis(50),
    )
    .unwrap()
    .expect("exclusive acquisition");
    let b = VzHostLock::acquire_test_path(
        &lock_path,
        VzHostLockMode::Shared,
        Duration::from_millis(50),
    )
    .unwrap();
    assert!(
        b.is_none(),
        "shared VZ host lock acquisition must wait while exclusive is held"
    );
    drop(a);
    drop(dir);
}

#[test]
fn vz_host_lock_shared_blocks_exclusive() {
    let (dir, lock_path) = isolated_vz_host_lock_path();
    let a = VzHostLock::acquire_test_path(
        &lock_path,
        VzHostLockMode::Shared,
        Duration::from_millis(50),
    )
    .unwrap()
    .expect("shared acquisition");
    let b = VzHostLock::acquire_test_path(
        &lock_path,
        VzHostLockMode::Exclusive,
        Duration::from_millis(50),
    )
    .unwrap();
    assert!(
        b.is_none(),
        "exclusive VZ host lock acquisition must wait while shared is held"
    );
    drop(a);
    drop(dir);
}

fn isolated_vz_host_lock_path() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let lock_path = dir.path().join("vz-host.lock");
    (dir, lock_path)
}
