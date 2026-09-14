use super::*;
use crate::net::policy_config::{
    DetectionLevel, SecurityPluginConfig, SecurityPluginMode, SecurityRuleProfile, SecurityRuleSource,
};
use crate::security_engine::SecurityEnforcementAction;
use std::collections::BTreeMap;

#[tokio::test]
async fn unmatched_network_deny_has_a_primary_row_and_closed_audit_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&path, 8).unwrap());
    let rules =
        SecurityRuleSet::compile_profile(&SecurityRuleProfile::parse_toml("").unwrap(), SecurityRuleSource::User)
            .unwrap();
    let engine = NetworkSecurity {
        db: db.clone(),
        rules: Arc::new(RwLock::new(Arc::new(rules))),
        plugins: Arc::new(RwLock::new(Arc::new(BTreeMap::new()))),
    };
    let event = SecurityEvent::new(RuntimeSecurityEventType::NetworkConnect)
        .with_network(NetworkSecurityEvent::Flow(super::super::tests::private_flow()));
    assert_eq!(
        engine.evaluate_and_record(event.clone()).await.unwrap().action,
        SecurityEnforcementAction::Block
    );
    db.flush_checked().await.unwrap();
    let reader = capsem_logger::DbReader::open(&path).unwrap();
    let rows = reader
        .query_raw_with_params("SELECT event_type,event_json FROM transport_events", &[])
        .unwrap();
    assert!(rows.contains("network.connect"));
    assert!(rows.contains("block"));
    assert!(reader.recent_security_rule_events(10).unwrap().is_empty());
    db.shutdown_blocking();
    assert!(engine.evaluate_and_record(event).await.is_err());
}

#[tokio::test]
async fn plugin_block_survives_the_allow_rule_ledger_emission() {
    let db = Arc::new(DbWriter::open_in_memory(8).unwrap());
    let rules = SecurityRuleSet::compile_profile(
        &SecurityRuleProfile::parse_toml(
            r#"
[profiles.rules.allow_network]
name = "allow_network"
action = "allow"
match = 'network.mode == "private"'
"#,
        )
        .unwrap(),
        SecurityRuleSource::User,
    )
    .unwrap();
    let plugins = BTreeMap::from([(
        "dummy_post_allow".into(),
        SecurityPluginConfig {
            mode: SecurityPluginMode::Block,
            detection_level: DetectionLevel::High,
        },
    )]);
    let engine = NetworkSecurity {
        db: db.clone(),
        rules: Arc::new(RwLock::new(Arc::new(rules))),
        plugins: Arc::new(RwLock::new(Arc::new(plugins))),
    };
    let event = SecurityEvent::new(RuntimeSecurityEventType::NetworkConnect)
        .with_network(NetworkSecurityEvent::Flow(super::super::tests::private_flow()));
    assert_eq!(
        engine.evaluate_and_record(event).await.unwrap().action,
        SecurityEnforcementAction::Block
    );
    db.shutdown_blocking();
}
