//! Completed file observations never manufacture an enforceable pending call.
use super::*;

pub async fn emit_explicit_file_security_write_and_rules(
    db: &DbWriter,
    rules: &SecurityRuleSet,
    event: ExplicitFileSecurityEvent,
) -> Option<SecurityEventId> {
    let primary = explicit_primary_file_event(&event);
    let security_event = security_event_from_explicit_file_event(&event);
    let event_type = runtime_file_event_type(event.action);
    let event_id = emit_security_write(db, WriteOp::FileEvent(primary)).await?;
    if let Err(error) = emit_matching_security_rules(
        db,
        event_id.clone(),
        event_type,
        rules,
        &security_event,
        current_unix_ms(),
    )
    .await
    {
        tracing::warn!(error = %error, "failed to emit explicit file security rule ledger rows");
    }
    Some(event_id)
}

/// Record completed file facts with logging plugins only. No continuation
/// exists here: rule matches preserve the outcome that already happened.
pub async fn emit_observed_explicit_file_security_write_and_rules_with_plugins(
    db: &DbWriter,
    rules: &SecurityRuleSet,
    plugin_policy: impl Into<Arc<BTreeMap<String, SecurityPluginConfig>>>,
    event: ExplicitFileSecurityEvent,
) -> Result<Option<SecurityEventId>, String> {
    let primary = explicit_primary_file_event(&event);
    let security_event = security_event_from_explicit_file_event(&event);
    let event_type = runtime_file_event_type(event.action);
    let Some(event_id) = emit_security_write(db, WriteOp::FileEvent(primary)).await else {
        return Ok(None);
    };
    emit_matching_security_rules_for_evaluated_event(
        db,
        event_id.clone(),
        event_type,
        rules,
        plugin_policy,
        security_event,
        current_unix_ms(),
    )
    .await?;
    Ok(Some(event_id))
}
