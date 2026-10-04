use super::*;
pub(crate) mod activity;
pub(crate) mod bodies;
pub(super) use bodies::{handle_bodies_warc_export, handle_event_bodies};
mod response_cache;
mod rows;
pub(crate) use response_cache::{forget_session_responses, session_response_cache_lookup, SessionResponseCache};
use rows::{query_route_objects, query_route_typed_rows, route_query_objects};
pub(crate) mod stats_detail;
pub(super) use stats_detail::read_stats_detail_payload_from_session_db;
pub(crate) mod history;
pub(crate) mod timeline;
pub(super) use timeline::handle_timeline;
mod vm_info;
pub(super) use vm_info::populate_vm_info;

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
    let slot = match session_response_cache_lookup(&state, &id, &route_key, "security", &db_path).await? {
        SessionResponseCache::Hit(body) => return Ok(json_bytes_response(body)),
        SessionResponseCache::Miss(slot) => slot,
    };
    let rows = security_latest_for_vm(&state, &id, limit, false).await?;
    tracing::debug!(
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
    slot.store(&state, &body);
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
    let slot = match session_response_cache_lookup(&state, &id, &route_key, "security", &db_path).await? {
        SessionResponseCache::Hit(body) => return Ok(json_bytes_response(body)),
        SessionResponseCache::Miss(slot) => slot,
    };
    let rows = security_latest_for_vm(&state, &id, limit, true).await?;
    let body = serde_json::to_vec(&rows).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize detection latest response: {error}"),
        )
    })?;
    slot.store(&state, &body);
    Ok(json_bytes_response(Bytes::from(body)))
}

/// GET /vms/{id}/security/status -- security rule ledger aggregates.
///
/// Polled: answered from the ledger handle's counter snapshot, in memory.
pub(super) async fn handle_security_info(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<capsem_logger::SecurityRuleStats>, AppError> {
    Ok(Json(security_stats_for_vm(&state, &id).await?))
}

/// Every session the service knows, keyed by session id.
///
/// Registry maps are keyed by display name; a running persistent VM is also
/// in `instances` under its session id, and only that id lets the two sources
/// collapse into one row.
pub(super) fn service_session_dirs(state: &ServiceState) -> Vec<(String, PathBuf)> {
    let mut sessions = BTreeMap::new();
    {
        let instances = state.instances.lock().unwrap();
        for (id, info) in instances.iter() {
            sessions.insert(id.clone(), info.session_dir.clone());
        }
    }
    {
        let registry = state.persistent_registry.lock().unwrap();
        for entry in registry.list() {
            sessions
                .entry(persistent_entry_vm_id(entry))
                .or_insert_with(|| entry.session_dir.clone());
        }
    }
    sessions.into_iter().collect()
}

pub(crate) mod security;
pub(crate) use security::{
    is_detection_rule_event, read_security_session_ledger, security_latest_for_vm, security_stats_for_session,
    security_stats_for_vm,
};

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

/// The session's ready ledger handle, for a route that reads rows.
pub(super) async fn open_ready_session_db(
    state: &ServiceState,
    vm_id: &str,
    ledger: &str,
    db_path: &StdPath,
) -> Result<Arc<capsem_logger::DbHandle>, AppError> {
    let db = session_db(state, vm_id, ledger, db_path).await?;
    db.ready()
        .await
        .map_err(|error| ledger_route_error(vm_id, ledger, "ready", db_path, error))?;
    Ok(db)
}

/// The session's registered ledger handle, opened once and reused.
///
/// A polled route that reads the counter snapshot takes it from here and asks
/// the handle's memory, never the file: see `DbHandle::ledger_counters`.
pub(super) async fn session_db(
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
                .register_session_db_handle_async(vm_id, session_dir)
                .await
                .map_err(|error| ledger_route_error(vm_id, ledger, "open", db_path, error))?
        }
        None => {
            let session_dir = db_path
                .parent()
                .ok_or_else(|| ledger_route_error(vm_id, ledger, "resolve session dir", db_path, "missing parent"))?;
            let handle = state
                .register_session_db_handle_async(vm_id, session_dir)
                .await
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
    Ok(db)
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
        let stats = security_stats_for_session(&state, &vm_id, &session_dir.join("session.db")).await?;
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
        let stats = security_stats_for_session(&state, &vm_id, &session_dir.join("session.db")).await?;
        let count = security_detection_count(&stats);
        total += count;
        sessions.push(json!({ "vm_id": vm_id, "total": count }));
    }
    Ok(Json(json!({ "total": total, "sessions": sessions })))
}
