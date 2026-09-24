//! Row inserts for the DNS, audit, substitution, security and profile-mutation
//! ledgers.

use super::*;
use capsem_proto::forensic::SecurityForensicEvent;

const SECURITY_MSGPACK_CONTENT_TYPE: &str = "application/vnd.capsem.security+msgpack";

/// A security event's archived payload, and its structured projection when it
/// has one.
struct SecurityPayload {
    bytes: Vec<u8>,
    content_type: &'static str,
    projection: Option<SecurityForensicEvent>,
}

fn security_payload(event_json: &str, event_type: &str) -> SecurityPayload {
    match SecurityForensicEvent::from_json(event_json, event_type)
        .and_then(|event| event.encode().map(|encoded| (encoded, event)))
    {
        Ok((bytes, projection)) => SecurityPayload {
            bytes,
            content_type: SECURITY_MSGPACK_CONTENT_TYPE,
            projection: Some(projection),
        },
        // A malformed direct logger event is retained as evidence. Production
        // security-engine projections are valid objects and take the typed path.
        Err(error) => {
            tracing::warn!(%error, "security forensic payload is not a structured projection");
            SecurityPayload {
                bytes: event_json.as_bytes().to_vec(),
                content_type: "application/json",
                projection: None,
            }
        }
    }
}

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

/// The row keeps its event identity for correlation while disk flush stores a
/// shared counted rule snapshot. The matched event's payload goes to the
/// archive as this event's body and is read by `BodyDirection::Payload`.
/// Returns the payload's structured projection, parsed once here for the
/// archive, so the counters can read its plugin and credential activity.
pub(super) fn insert_security_rule_event(
    conn: &Connection,
    event: &SecurityRuleEvent,
    target: WriteTarget,
    bodies: &mut BodyArchive,
) -> rusqlite::Result<Option<SecurityForensicEvent>> {
    let payload = security_payload(&event.event_json, &event.event_type);
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (
            timestamp_unix_ms, event_id, event_type, rule_id,
            rule_action, detection_level, rule_json, trace_id, turn_id, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
            event.trace_id,
            event.turn_id,
            event.credential_ref,
        ],
    )?;
    bodies.stage(
        conn,
        EventBodyBlob {
            event_id: &event.event_id,
            event_type: "security.rule",
            source_table: "security_rule_events",
            direction: "payload",
            content_type: Some(payload.content_type),
            body: Some(&payload.bytes),
            original_bytes: None,
            trace_id: event.trace_id.as_deref(),
            turn_id: event.turn_id.as_deref(),
        },
    );
    Ok(payload.projection)
}

/// An ask's row, with the asked-about event archived beside it the way every
/// security payload is. The pending row and its resolution carry the same
/// event, and both are staged: the index holds one body per event, so the
/// second names the same bytes the first did.
pub(super) fn insert_security_ask_event(
    conn: &Connection,
    event: &SecurityAskEvent,
    target: WriteTarget,
    bodies: &mut BodyArchive,
) -> rusqlite::Result<()> {
    let payload = security_payload(&event.event_json, &event.event_type);
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (
            timestamp_unix_ms, ask_id, event_id, event_type, rule_id, rule_name,
            status, rule_json, resolver, reason, trace_id
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            event.resolver,
            event.reason,
            event.trace_id,
        ],
    )?;
    bodies.stage(
        conn,
        EventBodyBlob {
            event_id: &event.event_id,
            event_type: "security.ask",
            source_table: "security_ask_events",
            direction: "payload",
            content_type: Some(payload.content_type),
            body: Some(&payload.bytes),
            original_bytes: None,
            trace_id: event.trace_id.as_deref(),
            turn_id: event.trace_id.as_deref(),
        },
    );
    Ok(())
}

/// A decision transition's row, and the event it was made about in the
/// archive. The row is what a projection filters on -- stage, actor, the three
/// decisions -- and it is small; the event was the other 5-6 KB of every row, in
/// the table a session writes most often, and once mirrored in RAM.
pub(super) fn insert_security_decision_event(
    conn: &Connection,
    event: &SecurityDecisionEvent,
    target: WriteTarget,
    bodies: &mut BodyArchive,
) -> rusqlite::Result<()> {
    let payload = security_payload(&event.event_json, &event.event_type);
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (
            timestamp_unix_ms, event_id, event_type, stage, actor,
            rule_id, plugin_id, previous_decision, requested_decision,
            effective_decision, reason, trace_id, turn_id, credential_ref
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
            event.trace_id,
            event.turn_id,
            event.credential_ref,
        ],
    )?;
    bodies.stage(
        conn,
        EventBodyBlob {
            event_id: &event.event_id,
            event_type: "security.decision",
            source_table: "security_decision_events",
            direction: "payload",
            content_type: Some(payload.content_type),
            body: Some(&payload.bytes),
            original_bytes: None,
            trace_id: event.trace_id.as_deref(),
            turn_id: event.turn_id.as_deref(),
        },
    );
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

/// The network's current row: an insert on creation, an update on retirement.
pub(super) fn upsert_network(conn: &Connection, network: &NetworkRecord, target: WriteTarget) -> rusqlite::Result<()> {
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (id, name, subnet, state, created_unix_ms, retired_unix_ms)
                  VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                  name = excluded.name,
                  state = excluded.state,
                  retired_unix_ms = excluded.retired_unix_ms",
            target.table("network")
        ),
        params![
            network.id,
            network.name,
            network.subnet,
            network.state.as_str(),
            network.created_unix_ms,
            network.retired_unix_ms
        ],
    )?;
    Ok(())
}

/// A membership's current row, keyed by network and VM; the ledger keeps the
/// history of how it got there.
pub(super) fn upsert_network_membership(
    conn: &Connection,
    membership: &NetworkMembership,
    target: WriteTarget,
) -> rusqlite::Result<()> {
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (network_id, vm_id, address, state, updated_unix_ms)
                  VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(network_id, vm_id) DO UPDATE SET
                  address = excluded.address,
                  state = excluded.state,
                  updated_unix_ms = excluded.updated_unix_ms",
            target.table("network_members")
        ),
        params![
            membership.network_id,
            membership.vm_id,
            membership.address,
            membership.state.as_str(),
            membership.updated_unix_ms
        ],
    )?;
    Ok(())
}

pub(super) fn insert_transport_event(
    conn: &Connection,
    event: &TransportEvent,
    target: WriteTarget,
) -> rusqlite::Result<()> {
    // `event_id` is UNIQUE, and memory only holds what is not flushed yet: an
    // id already on disk must be refused here, as the memory table refuses one
    // it still holds, or the flush's INSERT OR REPLACE would overwrite the
    // recorded event with its replay.
    let on_disk = conn
        .prepare_cached("SELECT 1 FROM main.transport_events WHERE event_id = ?1")?
        .exists([&event.event_id])?;
    if on_disk {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE),
            Some("UNIQUE constraint failed: transport_events.event_id".to_string()),
        ));
    }
    execute_cached(
        conn,
        &format!(
            "INSERT INTO {} (event_id,timestamp_unix_ms,event_type,network_id,connection_id,event_json)
                  VALUES (?1,?2,?3,?4,?5,?6)",
            target.table("transport_events")
        ),
        params![
            event.event_id,
            event.timestamp_unix_ms,
            event.kind.as_str(),
            event.network_id,
            event.connection_id,
            event.event_json
        ],
    )?;
    Ok(())
}
