use super::*;
use crate::environment::{EnvironmentPolicyInputs, RuleCatalogInputs};
use crate::security_engine::{
    HttpSecurityEvent, RuntimeSecurityEventType, SecurityActionRegistry, SecurityDecisionKind, SecurityEmitError,
    SecurityEvent, SecurityEventEmitter, SecurityEventEngine,
};
use capsem_config::{environment::*, SettingsFile};
use std::sync::Arc;

struct Ignore;
impl SecurityEventEmitter for Ignore {
    fn emit(&self, _: SecurityEvent) -> Result<(), SecurityEmitError> {
        Ok(())
    }
}
fn specification() -> EnvironmentSpecification {
    toml::from_str("name='development'\n[image]\nreference='development:stable'\nresolved_digest='sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'").unwrap()
}
fn document(action: &str) -> serde_json::Value {
    let mut metadata: serde_json::Value =
        serde_json::from_slice::<serde_json::Value>(&bytes()).unwrap()["entries"][0]["metadata"].clone();
    metadata["id"] = serde_json::json!("endpoint");
    let access = serde_json::json!({"metadata": metadata,"parameters":{"host":{"type":"text","default":"api.example","max_bytes":128}},"definition":{"kind":"access","rule":{"name":"Endpoint","action":action,"match":"http.host == {{host}}"}}});
    metadata["id"] = serde_json::json!("observe");
    let detection = serde_json::json!({"metadata":metadata,"parameters":{"host":{"type":"text","default":"api.example","max_bytes":128}},"definition":{"kind":"detection","rule":{"name":"Observe","level":"high","match":"http.host == {{host}}"}}});
    serde_json::json!({"schema_version":1,"kind":"rules","entries":[access,detection]})
}
fn snapshot(value: serde_json::Value, identity: char) -> RuleCatalogContents {
    RuleCatalogContents::decode(
        source(CatalogKind::Rules),
        Digest::parse(&format!("sha256:{}", identity.to_string().repeat(64))).unwrap(),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap()
}
fn event() -> SecurityEvent {
    SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("live.example".into()),
        ..Default::default()
    })
}

#[test]
fn catalog_rule_compilation_preserves_exact_pinned_content_with_live_access_boot_detections_and_current_corp() {
    let pinned = snapshot(document("allow"), 'a');
    let mut refreshed = document("block");
    refreshed["entries"][1]["definition"]["rule"]["level"] = serde_json::json!("critical");
    let current = snapshot(refreshed, 'b');
    let catalogs = RuleCatalogInputs::new(&[&current], &[&pinned]).unwrap();
    let mut boot = specification();
    boot.access
        .catalog
        .selected
        .push(pinned.pin("endpoint", "1.0.0").unwrap());
    boot.detection
        .catalog
        .selected
        .push(pinned.pin("observe", "1.0.0").unwrap());
    let default_corp = SettingsFile::default();
    let defaults = EnvironmentPolicyInputs::new(&boot, &boot, &default_corp)
        .compile_rules_with_catalogs(&catalogs)
        .unwrap();
    let default_event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("api.example".into()),
        ..Default::default()
    });
    let engine = SecurityEventEngine::new(SecurityActionRegistry::new(), Arc::new(Ignore));
    let default_result = engine.apply_matching_rules_and_emit(&defaults, default_event).unwrap();
    assert_eq!(default_result.decision.effective, SecurityDecisionKind::Allow);
    assert!(default_result
        .detections
        .iter()
        .any(|d| d.rule_id.as_deref() == Some("environment.detection.catalog.observe")));
    boot.access.parameters.insert(
        "endpoint".into(),
        [("host".into(), EnvironmentRuleParameter::Text("boot.example".into()))].into(),
    );
    boot.detection.parameters.insert(
        "observe".into(),
        [("host".into(), EnvironmentRuleParameter::Text("live.example".into()))].into(),
    );
    let mut live = boot.clone();
    live.access
        .parameters
        .get_mut("endpoint")
        .unwrap()
        .insert("host".into(), EnvironmentRuleParameter::Text("live.example".into()));
    live.detection
        .parameters
        .get_mut("observe")
        .unwrap()
        .insert("host".into(), EnvironmentRuleParameter::Text("next.example".into()));
    let corp: SettingsFile = toml::from_str(
        "[corp.rules.mandatory]\nname='mandatory'\naction='block'\nmatch='http.host == \"live.example\"'",
    )
    .unwrap();
    for (corp, expected) in [
        (SettingsFile::default(), SecurityDecisionKind::Allow),
        (corp, SecurityDecisionKind::Block),
    ] {
        let rules = EnvironmentPolicyInputs::new(&boot, &live, &corp)
            .compile_rules_with_catalogs(&catalogs)
            .unwrap();
        let engine = SecurityEventEngine::new(SecurityActionRegistry::new(), Arc::new(Ignore));
        let result = engine.apply_matching_rules_and_emit(&rules, event()).unwrap();
        assert_eq!(result.decision.effective, expected);
        assert!(rules
            .rules()
            .iter()
            .any(|r| r.rule_id == "environment.access.catalog.endpoint"
                && r.action == capsem_config::SecurityRuleAction::Allow));
        let observation = result
            .detections
            .iter()
            .find(|d| d.rule_id.as_deref() == Some("environment.detection.catalog.observe"))
            .unwrap();
        assert_eq!(observation.detection_level, capsem_config::DetectionLevel::High);
        assert_eq!(observation.action, None);
    }
}

