use super::*;
use crate::net::policy_config::{
    DetectionLevel, SecurityPluginConfig, SecurityPluginMode, SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource,
};
use crate::security_engine::{evaluate_security_boundary, SecurityEnforcementAction, SecurityEvent};
use std::collections::BTreeMap;

#[test]
fn boot_generation_survives_json_consumers_without_numeric_rounding() {
    let mut flow = private_flow();
    flow.source.vm.as_mut().unwrap().generation = NonZeroU64::new(u64::MAX).unwrap();
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::NetworkConnect).with_network(NetworkSecurityEvent::Flow(flow));
    let wire = serde_json::to_value(event.serializable()).unwrap();
    assert_eq!(wire["network"]["source"]["vm"]["generation"], u64::MAX.to_string());
}

pub(crate) fn private_flow() -> NetworkFlow {
    let vm = |id: &str| {
        Some(NetworkVm {
            id: id.into(),
            name: id.into(),
            generation: NonZeroU64::new(9).unwrap(),
        })
    };
    NetworkFlow {
        connection_id: Uuid::from_u128(1),
        route: NetworkRoute::Private {
            network: NetworkIdentity {
                id: Uuid::from_u128(2),
                name: "eval".into(),
            },
        },
        side: NetworkSide::Source,
        protocol: NetworkProtocol::Tcp,
        source: NetworkEndpoint {
            vm: vm("client"),
            address: "10.128.0.2:45000".parse().unwrap(),
        },
        destination: NetworkEndpoint {
            vm: vm("redis"),
            address: "10.128.0.3:6379".parse().unwrap(),
        },
        report: None,
    }
}

fn rules(text: &str) -> SecurityRuleSet {
    SecurityRuleSet::compile_profile(
        &SecurityRuleProfile::parse_toml(text).unwrap(),
        SecurityRuleSource::User,
    )
    .unwrap()
}

#[test]
fn network_boundary_rejects_missing_facts() {
    let policy = rules("");
    let empty = SecurityEvent::new(RuntimeSecurityEventType::NetworkConnect);
    assert!(evaluate_security_boundary(&policy, BTreeMap::new(), empty).is_err());
}

#[tokio::test]
async fn network_ledger_rejects_relabeling_and_missing_facts_before_writing() {
    let writer = capsem_logger::DbWriter::open_in_memory(8).unwrap();
    let policy = rules(ALLOW_REDIS);
    let valid = SecurityEvent::new(RuntimeSecurityEventType::NetworkConnect)
        .with_network(NetworkSecurityEvent::Flow(private_flow()));
    for (event, kind) in [
        (valid, RuntimeSecurityEventType::HttpRequest),
        (
            SecurityEvent::new(RuntimeSecurityEventType::HttpRequest),
            RuntimeSecurityEventType::NetworkConnect,
        ),
        (
            SecurityEvent::new(RuntimeSecurityEventType::NetworkConnect),
            RuntimeSecurityEventType::NetworkConnect,
        ),
    ] {
        let id = crate::security_engine::SecurityEventId::new_uuid4();
        assert!(crate::security_engine::emit_matching_security_rules_with_decision(
            &writer,
            id.clone(),
            kind,
            &policy,
            &event,
            1,
        )
        .await
        .is_err());
        assert!(
            crate::security_engine::emit_matching_security_rules_with_decision_blocking(
                &writer, id, kind, &policy, &event, 1,
            )
            .is_err()
        );
    }
    writer.shutdown_blocking();
}

const ALLOW_REDIS: &str = r#"
[profiles.rules.redis]
name = "redis"
action = "allow"
match = 'network.mode == "private" && network.side == "source" && network.name == "eval" && network.source.vm_id == "client" && network.source.generation == "9" && network.destination.vm_id == "redis" && network.destination.port == "6379"'
"#;

#[test]
fn exact_owner_facts_allow_one_side_without_authorizing_the_other() {
    let policy = rules(ALLOW_REDIS);
    let mut flow = private_flow();
    for (side, expected) in [
        (NetworkSide::Source, SecurityEnforcementAction::Allow),
        (NetworkSide::Destination, SecurityEnforcementAction::Block),
    ] {
        flow.side = side;
        let event = SecurityEvent::new(RuntimeSecurityEventType::NetworkConnect)
            .with_network(NetworkSecurityEvent::Flow(flow.clone()));
        let result = evaluate_security_boundary(&policy, BTreeMap::new(), event).unwrap();
        assert_eq!(result.enforcement.action, expected);
    }
    flow.side = NetworkSide::Source;
    flow.source.vm.as_mut().unwrap().generation = NonZeroU64::new(10).unwrap();
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::NetworkConnect).with_network(NetworkSecurityEvent::Flow(flow));
    assert_eq!(
        evaluate_security_boundary(&policy, BTreeMap::new(), event)
            .unwrap()
            .enforcement
            .action,
        SecurityEnforcementAction::Block
    );
}

#[test]
fn network_authorization_retains_plugin_escalation_and_default_deny() {
    for (text, mode, expected) in [
        (ALLOW_REDIS, SecurityPluginMode::Allow, SecurityEnforcementAction::Allow),
        (ALLOW_REDIS, SecurityPluginMode::Ask, SecurityEnforcementAction::Ask),
        (ALLOW_REDIS, SecurityPluginMode::Block, SecurityEnforcementAction::Block),
        ("", SecurityPluginMode::Allow, SecurityEnforcementAction::Block),
    ] {
        let plugins = BTreeMap::from([(
            "dummy_post_allow".into(),
            SecurityPluginConfig {
                mode,
                detection_level: DetectionLevel::High,
            },
        )]);
        let event = SecurityEvent::new(RuntimeSecurityEventType::NetworkConnect)
            .with_network(NetworkSecurityEvent::Flow(private_flow()));
        let result = evaluate_security_boundary(&rules(text), plugins, event).unwrap();
        assert_eq!(result.enforcement.action, expected);
        assert!(result
            .event
            .plugin_executions
            .iter()
            .any(|execution| execution.plugin_id == "dummy_post_allow" && execution.applied));
    }
}

