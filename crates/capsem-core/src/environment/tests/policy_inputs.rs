use super::*;
use crate::environment::EnvironmentPolicyInputs;

#[test]
fn environment_policy_inputs_separate_boot_settings_from_live_access_without_copying() {
    let mut boot = specification();
    boot.secrets.insert(
        "api".into(),
        EnvironmentSecretBinding {
            binding_id: "00000000-0000-4000-8000-000000000001".into(),
            env_name: None,
        },
    );
    boot.access.custom.insert(
        "network".into(),
        toml::from_str("name='Network'\naction='block'\nmatch='http.host == \"example.org\"'").unwrap(),
    );
    boot.detection.custom.insert(
        "observe".into(),
        toml::from_str("name='Observe'\nlevel='low'\nmatch='http.host == \"example.org\"'").unwrap(),
    );
    boot.plugins.pii_filter = Some(capsem_config::plugins::PluginConfiguration {
        mode: capsem_config::SecurityPluginMode::Block,
        ..Default::default()
    });
    boot.mcps.catalog.selected.push(EnvironmentCatalogPin {
        id: "catalog-server".into(),
        version: "1.0.0".into(),
        catalog_reference: "ghcr.io/google/capsem/mcps:stable".into(),
        resolved_digest: format!("sha256:{}", "a".repeat(64)),
    });
    boot.mcps.servers.insert(
        "remote".into(),
        EnvironmentMcpDefinition::Remote {
            url: "https://boot.example.org/rpc".into(),
            headers: Default::default(),
            binding_id: None,
        },
    );
    boot.mcps.permissions.insert("remote".into(), Default::default());
    let mut live = boot.clone();
    live.name = "new-metadata".into();
    live.variables
        .insert("DEFERRED".into(), "must-not-enter-live-policy".into());
    live.secrets.get_mut("api").unwrap().binding_id = "00000000-0000-4000-8000-000000000002".into();
    live.packages.allows_package_manager = true;
    live.packages.registries = vec!["npm".into()];
    live.access.custom.get_mut("network").unwrap().action = capsem_config::SecurityRuleAction::Allow;
    live.detection.custom.get_mut("observe").unwrap().level = capsem_config::DetectionLevel::Critical;
    live.plugins.pii_filter.as_mut().unwrap().mode = capsem_config::SecurityPluginMode::Rewrite;
    live.mcps.catalog.selected[0].resolved_digest = format!("sha256:{}", "b".repeat(64));
    live.mcps.servers.insert(
        "remote".into(),
        EnvironmentMcpDefinition::Remote {
            url: "https://desired.example.org/rpc".into(),
            headers: Default::default(),
            binding_id: None,
        },
    );
    live.mcps.permissions.get_mut("remote").unwrap().default = EnvironmentCapabilityDecision::Allow;
    boot.validate().unwrap();
    live.validate().unwrap();
    let corporate = capsem_config::SettingsFile::default();
    let inputs = EnvironmentPolicyInputs::new(&boot, &live, &corporate);
    assert!(std::ptr::eq(inputs.access_rules(), &live.access));
    assert!(std::ptr::eq(inputs.secret_access(), &live.secrets));
    assert!(std::ptr::eq(inputs.package_access(), &live.packages));
    assert!(std::ptr::eq(inputs.mcp_permissions(), &live.mcps.permissions));
    assert!(std::ptr::eq(inputs.detection_rules(), &boot.detection));
    assert!(std::ptr::eq(inputs.plugins(), &boot.plugins));
    assert!(std::ptr::eq(inputs.mcp_catalog(), &boot.mcps.catalog));
    assert!(std::ptr::eq(inputs.mcp_servers(), &boot.mcps.servers));
    assert!(std::ptr::eq(inputs.corporate(), &corporate));
    assert_eq!(
        inputs.detection_rules().custom["observe"].level,
        capsem_config::DetectionLevel::Low
    );
    assert_eq!(
        inputs.plugins().pii_filter.as_ref().unwrap().mode,
        capsem_config::SecurityPluginMode::Block
    );
    assert_eq!(
        inputs.mcp_permissions()["remote"].default,
        EnvironmentCapabilityDecision::Allow
    );
}

#[test]
fn environment_policy_inputs_require_separate_current_corporate_configuration() {
    let boot = specification();
    let live = boot.clone();
    let mut first = capsem_config::SettingsFile::default();
    first.network.dns.upstreams = vec!["1.1.1.1:53".into()];
    let mut current = first.clone();
    current.network.dns.upstreams = vec!["9.9.9.9:53".into()];
    let old = EnvironmentPolicyInputs::new(&boot, &live, &first);
    let new = EnvironmentPolicyInputs::new(&boot, &live, &current);
    assert_eq!(old.corporate().network.dns.upstreams, ["1.1.1.1:53"]);
    assert_eq!(new.corporate().network.dns.upstreams, ["9.9.9.9:53"]);
    assert!(std::ptr::eq(new.corporate(), &current));
}
