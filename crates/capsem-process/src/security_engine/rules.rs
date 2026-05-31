use std::sync::{Arc, Mutex};

use capsem_core::net::mitm_proxy::RuntimeSecurityEngine;
use capsem_core::settings_profiles::{self, EffectiveRule, RuleDecision};
use capsem_security_engine::{
    CelEnforcementEvaluator, CelEnforcementRule, EventMutation, SecurityDecisionAction,
    SecurityEngine, SecurityEventType,
};
use tracing::{info, warn};

use super::RuntimeRuleMatchAccumulator;

pub(super) fn runtime_enforcement_rules_from_effective(
    effective: &settings_profiles::EffectiveVmSettings,
) -> Vec<CelEnforcementRule> {
    let mut rules: Vec<&EffectiveRule> = effective.rules.iter().collect();
    rules.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.id.cmp(&right.id))
    });
    rules
        .into_iter()
        .filter_map(runtime_enforcement_rule_from_effective)
        .collect()
}

pub(super) fn build_runtime_security_engine_from_rules(
    effective: Option<&settings_profiles::EffectiveVmSettings>,
    enforcement_rules: Vec<CelEnforcementRule>,
    detection_rules: Vec<capsem_security_engine::CelDetectionRule>,
    match_recorder: Option<RuntimeRuleMatchAccumulator>,
) -> Option<Arc<dyn RuntimeSecurityEngine>> {
    if enforcement_rules.is_empty() && detection_rules.is_empty() {
        return None;
    }

    let mut engine = SecurityEngine::default();
    if !enforcement_rules.is_empty() {
        let evaluator = match CelEnforcementEvaluator::compile(enforcement_rules) {
            Ok(evaluator) => evaluator,
            Err(error) => {
                warn!(
                    error = %error,
                    "failed to compile runtime enforcement rules; installing fail-closed security rule"
                );
                CelEnforcementEvaluator::compile(vec![CelEnforcementRule {
                    id: "runtime.compile_failed".into(),
                    pack_id: Some("runtime".into()),
                    condition: "true".into(),
                    decision: SecurityDecisionAction::Block,
                    reason: Some("runtime security rules failed to compile".into()),
                    mutations: Vec::new(),
                }])
                .expect("static fail-closed CEL rule must compile")
            }
        };
        engine.set_enforcement(Box::new(evaluator));
    }
    if !detection_rules.is_empty() {
        match capsem_security_engine::CelDetectionEvaluator::compile(detection_rules) {
            Ok(evaluator) => engine.set_detection(Box::new(evaluator)),
            Err(error) => {
                warn!(
                    error = %error,
                    "failed to compile runtime detection rules; continuing without runtime detection"
                );
            }
        }
    }
    if let Some(match_recorder) = match_recorder {
        engine.set_match_recorder(Box::new(match_recorder));
    }
    info!(
        profile_id = %effective
            .map(|effective| effective.profile_id.as_str())
            .unwrap_or("unknown"),
        "installed runtime security engine"
    );
    let runtime: Arc<dyn RuntimeSecurityEngine> = Arc::new(Mutex::new(engine));
    Some(runtime)
}

pub(super) fn cel_enforcement_rule_from_snapshot(
    rule: capsem_proto::ipc::RuntimeEnforcementRuleSnapshot,
) -> CelEnforcementRule {
    CelEnforcementRule {
        id: rule.id,
        pack_id: rule.pack_id,
        condition: rule.condition,
        decision: security_decision_action_from_snapshot(rule.decision),
        reason: rule.reason,
        mutations: Vec::new(),
    }
}

pub(super) fn cel_detection_rule_from_snapshot(
    rule: capsem_proto::ipc::RuntimeDetectionRuleSnapshot,
) -> capsem_security_engine::CelDetectionRule {
    capsem_security_engine::CelDetectionRule {
        id: rule.id,
        pack_id: rule.pack_id,
        sigma_id: rule.sigma_id,
        title: rule.title,
        condition: rule.condition,
        severity: severity_from_snapshot(rule.severity),
        confidence: confidence_from_snapshot(rule.confidence),
        tags: rule.tags,
    }
}

fn security_decision_action_from_snapshot(
    action: capsem_proto::ipc::RuntimeSecurityDecisionAction,
) -> SecurityDecisionAction {
    match action {
        capsem_proto::ipc::RuntimeSecurityDecisionAction::Allow => SecurityDecisionAction::Allow,
        capsem_proto::ipc::RuntimeSecurityDecisionAction::Ask => SecurityDecisionAction::Ask,
        capsem_proto::ipc::RuntimeSecurityDecisionAction::Block => SecurityDecisionAction::Block,
        capsem_proto::ipc::RuntimeSecurityDecisionAction::Rewrite => {
            SecurityDecisionAction::Rewrite
        }
        capsem_proto::ipc::RuntimeSecurityDecisionAction::Throttle => {
            SecurityDecisionAction::Throttle
        }
    }
}

