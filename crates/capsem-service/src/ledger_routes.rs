use super::*;

/// `GET /vms/{id}/timeline?trace_id=<X>&since=10m&limit=200&layers=tool,exec,...`
/// -- unified time-ordered event stream for one session. Used by the
/// `capsem_timeline` MCP tool.
///
/// W6 added `trace_id` to every layer; this handler filters with
/// with matching `trace_id` or pre-W4 NULL trace rows so older rows still
/// surface for the user.
pub(super) async fn handle_timeline(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<TimelineQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = params.limit.unwrap_or(200).min(2000);
    let since_filter = params
        .since
        .as_deref()
        .and_then(triage::parse_since)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    // Layers the caller wants. Default to all five. C1: filter against
    // a hard allowlist BEFORE building SQL so even a future careless
    // copy-paste of this format!() can't leak attacker-supplied
    // tokens into the query string.
    const ALLOWED_LAYERS: &[&str] = &["exec", "tool", "net", "fs", "model"];
    let layers: Vec<&str> = params
        .layers
        .as_deref()
        .map(|s| {
            s.split(',')
                .filter(|x| !x.is_empty())
                .filter(|x| ALLOWED_LAYERS.contains(x))
                .collect()
        })
        .unwrap_or_else(|| ALLOWED_LAYERS.to_vec());

    if layers.is_empty() {
        return Err(AppError(StatusCode::BAD_REQUEST, "no layers selected".into()));
    }

    let session_dir = resolve_session_dir(&state, &id)?;
    let cutoff = since_filter.map(secs_to_rfc3339);
    let db_path = session_dir.join("session.db");
    let route_key = format!(
        "timeline:layers={}:limit={}:since={}:trace={}",
        layers.join(","),
        limit,
        params.since.as_deref().unwrap_or(""),
        params.trace_id.as_deref().unwrap_or("")
    );
    if let Some(body) = session_response_cache_get(&state, &id, &route_key, &db_path) {
        return Ok(json_bytes_response(body));
    }
    let sql = timeline_base_sql();
    let rows = read_timeline_rows_from_session_db(&state, &id, &db_path, &sql)
        .await?
        .unwrap_or_default()
        .into_iter()
        .filter(|row| layers.contains(&row.layer.as_str()))
        .filter(|row| {
            params
                .trace_id
                .as_deref()
                .is_none_or(|trace_id| row.trace_id.as_deref() == Some(trace_id) || row.trace_id.is_none())
        })
        .filter(|row| cutoff.as_deref().is_none_or(|cutoff| row.timestamp.as_str() >= cutoff))
        .take(limit)
        .map(|row| row.to_values())
        .collect::<Vec<_>>();
    let json_str = serde_json::to_string(&json!({
        "columns": TIMELINE_COLUMNS,
        "rows": rows,
    }))
    .map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("timeline ledger serialization failed: {error}"),
        )
    })?;
    session_response_cache_store(&state, &id, &route_key, &db_path, json_str.as_bytes());

    Ok(json_bytes_response(Bytes::from(json_str)))
}

#[derive(Deserialize, Debug, Default)]
pub(super) struct SecurityLedgerQuery {
    /// Max rows. Default 100, capped at 2000.
    pub(super) limit: Option<usize>,
}

/// GET /vms/{id}/security/latest -- latest security rule ledger rows.
///
/// Rows include the stored rule snapshot and normalized SecurityEvent payload
/// that matched, because active rules may have changed by the time a responder
/// investigates the event.
pub(super) async fn handle_security_latest(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<SecurityLedgerQuery>,
) -> Result<axum::response::Response, AppError> {
    let limit = params.limit.unwrap_or(100).min(2000);
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    let route_key = format!("security_latest:limit={limit}");
    if let Some(body) = session_response_cache_get(&state, &id, &route_key, &db_path) {
        return Ok(json_bytes_response(body));
    }
    let rows = security_latest_for_vm(&state, &id, limit, false).await?;
    info!(
        route = "/vms/{id}/security/latest",
        vm_id = id.as_str(),
        limit,
        row_count = rows.len(),
        "security_latest"
    );
    let body = serde_json::to_vec(&rows).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize security latest response: {error}"),
        )
    })?;
    session_response_cache_store(&state, &id, &route_key, &db_path, &body);
    Ok(json_bytes_response(Bytes::from(body)))
}

/// GET /vms/{id}/detection/latest -- latest detection-bearing rule rows.
pub(super) async fn handle_detection_latest(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<SecurityLedgerQuery>,
) -> Result<axum::response::Response, AppError> {
    let limit = params.limit.unwrap_or(100).min(2000);
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    let route_key = format!("detection_latest:limit={limit}");
    if let Some(body) = session_response_cache_get(&state, &id, &route_key, &db_path) {
        return Ok(json_bytes_response(body));
    }
    let rows = security_latest_for_vm(&state, &id, limit, true).await?;
    let body = serde_json::to_vec(&rows).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize detection latest response: {error}"),
        )
    })?;
    session_response_cache_store(&state, &id, &route_key, &db_path, &body);
    Ok(json_bytes_response(Bytes::from(body)))
}

/// GET /vms/{id}/security/status -- security rule ledger aggregates.
pub(super) async fn handle_security_info(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    if let Some(body) = session_response_cache_get(&state, &id, "security_status", &db_path) {
        return Ok(json_bytes_response(body));
    }
    let stats = security_stats_for_vm(&state, &id).await?;
    let body = serde_json::to_vec(&stats).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize security status response: {error}"),
        )
    })?;
    session_response_cache_store(&state, &id, "security_status", &db_path, &body);
    Ok(json_bytes_response(Bytes::from(body)))
}

pub(super) fn service_session_dirs(state: &ServiceState) -> Vec<(String, PathBuf)> {
    session_dirs_for_profile(state, None)
}

pub(super) fn profile_session_dirs(state: &ServiceState, profile_id: &str) -> Vec<(String, PathBuf)> {
    session_dirs_for_profile(state, Some(profile_id))
}

/// Every session the service knows, keyed by session id.
///
/// Registry maps are keyed by display name; a running persistent VM is also
/// in `instances` under its session id, and only that id lets the two sources
/// collapse into one row.
fn session_dirs_for_profile(state: &ServiceState, profile_id: Option<&str>) -> Vec<(String, PathBuf)> {
    let wanted = |candidate: &str| profile_id.is_none_or(|profile_id| profile_id == candidate);
    let mut sessions = BTreeMap::new();
    {
        let instances = state.instances.lock().unwrap();
        for (id, info) in instances.iter().filter(|(_, info)| wanted(&info.profile_id)) {
            sessions.insert(id.clone(), info.session_dir.clone());
        }
    }
    {
        let registry = state.persistent_registry.lock().unwrap();
        for entry in registry.list().filter(|entry| wanted(&entry.profile_id)) {
            sessions
                .entry(persistent_entry_vm_id(entry))
                .or_insert_with(|| entry.session_dir.clone());
        }
    }
    sessions.into_iter().collect()
}

pub(super) fn is_detection_rule_event(event: &capsem_logger::SecurityRuleEvent) -> bool {
    event.detection_level != capsem_logger::SecurityDetectionLevel::None
}

#[derive(Clone, Debug)]
pub(super) struct SecuritySessionLedger {
    latest: Vec<capsem_logger::SecurityRuleEvent>,
    stats: capsem_logger::SecurityRuleStats,
    brokered_credentials: Vec<capsem_logger::BrokeredCredentialStat>,
}

impl Default for SecuritySessionLedger {
    fn default() -> Self {
        Self {
            latest: Vec::new(),
            stats: empty_security_rule_stats(),
            brokered_credentials: Vec::new(),
        }
    }
}

pub(super) fn empty_security_rule_stats() -> capsem_logger::SecurityRuleStats {
    capsem_logger::SecurityRuleStats {
        total: 0,
        by_action: Vec::new(),
        by_event_type: Vec::new(),
        by_level: Vec::new(),
        by_rule: Vec::new(),
    }
}

pub(super) fn ledger_route_error(
    vm_id: &str,
    ledger: &str,
    operation: &str,
    db_path: &StdPath,
    error: impl std::fmt::Display,
) -> AppError {
    let error = error.to_string();
    error!(
        vm_id,
        ledger,
        operation,
        db_path = %db_path.display(),
        error = %error,
        "session ledger route DB operation failed"
    );
    AppError(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("failed to {operation} {ledger} ledger for {vm_id}: {error}"),
    )
}

pub(super) async fn open_ready_session_db(
    state: &ServiceState,
    vm_id: &str,
    ledger: &str,
    db_path: &StdPath,
) -> Result<Arc<capsem_logger::DbHandle>, AppError> {
    if !db_path.exists() {
        error!(
            vm_id,
            ledger,
            operation = "ready",
            db_path = %db_path.display(),
            "session ledger DB is absent"
        );
        return Err(ledger_route_error(vm_id, ledger, "ready", db_path, "session.db absent"));
    }
    let db = match state.session_db_handle(vm_id) {
        Some(handle) if handle.path() == db_path => handle,
        Some(handle) => {
            warn!(
                vm_id,
                ledger,
                operation = "replace_stale_session_db_handle",
                cached_db_path = %handle.path().display(),
                db_path = %db_path.display(),
                "session DB handle path did not match resolved session path"
            );
            state.unregister_session_db_handle(vm_id);
            let session_dir = db_path
                .parent()
                .ok_or_else(|| ledger_route_error(vm_id, ledger, "resolve session dir", db_path, "missing parent"))?;
            state
                .register_session_db_handle(vm_id, session_dir)
                .map_err(|error| ledger_route_error(vm_id, ledger, "open", db_path, error))?
        }
        None => {
            let session_dir = db_path
                .parent()
                .ok_or_else(|| ledger_route_error(vm_id, ledger, "resolve session dir", db_path, "missing parent"))?;
            let handle = state
                .register_session_db_handle(vm_id, session_dir)
                .map_err(|error| ledger_route_error(vm_id, ledger, "open", db_path, error))?;
            info!(
                vm_id,
                ledger,
                operation = "lazy_register_session_db_handle",
                db_path = %db_path.display(),
                "registered missing session DB handle for route"
            );
            handle
        }
    };
    db.ready()
        .await
        .map_err(|error| ledger_route_error(vm_id, ledger, "ready", db_path, error))?;
    Ok(db)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn query_route_db_json(
    vm_id: &str,
    ledger: &str,
    operation: &str,
    query_name: &str,
    db_path: &StdPath,
    db: &capsem_logger::DbHandle,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<serde_json::Value, AppError> {
    let raw = db.query(sql, params).await.map_err(|error| {
        error!(
            vm_id,
            ledger,
            operation,
            query_name,
            db_path = %db_path.display(),
            error = %error,
            "session ledger route DB query failed"
        );
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{ledger} ledger query {query_name} failed for {vm_id}: {error}"),
        )
    })?;
    serde_json::from_str(&raw).map_err(|error| {
        error!(
            vm_id,
            ledger,
            operation = "parse query json",
            query_name,
            db_path = %db_path.display(),
            error = %error,
            "session ledger route DB query returned invalid JSON"
        );
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{ledger} ledger query {query_name} returned invalid json for {vm_id}: {error}"),
        )
    })
}

