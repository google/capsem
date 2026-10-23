//! Canonical append-only Ask lifecycle events.
use super::*;

pub async fn emit_security_ask_pending(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rule: &CompiledSecurityRule,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<SecurityEventId, String> {
    let ask_id = SecurityEventId::new_uuid4();
    let ask_event = security_ask_pending_event(ask_id.clone(), event_id, event_type, rule, event, timestamp_unix_ms)?;
    emit_security_write(db, WriteOp::SecurityAskEvent(ask_event))
        .await
        .ok_or_else(|| "ledger refused pending security ask row".to_string())?;
    Ok(ask_id)
}

pub fn emit_security_ask_pending_blocking(
    db: &DbWriter,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rule: &CompiledSecurityRule,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<SecurityEventId, String> {
    let ask_id = SecurityEventId::new_uuid4();
    let ask_event = security_ask_pending_event(ask_id.clone(), event_id, event_type, rule, event, timestamp_unix_ms)?;
    emit_security_write_blocking(db, WriteOp::SecurityAskEvent(ask_event))
        .ok_or_else(|| "ledger refused pending security ask row".to_string())?;
    Ok(ask_id)
}

pub fn emit_security_ask_resolution_blocking(
    db: &DbWriter,
    pending: &SecurityAskEvent,
    status: SecurityAskStatus,
    resolver: impl Into<String>,
    reason: Option<String>,
    timestamp_unix_ms: i64,
) -> Result<(), String> {
    let event = security_ask_resolution_event(pending, status, resolver, reason, timestamp_unix_ms)?;
    emit_security_write_blocking(db, WriteOp::SecurityAskEvent(event))
        .ok_or_else(|| "ledger refused resolved security ask row".to_string())?;
    Ok(())
}

pub async fn emit_security_ask_resolution(
    db: &DbWriter,
    pending: &SecurityAskEvent,
    status: SecurityAskStatus,
    resolver: impl Into<String>,
    reason: Option<String>,
    timestamp_unix_ms: i64,
) -> Result<(), String> {
    let event = security_ask_resolution_event(pending, status, resolver, reason, timestamp_unix_ms)?;
    emit_security_write(db, WriteOp::SecurityAskEvent(event))
        .await
        .ok_or_else(|| "ledger refused resolved security ask row".to_string())?;
    Ok(())
}

fn security_ask_resolution_event(
    pending: &SecurityAskEvent,
    status: SecurityAskStatus,
    resolver: impl Into<String>,
    reason: Option<String>,
    timestamp_unix_ms: i64,
) -> Result<SecurityAskEvent, String> {
    if matches!(status, SecurityAskStatus::Pending) {
        return Err("ask resolution status must be approved or denied".to_string());
    }
    let mut event = SecurityAskEvent::pending(SecurityAskPending {
        timestamp_unix_ms,
        ask_id: pending.ask_id.clone(),
        event_id: pending.event_id.clone(),
        event_type: pending.event_type.clone(),
        rule_id: pending.rule_id.clone(),
        rule_name: pending.rule_name.clone(),
        rule_json: pending.rule_json.clone(),
        event_json: pending.event_json.clone(),
    })
    .with_status(status)
    .with_resolver(resolver);
    if let Some(reason) = reason {
        event = event.with_reason(reason);
    }
    if let Some(trace_id) = pending.trace_id.clone() {
        event = event.with_trace_id(trace_id);
    }
    Ok(event)
}

pub fn security_ask_pending_event(
    ask_id: SecurityEventId,
    event_id: SecurityEventId,
    event_type: RuntimeSecurityEventType,
    rule: &CompiledSecurityRule,
    event: &SecurityEvent,
    timestamp_unix_ms: i64,
) -> Result<SecurityAskEvent, String> {
    let mut ask = SecurityAskEvent::pending(SecurityAskPending {
        timestamp_unix_ms,
        ask_id: ask_id.as_str().to_string(),
        event_id: event_id.as_str().to_string(),
        event_type: event_type.as_str().to_string(),
        rule_id: rule.rule_id.clone(),
        rule_name: rule.name.clone(),
        rule_json: serde_json::to_string(&compiled_rule_forensic_json(rule))
            .map_err(|error| format!("serialize security ask rule snapshot: {error}"))?,
        event_json: serde_json::to_string(&security_event_forensic_json(event))
            .map_err(|error| format!("serialize security ask event payload: {error}"))?,
    });
    if let Some(trace_id) = event.trace_id() {
        ask = ask.with_trace_id(trace_id);
    }
    Ok(ask)
}