fn severity_from_snapshot(
    severity: capsem_proto::ipc::RuntimeDetectionSeverity,
) -> capsem_security_engine::Severity {
    match severity {
        capsem_proto::ipc::RuntimeDetectionSeverity::Info => capsem_security_engine::Severity::Info,
        capsem_proto::ipc::RuntimeDetectionSeverity::Low => capsem_security_engine::Severity::Low,
        capsem_proto::ipc::RuntimeDetectionSeverity::Medium => {
            capsem_security_engine::Severity::Medium
        }
        capsem_proto::ipc::RuntimeDetectionSeverity::High => capsem_security_engine::Severity::High,
        capsem_proto::ipc::RuntimeDetectionSeverity::Critical => {
            capsem_security_engine::Severity::Critical
        }
    }
}

fn confidence_from_snapshot(
    confidence: capsem_proto::ipc::RuntimeDetectionConfidence,
) -> capsem_security_engine::Confidence {
    match confidence {
        capsem_proto::ipc::RuntimeDetectionConfidence::Low => {
            capsem_security_engine::Confidence::Low
        }
        capsem_proto::ipc::RuntimeDetectionConfidence::Medium => {
            capsem_security_engine::Confidence::Medium
        }
        capsem_proto::ipc::RuntimeDetectionConfidence::High => {
            capsem_security_engine::Confidence::High
        }
    }
}

fn runtime_enforcement_rule_from_effective(rule: &EffectiveRule) -> Option<CelEnforcementRule> {
    let event_type = SecurityEventType::parse(&rule.callback).ok()?;
    let condition = format!(
        "common.event_type == '{}' && ({})",
        event_type.as_str(),
        runtime_rule_condition(rule)
    );
    let decision = profile_decision_to_security_action(rule.decision);
    Some(CelEnforcementRule {
        id: runtime_effective_rule_id(rule),
        pack_id: Some(rule.provenance.profile_id.clone()),
        condition,
        decision,
        reason: rule.reason.clone(),
        mutations: runtime_rule_mutations(rule),
    })
}

fn runtime_rule_condition(rule: &EffectiveRule) -> String {
    rule.condition.clone()
}

fn runtime_effective_rule_id(rule: &EffectiveRule) -> String {
    if rule.id.starts_with("policy.") || rule.owner_setting_path.is_some() {
        rule.id.clone()
    } else {
        format!("policy.{}", rule.id)
    }
}

fn profile_decision_to_security_action(decision: RuleDecision) -> SecurityDecisionAction {
    match decision {
        RuleDecision::Allow => SecurityDecisionAction::Allow,
        RuleDecision::Ask => SecurityDecisionAction::Allow,
        RuleDecision::Block => SecurityDecisionAction::Block,
        RuleDecision::Rewrite => SecurityDecisionAction::Rewrite,
    }
}

fn runtime_rule_mutations(rule: &EffectiveRule) -> Vec<EventMutation> {
    if rule.decision != RuleDecision::Rewrite {
        return Vec::new();
    }
    let mut mutations = Vec::new();
    for header in &rule.strip_request_headers {
        mutations.push(EventMutation::StripHeader {
            path: format!("subject.headers.{header}"),
            reason: rule.reason.clone(),
        });
    }
    for header in &rule.strip_response_headers {
        mutations.push(EventMutation::StripHeader {
            path: format!("subject.headers.{header}"),
            reason: rule.reason.clone(),
        });
    }
    let Some(target) = rule.rewrite_target.as_deref() else {
        return mutations;
    };
    let Some(replacement) = rule.rewrite_value.as_deref() else {
        return mutations;
    };
    let Some((path, pattern)) = parse_rewrite_target(target) else {
        return mutations;
    };
    mutations.push(EventMutation::ReplaceRegex {
        path,
        pattern,
        replacement: replacement.to_string(),
        reason: rule.reason.clone(),
    });
    mutations
}

fn parse_rewrite_target(target: &str) -> Option<(String, String)> {
    let (path, pattern_expr) = target.split_once("=~")?;
    let path = path.trim();
    let pattern_expr = pattern_expr.trim();
    let pattern = pattern_expr
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))?;
    if path.is_empty() || pattern.is_empty() {
        return None;
    }
    Some((path.to_string(), pattern.to_string()))
}