pub(super) fn query_json_to_objects(raw: serde_json::Value) -> Vec<serde_json::Value> {
    let columns: Vec<String> = raw
        .get("columns")
        .and_then(|value| value.as_array())
        .map(|columns| {
            columns
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let rows = raw
        .get("rows")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let mut objects = Vec::with_capacity(rows.len());
    for row in rows {
        let values = row.as_array().cloned().unwrap_or_default();
        let mut object = serde_json::Map::new();
        for (index, column) in columns.iter().enumerate() {
            object.insert(
                column.clone(),
                values.get(index).cloned().unwrap_or(serde_json::Value::Null),
            );
        }
        objects.push(serde_json::Value::Object(object));
    }
    objects
}

pub(super) async fn query_route_objects(
    vm_id: &str,
    ledger: &str,
    query_name: &str,
    db_path: &StdPath,
    db: &capsem_logger::DbHandle,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<Vec<serde_json::Value>, AppError> {
    let raw = query_route_db_json(vm_id, ledger, "query", query_name, db_path, db, sql, params).await?;
    Ok(query_json_to_objects(raw))
}

pub(super) async fn query_route_typed_rows<T>(
    vm_id: &str,
    ledger: &str,
    query_name: &str,
    db_path: &StdPath,
    db: &capsem_logger::DbHandle,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<Vec<T>, AppError>
where
    T: DeserializeOwned,
{
    let objects = query_route_objects(vm_id, ledger, query_name, db_path, db, sql, params).await?;
    objects
        .into_iter()
        .map(|object| {
            serde_json::from_value::<T>(object).map_err(|error| {
                error!(
                    vm_id,
                    ledger,
                    operation = "decode query rows",
                    query_name,
                    db_path = %db_path.display(),
                    error = %error,
                    "session ledger route DB query mapping failed"
                );
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("{ledger} ledger query {query_name} mapping failed for {vm_id}: {error}"),
                )
            })
        })
        .collect()
}

pub(super) fn main_ledger_route_error(
    ledger: &str,
    operation: &str,
    db_path: &StdPath,
    error: impl std::fmt::Display,
) -> AppError {
    let error = error.to_string();
    error!(
        ledger,
        operation,
        db_path = %db_path.display(),
        error = %error,
        "main ledger route DB operation failed"
    );
    AppError(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("failed to {operation} {ledger} main ledger: {error}"),
    )
}

const SECURITY_LATEST_SQL: &str = r#"
SELECT timestamp_unix_ms, event_id, event_type, rule_id,
       rule_action, detection_level, rule_json, event_json, trace_id,
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

const BROKERED_CREDENTIAL_STATS_SQL: &str = r#"
SELECT MAX(provider) AS provider, substitution_ref AS credential_ref, COUNT(*) AS observed_count,
       SUM(CASE WHEN outcome = 'injected' THEN 1 ELSE 0 END) AS injected_count,
       MAX(timestamp) AS last_seen
FROM substitution_events
WHERE material_class = 'credential'
GROUP BY substitution_ref
ORDER BY MAX(timestamp) DESC
LIMIT 100
"#;

pub(super) async fn read_security_session_ledger(
    state: &ServiceState,
    vm_id: &str,
    db_path: &StdPath,
) -> Result<Option<SecuritySessionLedger>, AppError> {
    let db = open_ready_session_db(state, vm_id, "security", db_path).await?;
    let latest = query_route_typed_rows::<capsem_logger::SecurityRuleEvent>(
        vm_id,
        "security",
        "latest",
        db_path,
        &db,
        SECURITY_LATEST_SQL,
        &[json!(2000)],
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
    Ok(Some(SecuritySessionLedger {
        latest,
        stats,
        brokered_credentials,
    }))
}

pub(super) async fn read_profile_security_ledgers(
    state: &ServiceState,
    profile_id: &str,
) -> Result<Vec<(String, SecuritySessionLedger)>, AppError> {
    let mut ledgers = Vec::new();
    for (vm_id, session_dir) in profile_session_dirs(state, profile_id) {
        let Some(session) = read_security_session_ledger(state, &vm_id, &session_dir.join("session.db")).await? else {
            continue;
        };
        ledgers.push((vm_id, session));
    }
    Ok(ledgers)
}

#[derive(Clone, Debug)]
pub(super) struct HistorySessionLedger {
    pub(super) entries: Vec<capsem_logger::HistoryEntry>,
    pub(super) processes: Vec<capsem_logger::ProcessEntry>,
    pub(super) counts: capsem_logger::HistoryCounts,
}

impl Default for HistorySessionLedger {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            processes: Vec::new(),
            counts: capsem_logger::HistoryCounts {
                exec_count: 0,
                audit_count: 0,
            },
        }
    }
}

const HISTORY_ENTRIES_SQL: &str = r#"
SELECT timestamp, 'exec' AS layer, command, exit_code, duration_ms,
       stdout_preview, stderr_preview,
       json_object(
           'source', source,
           'trace_id', trace_id,
           'process_name', process_name,
           'exec_id', exec_id
       ) AS details
FROM exec_events
UNION ALL
SELECT timestamp, 'audit' AS layer, argv AS command, exit_code, NULL AS duration_ms,
       NULL AS stdout_preview, NULL AS stderr_preview,
       json_object(
           'pid', pid,
           'ppid', ppid,
           'uid', uid,
           'exe', exe,
           'comm', comm,
           'cwd', cwd,
           'tty', tty,
           'session_id', session_id,
           'audit_id', audit_id,
           'parent_exe', parent_exe
       ) AS details
FROM audit_events
ORDER BY timestamp DESC
"#;

const HISTORY_PROCESSES_SQL: &str = r#"
SELECT exe, COUNT(*) AS command_count,
       MIN(timestamp) AS first_seen,
       MAX(timestamp) AS last_seen
FROM audit_events
GROUP BY exe
ORDER BY command_count DESC
LIMIT ?
"#;

const HISTORY_COUNTS_SQL: &str = r#"
SELECT
    (SELECT COUNT(*) FROM exec_events) AS exec_count,
    (SELECT COUNT(*) FROM audit_events) AS audit_count
"#;

pub(super) async fn read_history_session_ledger(
    state: &ServiceState,
    vm_id: &str,
    db_path: &StdPath,
) -> Result<Option<HistorySessionLedger>, AppError> {
    let db = open_ready_session_db(state, vm_id, "history", db_path).await?;
    let mut entries = query_route_typed_rows::<capsem_logger::HistoryEntry>(
        vm_id,
        "history",
        "entries",
        db_path,
        &db,
        HISTORY_ENTRIES_SQL,
        &[],
    )
    .await?;
    for entry in &mut entries {
        if let serde_json::Value::String(details) = &entry.details {
            entry.details = serde_json::from_str(details)
                .map_err(|error| ledger_route_error(vm_id, "history", "parse entry details", db_path, error))?;
        }
    }
    let processes = query_route_typed_rows(
        vm_id,
        "history",
        "processes",
        db_path,
        &db,
        HISTORY_PROCESSES_SQL,
        &[json!(i64::MAX)],
    )
    .await?;
    let counts = query_route_typed_rows::<capsem_logger::HistoryCounts>(
        vm_id,
        "history",
        "counts",
        db_path,
        &db,
        HISTORY_COUNTS_SQL,
        &[],
    )
    .await?
    .into_iter()
    .next()
    .unwrap_or(capsem_logger::HistoryCounts {
        exec_count: 0,
        audit_count: 0,
    });
    Ok(Some(HistorySessionLedger {
        entries,
        processes,
        counts,
    }))
}

const TIMELINE_RECOVERY_LIMIT: usize = 50_000;
const TIMELINE_COLUMNS: [&str; 7] = [
    "timestamp",
    "layer",
    "ref",
    "summary",
    "status",
    "duration_ms",
    "trace_id",
];

#[derive(Clone, Debug)]
pub(super) struct TimelineRow {
    timestamp: String,
    layer: String,
    ref_value: serde_json::Value,
    summary: String,
    status: serde_json::Value,
    duration_ms: serde_json::Value,
    trace_id: Option<String>,
}

impl TimelineRow {
    fn to_values(&self) -> Vec<serde_json::Value> {
        vec![
            json!(self.timestamp),
            json!(self.layer),
            self.ref_value.clone(),
            json!(self.summary),
            self.status.clone(),
            self.duration_ms.clone(),
            self.trace_id
                .as_ref()
                .map(|trace_id| json!(trace_id))
                .unwrap_or(serde_json::Value::Null),
        ]
    }
}

pub(super) fn timeline_base_sql() -> String {
    let parts = [
        "SELECT timestamp, 'exec' AS layer, exec_id AS ref, command AS summary, \
         exit_code AS status, duration_ms, trace_id FROM exec_events",
        "SELECT COALESCE(NULLIF(tc.timestamp, ''), '1970-01-01T00:00:00Z') AS timestamp, \
         'tool' AS layer, tc.event_id AS ref, \
         COALESCE(tc.server_name, tc.origin) || '/' || tc.tool_name || COALESCE(' (call_id=' || tc.call_id || ')', '') AS summary, \
         tc.decision AS status, tc.duration_ms AS duration_ms, tc.trace_id AS trace_id \
         FROM tool_calls tc \
         WHERE tc.origin IN ('model', 'native', 'mcp', 'builtin', 'local', 'mcp_proxy')",
        "SELECT timestamp, 'net' AS layer, id AS ref, \
         COALESCE(method, 'GET') || ' ' || domain || COALESCE(path, '') AS summary, \
         status_code AS status, duration_ms, trace_id FROM net_events",
        "SELECT timestamp, 'fs' AS layer, id AS ref, action || ' ' || path AS summary, \
         NULL AS status, NULL AS duration_ms, trace_id FROM fs_events",
        "SELECT timestamp, 'model' AS layer, id AS ref, \
         provider || '/' || COALESCE(model, '?') AS summary, \
         status_code AS status, duration_ms, trace_id FROM model_calls",
    ];
    format!(
        "SELECT * FROM ({}) ORDER BY timestamp ASC LIMIT {TIMELINE_RECOVERY_LIMIT}",
        parts.join(" UNION ALL ")
    )
}

pub(super) fn json_value_as_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

pub(super) fn timeline_rows_from_query_json(raw: serde_json::Value) -> Vec<TimelineRow> {
    let columns = raw
        .get("columns")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let rows = raw
        .get("rows")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let column_index = |name: &str| columns.iter().position(|column| column.as_str() == Some(name));
    let Some(timestamp_idx) = column_index("timestamp") else {
        return Vec::new();
    };
    let Some(layer_idx) = column_index("layer") else {
        return Vec::new();
    };
    let Some(ref_idx) = column_index("ref") else {
        return Vec::new();
    };
    let Some(summary_idx) = column_index("summary") else {
        return Vec::new();
    };
    let Some(status_idx) = column_index("status") else {
        return Vec::new();
    };
    let Some(duration_idx) = column_index("duration_ms") else {
        return Vec::new();
    };
    let Some(trace_idx) = column_index("trace_id") else {
        return Vec::new();
    };

    rows.into_iter()
        .filter_map(|row| {
            let row = row.as_array()?;
            Some(TimelineRow {
                timestamp: json_value_as_string(row.get(timestamp_idx)?)?,
                layer: json_value_as_string(row.get(layer_idx)?)?,
                ref_value: row.get(ref_idx).cloned().unwrap_or(serde_json::Value::Null),
                summary: json_value_as_string(row.get(summary_idx)?)?,
                status: row.get(status_idx).cloned().unwrap_or(serde_json::Value::Null),
                duration_ms: row.get(duration_idx).cloned().unwrap_or(serde_json::Value::Null),
                trace_id: row.get(trace_idx).and_then(json_value_as_string),
            })
        })
        .collect()
}

pub(super) async fn read_timeline_rows_from_session_db(
    state: &ServiceState,
    vm_id: &str,
    db_path: &StdPath,
    sql: &str,
) -> Result<Option<Vec<TimelineRow>>, AppError> {
    let db = open_ready_session_db(state, vm_id, "timeline", db_path).await?;
    let raw = query_route_db_json(vm_id, "timeline", "query", "timeline", db_path, &db, sql, &[]).await?;
    Ok(Some(timeline_rows_from_query_json(raw)))
}

pub(super) async fn history_ledger_for_vm(state: &ServiceState, id: &str) -> Result<HistorySessionLedger, AppError> {
    let session_dir = resolve_session_dir(state, id)?;
    Ok(read_history_session_ledger(state, id, &session_dir.join("session.db"))
        .await?
        .unwrap_or_default())
}

pub(super) fn history_entry_matches_search(entry: &capsem_logger::HistoryEntry, query: &str) -> bool {
    entry.command.contains(query)
        || entry
            .stdout_preview
            .as_deref()
            .is_some_and(|value| value.contains(query))
        || entry
            .stderr_preview
            .as_deref()
            .is_some_and(|value| value.contains(query))
        || entry.details.to_string().contains(query)
}

pub(super) fn query_history_ledger(session: &HistorySessionLedger, params: &api::HistoryQuery) -> api::HistoryResponse {
    let mut entries = session
        .entries
        .iter()
        .filter(|entry| params.layer == "all" || entry.layer == params.layer)
        .filter(|entry| {
            params
                .search
                .as_deref()
                .is_none_or(|query| history_entry_matches_search(entry, query))
        })
        .cloned()
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| right.timestamp.cmp(&left.timestamp));
    let total = entries.len() as u64;
    let commands = entries
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .collect::<Vec<_>>();
    let has_more = (params.offset + commands.len()) < total as usize;
    api::HistoryResponse {
        commands,
        total,
        has_more,
    }
}

const STATS_DETAIL_MODEL_STATS_SQL: &str = r#"
SELECT provider, COALESCE(model, 'unknown') AS model,
       COUNT(*) AS call_count,
       COALESCE(SUM(input_tokens), 0) AS input_tokens,
       COALESCE(SUM(output_tokens), 0) AS output_tokens,
       COALESCE(SUM(estimated_cost_usd), 0.0) AS estimated_cost_usd,
       COALESCE(SUM(duration_ms), 0) AS duration_ms
FROM model_calls
GROUP BY provider, model
ORDER BY call_count DESC, provider ASC
"#;

const STATS_DETAIL_MODEL_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, provider, model, method, path, status_code,
       input_tokens, output_tokens, duration_ms, response_bytes,
       stop_reason, trace_id, credential_ref
FROM model_calls
ORDER BY id DESC
LIMIT 200
"#;

const STATS_DETAIL_TOOL_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, process_name, server_name, tool_name, method, call_id,
       model_call_id, model_parent_missing,
       decision, duration_ms, bytes, arguments, response_preview,
       error_message, source, credential_ref
FROM (
    SELECT tc.event_id,
           COALESCE(NULLIF(tc.timestamp, ''), mc.timestamp) AS timestamp,
           tc.process_name,
           COALESCE(tc.server_name, 'model') AS server_name,
           tc.tool_name,
           tc.method,
           tc.call_id,
           tc.model_call_id,
           CASE
               WHEN tc.model_call_id IS NOT NULL AND mc.id IS NULL THEN 1
               ELSE 0
           END AS model_parent_missing,
           tc.decision,
           COALESCE(tc.duration_ms, mc.duration_ms, 0) AS duration_ms,
           COALESCE(LENGTH(tc.arguments), 0) + COALESCE(LENGTH(COALESCE(tc.response_preview, tr.content_preview)), 0) AS bytes,
           tc.arguments,
           COALESCE(tc.response_preview, tr.content_preview) AS response_preview,
           tc.error_message,
           tc.origin AS source,
           COALESCE(tc.credential_ref, tr.credential_ref) AS credential_ref
    FROM tool_calls tc
    LEFT JOIN model_calls mc ON tc.model_call_id = mc.id
    LEFT JOIN tool_responses tr ON tc.call_id = tr.call_id
    WHERE tc.origin IN ('model', 'native', 'mcp', 'builtin', 'local', 'mcp_proxy')
)
ORDER BY timestamp DESC
LIMIT 200
"#;

const STATS_DETAIL_HTTP_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, domain, port, method, path, query, status_code,
       decision, duration_ms, bytes_sent, bytes_received, matched_rule, policy_rule,
       trace_id, credential_ref, request_headers, response_headers
FROM net_events
ORDER BY id DESC
LIMIT 200
"#;

const STATS_DETAIL_DNS_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, qname, qtype, qclass, rcode, decision,
       matched_rule, policy_rule, source_proto, process_name,
       upstream_resolver_ms, trace_id, credential_ref
FROM dns_events
ORDER BY id DESC
LIMIT 200
"#;

const STATS_DETAIL_FILE_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, action, path, size, trace_id, credential_ref
FROM fs_events
ORDER BY id DESC
LIMIT 200
"#;

const STATS_DETAIL_PROCESS_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, exec_id, command, exit_code, duration_ms,
       stdout_bytes, stderr_bytes, source, process_name, pid, trace_id,
       credential_ref
FROM exec_events
ORDER BY id DESC
LIMIT 100
"#;

const STATS_DETAIL_AUDIT_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, pid, ppid, uid, exe, comm, argv, cwd,
       exit_code, session_id, tty, audit_id, exec_event_id, parent_exe,
       trace_id, credential_ref
FROM audit_events
ORDER BY id DESC
LIMIT 100
"#;

const STATS_DETAIL_CREDENTIAL_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, material_class, source, event_type,
       event_type AS origin, outcome AS verb, provider,
       trace_id, context_json
FROM substitution_events
ORDER BY id DESC
LIMIT 100
"#;

const STATS_DETAIL_BODY_BLOBS_SQL: &str = r#"
SELECT event_id, direction, content_type, original_bytes,
       stored_bytes, truncated, body_hash, CAST(body AS TEXT) AS body
FROM event_body_blobs
WHERE event_id IN (
    SELECT event_id FROM net_events WHERE event_id IS NOT NULL ORDER BY id DESC LIMIT 200
)
OR event_id IN (
    SELECT event_id FROM model_calls WHERE event_id IS NOT NULL ORDER BY id DESC LIMIT 200
)
OR event_id IN (
    SELECT event_id FROM tool_calls WHERE event_id IS NOT NULL ORDER BY id DESC LIMIT 200
)
ORDER BY event_id, direction
"#;

pub(super) async fn stats_detail_query_objects(
    vm_id: &str,
    db_path: &StdPath,
    db: &capsem_logger::DbHandle,
    query_name: &str,
    sql: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    query_route_objects(vm_id, "stats_detail", query_name, db_path, db, sql, &[]).await
}

pub(super) fn body_blob_map(rows: Vec<serde_json::Value>) -> serde_json::Value {
    let mut by_event = serde_json::Map::new();
    for row in rows {
        let Some(event_id) = row.get("event_id").and_then(|value| value.as_str()) else {
            continue;
        };
        let entry = by_event
            .entry(event_id.to_string())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        if let serde_json::Value::Array(rows) = entry {
            rows.push(row);
        }
    }
    serde_json::Value::Object(by_event)
}

pub(super) fn stats_detail_db_fingerprint(db_path: &StdPath) -> Option<String> {
    let metadata = std::fs::metadata(db_path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    Some(format!("{}:{modified}", metadata.len()))
}

pub(super) fn session_response_cache_key(vm_id: &str, route_key: &str) -> String {
    format!("{vm_id}:{route_key}")
}

pub(super) fn session_response_cache_get(
    state: &ServiceState,
    vm_id: &str,
    route_key: &str,
    db_path: &StdPath,
) -> Option<Bytes> {
    let db_fingerprint = stats_detail_db_fingerprint(db_path)?;
    let cache_key = session_response_cache_key(vm_id, route_key);
    let cached = state
        .stats_detail_response_cache
        .lock()
        .unwrap()
        .get(&cache_key)
        .cloned()?;
    (cached.db_fingerprint == db_fingerprint).then(|| Bytes::from(cached.bytes))
}

pub(super) fn session_response_cache_store(
    state: &ServiceState,
    vm_id: &str,
    route_key: &str,
    db_path: &StdPath,
    bytes: &[u8],
) {
    let Some(db_fingerprint) = stats_detail_db_fingerprint(db_path) else {
        return;
    };
    let cache_key = session_response_cache_key(vm_id, route_key);
    state.stats_detail_response_cache.lock().unwrap().insert(
        cache_key,
        CachedStatsDetailResponse {
            db_fingerprint,
            bytes: bytes.to_vec(),
        },
    );
}

pub(super) async fn read_stats_detail_payload_from_session_db(
    state: &ServiceState,
    vm_id: &str,
    db_path: &StdPath,
) -> Result<serde_json::Value, AppError> {
    let db = open_ready_session_db(state, vm_id, "stats_detail", db_path).await?;
    Ok(json!({
        "model_stats": stats_detail_query_objects(vm_id, db_path, &db, "model_stats", STATS_DETAIL_MODEL_STATS_SQL).await?,
        "model_events": stats_detail_query_objects(vm_id, db_path, &db, "model_events", STATS_DETAIL_MODEL_EVENTS_SQL).await?,
        "tool_events": stats_detail_query_objects(vm_id, db_path, &db, "tool_events", STATS_DETAIL_TOOL_EVENTS_SQL).await?,
        "http_events": stats_detail_query_objects(vm_id, db_path, &db, "http_events", STATS_DETAIL_HTTP_EVENTS_SQL).await?,
        "dns_events": stats_detail_query_objects(vm_id, db_path, &db, "dns_events", STATS_DETAIL_DNS_EVENTS_SQL).await?,
        "file_events": stats_detail_query_objects(vm_id, db_path, &db, "file_events", STATS_DETAIL_FILE_EVENTS_SQL).await?,
        "process_events": stats_detail_query_objects(vm_id, db_path, &db, "process_events", STATS_DETAIL_PROCESS_EVENTS_SQL).await?,
        "audit_events": stats_detail_query_objects(vm_id, db_path, &db, "audit_events", STATS_DETAIL_AUDIT_EVENTS_SQL).await?,
        "credential_events": stats_detail_query_objects(vm_id, db_path, &db, "credential_events", STATS_DETAIL_CREDENTIAL_EVENTS_SQL).await?,
        "body_blobs": body_blob_map(stats_detail_query_objects(vm_id, db_path, &db, "body_blobs", STATS_DETAIL_BODY_BLOBS_SQL).await?),
    }))
}

const STATS_RESPONSE_SQL: &str = r#"
SELECT json_object(
    'global', json_object(
        'total_sessions', (SELECT COUNT(*) FROM sessions),
        'total_input_tokens', (SELECT COALESCE(SUM(total_input_tokens), 0) FROM sessions),
        'total_output_tokens', (SELECT COALESCE(SUM(total_output_tokens), 0) FROM sessions),
        'total_estimated_cost', (SELECT COALESCE(SUM(total_estimated_cost), 0.0) FROM sessions),
        'total_tool_calls', (SELECT COALESCE(SUM(total_tool_calls), 0) FROM sessions),
        'total_file_events', (SELECT COALESCE(SUM(total_file_events), 0) FROM sessions),
        'total_requests', (SELECT COALESCE(SUM(total_requests), 0) FROM sessions),
        'total_allowed', (SELECT COALESCE(SUM(allowed_requests), 0) FROM sessions),
        'total_denied', (SELECT COALESCE(SUM(denied_requests), 0) FROM sessions)
    ),
    'sessions', json(COALESCE((
        SELECT json_group_array(json_object(
            'id', id,
            'mode', mode,
            'command', command,
            'status', status,
            'created_at', created_at,
            'stopped_at', stopped_at,
            'scratch_disk_size_gb', scratch_disk_size_gb,
            'ram_bytes', ram_bytes,
            'total_requests', total_requests,
            'allowed_requests', allowed_requests,
            'denied_requests', denied_requests,
            'total_input_tokens', total_input_tokens,
            'total_output_tokens', total_output_tokens,
            'total_estimated_cost', total_estimated_cost,
            'total_tool_calls', total_tool_calls,
            'total_file_events', total_file_events,
            'compressed_size_bytes', compressed_size_bytes,
            'vacuumed_at', vacuumed_at,
            'storage_mode', storage_mode,
            'rootfs_hash', rootfs_hash,
            'rootfs_version', rootfs_version,
            'forked_from', forked_from,
            'persistent', CASE WHEN persistent THEN json('true') ELSE json('false') END,
            'exec_count', exec_count,
            'audit_event_count', audit_event_count
        ))
        FROM (
            SELECT id, mode, command, status, created_at, stopped_at,
                   scratch_disk_size_gb, ram_bytes, total_requests, allowed_requests,
                   denied_requests, total_input_tokens, total_output_tokens,
                   total_estimated_cost, total_tool_calls, total_file_events,
                   compressed_size_bytes, vacuumed_at, storage_mode, rootfs_hash,
                   rootfs_version, forked_from, persistent, exec_count, audit_event_count
            FROM sessions
            ORDER BY created_at DESC
            LIMIT ?
        )
    ), '[]')),
    'top_providers', json(COALESCE((
        SELECT json_group_array(json_object(
            'provider', provider,
            'call_count', call_count,
            'input_tokens', input_tokens,
            'output_tokens', output_tokens,
            'estimated_cost', estimated_cost,
            'total_duration_ms', total_duration_ms
        ))
        FROM (
            SELECT provider,
                   SUM(call_count) AS call_count,
                   SUM(input_tokens) AS input_tokens,
                   SUM(output_tokens) AS output_tokens,
                   SUM(estimated_cost) AS estimated_cost,
                   SUM(total_duration_ms) AS total_duration_ms
            FROM ai_usage
            GROUP BY provider
            ORDER BY SUM(call_count) DESC
            LIMIT ?
        )
    ), '[]')),
    'top_tools', json(COALESCE((
        SELECT json_group_array(json_object(
            'tool_name', tool_name,
            'call_count', call_count,
            'total_bytes', total_bytes,
            'total_duration_ms', total_duration_ms
        ))
        FROM (
            SELECT tool_name,
                   SUM(call_count) AS call_count,
                   SUM(total_bytes) AS total_bytes,
                   SUM(total_duration_ms) AS total_duration_ms
            FROM tool_usage
            GROUP BY tool_name
            ORDER BY SUM(call_count) DESC
            LIMIT ?
        )
    ), '[]')),
    'top_mcp_tools', json(COALESCE((
        SELECT json_group_array(json_object(
            'tool_name', tool_name,
            'server_name', server_name,
            'call_count', call_count,
            'total_bytes', total_bytes,
            'total_duration_ms', total_duration_ms
        ))
        FROM (
            SELECT tool_name,
                   server_name,
                   SUM(call_count) AS call_count,
                   SUM(total_bytes) AS total_bytes,
                   SUM(total_duration_ms) AS total_duration_ms
            FROM mcp_usage
            GROUP BY tool_name, server_name
            ORDER BY SUM(call_count) DESC
            LIMIT ?
        )
    ), '[]'))
) AS payload
"#;

pub(super) async fn read_stats_response_from_main_db_handle(state: &ServiceState) -> Result<Vec<u8>, AppError> {
    let db_path = state.main_db_path();
    let db = &state.profile_mutation_db;
    let db_epoch = db.read_cache_epoch();
    if let Some(cached) = state.stats_response_cache.lock().unwrap().clone() {
        if cached.db_epoch == db_epoch {
            return Ok(cached.bytes);
        }
    }

    db.ready()
        .await
        .map_err(|error| main_ledger_route_error("stats", "ready", &db_path, error))?;

    let mut raw = db
        .query_many(vec![(
            STATS_RESPONSE_SQL.to_string(),
            vec![json!(100), json!(20), json!(20), json!(20)],
        )])
        .await
        .map_err(|error| main_ledger_route_error("stats", "query response", &db_path, error))?
        .into_iter();
    let raw = raw
        .next()
        .ok_or_else(|| main_ledger_route_error("stats", "query response", &db_path, "no rows"))?;
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|error| main_ledger_route_error("stats", "parse response query", &db_path, error))?;
    let payload = parsed
        .get("rows")
        .and_then(|rows| rows.as_array())
        .and_then(|rows| rows.first())
        .and_then(|row| row.as_array())
        .and_then(|row| row.first())
        .ok_or_else(|| main_ledger_route_error("stats", "read response payload", &db_path, "missing payload"))?;
    match payload {
        serde_json::Value::String(payload) => {
            let bytes = payload.as_bytes().to_vec();
            *state.stats_response_cache.lock().unwrap() = Some(CachedStatsResponse {
                db_epoch,
                bytes: bytes.clone(),
            });
            Ok(bytes)
        }
        serde_json::Value::Object(_) => {
            let bytes = serde_json::to_vec(payload)
                .map_err(|error| main_ledger_route_error("stats", "serialize response payload", &db_path, error))?;
            *state.stats_response_cache.lock().unwrap() = Some(CachedStatsResponse {
                db_epoch,
                bytes: bytes.clone(),
            });
            Ok(bytes)
        }
        other => Err(main_ledger_route_error(
            "stats",
            "read response payload",
            &db_path,
            format!("unexpected payload type: {other}"),
        )),
    }
}

pub(super) fn hydrate_startup_route_caches(state: &ServiceState) -> Result<(), AppError> {
    rebuild_profile_status_cache(state).map_err(|AppError(status, message)| {
        AppError(status, format!("failed to build profile status cache: {message}"))
    })?;
    Ok(())
}

pub(super) async fn apply_session_db_status(state: &ServiceState, info: &mut SandboxInfo, session_dir: &StdPath) {
    let db_path = session_db_path_for_session_dir(session_dir);
    if !db_path.exists() {
        info.session_db = Some(api::SessionDbStatus {
            ready: false,
            error: Some("session.db absent".to_string()),
        });
        info!(
            vm_id = info.id.as_str(),
            operation = "session_db_status",
            db_path = %db_path.display(),
            ready = false,
            "session DB absent while building session status"
        );
        return;
    }
    match open_ready_session_db(state, &info.id, "session status", &db_path).await {
        Ok(_) => {
            info.session_db = Some(api::SessionDbStatus {
                ready: true,
                error: None,
            });
            info!(
                vm_id = info.id.as_str(),
                operation = "session_db_status",
                db_path = %db_path.display(),
                ready = true,
                "session DB ready for session status"
            );
        }
        Err(error) => {
            let message = error.1;
            info.session_db = Some(api::SessionDbStatus {
                ready: false,
                error: Some(message.clone()),
            });
            warn!(
                vm_id = info.id.as_str(),
                operation = "session_db_status",
                db_path = %db_path.display(),
                ready = false,
                error = %message,
                "session DB not ready for session status"
            );
        }
    }
}

pub(super) async fn security_latest_for_vm(
    state: &ServiceState,
    vm_id: &str,
    limit: usize,
    detection_only: bool,
) -> Result<Vec<capsem_logger::SecurityRuleEvent>, AppError> {
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

pub(super) async fn security_stats_for_vm(
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

pub(super) fn security_detection_count(stats: &capsem_logger::SecurityRuleStats) -> u64 {
    stats
        .by_level
        .iter()
        .filter(|count| count.detection_level != "none")
        .map(|count| count.count)
        .sum()
}

pub(super) async fn handle_service_security_latest(
    State(state): State<Arc<ServiceState>>,
    Query(params): Query<SecurityLedgerQuery>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let limit = params.limit.unwrap_or(100).min(2000);
    let mut rows = Vec::new();
    for (vm_id, session_dir) in service_session_dirs(&state) {
        let Some(session) = read_security_session_ledger(&state, &vm_id, &session_dir.join("session.db")).await? else {
            continue;
        };
        for event in session.latest.into_iter().take(limit) {
            rows.push(json!({ "vm_id": vm_id, "event": event }));
        }
    }
    rows.sort_by(|left, right| {
        right["event"]["timestamp_unix_ms"]
            .as_i64()
            .cmp(&left["event"]["timestamp_unix_ms"].as_i64())
    });
    rows.truncate(limit);
    Ok(Json(rows))
}

pub(super) async fn handle_service_detection_latest(
    State(state): State<Arc<ServiceState>>,
    Query(params): Query<SecurityLedgerQuery>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let limit = params.limit.unwrap_or(100).min(2000);
    let mut rows = Vec::new();
    for (vm_id, session_dir) in service_session_dirs(&state) {
        let Some(session) = read_security_session_ledger(&state, &vm_id, &session_dir.join("session.db")).await? else {
            continue;
        };
        for event in session.latest.into_iter().take(limit) {
            if is_detection_rule_event(&event) {
                rows.push(json!({ "vm_id": vm_id, "event": event }));
            }
        }
    }
    rows.sort_by(|left, right| {
        right["event"]["timestamp_unix_ms"]
            .as_i64()
            .cmp(&left["event"]["timestamp_unix_ms"].as_i64())
    });
    rows.truncate(limit);
    Ok(Json(rows))
}

pub(super) async fn handle_service_security_status(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut total = 0_u64;
    let mut sessions = Vec::new();
    for (vm_id, session_dir) in service_session_dirs(&state) {
        let Some(session) = read_security_session_ledger(&state, &vm_id, &session_dir.join("session.db")).await? else {
            continue;
        };
        let stats = session.stats;
        total += stats.total;
        sessions.push(json!({ "vm_id": vm_id, "stats": stats }));
    }
    Ok(Json(json!({ "total": total, "sessions": sessions })))
}

pub(super) async fn handle_service_detection_status(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut total = 0_u64;
    let mut sessions = Vec::new();
    for (vm_id, session_dir) in service_session_dirs(&state) {
        let Some(session) = read_security_session_ledger(&state, &vm_id, &session_dir.join("session.db")).await? else {
            continue;
        };
        let count = security_detection_count(&session.stats);
        total += count;
        sessions.push(json!({ "vm_id": vm_id, "total": count }));
    }
    Ok(Json(json!({ "total": total, "sessions": sessions })))
}

pub(super) fn default_plugin_config(mode: SecurityPluginMode) -> SecurityPluginConfig {
    SecurityPluginConfig {
        mode,
        detection_level: DetectionLevel::Informational,
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PluginCatalogEntry {
    name: &'static str,
    description: &'static str,
    default_config: SecurityPluginConfig,
    stage: PluginStage,
    version: &'static str,
}

static PLUGIN_CATALOG: LazyLock<BTreeMap<String, PluginCatalogEntry>> = LazyLock::new(|| {
    BTreeMap::from([
        (
            "credential_broker".to_string(),
            PluginCatalogEntry {
                name: "Credential Broker",
                description: "captures observed credentials into brokered credential references",
                default_config: default_plugin_config(SecurityPluginMode::Rewrite),
                stage: PluginStage::Preprocess,
                version: "1",
            },
        ),
        (
            "log_sanitizer".to_string(),
            PluginCatalogEntry {
                name: "Log Sanitizer",
                description: "sanitizes credential material before durable security ledger writes",
                default_config: default_plugin_config(SecurityPluginMode::Rewrite),
                stage: PluginStage::Logging,
                version: "1",
            },
        ),
        (
            "dummy_pre_eicar".to_string(),
            PluginCatalogEntry {
                name: "Dummy Preprocess EICAR",
                description: "debug preprocess plugin that blocks harmless EICAR test content",
                default_config: default_plugin_config(SecurityPluginMode::Disable),
                stage: PluginStage::Preprocess,
                version: "1",
            },
        ),
        (
            "dummy_post_allow".to_string(),
            PluginCatalogEntry {
                name: "Dummy Postprocess Allow",
                description: "debug postprocess plugin that requests allow to prove block is absolute",
                default_config: default_plugin_config(SecurityPluginMode::Disable),
                stage: PluginStage::Postprocess,
                version: "1",
            },
        ),
    ])
});

pub(super) fn plugin_catalog() -> &'static BTreeMap<String, PluginCatalogEntry> {
    &PLUGIN_CATALOG
}

pub(super) fn validate_profile_route_id_from_state(
    state: &ServiceState,
    profile_id: String,
) -> Result<String, AppError> {
    if profile_id.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "profile id must not be empty".to_string(),
        ));
    }
    if !state
        .profile_summary_cache
        .lock()
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("profile summary cache lock poisoned: {error}"),
            )
        })?
        .iter()
        .any(|summary| summary.id == profile_id)
    {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            format!("profile not found: {profile_id}"),
        ));
    }
    Ok(profile_id)
}

