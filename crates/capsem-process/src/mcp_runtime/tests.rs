use std::collections::HashMap;

use capsem_core::net::domain_policy::{Action, DomainPolicy};

use capsem_core::mcp::policy::McpUserConfig;

use super::{build_builtin_env, build_servers_with_builtin, insert_builtin_domain_policy_env};

#[test]
fn builtin_domain_policy_env_carries_allow_and_block_lists() {
    let policy = DomainPolicy::new(
        &["example.com".to_string(), "*.trusted.test".to_string()],
        &["blocked.test".to_string()],
        Action::Deny,
    );
    let mut env = HashMap::new();

    insert_builtin_domain_policy_env(&mut env, &policy);

    assert_eq!(
        env.get("CAPSEM_DOMAIN_ALLOW").map(String::as_str),
        Some("example.com,*.trusted.test")
    );
    assert_eq!(
        env.get("CAPSEM_DOMAIN_BLOCK").map(String::as_str),
        Some("blocked.test")
    );
}

#[test]
fn builtin_domain_policy_env_leaves_open_policy_unset() {
    let policy = DomainPolicy::new(&[], &[], Action::Allow);
    let mut env = HashMap::new();

    insert_builtin_domain_policy_env(&mut env, &policy);

    assert!(!env.contains_key("CAPSEM_DOMAIN_ALLOW"));
    assert!(!env.contains_key("CAPSEM_DOMAIN_BLOCK"));
}

#[test]
fn build_builtin_env_includes_session_paths_and_domain_policy() {
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
    assert_eq!(
        env.get("CAPSEM_DOMAIN_ALLOW").map(String::as_str),
        Some("example.com")
    );
    assert_eq!(
        env.get("CAPSEM_DOMAIN_BLOCK").map(String::as_str),
        Some("blocked.test")
    );
}

#[test]
fn build_servers_with_builtin_preserves_local_session_and_domain_env() {
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
    assert_eq!(
        local.env.get("CAPSEM_DOMAIN_ALLOW").map(String::as_str),
        Some("example.com")
    );
    assert_eq!(
        local.env.get("CAPSEM_DOMAIN_BLOCK").map(String::as_str),
        Some("blocked.test")
    );
}
