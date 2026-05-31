use capsem_core::mcp::policy::McpUserConfig;
use capsem_network_engine::domain_policy::{Action, DomainPolicy};

use super::{build_builtin_env, build_servers_with_builtin};

#[test]
fn build_builtin_env_includes_session_paths_without_domain_policy_env() {
    let policy = DomainPolicy::new(
        &["example.com".to_string()],
        &["blocked.test".to_string()],
        Action::Deny,
    );

    let env = build_builtin_env(std::path::Path::new("/tmp/capsem/session"), &policy);

    assert_eq!(
        env.get("CAPSEM_SESSION_DIR").map(String::as_str),
        Some("/tmp/capsem/session")
    );
    assert_eq!(
        env.get("CAPSEM_SESSION_DB").map(String::as_str),
        Some("/tmp/capsem/session/session.db")
    );
    assert!(!env.contains_key("CAPSEM_DOMAIN_ALLOW"));
    assert!(!env.contains_key("CAPSEM_DOMAIN_BLOCK"));
    assert!(!env.contains_key("CAPSEM_DOMAIN_DEFAULT"));
}

#[test]
fn build_servers_with_builtin_preserves_local_session_env_without_domain_policy() {
    let dir = tempfile::tempdir().unwrap();
    let builtin = dir.path().join("capsem-mcp-builtin");
    std::fs::write(&builtin, b"fake").unwrap();
    let session = dir.path().join("session");
    let policy = DomainPolicy::new(
        &["example.com".to_string()],
        &["blocked.test".to_string()],
        Action::Deny,
    );

    let servers = build_servers_with_builtin(
        &McpUserConfig::default(),
        &McpUserConfig::default(),
        Some(&builtin),
        &session,
        &policy,
    );

    let local = servers
        .iter()
        .find(|server| server.name == "local")
        .expect("local builtin server should be present");
    assert_eq!(local.command.as_deref(), Some(builtin.to_str().unwrap()));
    assert_eq!(
        local.env.get("CAPSEM_SESSION_DIR").map(String::as_str),
        Some(session.to_str().unwrap())
    );
    assert_eq!(
        local.env.get("CAPSEM_SESSION_DB").map(String::as_str),
        Some(session.join("session.db").to_str().unwrap())
    );
    assert!(!local.env.contains_key("CAPSEM_DOMAIN_ALLOW"));
    assert!(!local.env.contains_key("CAPSEM_DOMAIN_BLOCK"));
    assert!(!local.env.contains_key("CAPSEM_DOMAIN_DEFAULT"));
}
