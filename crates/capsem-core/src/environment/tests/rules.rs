use super::*;
use crate::security_engine::{
    HttpSecurityEvent, McpSecurityEvent, RuntimeSecurityEventType, SecurityActionRegistry, SecurityDecisionKind,
    SecurityEmitError, SecurityEvent, SecurityEventEmitter, SecurityEventEngine,
};
use capsem_config::{DetectionLevel, SecurityRuleAction, SettingsFile};
use std::sync::Arc;

struct Ignore;
impl SecurityEventEmitter for Ignore {
    fn emit(&self, _: SecurityEvent) -> Result<(), SecurityEmitError> {
        Ok(())
    }
}

fn http_event() -> SecurityEvent {
    SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("scoped.example".into()),
        ..Default::default()
    })
}

#[test]
fn environment_rule_composition_uses_live_access_boot_detection_and_fresh_corporate_rules() {
    let mut boot = specification();
    boot.access.custom.insert(
        "scoped".into(),
        toml::from_str("name='Scoped access'\naction='block'\nmatch='http.host == \"scoped.example\"'").unwrap(),
    );
    boot.detection.custom.insert(
        "observe".into(),
        toml::from_str("name='Observe'\nlevel='high'\nmatch='http.host == \"scoped.example\"'").unwrap(),
    );
    let mut live = boot.clone();
    live.access.custom.get_mut("scoped").unwrap().action = SecurityRuleAction::Allow;
    live.detection.custom.get_mut("observe").unwrap().level = DetectionLevel::Critical;
    let current: SettingsFile = toml::from_str(
        "[corp.rules.restricted]\nname='restricted'\naction='block'\nmatch='http.host == \"scoped.example\"'",
    )
    .unwrap();
    let engine = SecurityEventEngine::new(SecurityActionRegistry::new(), Arc::new(Ignore));
    for (corp, expected) in [
        (SettingsFile::default(), SecurityDecisionKind::Allow),
        (current, SecurityDecisionKind::Block),
    ] {
        let rules = EnvironmentPolicyInputs::new(&boot, &live, &corp)
            .compile_rules()
            .unwrap();
        let result = engine.apply_matching_rules_and_emit(&rules, http_event()).unwrap();
        assert_eq!(result.decision.effective, expected);
        let observed: Vec<_> = result
            .detections
            .iter()
            .filter(|d| d.rule_id.as_deref() == Some("environment.detection.observe"))
            .collect();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].detection_level, DetectionLevel::High);
        assert_eq!(observed[0].action, None);
        let access = rules
            .rules()
            .iter()
            .find(|r| r.rule_id == "environment.access.scoped")
            .unwrap();
        assert_eq!(access.action, SecurityRuleAction::Allow);
        assert!(!access.corp_locked);
    }
}

#[test]
fn environment_rule_composition_includes_scoped_mcp_permissions_in_real_decisions() {
    let mut boot = specification();
    boot.mcps.permissions.insert(
        "local".into(),
        EnvironmentMcpPermissions {
            default: EnvironmentCapabilityDecision::Block,
            tools: [("read".into(), EnvironmentCapabilityDecision::Allow)].into(),
            ..Default::default()
        },
    );
    let corp = SettingsFile::default();
    let rules = EnvironmentPolicyInputs::new(&boot, &boot, &corp)
        .compile_rules()
        .unwrap();
    let engine = SecurityEventEngine::new(SecurityActionRegistry::new(), Arc::new(Ignore));
    for (tool, expected) in [
        ("read", SecurityDecisionKind::Allow),
        ("write", SecurityDecisionKind::Block),
    ] {
        let event = SecurityEvent::new(RuntimeSecurityEventType::McpToolCall).with_mcp(McpSecurityEvent {
            server_name: Some("local".into()),
            tool_call_name: Some(format!("local__{tool}")),
            ..Default::default()
        });
        assert_eq!(
            engine
                .apply_matching_rules_and_emit(&rules, event)
                .unwrap()
                .decision
                .effective,
            expected
        );
    }
}

#[test]
fn environment_rule_composition_refuses_unavailable_catalogs_invalid_rules_and_oversize_sections() {
    let corp = SettingsFile::default();
    let pin = EnvironmentCatalogPin {
        id: "preset".into(),
        version: "1".into(),
        catalog_reference: "registry.example/rules:stable".into(),
        resolved_digest: format!("sha256:{}", "a".repeat(64)),
    };
    for detection in [false, true] {
        let mut selected = specification();
        if detection {
            selected.detection.catalog.selected.push(pin.clone());
        } else {
            selected.access.catalog.selected.push(pin.clone());
        }
        assert!(EnvironmentPolicyInputs::new(&selected, &selected, &corp)
            .compile_rules()
            .is_err());
    }
    let mut invalid = specification();
    invalid.access.parameters.insert("missing".into(), Default::default());
    assert!(EnvironmentPolicyInputs::new(&invalid, &invalid, &corp)
        .compile_rules()
        .is_err());
    invalid.access.parameters.clear();
    let rule: EnvironmentAccessRule =
        toml::from_str("name='Invalid'\naction='allow'\nmatch='http.unknown == \"x\"'").unwrap();
    invalid.access.custom.insert("invalid".into(), rule);
    assert!(EnvironmentPolicyInputs::new(&invalid, &invalid, &corp)
        .compile_rules()
        .is_err());
    invalid.access.custom.clear();
    for n in 0..=capsem_proto::ENVIRONMENT_MAX_RULES {
        invalid.detection.custom.insert(
            format!("probe{n}"),
            toml::from_str("name='Probe'\nlevel='high'\nmatch='http.valid == \"true\"'").unwrap(),
        );
    }
    assert!(EnvironmentPolicyInputs::new(&invalid, &invalid, &corp)
        .compile_rules()
        .is_err());
    let invalid_corp: SettingsFile =
        toml::from_str("[corp.rules.invalid]\nname='invalid'\naction='block'\nmatch='http.typo == \"x\"'").unwrap();
    let boot = specification();
    assert!(EnvironmentPolicyInputs::new(&boot, &boot, &invalid_corp)
        .compile_rules()
        .is_err());
}