pub(super) fn profile_plugin_scope(state: &ServiceState, profile_id: String) -> Result<PluginScope, AppError> {
    Ok(PluginScope {
        kind: PluginScopeKind::Profile,
        profile_id: validate_profile_route_id_from_state(state, profile_id)?,
    })
}

pub(super) fn effective_plugin_policy(
    state: &ServiceState,
    profile_id: &str,
) -> BTreeMap<String, SecurityPluginConfig> {
    let mut policy: BTreeMap<_, _> = plugin_catalog()
        .iter()
        .map(|(id, entry)| (id.clone(), entry.default_config))
        .collect();
    if let Some(profile_policy) = state.profile_plugin_policy_cache.lock().unwrap().get(profile_id) {
        for (id, config) in profile_policy {
            policy.insert(id.clone(), *config);
        }
    }
    if let Some(overrides) = state.plugin_policy_by_profile.lock().unwrap().get(profile_id) {
        for (id, config) in overrides {
            policy.insert(id.clone(), *config);
        }
    }
    policy
}

pub(super) async fn plugin_info_for(
    state: &ServiceState,
    plugin_id: &str,
    scope: PluginScope,
    include_runtime: bool,
) -> Result<PluginInfo, AppError> {
    let catalog = plugin_catalog();
    let Some(catalog_entry) = catalog.get(plugin_id).copied() else {
        return Err(AppError(StatusCode::NOT_FOUND, format!("unknown plugin: {plugin_id}")));
    };
    let effective = effective_plugin_policy(state, &scope.profile_id);
    let config = effective
        .get(plugin_id)
        .copied()
        .unwrap_or(catalog_entry.default_config);
    let overridden = state
        .plugin_policy_by_profile
        .lock()
        .unwrap()
        .get(&scope.profile_id)
        .is_some_and(|policy| policy.contains_key(plugin_id));
    let runtime = if include_runtime {
        plugin_runtime_status(state, &scope.profile_id, plugin_id, config).await
    } else {
        plugin_runtime_config_status(config)
    };
    let detail_routes = plugin_detail_routes(plugin_id, &scope);
    Ok(PluginInfo {
        id: plugin_id.to_string(),
        name: catalog_entry.name,
        config,
        default_config: catalog_entry.default_config,
        overridden,
        scope,
        description: catalog_entry.description,
        stage: catalog_entry.stage,
        version: catalog_entry.version,
        capabilities: plugin_capabilities(plugin_id),
        runtime,
        detail_routes,
    })
}