#[test]
fn catalog_rule_compilation_refuses_current_revocation_omission_or_unavailable_exact_content() {
    let pinned = snapshot(document("allow"), 'a');
    let mut boot = specification();
    boot.access
        .catalog
        .selected
        .push(pinned.pin("endpoint", "1.0.0").unwrap());
    let corp = SettingsFile::default();
    for case in 0..3 {
        let mut value = document("block");
        match case {
            0 => {
                value["entries"][0]["metadata"]["status"] = serde_json::json!("revoked");
                value["entries"][0]["metadata"]["status_reason"] = serde_json::json!("Withdrawn");
            }
            1 => {
                value["entries"].as_array_mut().unwrap().remove(0);
            }
            _ => value["entries"][0]["metadata"]["compatibility"]["min_runtime_contract"] = serde_json::json!(2),
        }
        let current = snapshot(value, 'b');
        let catalogs = RuleCatalogInputs::new(&[&current], &[&pinned]).unwrap();
        assert!(EnvironmentPolicyInputs::new(&boot, &boot, &corp)
            .compile_rules_with_catalogs(&catalogs)
            .is_err());
    }
    let current = snapshot(document("block"), 'b');
    let catalogs = RuleCatalogInputs::new(&[&current], &[]).unwrap();
    assert!(
        EnvironmentPolicyInputs::new(&boot, &boot, &corp)
            .compile_rules_with_catalogs(&catalogs)
            .is_err(),
        "never substitute refreshed content for a missing pinned digest"
    );
    let catalogs = RuleCatalogInputs::new(&[], &[&pinned]).unwrap();
    assert!(
        EnvironmentPolicyInputs::new(&boot, &boot, &corp)
            .compile_rules_with_catalogs(&catalogs)
            .is_err(),
        "cached content alone is not current admission"
    );
}

#[test]
fn catalog_rule_compilation_rejects_wrong_kind_custom_collisions_and_unknown_parameters() {
    let current = snapshot(document("allow"), 'a');
    let catalogs = RuleCatalogInputs::new(&[&current], &[]).unwrap();
    let corp = SettingsFile::default();
    let mut boot = specification();
    boot.access
        .catalog
        .selected
        .push(current.pin("observe", "1.0.0").unwrap());
    assert!(EnvironmentPolicyInputs::new(&boot, &boot, &corp)
        .compile_rules_with_catalogs(&catalogs)
        .is_err());
    boot.access.catalog.selected.clear();
    boot.detection
        .catalog
        .selected
        .push(current.pin("endpoint", "1.0.0").unwrap());
    assert!(EnvironmentPolicyInputs::new(&boot, &boot, &corp)
        .compile_rules_with_catalogs(&catalogs)
        .is_err());
    boot.detection.catalog.selected.clear();
    boot.access
        .catalog
        .selected
        .push(current.pin("endpoint", "1.0.0").unwrap());
    boot.access.custom.insert(
        "endpoint".into(),
        toml::from_str("name='Collision'\naction='allow'\nmatch='http.valid == \"true\"'").unwrap(),
    );
    assert!(EnvironmentPolicyInputs::new(&boot, &boot, &corp)
        .compile_rules_with_catalogs(&catalogs)
        .is_err());
    boot.access.custom.clear();
    boot.access.parameters.insert(
        "endpoint".into(),
        [("unknown".into(), EnvironmentRuleParameter::Boolean(true))].into(),
    );
    assert!(EnvironmentPolicyInputs::new(&boot, &boot, &corp)
        .compile_rules_with_catalogs(&catalogs)
        .is_err());
    boot.access.parameters.clear();
    boot.access
        .catalog
        .selected
        .push(boot.access.catalog.selected[0].clone());
    assert!(EnvironmentPolicyInputs::new(&boot, &boot, &corp)
        .compile_rules_with_catalogs(&catalogs)
        .is_err());
}

#[test]
fn catalog_rule_compilation_bounds_snapshot_inputs_and_refuses_ambiguous_current_sources() {
    let current = snapshot(document("allow"), 'a');
    let other = snapshot(document("block"), 'b');
    assert!(RuleCatalogInputs::new(&[&current, &other], &[]).is_err());
    let maximum = capsem_proto::ENVIRONMENT_MAX_CATALOG_SELECTIONS * 2;
    let bytes = serde_json::to_vec(&document("allow")).unwrap();
    let snapshots: Vec<_> = (0..=maximum)
        .map(|n| {
            let descriptor = CatalogSourceDescriptor {
                reference: format!("registry.example/catalog-{n}:stable"),
                schema_version: 1,
            };
            let source = ValidatedCatalogSource::new(
                CatalogKind::Rules,
                SelectedCatalogSource {
                    source: &descriptor,
                    origin: CatalogSourceOrigin::Manifest,
                },
            )
            .unwrap();
            RuleCatalogContents::decode(source, digest(), bytes.clone()).unwrap()
        })
        .collect();
    let references: Vec<_> = snapshots.iter().collect();
    assert!(RuleCatalogInputs::new(&references[..maximum], &[]).is_ok());
    assert!(RuleCatalogInputs::new(&references, &[]).is_err());
    assert!(RuleCatalogInputs::new(&[], &references).is_err());
}
