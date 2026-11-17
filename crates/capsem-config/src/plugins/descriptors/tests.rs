use super::*;
use std::collections::BTreeSet;

#[test]
fn plugin_descriptors_cover_every_builtin_and_match_typed_modes() {
    let descriptors = plugin_descriptors();
    assert_eq!(descriptors.len(), PluginId::ALL.len());
    let mut ids = BTreeSet::new();
    for descriptor in descriptors {
        assert!(ids.insert(descriptor.id));
        assert_eq!(plugin_descriptor(descriptor.id), descriptor);
        assert_eq!(descriptor.version, 1);
        assert_eq!(
            descriptor.required,
            !descriptor.modes.contains(&SecurityPluginMode::Disable)
        );
        let expected_modes = match descriptor.id {
            PluginId::PiiFilter => PiiFilterOptions::MODES,
            PluginId::SecretExfiltration => SecretExfiltrationOptions::MODES,
            PluginId::Utf8Guard => Utf8GuardOptions::MODES,
            PluginId::CredentialBroker => CredentialBrokerOptions::MODES,
            PluginId::LogSanitizer => LogSanitizerOptions::MODES,
            PluginId::DummyPreEicar => DummyPreEicarOptions::MODES,
            PluginId::DummyPostAllow => DummyPostAllowOptions::MODES,
        };
        assert_eq!(descriptor.modes, expected_modes);
        let mut fields = BTreeSet::new();
        for field in &descriptor.fields {
            assert!(fields.insert(field.setting.id.as_str()));
            assert!(
                field.setting.id != "enabled" && !field.setting.id.ends_with(".enabled"),
                "mode is the only persisted on/off control"
            );
            if field.setting.id.ends_with("mode") {
                assert_eq!(field.setting.setting_type, crate::SettingType::Text);
                assert_eq!(field.setting.metadata.widget, Some(crate::Widget::Select));
                assert!(descriptor
                    .modes
                    .iter()
                    .any(|mode| field.setting.default_value.as_text() == Some(mode.as_str())));
                assert_eq!(
                    field.setting.metadata.choices,
                    descriptor.modes.iter().map(|mode| mode.as_str()).collect::<Vec<_>>()
                );
            }
            for dependency in &field.dependencies {
                assert!(descriptor
                    .fields
                    .iter()
                    .any(|other| other.setting.id == dependency.field));
                assert_eq!(dependency.not_equals, crate::SettingValue::Text("disable".into()));
            }
        }
    }
}

#[test]
fn plugin_descriptor_field_defaults_equal_the_typed_toml_defaults() {
    let defaults = EnvironmentPlugins {
        pii_filter: Some(Default::default()),
        secret_exfiltration: Some(Default::default()),
        utf8_guard: Some(Default::default()),
        credential_broker: Some(Default::default()),
        log_sanitizer: Some(Default::default()),
        dummy_pre_eicar: Some(Default::default()),
        dummy_post_allow: Some(Default::default()),
    };
    let defaults = toml::Value::try_from(&defaults).unwrap();
    for descriptor in plugin_descriptors() {
        let plugin = &defaults[descriptor.id.as_str()];
        for field in &descriptor.fields {
            let mut actual = plugin;
            for part in field.setting.id.split('.') {
                actual = actual
                    .get(part)
                    .unwrap_or_else(|| panic!("{} {} missing typed default", descriptor.id.as_str(), field.setting.id));
            }
            match &field.setting.default_value {
                SettingValue::Bool(expected) => assert_eq!(actual.as_bool(), Some(*expected)),
                SettingValue::Text(expected) => assert_eq!(actual.as_str(), Some(expected.as_str())),
                other => panic!("unexpected current builtin field type {other:?}"),
            }
        }
    }
}

#[test]
fn plugin_descriptors_expose_nested_pii_and_only_real_plugin_specific_options() {
    let pii = plugin_descriptor(PluginId::PiiFilter);
    assert_eq!(
        pii.groups.iter().map(|group| group.path.as_str()).collect::<Vec<_>>(),
        ["inbound", "outbound"]
    );
    for direction in ["inbound", "outbound"] {
        for option in ["email", "phone", "card", "ssn"] {
            let path = format!("{direction}.options.{option}");
            let field = pii.fields.iter().find(|field| field.setting.id == path).unwrap();
            assert_eq!(field.setting.default_value, crate::SettingValue::Bool(true));
            assert_eq!(field.dependencies[0].field, format!("{direction}.mode"));
        }
    }
    for id in [
        PluginId::SecretExfiltration,
        PluginId::LogSanitizer,
        PluginId::DummyPreEicar,
        PluginId::DummyPostAllow,
    ] {
        assert!(!plugin_descriptor(id)
            .fields
            .iter()
            .any(|field| field.setting.id.starts_with("options.")));
    }
    let broker = plugin_descriptor(PluginId::CredentialBroker);
    assert!(broker.required);
    assert_eq!(
        broker.detail_capabilities,
        [PluginDetailCapability::CredentialInventory]
    );
    assert!(broker
        .fields
        .iter()
        .any(|field| field.setting.id == "options.capture_enabled"
            && field.setting.default_value == crate::SettingValue::Bool(true)));
    assert_eq!(plugin_descriptor(PluginId::Utf8Guard).modes, Utf8GuardOptions::MODES);
    for id in [PluginId::PiiFilter, PluginId::SecretExfiltration, PluginId::Utf8Guard] {
        assert_eq!(plugin_descriptor(id).event_families, [PluginEventFamily::Http]);
        assert!(!plugin_descriptor(id).limitations.is_empty());
    }
    for id in [PluginId::DummyPreEicar, PluginId::DummyPostAllow] {
        assert!(plugin_descriptor(id).diagnostic);
        assert_eq!(
            plugin_descriptor(id)
                .fields
                .iter()
                .find(|field| field.setting.id == "mode")
                .unwrap()
                .setting
                .default_value
                .as_text(),
            Some("disable")
        );
    }
}