pub(super) fn plugin_capabilities(plugin_id: &str) -> PluginCapabilities {
    match plugin_id {
        "credential_broker" => PluginCapabilities {
            event_families: vec!["http", "file", "mcp"],
            credential_providers: capsem_core::credential_broker::CredentialProvider::all()
                .iter()
                .map(|provider| provider.as_str())
                .collect(),
            credential_sources: vec![
                "http.authorization",
                "http.body.oauth_token",
                "file.env",
                "mcp.auth_reference",
            ],
        },
        "dummy_pre_eicar" => PluginCapabilities {
            event_families: vec!["http", "model", "file", "mcp"],
            credential_providers: Vec::new(),
            credential_sources: Vec::new(),
        },
        "dummy_post_allow" => PluginCapabilities {
            event_families: vec!["http", "model", "file", "mcp"],
            credential_providers: Vec::new(),
            credential_sources: Vec::new(),
        },
        "log_sanitizer" => PluginCapabilities {
            event_families: vec!["http", "model", "file", "mcp"],
            credential_providers: Vec::new(),
            credential_sources: vec!["security_event.credential_observations"],
        },
        _ => PluginCapabilities {
            event_families: Vec::new(),
            credential_providers: Vec::new(),
            credential_sources: Vec::new(),
        },
    }
}

