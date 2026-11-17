use super::*;

#[test]
fn environment_plugins_use_typed_options_and_independent_pii_directions() {
    let source = format!("{BASE}\n[plugins.pii_filter]\nmode='rewrite'\ndetection_level='high'\n[plugins.pii_filter.options.inbound]\nemail=false\nphone=true\n[plugins.pii_filter.options.outbound]\nemail=true\nphone=false\n[plugins.utf8_guard]\nmode='block'\n[plugins.utf8_guard.options]\ninbound=false\noutbound=true\n[plugins.secret_exfiltration]\nmode='ask'\n[plugins.dummy_pre_eicar]\nmode='block'\n[plugins.dummy_post_allow]\nmode='allow'\n");
    let doc = EnvironmentDocument::from_toml(&source).unwrap();
    assert!(EnvironmentDocument::from_toml(&doc.to_toml().unwrap()).unwrap() == doc);
    let pii = doc.specification.plugins.pii_filter.as_ref().unwrap();
    assert!(!pii.options.inbound.email && pii.options.inbound.phone);
    assert!(pii.options.outbound.email && !pii.options.outbound.phone);
    for bad in [
        source.replace("mode='rewrite'", "mode='rewrite'\nenabled=true"),
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
