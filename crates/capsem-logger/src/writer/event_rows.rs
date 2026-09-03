//! Row inserts for the DNS, audit, substitution, security and profile-mutation
//! ledgers.

use super::*;

pub(super) fn insert_dns_event(conn: &Connection, event: &DnsEvent, target: WriteTarget) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(event.timestamp);
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (
            event_id, timestamp, qname, qtype, qclass, rcode, decision, matched_rule,
            answer_ip, source_proto, process_name, upstream_resolver_ms, trace_id, turn_id,
            policy_mode, policy_action, policy_rule, policy_reason, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            target.table("dns_events")
        ),
        params![
            event.event_id.clone().unwrap_or_else(new_event_id),
            timestamp,
            event.qname,
            i64::from(event.qtype),
            i64::from(event.qclass),
            i64::from(event.rcode),
            event.decision,
            event.matched_rule,
            event.answer_ip,
            event.source_proto,
            event.process_name,
            event.upstream_resolver_ms as i64,
            event.trace_id,
            event.trace_id,
            event.policy_mode,
            event.policy_action,
            event.policy_rule,
            event.policy_reason,
            event.credential_ref,
        ],
    )?;
    Ok(())
}

pub(super) fn insert_audit_event(conn: &Connection, event: &AuditEvent, target: WriteTarget) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(event.timestamp);
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (
            event_id, timestamp, pid, ppid, uid, exe, comm, argv, cwd,
            session_id, tty, audit_id, exec_event_id, parent_exe, trace_id, turn_id, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            target.table("audit_events")
        ),
        params![
            event.event_id.clone().unwrap_or_else(new_event_id),
            timestamp,
            i64::from(event.pid),
            i64::from(event.ppid),
            i64::from(event.uid),
            event.exe,
            event.comm,
            event.argv,
            event.cwd,
            event.session_id.map(i64::from),
            event.tty,
            event.audit_id,
            event.exec_event_id,
            event.parent_exe,
            event.trace_id,
            event.trace_id,
            event.credential_ref,
        ],
    )?;
    Ok(())
}

pub(super) fn insert_substitution_event(
    conn: &Connection,
    event: &SubstitutionEvent,
    target: WriteTarget,
) -> rusqlite::Result<()> {
    let timestamp = format_timestamp(event.timestamp);
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (
            event_id, timestamp, material_class, source, event_type, algorithm,
            substitution_ref, outcome, provider, confidence, trace_id, turn_id, context_json
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            target.table("substitution_events")
        ),
        params![
            event.event_id.clone().unwrap_or_else(new_event_id),
            timestamp,
            event.material_class,
            event.source,
            event.event_type,
            event.algorithm,
            event.substitution_ref,
            event.outcome,
            event.provider,
            event.confidence,
            event.trace_id,
            event.trace_id,
            event.context_json,
        ],
    )?;
    Ok(())
}

pub(super) fn insert_security_rule_event(
    conn: &Connection,
    event: &SecurityRuleEvent,
    target: WriteTarget,
) -> rusqlite::Result<()> {
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (
            timestamp_unix_ms, event_id, event_type, rule_id,
            rule_action, detection_level, rule_json, event_json, trace_id, turn_id, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            target.table("security_rule_events")
        ),
        params![
            event.timestamp_unix_ms,
            event.event_id,
            event.event_type,
            event.rule_id,
            event.rule_action.as_str(),
            event.detection_level.as_str(),
            event.rule_json,
            event.event_json,
            event.trace_id,
            event.turn_id,
            event.credential_ref,
        ],
    )?;
    Ok(())
}

pub(super) fn insert_security_ask_event(
    conn: &Connection,
    event: &SecurityAskEvent,
    target: WriteTarget,
) -> rusqlite::Result<()> {
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (
            timestamp_unix_ms, ask_id, event_id, event_type, rule_id, rule_name,
            status, rule_json, event_json, resolver, reason, trace_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            target.table("security_ask_events")
        ),
        params![
            event.timestamp_unix_ms,
            event.ask_id,
            event.event_id,
            event.event_type,
            event.rule_id,
            event.rule_name,
            event.status.as_str(),
            event.rule_json,
            event.event_json,
            event.resolver,
            event.reason,
            event.trace_id,
        ],
    )?;
    Ok(())
}

pub(super) fn insert_security_decision_event(
    conn: &Connection,
    event: &SecurityDecisionEvent,
    target: WriteTarget,
) -> rusqlite::Result<()> {
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (
            timestamp_unix_ms, event_id, event_type, stage, actor,
            rule_id, plugin_id, previous_decision, requested_decision,
            effective_decision, reason, event_json, trace_id, turn_id, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            target.table("security_decision_events")
        ),
        params![
            event.timestamp_unix_ms,
            event.event_id,
            event.event_type,
            event.stage.as_str(),
            event.actor,
            event.rule_id,
            event.plugin_id,
            event.previous_decision.as_str(),
            event.requested_decision.as_str(),
            event.effective_decision.as_str(),
            event.reason,
            event.event_json,
            event.trace_id,
            event.turn_id,
            event.credential_ref,
        ],
    )?;
    Ok(())
}

pub(super) fn insert_profile_mutation_event(
    conn: &Connection,
    event: &ProfileMutationEvent,
    target: WriteTarget,
) -> rusqlite::Result<()> {
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (
            timestamp_unix_ms, mutation_id, profile_id, actor, category, filename,
            affected_path, target_kind, target_key, operation, rule_id,
            old_hash, old_size, new_hash, new_size, status, error, trace_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            target.table("profile_mutation_events")
        ),
        params![
            event.timestamp_unix_ms,
            event.mutation_id,
            event.profile_id,
            event.actor,
            event.category,
            event.filename,
            event.affected_path,
            event.target_kind,
            event.target_key,
            event.operation,
            event.rule_id,
            event.old_hash,
            event.old_size as i64,
            event.new_hash,
            event.new_size as i64,
            event.status.as_str(),
            event.error,
            event.trace_id,
        ],
    )?;
    Ok(())
}