pub(super) fn plugin_detail_routes(plugin_id: &str, scope: &PluginScope) -> Vec<PluginDetailRoute> {
    match plugin_id {
        "credential_broker" => vec![
            PluginDetailRoute {
                id: "credential_broker_credentials",
                label: "Credential Broker",
                kind: PluginDetailRouteKind::CredentialBroker,
                path: format!(
                    "/profiles/{}/plugins/credential_broker/credentials/info",
                    scope.profile_id
                ),
            },
            PluginDetailRoute {
                id: "credential_broker_credentials_reload",
                label: "Retry Credential Store",
                kind: PluginDetailRouteKind::CredentialBroker,
                path: format!(
                    "/profiles/{}/plugins/credential_broker/credentials/reload",
                    scope.profile_id
                ),
            },
        ],
        _ => Vec::new(),
    }
}

pub(super) async fn plugin_runtime_status(
    state: &ServiceState,
    profile_id: &str,
    plugin_id: &str,
    config: SecurityPluginConfig,
) -> PluginRuntimeStatus {
    let mut status = plugin_runtime_config_status(config);
    hydrate_plugin_execution_runtime(state, profile_id, plugin_id, &mut status).await;
    if plugin_id == "credential_broker" {
        hydrate_credential_broker_runtime(state, profile_id, &mut status).await;
    }
    status
}

pub(super) fn plugin_runtime_config_status(config: SecurityPluginConfig) -> PluginRuntimeStatus {
    PluginRuntimeStatus {
        enabled: config.mode != SecurityPluginMode::Disable,
        event_count: 0,
        execution_count: 0,
        applied_count: 0,
        skipped_count: 0,
        total_duration_us: 0,
        max_duration_us: 0,
        detection_count: 0,
        block_count: 0,
        rewrite_count: 0,
        last_error: None,
        brokered_credentials: Vec::new(),
    }
}

pub(super) async fn hydrate_plugin_execution_runtime(
    state: &ServiceState,
    profile_id: &str,
    plugin_id: &str,
    status: &mut PluginRuntimeStatus,
) {
    let sessions = match read_profile_security_ledgers(state, profile_id).await {
        Ok(sessions) => sessions,
        Err(error) => {
            status.last_error = Some(format!("failed to read security ledger: {}", error.1));
            return;
        }
    };
    let mut seen_executions = HashSet::<(String, String)>::new();
    let mut seen_detections = HashSet::<(String, String)>::new();
    for (_vm_id, session) in sessions {
        for event in session.latest {
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&event.event_json) else {
                status.last_error = Some(format!(
                    "failed to parse plugin execution payload for {}",
                    event.event_id
                ));
                continue;
            };
            if let Some(executions) = payload.get("plugin_executions").and_then(serde_json::Value::as_array) {
                for execution in executions {
                    if execution.get("plugin_id").and_then(serde_json::Value::as_str) != Some(plugin_id) {
                        continue;
                    }
                    let stage = execution
                        .get("stage")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown");
                    if !seen_executions.insert((event.event_id.clone(), format!("{plugin_id}:{stage}"))) {
                        continue;
                    }
                    status.execution_count += 1;
                    if execution
                        .get("applied")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                    {
                        status.applied_count += 1;
                    } else {
                        status.skipped_count += 1;
                    }
                    let duration_us = execution
                        .get("duration_us")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    status.total_duration_us = status.total_duration_us.saturating_add(duration_us);
                    status.max_duration_us = status.max_duration_us.max(duration_us);
                }
            }
            if let Some(detections) = payload.get("detections").and_then(serde_json::Value::as_array) {
                for detection in detections {
                    if detection.get("source").and_then(serde_json::Value::as_str) != Some("plugin")
                        || detection.get("plugin_id").and_then(serde_json::Value::as_str) != Some(plugin_id)
                    {
                        continue;
                    }
                    if seen_detections.insert((event.event_id.clone(), plugin_id.to_string())) {
                        status.detection_count += 1;
                    }
                }
            }
        }
    }
}

