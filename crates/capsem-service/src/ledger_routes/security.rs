//! The security ledger a session keeps, and the routes that read it.
//!
//! One place owns the rule-match window: the SQL that selects it, the stats
//! that summarize it, and the two ways to read it -- the row alone for the
//! list views, and the row plus its archived forensic payload for the
//! hydration paths that actually parse one.

use super::*;

pub(crate) fn is_detection_rule_event(event: &capsem_logger::SecurityRuleMatch) -> bool {
    event.detection_level != capsem_logger::SecurityDetectionLevel::None
}

#[derive(Clone, Debug)]
pub(crate) struct SecuritySessionLedger {
    pub(crate) latest: Vec<capsem_logger::SecurityRuleMatch>,
    pub(crate) stats: capsem_logger::SecurityRuleStats,
    pub(crate) brokered_credentials: Vec<capsem_logger::BrokeredCredentialStat>,
    /// The archived forensic payload of each match in `latest`, by event id,
    /// and empty unless the caller asked for it. The payload is a body now,
    /// and only the hydration paths that actually parse it should pay to read
    /// it -- the list views want the row.
    pub(crate) payloads: BTreeMap<String, String>,
}

impl Default for SecuritySessionLedger {
    fn default() -> Self {
        Self {
            latest: Vec::new(),
            stats: empty_security_rule_stats(),
            brokered_credentials: Vec::new(),
            payloads: BTreeMap::new(),
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
SELECT timestamp_unix_ms, event_id, event_type, rule_id,
       rule_action, detection_level, rule_json, trace_id,
       turn_id, credential_ref
FROM security_rule_events
ORDER BY timestamp_unix_ms DESC, id DESC
LIMIT ?
"#;

const SECURITY_STATS_TOTAL_SQL: &str = r#"SELECT COUNT(*) AS total FROM security_rule_events"#;

const SECURITY_STATS_BY_ACTION_SQL: &str = r#"
SELECT rule_action, COUNT(*) AS count
FROM security_rule_events
GROUP BY rule_action
ORDER BY rule_action
"#;

const SECURITY_STATS_BY_EVENT_TYPE_SQL: &str = r#"
SELECT event_type, COUNT(*) AS count
FROM security_rule_events
GROUP BY event_type
ORDER BY event_type
"#;

const SECURITY_STATS_BY_LEVEL_SQL: &str = r#"
SELECT detection_level, COUNT(*) AS count
FROM security_rule_events
GROUP BY detection_level
ORDER BY detection_level
"#;

const SECURITY_STATS_BY_RULE_SQL: &str = r#"
SELECT
    sre.rule_id,
    sre.rule_action,
    sre.detection_level,
    COUNT(*) AS count,
    (
        SELECT latest.event_id
        FROM security_rule_events latest
        WHERE latest.rule_id = sre.rule_id
          AND latest.rule_action = sre.rule_action
          AND latest.detection_level = sre.detection_level
        ORDER BY latest.timestamp_unix_ms DESC, latest.id DESC
        LIMIT 1
    ) AS latest_event_id,
    MAX(sre.timestamp_unix_ms) AS latest_timestamp_unix_ms
FROM security_rule_events sre
GROUP BY sre.rule_id, sre.rule_action, sre.detection_level
ORDER BY latest_timestamp_unix_ms DESC
"#;

/// How many recent matches a security ledger read reports on. The payload
/// read, when one is asked for, covers the same window.
const SECURITY_LATEST_LIMIT: usize = 2000;

pub(crate) async fn read_security_session_ledger(
    state: &ServiceState,
    vm_id: &str,
    db_path: &StdPath,
) -> Result<Option<SecuritySessionLedger>, AppError> {
    read_security_ledger(state, vm_id, db_path, false).await
}

/// The same ledger, plus each match's archived forensic payload.
///
/// Only the plugin and credential hydration paths need this: they parse the
/// payload to count plugin executions and credential observations. Every
/// other reader wants the row, which is why the payload left it.
pub(crate) async fn read_security_session_ledger_with_payloads(
    state: &ServiceState,
    vm_id: &str,
    db_path: &StdPath,
) -> Result<Option<SecuritySessionLedger>, AppError> {
    read_security_ledger(state, vm_id, db_path, true).await
}

async fn read_security_ledger(
    state: &ServiceState,
    vm_id: &str,
    db_path: &StdPath,
    with_payloads: bool,
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
    let total_row = query_route_objects(
        vm_id,
        "security",
        "stats_total",
        db_path,
        &db,
        SECURITY_STATS_TOTAL_SQL,
        &[],
    )
    .await?
    .into_iter()
    .next()
    .unwrap_or_else(|| json!({ "total": 0 }));
    let stats = capsem_logger::SecurityRuleStats {
        total: total_row.get("total").and_then(serde_json::Value::as_u64).unwrap_or(0),
        by_action: query_route_typed_rows(
            vm_id,
            "security",
            "stats_by_action",
            db_path,
            &db,
            SECURITY_STATS_BY_ACTION_SQL,
            &[],
        )
        .await?,
        by_event_type: query_route_typed_rows(
            vm_id,
            "security",
            "stats_by_event_type",
            db_path,
            &db,
            SECURITY_STATS_BY_EVENT_TYPE_SQL,
            &[],
        )
        .await?,
        by_level: query_route_typed_rows(
            vm_id,
            "security",
            "stats_by_level",
            db_path,
            &db,
            SECURITY_STATS_BY_LEVEL_SQL,
            &[],
        )
        .await?,
        by_rule: query_route_typed_rows(
            vm_id,
            "security",
            "stats_by_rule",
            db_path,
            &db,
            SECURITY_STATS_BY_RULE_SQL,
            &[],
        )
        .await?,
    };
    let brokered_credentials = query_route_typed_rows(
        vm_id,
        "security",
        "brokered_credentials",
        db_path,
        &db,
        BROKERED_CREDENTIAL_STATS_SQL,
        &[],
    )
    .await?;
    // Keyed to `latest`, not chosen again by its own ordering: two windows
    // picked independently -- one by timestamp, one by index id -- drift the
    // moment a flush lands between them, and a page of rows carrying another
    // page's payloads is a projection that lies.
    let payloads = if with_payloads {
        security_payloads(vm_id, db_path, &db, &latest).await?
    } else {
        BTreeMap::new()
    };
    Ok(Some(SecuritySessionLedger {
        latest,
        stats,
        brokered_credentials,
        payloads,
    }))
}

/// What one hydration pass may hold in archived payloads at once.
///
/// The window is 2000 matches and a body may be 10 MiB, so the row count alone
/// permits 20 GiB. In practice a payload is about a kilobyte and the whole
/// window fits in single-digit megabytes; this is the ceiling for the session
/// that is not typical, and reaching it is reported rather than hidden.
const SECURITY_PAYLOAD_BUDGET_BYTES: usize = 64 * 1024 * 1024;

/// The archived payload of each match in `latest`, by event id.
///
/// The DB handle owns the archive; this is one read for the whole window
/// rather than one per row, and a payload that is not valid UTF-8 is not a
/// payload this ledger wrote.
///
/// A session whose archive cannot be opened, or that gave up mid-session,
/// simply has no bodies to return: the map comes back short or empty and the
/// caller's own accounting degrades with it -- fewer plugin executions, fewer
/// brokered credentials -- with `last_error` naming the events it could not
/// read. That is the intended failure: an archive that lost bodies must not
/// take the rest of the ledger down with it, and must not be reported as a
/// session that had none.
async fn security_payloads(
    vm_id: &str,
    db_path: &StdPath,
    db: &capsem_logger::DbHandle,
    latest: &[capsem_logger::SecurityRuleMatch],
) -> Result<BTreeMap<String, String>, AppError> {
    let event_ids: Vec<&str> = latest.iter().map(|event| event.event_id.as_str()).collect();
    let archived = db
        .read_bodies_for_events(
            &event_ids,
            "security_rule_events",
            capsem_logger::BodyDirection::Payload,
            SECURITY_PAYLOAD_BUDGET_BYTES,
        )
        .await
        .map_err(|error| ledger_route_error(vm_id, "security", "read payloads of", db_path, &error))?;
    if archived.truncated_rows > 0 {
        warn!(
            vm_id,
            truncated_rows = archived.truncated_rows,
            budget_bytes = SECURITY_PAYLOAD_BUDGET_BYTES,
            "security payload budget reached; plugin and credential hydration will undercount"
        );
    }
    Ok(archived
        .bodies
        .into_iter()
        .filter_map(|body| {
            String::from_utf8(body.bytes)
                .ok()
                .map(|payload| (body.event_id, payload))
        })
        .collect())
}

pub(crate) async fn read_profile_security_ledgers(
    state: &ServiceState,
    profile_id: &str,
) -> Result<Vec<(String, SecuritySessionLedger)>, AppError> {
    let mut ledgers = Vec::new();
    for (vm_id, session_dir) in profile_session_dirs(state, profile_id) {
        let Some(session) =
            read_security_session_ledger_with_payloads(state, &vm_id, &session_dir.join("session.db")).await?
        else {
            continue;
        };
        ledgers.push((vm_id, session));
    }
    Ok(ledgers)
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

pub(crate) async fn security_stats_for_vm(
    state: &ServiceState,
    vm_id: &str,
) -> Result<capsem_logger::SecurityRuleStats, AppError> {
    let session_dir = resolve_session_dir(state, vm_id)?;
    Ok(
        read_security_session_ledger(state, vm_id, &session_dir.join("session.db"))
            .await?
            .map(|session| session.stats)
            .unwrap_or_else(empty_security_rule_stats),
    )
}
