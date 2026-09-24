//! The security ledger a session keeps, and the routes that read it.
//!
//! One place owns the rule-match window: the SQL that selects it, and the
//! stats that summarize the whole session, which come from the ledger's
//! counter snapshot rather than from the window.

use super::*;

pub(crate) fn is_detection_rule_event(event: &capsem_logger::SecurityRuleMatch) -> bool {
    event.detection_level != capsem_logger::SecurityDetectionLevel::None
}

#[derive(Clone, Debug)]
pub(crate) struct SecuritySessionLedger {
    pub(crate) latest: Vec<capsem_logger::SecurityRuleMatch>,
    pub(crate) stats: capsem_logger::SecurityRuleStats,
}

impl Default for SecuritySessionLedger {
    fn default() -> Self {
        Self {
            latest: Vec::new(),
            stats: empty_security_rule_stats(),
        }
    }
}

pub(crate) fn empty_security_rule_stats() -> capsem_logger::SecurityRuleStats {
    capsem_logger::SecurityRuleStats {
        total: 0,
        by_action: Vec::new(),
        by_event_type: Vec::new(),
        by_level: Vec::new(),
        by_rule: Vec::new(),
    }
}

// The matched event's payload is archive-backed: it is read by event id with
// `BodyDirection::Payload`, and the index metadata travels with the other
// bodies in the stats-detail payload.
const SECURITY_LATEST_SQL: &str = r#"
SELECT event.timestamp_unix_ms, event.event_id, event.event_type, event.rule_id,
       event.rule_action, event.detection_level,
       COALESCE(event.rule_json, run.rule_json) AS rule_json, event.trace_id,
       event.turn_id, event.credential_ref
FROM security_rule_events AS event
LEFT JOIN security_rule_runs AS run ON run.id = event.run_id
ORDER BY event.timestamp_unix_ms DESC, event.id DESC
LIMIT ?
"#;

/// How many recent matches a security ledger read reports on.
const SECURITY_LATEST_LIMIT: usize = 2000;

pub(crate) async fn read_security_session_ledger(
    state: &ServiceState,
    vm_id: &str,
    db_path: &StdPath,
) -> Result<Option<SecuritySessionLedger>, AppError> {
    read_security_ledger(state, vm_id, db_path).await
}

async fn read_security_ledger(
    state: &ServiceState,
    vm_id: &str,
    db_path: &StdPath,
) -> Result<Option<SecuritySessionLedger>, AppError> {
    let db = open_ready_session_db(state, vm_id, "security", db_path).await?;
    let latest = query_route_typed_rows::<capsem_logger::SecurityRuleMatch>(
        vm_id,
        "security",
        "latest",
        db_path,
        &db,
        SECURITY_LATEST_SQL,
        &[json!(SECURITY_LATEST_LIMIT)],
    )
    .await?;
    let stats = security_stats(vm_id, db_path, &db).await?;
    Ok(Some(SecuritySessionLedger { latest, stats }))
}

pub(crate) async fn security_latest_for_vm(
    state: &ServiceState,
    vm_id: &str,
    limit: usize,
    detection_only: bool,
) -> Result<Vec<capsem_logger::SecurityRuleMatch>, AppError> {
    let session_dir = resolve_session_dir(state, vm_id)?;
    let Some(session) = read_security_session_ledger(state, vm_id, &session_dir.join("session.db")).await? else {
        return Ok(Vec::new());
    };
    Ok(session
        .latest
        .iter()
        .filter(|event| !detection_only || is_detection_rule_event(event))
        .take(limit)
        .cloned()
        .collect())
}

/// The security ledger's aggregates alone.
///
/// A primary-key read of the counter snapshot, not the ledger: the route is
/// polled on a timer and reports six counts.
pub(crate) async fn security_stats_for_vm(
    state: &ServiceState,
    vm_id: &str,
) -> Result<capsem_logger::SecurityRuleStats, AppError> {
    let session_dir = resolve_session_dir(state, vm_id)?;
    let db_path = session_dir.join("session.db");
    let db = open_ready_session_db(state, vm_id, "security", &db_path).await?;
    security_stats(vm_id, &db_path, &db).await
}

/// The session's rule-match statistics, from its counter snapshot.
///
/// The one way this crate computes `SecurityRuleStats`: the status poll and
/// the full ledger read both come here, so they cannot disagree. They count
/// every match the session made, not the window `latest` shows.
async fn security_stats(
    vm_id: &str,
    db_path: &StdPath,
    db: &capsem_logger::DbHandle,
) -> Result<capsem_logger::SecurityRuleStats, AppError> {
    let counters = db
        .ledger_counters()
        .await
        .map_err(|error| ledger_route_error(vm_id, "security", "counters", db_path, &error))?;
    Ok(super::activity::security_stats(&counters))
}