pub(super) fn credential_ref_from_security_payload(value: &serde_json::Value) -> Option<String> {
    value
        .get("credential_ref")
        .or_else(|| value.get("substitution_ref"))
        .or_else(|| value.get("reference"))
        .or_else(|| value.get("ref"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

pub(super) fn credential_provider_from_security_payload(value: &serde_json::Value) -> Option<String> {
    value
        .get("provider")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

pub(super) fn merge_brokered_credential_status(
    credentials: &mut BTreeMap<(Option<String>, String), BrokeredCredentialStatus>,
    provider: Option<String>,
    credential_ref: String,
    observed_delta: u64,
    injected_delta: u64,
    last_seen: Option<String>,
) {
    let key = (provider.clone(), credential_ref.clone());
    let replay_available =
        capsem_core::credential_broker::broker_reference_replay_available(provider.as_deref(), &credential_ref);
    credentials
        .entry(key)
        .and_modify(|existing| {
            existing.observed_count = existing.observed_count.saturating_add(observed_delta);
            existing.injected_count = existing.injected_count.saturating_add(injected_delta);
            existing.replay_available |= replay_available;
            if last_seen.as_deref() > existing.last_seen.as_deref() {
                existing.last_seen = last_seen.clone();
            }
        })
        .or_insert(BrokeredCredentialStatus {
            provider,
            credential_ref,
            observed_count: observed_delta,
            injected_count: injected_delta,
            replay_available,
            last_seen,
        });
}

pub(super) async fn hydrate_credential_broker_runtime(
    state: &ServiceState,
    profile_id: &str,
    status: &mut PluginRuntimeStatus,
) {
    let sessions = match read_profile_security_ledgers(state, profile_id).await {
        Ok(sessions) => sessions,
        Err(error) => {
            status.last_error = Some(format!("failed to read security ledger: {}", error.1));
            return;
        }
    };
    let mut credentials: BTreeMap<(Option<String>, String), BrokeredCredentialStatus> = BTreeMap::new();
    let mut seen = HashSet::<(String, String, String, String)>::new();
    for (_vm_id, session) in sessions {
        for credential in &session.brokered_credentials {
            merge_brokered_credential_status(
                &mut credentials,
                credential.provider.clone(),
                credential.credential_ref.clone(),
                credential.observed_count,
                credential.injected_count,
                credential.last_seen.clone(),
            );
            status.event_count = status
                .event_count
                .saturating_add(credential.observed_count.saturating_add(credential.injected_count));
            status.rewrite_count = status.rewrite_count.saturating_add(credential.injected_count);
        }
        for event in session.latest {
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&event.event_json) else {
                status.last_error = Some(format!(
                    "failed to parse credential broker payload for {}",
                    event.event_id
                ));
                continue;
            };
            for (field, observed_delta, injected_delta) in [
                ("credential_observations", 1_u64, 0_u64),
                ("credential_injections", 0_u64, 1_u64),
            ] {
                let Some(items) = payload.get(field).and_then(serde_json::Value::as_array) else {
                    continue;
                };
                for item in items {
                    let Some(credential_ref) = credential_ref_from_security_payload(item) else {
                        continue;
                    };
                    let source = item.get("source").and_then(serde_json::Value::as_str).unwrap_or("");
                    if !seen.insert((
                        event.event_id.clone(),
                        field.to_string(),
                        credential_ref.clone(),
                        source.to_string(),
                    )) {
                        continue;
                    }
                    status.event_count = status.event_count.saturating_add(1);
                    status.rewrite_count = status.rewrite_count.saturating_add(injected_delta);
                    merge_brokered_credential_status(
                        &mut credentials,
                        credential_provider_from_security_payload(item),
                        credential_ref,
                        observed_delta,
                        injected_delta,
                        Some(event.timestamp_unix_ms.to_string()),
                    );
                }
            }
        }
    }
    let mut values: Vec<_> = credentials.into_values().collect();
    values.sort_by(|left, right| right.last_seen.cmp(&left.last_seen));
    status.brokered_credentials = values;
}

pub(super) async fn handle_profile_plugins(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let scope = profile_plugin_scope(&state, profile_id)?;
    let cache_key = format!("plugins:list:{}", scope.profile_id);
    if let Some(body) = state
        .profile_plugin_response_cache
        .lock()
        .unwrap()
        .get(&cache_key)
        .cloned()
    {
        return Ok(json_bytes_response(body));
    }
    let response = list_plugins_for_scope(&state, scope).await?;
    let body = serde_json::to_vec(&response).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize plugin list response: {error}"),
        )
    })?;
    state
        .profile_plugin_response_cache
        .lock()
        .unwrap()
        .insert(cache_key, Bytes::from(body.clone()));
    Ok(json_bytes_response(Bytes::from(body)))
}

pub(super) async fn handle_profile_plugins_info(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let scope = profile_plugin_scope(&state, profile_id)?;
    let plugins = effective_plugin_policy(&state, &scope.profile_id);
    Ok(Json(json!({
        "scope": scope,
        "plugin_count": plugins.len(),
        "enabled_count": plugins
            .values()
            .filter(|config| config.mode != SecurityPluginMode::Disable)
            .count(),
    })))
}

pub(super) async fn handle_profile_credential_broker_credentials_info(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<CredentialBrokerDetailResponse>, AppError> {
    let scope = profile_plugin_scope(&state, profile_id)?;
    let config = effective_plugin_policy(&state, &scope.profile_id)
        .get("credential_broker")
        .copied()
        .unwrap_or_else(|| default_plugin_config(SecurityPluginMode::Rewrite));
    let runtime = plugin_runtime_status(&state, &scope.profile_id, "credential_broker", config).await;
    Ok(Json(CredentialBrokerDetailResponse {
        scope,
        plugin_id: "credential_broker",
        store: capsem_core::credential_broker::credential_store_status(),
        inventory: runtime.brokered_credentials,
        grants: CredentialBrokerGrantStatus {
            profile_enabled: config.mode != SecurityPluginMode::Disable,
            vm_grants: Vec::new(),
            fork_default: CredentialBrokerForkGrantDefault::InheritProfile,
        },
        corp_constraints: Vec::new(),
    }))
}

pub(super) async fn handle_profile_credential_broker_credentials_reload(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<CredentialBrokerDetailResponse>, AppError> {
    let profile_id = validate_profile_route_id(profile_id)?;
    match capsem_core::credential_broker::hydrate_credential_runtime_cache_from_durable_store() {
        Ok(count) => info!(
            component = "credential_store",
            profile_id = profile_id.as_str(),
            loaded_count = count,
            status = "ready",
            "credential store retry hydrated runtime cache"
        ),
        Err(error) => warn!(
            component = "credential_store",
            profile_id = profile_id.as_str(),
            error = %error,
            status = "degraded",
            "credential store retry failed"
        ),
    }
    for (vm_id, _) in profile_session_dirs(&state, &profile_id) {
        state.unregister_session_db_handle(&vm_id);
    }
    handle_profile_credential_broker_credentials_info(State(state), Path(profile_id)).await
}

pub(super) async fn list_plugins_for_scope(
    state: &Arc<ServiceState>,
    scope: PluginScope,
) -> Result<PluginListResponse, AppError> {
    let mut plugins = Vec::new();
    for plugin_id in plugin_catalog().keys() {
        plugins.push(plugin_info_for(state, plugin_id, scope.clone(), false).await?);
    }
    Ok(PluginListResponse { scope, plugins })
}

pub(super) async fn handle_profile_plugin_info(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, plugin_id)): Path<(String, String)>,
) -> Result<Json<PluginInfo>, AppError> {
    Ok(Json(
        plugin_info_for(&state, &plugin_id, profile_plugin_scope(&state, profile_id)?, true).await?,
    ))
}

pub(super) async fn handle_profile_plugin_update(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, plugin_id)): Path<(String, String)>,
    Json(update): Json<PluginUpdate>,
) -> Result<Json<PluginInfo>, AppError> {
    let scope = profile_plugin_scope(&state, profile_id)?;
    let catalog = plugin_catalog();
    let Some(catalog_entry) = catalog.get(&plugin_id).copied() else {
        return Err(AppError(StatusCode::NOT_FOUND, format!("unknown plugin: {plugin_id}")));
    };
    let mut config = effective_plugin_policy(&state, &scope.profile_id)
        .get(&plugin_id)
        .copied()
        .unwrap_or(catalog_entry.default_config);
    if let Some(mode) = update.mode {
        config.mode = mode;
    }
    if let Some(detection_level) = update.detection_level {
        config.detection_level = detection_level;
    }

    let mut profile = profile_for_route(scope.profile_id.clone())?;
    let event = write_profile_mutation_event(
        &state,
        profile
            .set_plugin_config(&plugin_id, config, "service-api")
            .map_err(|error| AppError(StatusCode::BAD_REQUEST, error))?,
    )
    .await?;
    log_profile_mutation_applied("profile_plugin_edit", &event);
    state
        .plugin_policy_by_profile
        .lock()
        .unwrap()
        .entry(scope.profile_id.clone())
        .or_default()
        .insert(plugin_id.clone(), config);
    let _reload = handle_reload_config_for_profile(Arc::clone(&state), Some(&scope.profile_id)).await?;
    let info = plugin_info_for(&state, &plugin_id, scope, true).await?;
    Ok(Json(info))
}

#[cfg(test)]
pub(super) async fn update_plugin_for_scope(
    state: &Arc<ServiceState>,
    plugin_id: String,
    scope: PluginScope,
    update: PluginUpdate,
) -> Result<Json<PluginInfo>, AppError> {
    let catalog = plugin_catalog();
    let Some(catalog_entry) = catalog.get(&plugin_id).copied() else {
        return Err(AppError(StatusCode::NOT_FOUND, format!("unknown plugin: {plugin_id}")));
    };
    let mut config = effective_plugin_policy(state, &scope.profile_id)
        .get(&plugin_id)
        .copied()
        .unwrap_or(catalog_entry.default_config);
    if let Some(mode) = update.mode {
        config.mode = mode;
    }
    if let Some(detection_level) = update.detection_level {
        config.detection_level = detection_level;
    }
    state
        .plugin_policy_by_profile
        .lock()
        .unwrap()
        .entry(scope.profile_id.clone())
        .or_default()
        .insert(plugin_id.clone(), config);
    Ok(Json(plugin_info_for(state, &plugin_id, scope, false).await?))
}

#[derive(Debug, Default)]
pub(super) struct ServiceEvaluateEmitter;

impl SecurityEventEmitter for ServiceEvaluateEmitter {
    fn emit(&self, _event: SecurityEvent) -> Result<(), SecurityEmitError> {
        Ok(())
    }
}

