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
    assert_eq!(first.plugins.len(), 7);
    let mut compiled_ids = capsem_config::plugins::PluginId::ALL.map(|id| id.as_str()).to_vec();
    compiled_ids.sort_unstable();
    assert_eq!(ids(&first), compiled_ids);
    for (id, plugin) in first.plugins.iter() {
        let descriptor = plugin.descriptor().expect("builtins reference canonical descriptors");
        assert_eq!(descriptor.id.as_str(), id);
        assert_eq!(descriptor.stage.as_str(), plugin.stage().as_str());
        assert!(
            Arc::ptr_eq(plugin, &second.plugins[id]),
            "plugin {id} is shared, not rebuilt"
        );
    }
}

#[test]
fn builtin_descriptor_stage_mismatch_is_refused_at_registration() {
    struct WrongStage(&'static str);
    impl super::super::SecurityPlugin for WrongStage {
        fn id(&self) -> &'static str {
            self.0
        }
        fn stage(&self) -> super::super::SecurityPluginStage {
            super::super::SecurityPluginStage::Logging
        }
        fn apply(
            &self,
            _: super::super::SecurityEvent,
            _: &crate::environment::CompiledPluginConfiguration,
        ) -> Result<super::super::SecurityPluginResult, super::super::SecurityActionError> {
            unreachable!("registration must refuse before execution")
        }
    }
    let error = SecurityActionRegistry::new()
        .register_plugin(WrongStage("pii_filter"))
        .err()
        .expect("stage drift must fail");
    assert!(error.to_string().contains("descriptor stage"));
    let custom = WrongStage("external-test-plugin");
    assert!(super::super::SecurityPlugin::descriptor(&custom).is_none());
}

#[test]
fn registering_on_a_clone_does_not_change_the_original_registry() {
    let original = SecurityActionRegistry::new();
    let extended = original.clone().register_plugin(DummyPreEicarPlugin).unwrap();
    let mut policy = BTreeMap::new();
    policy.insert(
        "dummy_pre_eicar".to_string(),
        super::super::SecurityPluginConfig {
            mode: super::super::SecurityPluginMode::Block,
            ..disabled_plugin()
        },
    );
    let event = super::super::SecurityEvent::new(super::super::RuntimeSecurityEventType::DnsQuery);
    let stage = super::super::SecurityPluginStage::Preprocess;
    assert!(
        original
            .with_plugin_policy(policy.clone())
            .apply_security_plugins(stage, event.clone())
            .is_err(),
        "an unregistered active plugin must fail closed"
    );
    assert!(extended
        .with_plugin_policy(policy)
        .apply_security_plugins(stage, event)
        .is_ok());
}

#[test]
fn the_plugin_policy_is_per_registry_not_shared() {
    let mut policy = BTreeMap::new();
    policy.insert("credential_broker".to_string(), disabled_plugin());
    let configured = SecurityActionRegistry::with_builtin_actions().with_plugin_policy(policy);
    let plain = SecurityActionRegistry::with_builtin_actions();
    let super::super::runtime_policy::RuntimePluginPolicy::Settings(configured_policy) = configured.policy else {
        panic!("settings policy")
    };
    let super::super::runtime_policy::RuntimePluginPolicy::Settings(plain_policy) = plain.policy else {
        panic!("settings policy")
    };
    assert_eq!(configured_policy.len(), 1);
    assert!(
        plain_policy.is_empty(),
        "one registry's policy never leaks into the shared set"
    );
}
