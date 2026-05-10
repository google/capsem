use std::collections::HashMap;

use capsem_core::net::domain_policy::{Action, DomainPolicy};

use super::insert_builtin_domain_policy_env;

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
