use super::super::SettingsFile;

#[test]
fn corporate_environment_plugin_options_roundtrip_without_direction_loss() {
    let source = r#"
[environment_plugins.pii_filter.inbound]
mode = "block"
detection_level = "high"
[environment_plugins.pii_filter.inbound.options]
email = false
phone = true
card = false
ssn = true
[environment_plugins.pii_filter.outbound]
mode = "allow"
[environment_plugins.credential_broker.options]
capture_enabled = false
"#;
    let parsed: SettingsFile = toml::from_str(source).unwrap();
    parsed.validate_metadata_contract().unwrap();
    let text = toml::to_string(&parsed).unwrap();
    let observed: toml::Table = toml::from_str(&text).unwrap();
    assert_eq!(
        observed["environment_plugins"]["pii_filter"]["inbound"]["mode"].as_str(),
        Some("block")
    );
    assert_eq!(
        observed["environment_plugins"]["pii_filter"]["inbound"]["options"]["email"].as_bool(),
        Some(false)
    );
    assert_eq!(
        observed["environment_plugins"]["pii_filter"]["outbound"]["mode"].as_str(),
        Some("allow")
    );
    assert_eq!(
        observed["environment_plugins"]["credential_broker"]["options"]["capture_enabled"].as_bool(),
        Some(false)
    );
    assert_eq!(toml::from_str::<SettingsFile>(&text).unwrap(), parsed);
}

#[test]
fn corporate_environment_plugin_options_reject_unknown_shape_and_mandatory_disable() {
    for source in [
        "[environment_plugins.pii_filter.inbound.options]\nunknown=true",
        "[environment_plugins.pii_filter.inbound.options]\nemail='false'",
        "[environment_plugins.unknown]\nmode='block'",
        "[environment_plugins.credential_broker]\nmode='disable'",
        "[environment_plugins.log_sanitizer]\nmode='disable'",
        "[environment_plugins.pii_filter.inbound]\nmode='log'",
    ] {
        let accepted =
            toml::from_str::<SettingsFile>(source).is_ok_and(|file| file.validate_metadata_contract().is_ok());
        assert!(!accepted, "invalid corporate plugin shape was accepted: {source}");
    }
}

#[test]
fn corporate_environment_plugin_files_merge_whole_entries_not_individual_options() {
    let first: SettingsFile = toml::from_str("[environment_plugins.pii_filter.inbound]\nmode='block'\n[environment_plugins.pii_filter.inbound.options]\nemail=false").unwrap();
    let fallback: SettingsFile = toml::from_str(
        "[environment_plugins.pii_filter.outbound]\nmode='rewrite'\n[environment_plugins.utf8_guard]\nmode='block'",
    )
    .unwrap();
    let mut merged = first.environment_plugins.unwrap();
    merged.merge_first_wins(fallback.environment_plugins.unwrap());
    let pii = merged.pii_filter.unwrap();
    assert_eq!(pii.inbound.mode, crate::SecurityPluginMode::Block);
    assert!(!pii.inbound.options.email);
    assert_eq!(pii.outbound.mode, crate::SecurityPluginMode::Disable);
    assert_eq!(merged.utf8_guard.unwrap().mode, crate::SecurityPluginMode::Block);
}
