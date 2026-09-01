use super::*;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
fn mode_of(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
#[test]
fn write_secret_file_creates_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("credential-store.json");
    write_secret_file(&path, b"{}").unwrap();
    assert_eq!(
        mode_of(&path),
        0o600,
        "plaintext secret store must be created owner-only, never briefly 0644"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"{}");
}

#[cfg(unix)]
#[test]
fn write_secret_file_downgrades_permissive_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("credential-store.json");
    std::fs::write(&path, b"stale").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    write_secret_file(&path, b"{\"k\":\"v\"}").unwrap();
    assert_eq!(mode_of(&path), 0o600);
    assert_eq!(std::fs::read(&path).unwrap(), b"{\"k\":\"v\"}");
}

#[cfg(unix)]
#[test]
fn write_secret_file_leaves_no_temp_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("credential-store.json");
    write_secret_file(&path, b"{}").unwrap();
    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name != "credential-store.json")
        .collect();
    assert!(
        leftovers.is_empty(),
        "atomic write must not leave temp files: {leftovers:?}"
    );
}
