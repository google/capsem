//! Rule rows: which rules matched an event, and the decision they record.
//!
//! A row's `decision.effective` is the outcome that was enforced. A caller
//! that enforces the rule's decision gets it recorded; a caller recording an
//! event whose outcome already happened keeps that outcome. The emitter
//! applied neither, and stored the default allow beside `rule_action =
//! "block"` for every blocked request (google/capsem#203).
use super::*;

/// Whether the caller enforces the rule's decision on this event.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RuleRowDecision {
    Enforced,
    AsArrived,
}

/// Write the rule rows for an event whose outcome already happened -- a file
/// the monitor saw change, a process the guest audited, a DNS answer or HTTP
/// exchange already enforced. Rows keep the event's decision as it arrived,
/// seeded by the caller from what was enforced, and never claim a block no
/// one applied.
pub async fn emit_matching_security_rules(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rules: &SecurityRuleSet,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<usize, String> {
    emit_rule_rows(
        db,
        event_id,
        event_type,
        rules,
        event,
        timestamp_unix_ms,
        RuleRowDecision::AsArrived,
    )
    .await
    .map(|emission| emission.emitted)
}

/// Write the rule rows and return the decision the caller must enforce. Rows
/// record that decision, because the caller applies it.
pub async fn emit_matching_security_rules_with_decision(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rules: &SecurityRuleSet,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<SecurityRuleEmission, String> {
    emit_rule_rows(
        db,
        event_id,
        event_type,
        rules,
        event,
        timestamp_unix_ms,
        RuleRowDecision::Enforced,
    )
    .await
}

/// `emit_matching_security_rules` on a thread that may block.
pub fn emit_matching_security_rules_blocking(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rules: &SecurityRuleSet,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<usize, String> {
    emit_rule_rows_blocking(
        db,
        event_id,
        event_type,
        rules,
        event,
        timestamp_unix_ms,
        RuleRowDecision::AsArrived,
    )
    .map(|emission| emission.emitted)
}

/// `emit_matching_security_rules_with_decision` on a thread that may block.
pub fn emit_matching_security_rules_with_decision_blocking(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rules: &SecurityRuleSet,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<SecurityRuleEmission, String> {
    emit_rule_rows_blocking(
        db,
        event_id,
        event_type,
        rules,
        event,
        timestamp_unix_ms,
        RuleRowDecision::Enforced,
    )
}

async fn emit_rule_rows(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rules: &SecurityRuleSet,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
    recorded: RuleRowDecision,
) -> Result<SecurityRuleEmission, String> {
    event.validate_network(event_type).map_err(|error| error.to_string())?;
    let evaluation = rules.evaluate(event)?;
    let selected_rule = selected_enforcement_rule(&evaluation);
    let mut emitted = 0;
    let mut enriched_event = event_with_rule_detections(event, evaluation.detections());
    let mut enforcement = security_enforcement_decision(selected_rule, &mut enriched_event);
    let mut decision_state = enriched_event.decision.clone();
    let mut rule_events = Vec::new();
    if let Some(rule) = selected_rule {
        emit_security_decision_transition(
            db,
            event_id.clone(),
            event_type,
            rule,
            &enriched_event,
            &mut decision_state,
            timestamp_unix_ms,
        )
        .await?;
        if recorded == RuleRowDecision::Enforced {
            enriched_event.decision = decision_state.clone();
        }
    }
    for rule in evaluation.matched_rules() {
        let rule_event = security_rule_event(event_id.clone(), event_type, rule, &enriched_event, timestamp_unix_ms)?;
        trace_security_rule_match(&rule_event, rule);
        emit_security_write(db, WriteOp::SecurityRuleEvent(rule_event.clone())).await;
        rule_events.push(rule_event);
        emitted += 1;
    }
    if matches!(enforcement.action, SecurityEnforcementAction::Ask) {
        let Some(rule) = selected_rule else {
            return Err("ask enforcement decision did not carry a rule".to_string());
        };
        let ask_id = emit_security_ask_pending(
            db,
            event_id.clone(),
            event_type,
            rule,
            &enriched_event,
            timestamp_unix_ms,
        )
        .await?;
        enforcement.ask_id = Some(ask_id);
    }
    Ok(SecurityRuleEmission {
        event_id,
        emitted,
        enforcement,
        event: enriched_event,
        rule_events,
    })
}

fn emit_rule_rows_blocking(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rules: &SecurityRuleSet,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
    recorded: RuleRowDecision,
) -> Result<SecurityRuleEmission, String> {
    event.validate_network(event_type).map_err(|error| error.to_string())?;
    let evaluation = rules.evaluate(event)?;
    let selected_rule = selected_enforcement_rule(&evaluation);
    let mut emitted = 0;
    let mut enriched_event = event_with_rule_detections(event, evaluation.detections());
    let mut enforcement = security_enforcement_decision(selected_rule, &mut enriched_event);
    let mut decision_state = enriched_event.decision.clone();
    let mut rule_events = Vec::new();
    if let Some(rule) = selected_rule {
        emit_security_decision_transition_blocking(
            db,
            event_id.clone(),
            event_type,
            rule,
            &enriched_event,
            &mut decision_state,
            timestamp_unix_ms,
        )?;
        if recorded == RuleRowDecision::Enforced {
            enriched_event.decision = decision_state.clone();
        }
    }
    for rule in evaluation.matched_rules() {
        let rule_event = security_rule_event(event_id.clone(), event_type, rule, &enriched_event, timestamp_unix_ms)?;
        trace_security_rule_match(&rule_event, rule);
        emit_security_write_blocking(db, WriteOp::SecurityRuleEvent(rule_event.clone()));
        rule_events.push(rule_event);
        emitted += 1;
    }
    if matches!(enforcement.action, SecurityEnforcementAction::Ask) {
        let Some(rule) = selected_rule else {
            return Err("ask enforcement decision did not carry a rule".to_string());
        };
        let ask_id = emit_security_ask_pending_blocking(
            db,
            event_id.clone(),
            event_type,
            rule,
            &enriched_event,
            timestamp_unix_ms,
        )?;
        enforcement.ask_id = Some(ask_id);
    }
    Ok(SecurityRuleEmission {
        event_id,
        emitted,
        enforcement,
        event: enriched_event,
        rule_events,
    })
}
