use super::*;

#[test]
fn environment_plugins_use_typed_options_and_independent_pii_directions() {
    let source = format!(
        "{BASE}\n{}",
        r#"
[plugins.pii_filter.inbound]
mode='rewrite'
detection_level='high'
[plugins.pii_filter.inbound.options]
email=false
phone=true
[plugins.pii_filter.outbound]
mode='block'
detection_level='low'
[plugins.pii_filter.outbound.options]
email=true
phone=false
[plugins.utf8_guard]
mode='block'
[plugins.utf8_guard.options]
inbound=false
outbound=true
[plugins.secret_exfiltration]
mode='ask'
[plugins.dummy_pre_eicar]
mode='block'
[plugins.dummy_post_allow]
mode='allow'
"#
    );
    let doc = EnvironmentDocument::from_toml(&source).unwrap();
    assert!(EnvironmentDocument::from_toml(&doc.to_toml().unwrap()).unwrap() == doc);
    let pii = doc.specification.plugins.pii_filter.as_ref().unwrap();
    assert!(!pii.inbound.options.email && pii.inbound.options.phone);
    assert!(pii.outbound.options.email && !pii.outbound.options.phone);
    assert_eq!(pii.inbound.mode, crate::SecurityPluginMode::Rewrite);
    assert_eq!(pii.outbound.mode, crate::SecurityPluginMode::Block);
    assert_eq!(pii.inbound.detection_level, crate::DetectionLevel::High);
    assert_eq!(pii.outbound.detection_level, crate::DetectionLevel::Low);
    assert!(!toml::to_string(pii).unwrap().contains("enabled"));
    for bad in [
        source.replace("mode='rewrite'", "mode='rewrite'\nenabled=true"),
        source.replace(
            "[plugins.pii_filter.inbound.options]",
            "[plugins.pii_filter.inbound.options]\nenabled=true",
        ),
        format!("{BASE}\n[plugins.pii_filter]\nmode='rewrite'"),
        source.replace(
            "inbound=false\noutbound=true",
            "inbound=false\noutbound=true\nemail=true",
        ),
        source.replace(
            "[plugins.secret_exfiltration]\nmode='ask'",
            "[plugins.secret_exfiltration]\nmode='ask'\n[plugins.secret_exfiltration.options]\nphone=true",
        ),
    ] {
        assert!(EnvironmentDocument::from_toml(&bad).is_err());
    }
}

#[test]
fn environment_plugins_cannot_disable_required_broker_or_log_sanitization() {
    for id in ["credential_broker", "log_sanitizer"] {
        for mode in ["disable", "allow", "ask", "block"] {
            assert!(EnvironmentDocument::from_toml(&format!("{BASE}\n[plugins.{id}]\nmode='{mode}'\n")).is_err());
        }
        assert!(EnvironmentDocument::from_toml(&format!("{BASE}\n[plugins.{id}]\nmode='rewrite'\n")).is_ok());
    }
    let doc = EnvironmentDocument::from_toml(&format!(
        "{BASE}\n[plugins.credential_broker.options]\ncapture_enabled=false\n"
    ))
    .unwrap();
    let broker = doc.specification.plugins.credential_broker.as_ref().unwrap();
    assert!(!broker.options.capture_enabled);
    assert_eq!(broker.mode, crate::SecurityPluginMode::Rewrite);
    assert!(EnvironmentDocument::from_toml(&doc.to_toml().unwrap()).unwrap() == doc);
    assert!(doc.to_toml().unwrap().contains("mode = \"rewrite\""));
    for id in ["utf8_guard", "dummy_pre_eicar", "dummy_post_allow"] {
        assert!(EnvironmentDocument::from_toml(&format!("{BASE}\n[plugins.{id}]\nmode='rewrite'\n")).is_err());
    }
    assert!(EnvironmentDocument::from_toml(&format!("{BASE}\n[plugins.unknown]\nmode='allow'\n")).is_err());
}

#[test]
fn environment_plugins_defaults_match_typed_identity_and_mode_contracts() {
    use crate::plugins::*;
    use crate::SecurityPluginMode::{Disable, Rewrite};
    assert_eq!(
        PluginId::ALL.map(PluginId::as_str),
        [
            "pii_filter",
            "secret_exfiltration",
            "utf8_guard",
            "credential_broker",
            "log_sanitizer",
            "dummy_pre_eicar",
            "dummy_post_allow"
        ]
    );
    assert_eq!(PluginConfiguration::<PiiFilterOptions>::default().mode, Disable);
    let pii = PiiFilterConfiguration::default();
    assert_eq!(pii.inbound.mode, Disable);
    assert_eq!(pii.outbound.mode, Disable);
    pii.validate().unwrap();
    assert_eq!(
        PluginConfiguration::<SecretExfiltrationOptions>::default().mode,
        Disable
    );
    assert_eq!(PluginConfiguration::<Utf8GuardOptions>::default().mode, Disable);
    assert_eq!(PluginConfiguration::<DummyPreEicarOptions>::default().mode, Disable);
    assert_eq!(PluginConfiguration::<DummyPostAllowOptions>::default().mode, Disable);
    let broker = PluginConfiguration::<CredentialBrokerOptions>::default();
    let sanitizer = PluginConfiguration::<LogSanitizerOptions>::default();
    assert_eq!(broker.mode, Rewrite);
    assert_eq!(sanitizer.mode, Rewrite);
    assert!(broker.options.capture_enabled);
    broker.validate().unwrap();
    sanitizer.validate().unwrap();
    assert_eq!(CredentialBrokerOptions::ID, PluginId::CredentialBroker);
    assert_eq!(LogSanitizerOptions::ID, PluginId::LogSanitizer);
}
