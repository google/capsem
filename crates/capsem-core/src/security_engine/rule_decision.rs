//! How a matched rule becomes an enforcement decision: which rule enforces,
//! what decision it requests, and the enforcement it produces.
use super::*;

pub(super) fn requested_decision_for_rule(action: SecurityRuleAction) -> SecurityDecisionKind {
    match action {
        SecurityRuleAction::Allow
        | SecurityRuleAction::Preprocess
        | SecurityRuleAction::Rewrite
        | SecurityRuleAction::Postprocess => SecurityDecisionKind::Allow,
        SecurityRuleAction::Ask => SecurityDecisionKind::Ask,
        SecurityRuleAction::Block => SecurityDecisionKind::Block,
    }
}

pub(super) fn decision_stage_for_rule(action: SecurityRuleAction) -> LoggedSecurityDecisionStage {
    match action {
        SecurityRuleAction::Preprocess => LoggedSecurityDecisionStage::Preprocess,
        SecurityRuleAction::Rewrite => LoggedSecurityDecisionStage::Rewrite,
        SecurityRuleAction::Postprocess => LoggedSecurityDecisionStage::Postprocess,
        SecurityRuleAction::Allow | SecurityRuleAction::Ask | SecurityRuleAction::Block => {
            LoggedSecurityDecisionStage::Rule
        }
    }
}

pub(super) fn selected_enforcement_rule<'a>(
    evaluation: &'a crate::net::policy_config::SecurityRuleEvaluation<'a>,
) -> Option<&'a CompiledSecurityRule> {
    evaluation.enforcement_rules().into_iter().next()
}

/// Escalate an enforcement decision to match the event's merged decision state.
///
/// Plugins request decisions on the same rail as rules, so a plugin running in
/// `ask` or `block` mode has to be able to raise an allowing rule verdict. The
/// merge behind `decision.effective` is escalate-only, so this can only ever
/// tighten the decision -- a plugin cannot talk a blocking rule down to allow.
pub(super) fn apply_event_decision_to_enforcement(
    event: &SecurityEvent,
    enforcement: &mut SecurityEnforcementDecision,
) {
    match event.decision.effective {
        SecurityDecisionKind::Block => enforcement.action = SecurityEnforcementAction::Block,
        SecurityDecisionKind::Ask => {
            if matches!(enforcement.action, SecurityEnforcementAction::Allow) {
                enforcement.action = SecurityEnforcementAction::Ask;
            }
        }
        SecurityDecisionKind::Allow => {}
    }
}

pub(super) fn requested_boundary_decision(
    rule: Option<&CompiledSecurityRule>,
    kind: RuntimeSecurityEventType,
) -> SecurityDecisionKind {
    match rule {
        Some(rule) => requested_decision_for_rule(rule.action),
        None if matches!(
            kind,
            RuntimeSecurityEventType::NetworkConnect | RuntimeSecurityEventType::NetworkProbe
        ) =>
        {
            SecurityDecisionKind::Block
        }
        None => SecurityDecisionKind::Allow,
    }
}

pub(super) fn security_enforcement_decision(
    rule: Option<&CompiledSecurityRule>,
    event: &mut SecurityEvent,
) -> SecurityEnforcementDecision {
    let Some(rule) = rule else {
        let mut decision = SecurityEnforcementDecision::allow();
        if requested_boundary_decision(None, event.event_type) == SecurityDecisionKind::Block {
            event.request_decision(SecurityDecisionKind::Block);
            decision.action = SecurityEnforcementAction::Block;
            decision.reason = Some("network operation requires an explicit allow rule".into());
        }
        return decision;
    };
    SecurityEnforcementDecision {
        action: match rule.action {
            SecurityRuleAction::Allow => SecurityEnforcementAction::Allow,
            SecurityRuleAction::Ask => SecurityEnforcementAction::Ask,
            SecurityRuleAction::Block => SecurityEnforcementAction::Block,
            SecurityRuleAction::Preprocess | SecurityRuleAction::Rewrite | SecurityRuleAction::Postprocess => {
                SecurityEnforcementAction::Allow
            }
        },
        rule_id: Some(rule.rule_id.clone()),
        rule_name: Some(rule.name.clone()),
        reason: rule.reason.clone(),
        ask_id: None,
    }
}
