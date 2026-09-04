use super::*;
use std::collections::BTreeMap;
use std::sync::Arc;

fn disabled_plugin() -> super::super::SecurityPluginConfig {
    serde_json::from_value(serde_json::json!({"mode": "disable"})).unwrap()
}

#[test]
fn every_call_returns_the_same_built_in_plugin_set() {
    let first = SecurityActionRegistry::with_builtin_actions();
    let second = SecurityActionRegistry::with_builtin_actions();
    let ids = |registry: &SecurityActionRegistry| registry.plugins.keys().cloned().collect::<Vec<_>>();
    assert_eq!(ids(&first), ids(&second));
    assert_eq!(first.plugins.len(), 4);
    for (id, plugin) in &first.plugins {
        assert!(
            Arc::ptr_eq(plugin, &second.plugins[id]),
            "plugin {id} is shared, not rebuilt"
        );
    }
}

#[test]
fn the_plugin_policy_is_per_registry_not_shared() {
    let mut policy = BTreeMap::new();
    policy.insert("credential_broker".to_string(), disabled_plugin());
    let configured = SecurityActionRegistry::with_builtin_actions().with_plugin_policy(policy);
    let plain = SecurityActionRegistry::with_builtin_actions();
    assert_eq!(configured.plugin_policy.len(), 1);
    assert!(
        plain.plugin_policy.is_empty(),
        "one registry's policy never leaks into the shared set"
    );
}