pub(super) async fn handle_enforcement_evaluate(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
    body: Bytes,
) -> Result<axum::response::Response, AppError> {
    let profile_id = validate_profile_route_id_from_state(&state, profile_id)?;
    let request_body = body;
    if let Some(response_body) = state
        .evaluate_last_response_cache
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|cached| {
            (cached.profile_id == profile_id && cached.request_body == request_body)
                .then(|| cached.response_body.clone())
        })
    {
        return Ok(json_bytes_response(response_body));
    }
    let mut response_cache_key = Vec::with_capacity(profile_id.len() + request_body.len() + 1);
    response_cache_key.extend_from_slice(profile_id.as_bytes());
    response_cache_key.push(0);
    response_cache_key.extend_from_slice(&request_body);
    if let Some(response_body) = state
        .evaluate_response_cache
        .lock()
        .unwrap()
        .get(&response_cache_key)
        .cloned()
    {
        *state.evaluate_last_response_cache.lock().unwrap() = Some(CachedEvaluateResponse {
            profile_id,
            request_body,
            response_body: response_body.clone(),
        });
        return Ok(json_bytes_response(response_body));
    }
    let request: EnforcementEvaluateRequest = serde_json::from_slice(&request_body).map_err(|error| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("invalid enforcement evaluation request: {error}"),
        )
    })?;
    let policy = effective_plugin_policy(&state, &profile_id);
    let cached_rule_set = {
        state
            .evaluate_rule_cache
            .lock()
            .unwrap()
            .get(&request.rules_toml)
            .cloned()
    };
    let rule_set = if let Some(rule_set) = cached_rule_set {
        rule_set
    } else {
        let profile = SecurityRuleProfile::parse_toml(&request.rules_toml)
            .map_err(|error| AppError(StatusCode::BAD_REQUEST, format!("invalid enforcement rules: {error}")))?;
        let rules = SecurityRuleProfile::compile(&profile, SecurityRuleSource::User)
            .map_err(|error| AppError(StatusCode::BAD_REQUEST, format!("invalid enforcement rules: {error}")))?;
        let rule_set = SecurityRuleSet::new(rules);
        state
            .evaluate_rule_cache
            .lock()
            .unwrap()
            .insert(request.rules_toml.clone(), rule_set.clone());
        rule_set
    };
    let event = request.event.into_security_event()?;
    let engine = SecurityEventEngine::new(
        SecurityActionRegistry::with_builtin_actions().with_plugin_policy(policy),
        Arc::new(ServiceEvaluateEmitter),
    );
    let event = engine
        .apply_matching_rules_and_emit(&rule_set, event)
        .map_err(|error| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("enforcement evaluation failed: {error}"),
            )
        })?;
    let response = EnforcementEvaluateResponse {
        event: event.serializable(),
    };
    let body = serde_json::to_vec(&response).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize enforcement evaluation response: {error}"),
        )
    })?;
    state
        .evaluate_response_cache
        .lock()
        .unwrap()
        .insert(response_cache_key, Bytes::from(body.clone()));
    *state.evaluate_last_response_cache.lock().unwrap() = Some(CachedEvaluateResponse {
        profile_id,
        request_body,
        response_body: Bytes::from(body.clone()),
    });
    Ok(json_bytes_response(Bytes::from(body)))
}

pub(super) async fn handle_detection_evaluate(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
    body: Bytes,
) -> Result<axum::response::Response, AppError> {
    handle_enforcement_evaluate(State(state), Path(profile_id), body).await
}

pub(super) fn enforcement_rule_source(source: SecurityRuleSource) -> api::EnforcementRuleSource {
    match source {
        SecurityRuleSource::BuiltinDefault => api::EnforcementRuleSource::BuiltinDefault,
        SecurityRuleSource::User => api::EnforcementRuleSource::Profile,
        SecurityRuleSource::Corp => api::EnforcementRuleSource::Corp,
    }
}

pub(super) fn enforcement_rule_source_str(source: api::EnforcementRuleSource) -> &'static str {
    match source {
        api::EnforcementRuleSource::BuiltinDefault => "builtin_default",
        api::EnforcementRuleSource::Profile => "profile",
        api::EnforcementRuleSource::Corp => "corp",
    }
}

pub(super) fn enforcement_rule_info(
    source: SecurityRuleSource,
    rule: CompiledSecurityRule,
) -> api::EnforcementRuleInfo {
    api::EnforcementRuleInfo {
        rule_id: rule.rule_id,
        source: enforcement_rule_source(source),
        provider: rule.provider,
        namespace: rule.namespace,
        rule_key: rule.rule_key,
        default_rule: rule.default_rule,
        enabled: rule.enabled,
        name: rule.name,
        action: rule.action,
        condition: rule.condition,
        detection_level: rule.detection_level,
        priority: rule.priority,
        corp_locked: rule.corp_locked,
        reason: rule.reason,
    }
}

pub(super) fn append_compiled_rules(
    output: &mut Vec<api::EnforcementRuleInfo>,
    source: SecurityRuleSource,
    profile: SecurityRuleProfile,
) -> Result<(), AppError> {
    let mut rules = profile
        .compile(source)
        .map_err(|error| AppError(StatusCode::BAD_REQUEST, format!("invalid enforcement rules: {error}")))?;
    output.extend(rules.drain(..).map(|rule| enforcement_rule_info(source, rule)));
    Ok(())
}

#[cfg(test)]
pub(super) fn profile_security_rule_profile_for_route(profile_id: &str) -> Result<SecurityRuleProfile, AppError> {
    let profile = profile_for_route(profile_id.to_string())?;
    profile_security_rule_profile_for_config(&profile)
}

pub(super) fn profile_security_rule_profile_for_config(profile: &Profile) -> Result<SecurityRuleProfile, AppError> {
    let profile_id = profile.config().id.clone();
    profile
        .config()
        .security_rule_profile_from_files(profile.config_root())
        .map_err(|error| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("invalid profile rule files for {profile_id}: {error}"),
            )
        })
}

pub(super) fn list_enforcement_rules_for_profile_config(
    profile: &Profile,
    corp: &SettingsFile,
) -> Result<Vec<api::EnforcementRuleInfo>, AppError> {
    let mut rules = Vec::new();
    append_compiled_rules(
        &mut rules,
        SecurityRuleSource::BuiltinDefault,
        ProviderRuleProfile::builtin_security_defaults(),
    )?;
    let profile_rules = profile_security_rule_profile_for_config(profile)?;
    append_compiled_rules(&mut rules, SecurityRuleSource::User, profile_rules)?;
    append_compiled_rules(
        &mut rules,
        SecurityRuleSource::Corp,
        SecurityRuleProfile {
            corp: corp.corp.clone(),
            profiles: corp.profiles.clone(),
            ai: corp.ai.clone(),
            ..SecurityRuleProfile::default()
        },
    )?;
    rules.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.rule_id.cmp(&right.rule_id))
    });
    Ok(rules)
}

pub(super) fn enforcement_info_for_rules(
    profile_id: String,
    rules: &[api::EnforcementRuleInfo],
) -> api::EnforcementInfoResponse {
    let mut source_counts = BTreeMap::new();
    let mut action_counts = BTreeMap::new();
    for rule in rules {
        *source_counts
            .entry(enforcement_rule_source_str(rule.source).to_string())
            .or_insert(0) += 1;
        *action_counts.entry(rule.action.as_str().to_string()).or_insert(0) += 1;
    }
    api::EnforcementInfoResponse {
        profile_id,
        rule_count: rules.len(),
        default_rule_count: rules.iter().filter(|rule| rule.default_rule).count(),
        custom_rule_count: rules.iter().filter(|rule| !rule.default_rule).count(),
        detection_rule_count: rules.iter().filter(|rule| rule.detection_level.is_some()).count(),
        corp_locked_rule_count: rules.iter().filter(|rule| rule.corp_locked).count(),
        source_counts,
        action_counts,
    }
}

pub(super) fn cached_rules_for_profile(
    state: &ServiceState,
    profile_id: &str,
) -> Result<Vec<api::EnforcementRuleInfo>, AppError> {
    if profile_id.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "profile id must not be empty".to_string(),
        ));
    }
    state
        .profile_rule_cache
        .lock()
        .unwrap()
        .get(profile_id)
        .cloned()
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("profile not found: {profile_id}")))
}

pub(super) async fn handle_enforcement_info(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<api::EnforcementInfoResponse>, AppError> {
    let rules = cached_rules_for_profile(&state, &profile_id)?;
    Ok(Json(enforcement_info_for_rules(profile_id, &rules)))
}

pub(super) async fn handle_detection_info(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<api::DetectionInfoResponse>, AppError> {
    let rules = cached_rules_for_profile(&state, &profile_id)?
        .into_iter()
        .filter(|rule| rule.detection_level.is_some())
        .collect::<Vec<_>>();
    Ok(Json(enforcement_info_for_rules(profile_id, &rules)))
}

pub(super) async fn handle_enforcement_rules_list(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let cache_key = format!("enforcement_rules:{profile_id}");
    if let Some(body) = state
        .profile_rule_response_cache
        .lock()
        .unwrap()
        .get(&cache_key)
        .cloned()
    {
        return Ok(json_bytes_response(body));
    }
    let response = api::EnforcementRuleListResponse {
        rules: cached_rules_for_profile(&state, &profile_id)?,
        profile_id: profile_id.clone(),
    };
    let body = serde_json::to_vec(&response).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize enforcement rules response: {error}"),
        )
    })?;
    state
        .profile_rule_response_cache
        .lock()
        .unwrap()
        .insert(cache_key, Bytes::from(body.clone()));
    Ok(json_bytes_response(Bytes::from(body)))
}

pub(super) async fn handle_detection_rules_list(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let cache_key = format!("detection_rules:{profile_id}");
    if let Some(body) = state
        .profile_rule_response_cache
        .lock()
        .unwrap()
        .get(&cache_key)
        .cloned()
    {
        return Ok(json_bytes_response(body));
    }
    let response = api::DetectionRuleListResponse {
        rules: cached_rules_for_profile(&state, &profile_id)?
            .into_iter()
            .filter(|rule| rule.detection_level.is_some())
            .collect(),
        profile_id: profile_id.clone(),
    };
    let body = serde_json::to_vec(&response).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize detection rules response: {error}"),
        )
    })?;
    state
        .profile_rule_response_cache
        .lock()
        .unwrap()
        .insert(cache_key, Bytes::from(body.clone()));
    Ok(json_bytes_response(Bytes::from(body)))
}

pub(super) async fn handle_enforcement_rule_upsert(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, rule_id)): Path<(String, String)>,
    Json(rule): Json<SecurityRule>,
) -> Result<Json<EnforcementRuleResponse>, AppError> {
    log_profile_mutation_route_request("enforcement_rule_upsert", &profile_id, "rule", &rule_id, "upsert");
    if rule.corp_locked {
        log_profile_mutation_route_rejected(
            "enforcement_rule_upsert",
            &profile_id,
            "rule",
            &rule_id,
            "upsert",
            "enforcement rule endpoint writes user profile rules only; corp_locked rules must come from corp config",
        );
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "enforcement rule endpoint writes user profile rules only; corp_locked rules must come from corp config"
                .to_string(),
        ));
    }
    let compiled = validate_single_user_profile_rule(&rule_id, &rule).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "enforcement_rule_upsert",
            &profile_id,
            "rule",
            &rule_id,
            "upsert",
            &error.1,
        );
    })?;
    let mut profile = profile_for_route(profile_id.clone()).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "enforcement_rule_upsert",
            &profile_id,
            "rule",
            &rule_id,
            "upsert",
            &error.1,
        );
    })?;
    let summary = profile
        .upsert_profile_rule(&rule_id, rule.clone(), "service-api")
        .map_err(|error| {
            log_profile_mutation_route_rejected(
                "enforcement_rule_upsert",
                &profile_id,
                "rule",
                &rule_id,
                "upsert",
                &error,
            );
            AppError(StatusCode::BAD_REQUEST, error)
        })?;
    let event = write_profile_mutation_event(&state, summary).await?;
    state.refresh_profile_rule_cache_off_worker(profile_id.clone()).await?;
    log_profile_mutation_applied("enforcement_rule_upsert", &event);
    Ok(Json(EnforcementRuleResponse {
        rule_id,
        compiled_rule_id: compiled.rule_id,
        rule,
    }))
}

