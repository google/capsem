use super::*;

#[cfg(feature = "native-acceptance")]
mod native_file_content;

#[test]
fn private_run_dir_restricts_existing_runtime_before_sockets() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let root = tempfile::tempdir().unwrap();
    let run = root.path().join("run");
    std::fs::create_dir(&run).unwrap();
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(run.join("existing-session"), b"keep").unwrap();

    assert_eq!(private_run_dir(&run).unwrap(), run.canonicalize().unwrap());
    assert_eq!(std::fs::metadata(&run).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(std::fs::read(run.join("existing-session")).unwrap(), b"keep");

    let link = root.path().join("link");
    symlink(&run, &link).unwrap();
    assert!(private_run_dir(&link).is_err());
}

#[test]
fn gateway_readiness_requires_complete_stamped_runtime_files() {
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("gateway.token");
    let port = dir.path().join("gateway.port");
    let preview = dir.path().join("preview.port");
    for path in [&token, &port, &preview] {
        std::fs::write(path, "").unwrap();
    }
    assert!(!gateway_runtime_ready(&token, &port, &preview));

    std::fs::write(&token, "a".repeat(64)).unwrap();
    std::fs::write(&port, "19222").unwrap();
    std::fs::write(&preview, "19223").unwrap();
    assert!(gateway_runtime_ready(&token, &port, &preview));

    std::fs::write(&token, "short").unwrap();
    assert!(!gateway_runtime_ready(&token, &port, &preview));
    std::fs::write(&token, "a".repeat(64)).unwrap();
    std::fs::write(&preview, "0").unwrap();
    assert!(!gateway_runtime_ready(&token, &port, &preview));
}
