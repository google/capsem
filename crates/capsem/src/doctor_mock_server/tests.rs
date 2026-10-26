use super::*;

#[test]
fn doctor_mock_server_addr_is_iptables_redirect_target() {
    assert_eq!(DOCTOR_MOCK_SERVER_ADDR, "127.0.0.1:3713");
}

#[test]
fn doctor_mock_server_default_lock_path_is_stable() {
    assert_eq!(
        DoctorMockServerLock::path_for_addr(DOCTOR_MOCK_SERVER_ADDR),
        std::env::temp_dir().join("capsem-mock-server-127-0-0-1-3713.lock")
    );
}

#[test]
fn doctor_mock_server_uses_shared_lock_across_private_tempdirs() {
    let root = tempfile::tempdir().unwrap();
    let shared = root.path().join("shared");
    let private = root.path().join("private");
    std::fs::create_dir(&shared).unwrap();
    std::fs::create_dir(&private).unwrap();
    let path = shared.join("capsem-mock-server-shared-probe.lock");
    let _held =
        capsem_foundation::unix::lock::acquire(&path, capsem_foundation::unix::lock::LockMode::Exclusive).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "doctor_mock_server::tests::doctor_mock_server_shared_lock_child",
        ])
        .env("CAPSEM_DOCTOR_LOCK_TEST", "1")
        .env("CAPSEM_MOCK_SERVER_LOCK_DIR", &shared)
        .env("TMPDIR", private)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn doctor_mock_server_shared_lock_child() {
    if std::env::var_os("CAPSEM_DOCTOR_LOCK_TEST").is_none() {
        return;
    }
    let shared = PathBuf::from(std::env::var_os("CAPSEM_MOCK_SERVER_LOCK_DIR").unwrap());
    assert_eq!(
        DoctorMockServerLock::path_for_addr("shared-probe"),
        shared.join("capsem-mock-server-shared-probe.lock"),
    );
    assert!(DoctorMockServerLock::acquire("shared-probe", Duration::ZERO).is_err());
}

#[test]
fn doctor_mock_server_lock_is_exclusive_and_reacquirable() {
    let dir = tempfile::tempdir().unwrap();
    let addr = dir.path().file_name().unwrap().to_string_lossy();
    let path = DoctorMockServerLock::path_for_addr(&addr);
    let first = DoctorMockServerLock::acquire(&addr, Duration::ZERO).unwrap();
    assert!(DoctorMockServerLock::acquire(&addr, Duration::ZERO).is_err());
    drop(first);
    drop(DoctorMockServerLock::acquire(&addr, Duration::ZERO).unwrap());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn doctor_mock_server_is_bound_to_the_cli_parent() {
    let command = doctor_mock_server_command(Path::new("/tmp/capsem-mock-server"));
    let args: Vec<_> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args,
        vec![
            "--addr".to_string(),
            DOCTOR_MOCK_SERVER_ADDR.to_string(),
            "--parent-pid".to_string(),
            std::process::id().to_string(),
        ]
    );
}

#[test]
fn mock_server_binary_prefers_installed_sibling() {
    let fixture = tempfile::tempdir().unwrap();
    let installed_bin = fixture.path().join("installed/bin");
    let source_bin = fixture.path().join("source/cache/target/cargo/debug");
    std::fs::create_dir_all(&installed_bin).unwrap();
    std::fs::create_dir_all(&source_bin).unwrap();
    let executable = installed_bin.join("capsem");
    let installed_mock = installed_bin.join("capsem-mock-server");
    let source_mock = source_bin.join("capsem-mock-server");
    std::fs::write(&executable, b"capsem").unwrap();
    std::fs::write(&installed_mock, b"installed").unwrap();
    std::fs::write(&source_mock, b"source").unwrap();

    assert_eq!(
        find_mock_server_binary(
            &executable,
            &fixture.path().join("source"),
            &fixture.path().join("crates/capsem"),
        ),
        Some(installed_mock)
    );
}
