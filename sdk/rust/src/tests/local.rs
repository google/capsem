use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::Hypervisor;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn no_argument_constructor_uses_the_shared_runtime_path() {
    let _lock = ENV_LOCK.lock().unwrap();
    let root = tempfile::tempdir().unwrap();
    let _paths = capsem_foundation::paths::CapsemPathsGuard::redirect(root.path());
    std::fs::create_dir(root.path().join("run")).unwrap();
    metadata(&root.path().join("run"), "19222", "secret");
    assert!(Hypervisor::local().is_ok());
}

fn metadata(root: &Path, port: &str, token: &str) {
    std::fs::write(root.join("gateway.port"), port).unwrap();
    std::fs::write(root.join("gateway.token"), token).unwrap();
    std::fs::set_permissions(root.join("gateway.token"), std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[tokio::test]
async fn local_connection_uses_loopback_and_fresh_private_credentials() {
    let mut server = super::gateway().await;
    let root = tempfile::tempdir().unwrap();
    let port = server.url.rsplit(':').next().unwrap();
    metadata(root.path(), port, "first-secret");
    let first = Hypervisor::local_in(root.path()).unwrap();
    metadata(root.path(), port, "rotated-secret");
    let second = Hypervisor::local_in(root.path()).unwrap();
    for (client, expected) in [(first, "Bearer first-secret"), (second, "Bearer rotated-secret")] {
        client.list().await.unwrap();
        let (request, _) = server.received.recv().await.unwrap();
        assert_eq!(request.headers["authorization"], expected);
        assert_eq!(request.uri.path(), "/vms/list");
    }
}

#[test]
fn local_missing_malformed_public_and_linked_metadata_is_refused() {
    let root = tempfile::tempdir().unwrap();
    assert!(Hypervisor::local_in(root.path()).is_err());
    for port in ["0", "65536", "-1", "1.5", "evil.example:80", "1\n2", ""] {
        metadata(root.path(), port, "private-secret");
        let error = Hypervisor::local_in(root.path()).unwrap_err().to_string();
        assert!(error.contains("Start Capsem"));
        assert!(!error.contains("private-secret"));
    }
    for token in ["", "a\nb", "a\rb", &"x".repeat(5000)] {
        metadata(root.path(), "19222", token);
        assert!(Hypervisor::local_in(root.path()).is_err());
    }
    metadata(root.path(), "19222", "secret");
    let token = root.path().join("gateway.token");
    std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Hypervisor::local_in(root.path()).is_err());
    std::fs::rename(&token, root.path().join("secret")).unwrap();
    std::os::unix::fs::symlink(root.path().join("secret"), &token).unwrap();
    assert!(Hypervisor::local_in(root.path()).is_err());
}
