use super::*;

fn private(path: &Path, value: &str) {
    std::fs::write(path, value).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[test]
fn native_connection_uses_the_selected_private_runtime_port_and_token() {
    let run = tempfile::tempdir().unwrap();
    private(&run.path().join("gateway.port"), "65228");
    let token = "a".repeat(64);
    private(&run.path().join("gateway.token"), &token);
    let connection = read_connection(run.path()).unwrap();
    assert_eq!(connection.gateway_url, "http://127.0.0.1:65228");
    assert_eq!(connection.token, token);
    let serialized = serde_json::to_value(connection).unwrap();
    assert_eq!(serialized["gatewayUrl"], "http://127.0.0.1:65228");
}

#[test]
fn native_connection_refuses_missing_invalid_or_oversized_ports() {
    let run = tempfile::tempdir().unwrap();
    private(&run.path().join("gateway.token"), &"a".repeat(64));
    assert!(read_connection(run.path()).is_err());
    for invalid in ["", "0", "65536", "123456", "+123", "123\n", "http://remote", "1/abc"] {
        private(&run.path().join("gateway.port"), invalid);
        assert!(read_connection(run.path()).is_err(), "accepted invalid port");
    }
    private(&run.path().join("gateway.port"), "65535");
    assert_eq!(
        read_connection(run.path()).unwrap().gateway_url,
        "http://127.0.0.1:65535"
    );
}

#[cfg(unix)]
#[test]
fn native_connection_refuses_public_or_symlinked_port_files() {
    use std::os::unix::fs::PermissionsExt;
    let run = tempfile::tempdir().unwrap();
    private(&run.path().join("gateway.token"), &"a".repeat(64));
    let port = run.path().join("gateway.port");
    private(&port, "65228");
    std::fs::set_permissions(&port, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_connection(run.path()).is_err());
    std::fs::remove_file(&port).unwrap();
    let target = run.path().join("other.port");
    private(&target, "65228");
    std::os::unix::fs::symlink(target, port).unwrap();
    assert!(read_connection(run.path()).is_err());
}

#[test]
fn native_connection_refuses_a_replacement_owner_between_port_and_token() {
    let mut tokens = ["a".repeat(64), "b".repeat(64)].into_iter();
    assert!(connection_with(|| Ok(tokens.next().unwrap()), || Ok(65228)).is_err());
}