pub(super) async fn handle_detection_rule_upsert(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, rule_id)): Path<(String, String)>,
    Json(rule): Json<SecurityRule>,
) -> Result<Json<EnforcementRuleResponse>, AppError> {
    log_profile_mutation_route_request("detection_rule_upsert", &profile_id, "rule", &rule_id, "upsert");
    if rule.detection_level.is_none() {
        log_profile_mutation_route_rejected(
            "detection_rule_upsert",
            &profile_id,
            "rule",
            &rule_id,
            "upsert",
            "detection rule endpoint requires detection_level",
        );
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "detection rule endpoint requires detection_level".to_string(),
        ));
    }
    if rule.corp_locked {
        log_profile_mutation_route_rejected(
            "detection_rule_upsert",
            &profile_id,
            "rule",
            &rule_id,
            "upsert",
            "detection rule endpoint writes user profile rules only; corp_locked rules must come from corp config",
        );
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "detection rule endpoint writes user profile rules only; corp_locked rules must come from corp config"
                .to_string(),
        ));
    }
    let compiled = validate_single_user_profile_rule(&rule_id, &rule).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "detection_rule_upsert",
            &profile_id,
            "rule",
            &rule_id,
            "upsert",
            &error.1,
        );
    })?;
    let mut profile = profile_for_route(profile_id.clone()).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "detection_rule_upsert",
            &profile_id,
            "rule",
            &rule_id,
            "upsert",
            &error.1,
        );
    })?;
    let summary = profile
        .upsert_profile_rule(&rule_id, rule.clone(), "service-api")
        .map_err(|error| {
            log_profile_mutation_route_rejected(
                "detection_rule_upsert",
                &profile_id,
                "rule",
                &rule_id,
                "upsert",
                &error,
            );
            AppError(StatusCode::BAD_REQUEST, error)
        })?;
    let event = write_profile_mutation_event(&state, summary).await?;
    state.refresh_profile_rule_cache_off_worker(profile_id.clone()).await?;
    log_profile_mutation_applied("detection_rule_upsert", &event);
    Ok(Json(EnforcementRuleResponse {
        rule_id,
        compiled_rule_id: compiled.rule_id,
        rule,
    }))
}

pub(super) async fn handle_enforcement_rule_delete(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, rule_id)): Path<(String, String)>,
) -> Result<Json<EnforcementRuleDeleteResponse>, AppError> {
    log_profile_mutation_route_request("enforcement_rule_delete", &profile_id, "rule", &rule_id, "delete");
    let mut profile = profile_for_route(profile_id.clone()).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "enforcement_rule_delete",
            &profile_id,
            "rule",
            &rule_id,
            "delete",
            &error.1,
        );
    })?;
    let summary = profile.delete_profile_rule(&rule_id, "service-api").map_err(|error| {
        let status = if error.contains("not found") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        };
        log_profile_mutation_route_rejected(
            "enforcement_rule_delete",
            &profile_id,
            "rule",
            &rule_id,
            "delete",
            &error,
        );
        AppError(status, error)
    })?;
    let event = write_profile_mutation_event(&state, summary).await?;
    state.refresh_profile_rule_cache_off_worker(profile_id.clone()).await?;
    log_profile_mutation_applied("enforcement_rule_delete", &event);
    Ok(Json(EnforcementRuleDeleteResponse { rule_id, deleted: true }))
}

pub(super) async fn handle_detection_rule_delete(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, rule_id)): Path<(String, String)>,
) -> Result<Json<EnforcementRuleDeleteResponse>, AppError> {
    log_profile_mutation_route_request("detection_rule_delete", &profile_id, "rule", &rule_id, "delete");
    let mut profile = profile_for_route(profile_id.clone()).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "detection_rule_delete",
            &profile_id,
            "rule",
            &rule_id,
            "delete",
            &error.1,
        );
    })?;
    let summary = profile.delete_profile_rule(&rule_id, "service-api").map_err(|error| {
        let status = if error.contains("not found") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        };
        log_profile_mutation_route_rejected("detection_rule_delete", &profile_id, "rule", &rule_id, "delete", &error);
        AppError(status, error)
    })?;
    let event = write_profile_mutation_event(&state, summary).await?;
    state.refresh_profile_rule_cache_off_worker(profile_id.clone()).await?;
    log_profile_mutation_applied("detection_rule_delete", &event);
    Ok(Json(EnforcementRuleDeleteResponse { rule_id, deleted: true }))
}

pub(super) async fn handle_enforcement_reload(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _profile_id = validate_profile_route_id(profile_id)?;
    handle_reload_config(State(state)).await
}

pub(super) async fn handle_detection_reload(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    handle_enforcement_reload(State(state), Path(profile_id)).await
}

pub(super) fn validate_single_user_profile_rule(
    rule_id: &str,
    rule: &SecurityRule,
) -> Result<capsem_core::net::policy_config::CompiledSecurityRule, AppError> {
    let profile = SecurityRuleProfile {
        profiles: SecurityRuleGroup {
            rules: BTreeMap::from([(rule_id.to_string(), rule.clone())]),
        },
        ..SecurityRuleProfile::default()
    };
    let mut compiled = profile
        .compile(SecurityRuleSource::User)
        .map_err(|error| AppError(StatusCode::BAD_REQUEST, format!("invalid enforcement rule: {error}")))?;
    compiled.pop().ok_or_else(|| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "valid enforcement rule did not compile".to_string(),
        )
    })
}

impl EnforcementEventInput {
    fn into_security_event(self) -> Result<SecurityEvent, AppError> {
        let event_type = match self.event_type.as_str() {
            "http.request" => RuntimeSecurityEventType::HttpRequest,
            "dns.query" => RuntimeSecurityEventType::DnsQuery,
            "mcp.tool_call" => RuntimeSecurityEventType::McpToolCall,
            "mcp.tool_list" => RuntimeSecurityEventType::McpToolList,
            "mcp.event" => RuntimeSecurityEventType::McpEvent,
            "model.call" => RuntimeSecurityEventType::ModelCall,
            "file.event" => RuntimeSecurityEventType::FileEvent,
            "file.import" => RuntimeSecurityEventType::FileImport,
            "file.export" => RuntimeSecurityEventType::FileExport,
            "process.exec" => RuntimeSecurityEventType::ProcessExec,
            "process.exec_complete" => RuntimeSecurityEventType::ProcessExecComplete,
            "process.audit" => RuntimeSecurityEventType::ProcessAudit,
            other => Err(AppError(
                StatusCode::BAD_REQUEST,
                format!("unsupported enforcement event_type: {other}"),
            ))?,
        };

        let mut event = SecurityEvent::new(event_type);
        if self.http_host.is_some()
            || self.http_method.is_some()
            || self.http_path.is_some()
            || self.http_query.is_some()
            || self.http_status.is_some()
            || self.http_body.is_some()
        {
            event = event.with_http(HttpSecurityEvent {
                host: self.http_host,
                method: self.http_method,
                path: self.http_path,
                query: self.http_query,
                status: self.http_status,
                body: self.http_body,
            });
        }
        if self.dns_qname.is_some() || self.dns_qtype.is_some() {
            event = event.with_dns(DnsSecurityEvent {
                qname: self.dns_qname,
                qtype: self.dns_qtype,
            });
        }
        if self.mcp_method.is_some()
            || self.mcp_server_name.is_some()
            || self.mcp_tool_call_name.is_some()
            || self.mcp_tool_list.is_some()
            || self.mcp_request_preview.is_some()
            || self.mcp_response_preview.is_some()
        {
            let mcp = McpSecurityEvent {
                method: self.mcp_method,
                server_name: self.mcp_server_name,
                tool_call_name: self.mcp_tool_call_name,
                tool_list: self.mcp_tool_list,
                ..Default::default()
            }
            .with_request_preview(self.mcp_request_preview.as_deref())
            .with_response_preview(self.mcp_response_preview.as_deref());
            event = event.with_mcp(mcp);
        }
        if self.model_provider.is_some()
            || self.model_name.is_some()
            || self.model_request_body.is_some()
            || self.model_response_body.is_some()
            || self.model_tool_calls.is_some()
        {
            event = event.with_model(ModelSecurityEvent {
                provider: self.model_provider,
                name: self.model_name,
                request_body: self.model_request_body,
                response_body: self.model_response_body,
                tool_calls: self.model_tool_calls,
            });
        }
        if matches!(
            event_type,
            RuntimeSecurityEventType::FileEvent
                | RuntimeSecurityEventType::FileImport
                | RuntimeSecurityEventType::FileExport
        ) || self.file_import_content.is_some()
            || self.file_path.is_some()
            || self.file_name.is_some()
            || self.file_ext.is_some()
            || self.file_mime_type.is_some()
            || self.file_content.is_some()
        {
            let mut file = FileSecurityEvent::default();
            match event_type {
                RuntimeSecurityEventType::FileImport => {
                    file.import_path = self.file_path;
                    file.import_name = self.file_name;
                    file.import_ext = self.file_ext;
                    file.import_mime_type = self.file_mime_type;
                    file.import_content = self.file_import_content.or(self.file_content);
                }
                RuntimeSecurityEventType::FileExport => {
                    file.export_path = self.file_path;
                    file.export_name = self.file_name;
                    file.export_ext = self.file_ext;
                    file.export_mime_type = self.file_mime_type;
                    file.export_content = self.file_content;
                }
                _ => {
                    file.content = self.file_content.or(self.file_import_content);
                    file.read_path = self.file_path;
                    file.read_name = self.file_name;
                    file.read_ext = self.file_ext;
                    file.read_mime_type = self.file_mime_type;
                }
            }
            event = event.with_file(file);
        }
        if self.process_exec_id.is_some()
            || self.process_exec_path.is_some()
            || self.process_command.is_some()
            || self.process_exit_code.is_some()
            || self.process_stdout.is_some()
            || self.process_stderr.is_some()
        {
            event = event.with_process(ProcessSecurityEvent {
                exec_id: self.process_exec_id,
                exec_path: self.process_exec_path,
                name: None,
                command: self.process_command,
                exit_code: self.process_exit_code,
                stdout: self.process_stdout,
                stderr: self.process_stderr,
            });
        }
        if self.ip_value.is_some() || self.ip_version.is_some() {
            event = event.with_ip(IpSecurityEvent {
                value: self.ip_value,
                version: self.ip_version,
            });
        }
        if self.tcp_port.is_some() {
            event = event.with_tcp(TcpSecurityEvent { port: self.tcp_port });
        }
        if self.udp_port.is_some() {
            event = event.with_udp(UdpSecurityEvent { port: self.udp_port });
        }
        Ok(event)
    }
}

#[derive(Deserialize, Debug, Default)]
pub(super) struct TimelineQuery {
    /// Filter to one trace_id. Rows with NULL trace_id are also returned
    /// (they pre-date W4's trace propagation).
    trace_id: Option<String>,
    /// Lookback window. "30m", "1h", "24h", "7d", "300s", or RFC3339.
    since: Option<String>,
    /// Max rows. Default 200, capped at 2000.
    limit: Option<usize>,
    /// Comma-separated subset of layers to include. Default all:
    /// "exec,mcp,net,fs,model".
    layers: Option<String>,
}

pub(super) fn secs_to_rfc3339(secs: u64) -> String {
    // Pure-stdlib RFC3339 (UTC, second precision). Mirrors the helper in
    // the support_bundle crate; we pay the duplication tax to keep
    // capsem-service free of `chrono`.
    let secs = secs as i64;
    let days = secs.div_euclid(86400);
    let secs_in_day = secs.rem_euclid(86400);
    let hh = (secs_in_day / 3600) as u32;
    let mm = ((secs_in_day % 3600) / 60) as u32;
    let ss = (secs_in_day % 60) as u32;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = i64::from(yoe) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}
