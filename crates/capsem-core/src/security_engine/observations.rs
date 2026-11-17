//! Shared detection enrichment; observations never request access decisions.
use super::*;
use capsem_config::{CompiledEnvironmentDetectionRule, SecurityRuleEvaluation};

pub(super) fn record_rule_detection(event: &mut SecurityEvent, rule: &CompiledSecurityRule) {
    let Some(detection_level) = rule.detection_level else {
        return;
    };
    event.record_detection(SecurityDetectionEvent {
        source: SecurityDetectionSource::Rule,
        detection_level,
        rule_id: Some(rule.rule_id.clone()),
        plugin_id: None,
        action: Some(rule.action),
        plugin_mode: None,
        reason: rule.reason.clone(),
    });
}

pub(super) fn record_observations(event: &mut SecurityEvent, observations: &[&CompiledEnvironmentDetectionRule]) {
    for observation in observations {
        let detection = SecurityDetectionEvent {
            source: SecurityDetectionSource::Rule,
            detection_level: observation.level,
            rule_id: Some(observation.rule_id.clone()),
            plugin_id: None,
            action: None,
            plugin_mode: None,
            reason: None,
        };
        // A boundary may hand its already evaluated event to the ledger.
        // Preserve one observation across that handoff without losing a
        // different severity or evidence from an action-bearing rule.
        if !event.detections.contains(&detection) {
            event.record_detection(detection);
        }
    }
}

pub(super) fn event_with_rule_detections(
    event: &SecurityEvent,
    evaluation: &SecurityRuleEvaluation<'_>,
) -> SecurityEvent {
    let mut enriched = event.clone();
    for rule in evaluation.detections() {
        record_rule_detection(&mut enriched, rule);
    }
    record_observations(&mut enriched, evaluation.observations());
    enriched
}