#[test]
fn lifecycle_and_synthetic_probe_cannot_masquerade_as_tcp_connects() {
    let mut flow = private_flow();
    flow.protocol = NetworkProtocol::SyntheticPing;
    flow.source.address.set_port(0);
    flow.destination.address.set_port(0);
    let probe = NetworkSecurityEvent::Flow(flow);
    probe.validate(RuntimeSecurityEventType::NetworkProbe).unwrap();
    assert!(probe.validate(RuntimeSecurityEventType::NetworkConnect).is_err());
    let result = evaluate_security_boundary(
        &rules(""),
        BTreeMap::new(),
        SecurityEvent::new(RuntimeSecurityEventType::NetworkProbe).with_network(probe),
    )
    .unwrap();
    assert_eq!(result.enforcement.action, SecurityEnforcementAction::Block);
    let lifecycle = NetworkSecurityEvent::Lifecycle {
        network: NetworkIdentity {
            id: Uuid::from_u128(2),
            name: "eval".into(),
        },
        vm: None,
        action: NetworkLifecycleAction::Retired,
    };
    lifecycle.validate(RuntimeSecurityEventType::NetworkLifecycle).unwrap();
    assert!(lifecycle.validate(RuntimeSecurityEventType::NetworkConnect).is_err());
    assert!(lifecycle.get("destination.ip").is_none());
}

#[test]
fn network_boundary_requires_an_explicit_allow() {
    let policy = rules("");
    let event = SecurityEvent::new(RuntimeSecurityEventType::NetworkConnect)
        .with_network(NetworkSecurityEvent::Flow(private_flow()));
    let result = evaluate_security_boundary(&policy, BTreeMap::new(), event).unwrap();
    assert_eq!(result.enforcement.action, SecurityEnforcementAction::Block);
}

#[tokio::test]
async fn network_ledger_decisions_cannot_restore_an_unmatched_allow() {
    let dir = tempfile::tempdir().unwrap();
    let writer = capsem_logger::DbWriter::open(&dir.path().join("session.db"), 8).unwrap();
    let event = SecurityEvent::new(RuntimeSecurityEventType::NetworkConnect)
        .with_network(NetworkSecurityEvent::Flow(private_flow()));
    let id = crate::security_engine::SecurityEventId::parse("abcdef123456").unwrap();
    let policy = rules("");
    let asynchronous = crate::security_engine::emit_matching_security_rules_with_decision(
        &writer,
        id.clone(),
        event.event_type,
        &policy,
        &event,
        1,
    )
    .await
    .unwrap();
    let synchronous = crate::security_engine::emit_matching_security_rules_with_decision_blocking(
        &writer,
        id,
        event.event_type,
        &policy,
        &event,
        1,
    )
    .unwrap();
    writer.shutdown_blocking();
    assert_eq!(asynchronous.enforcement.action, SecurityEnforcementAction::Block);
    assert_eq!(synchronous.enforcement.action, SecurityEnforcementAction::Block);
}

#[test]
fn valid_flow_has_no_payload_or_router_report_policy_fields() {
    let mut flow = private_flow();
    NetworkSecurityEvent::Flow(flow.clone())
        .validate(RuntimeSecurityEventType::NetworkConnect)
        .unwrap();
    flow.report = Some(NetworkReport {
        reason: NetworkReason::Complete,
        elapsed_ms: 10,
        bytes_sent: 7,
        bytes_received: 5,
    });
    let event = NetworkSecurityEvent::Flow(flow);
    event.validate(RuntimeSecurityEventType::NetworkClose).unwrap();
    assert!(event.validate(RuntimeSecurityEventType::NetworkConnect).is_err());
    for field in [
        "report.bytes_sent",
        "report.reason",
        "decision",
        "payload",
        "connection_id",
    ] {
        assert!(event.get(field).is_none());
    }
    let wire = serde_json::to_value(
        SecurityEvent::new(RuntimeSecurityEventType::NetworkClose)
            .with_network(event)
            .serializable(),
    )
    .unwrap();
    assert_eq!(wire["network"]["connection_id"], "00000000-0000-0000-0000-000000000001");
    assert_eq!(wire["network"]["report"]["bytes_sent"], 7);
    assert_eq!(wire["network"]["report"]["bytes_received"], 5);
    assert!(wire["network"].get("payload").is_none());
}

#[test]
fn untrusted_or_missing_endpoint_facts_fail_validation() {
    let original = private_flow();
    for mutation in 0..5 {
        let mut flow = original.clone();
        match mutation {
            0 => flow.source.vm = None,
            1 => flow.destination.vm = None,
            2 => flow.destination.vm.as_mut().unwrap().name = "redis\nforged audit row".into(),
            3 => flow.destination.address.set_port(0),
            _ => flow.connection_id = Uuid::nil(),
        }
        assert!(NetworkSecurityEvent::Flow(flow)
            .validate(RuntimeSecurityEventType::NetworkConnect)
            .is_err());
    }
}
