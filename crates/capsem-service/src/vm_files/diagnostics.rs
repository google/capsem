//! Log, panic, and triage routes: the read-only diagnostics a session exposes.

use super::*;

pub(crate) async fn handle_logs(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<LogsResponse>, AppError> {
    let known = {
        let instances = state.instances.lock().unwrap();
        match instances.get(&id) {
            Some(i) => Some(i.session_dir.clone()),
            None => find_persistent_entry_by_route_id(&state, &id).map(|e| e.session_dir),
        }
    };
    let session_dir = match known {
        Some(dir) => dir,
        None => {
            // VM might have crashed on boot. preserve_failed_session_dir
            // renames `sessions/<id>` to `sessions/<id>-failed-<suffix>`,
            // so the most recent `<id>-failed-*` still has the logs the
            // user needs to debug the crash. Without this branch
            // `capsem logs <id>` just returns 404 after a boot failure,
            // which is exactly when logs matter most. The lookup lists
            // sessions/, so it runs off the worker.
            let failed_id = id.clone();
            state
                .off_worker(move |state| find_failed_session_dir(&state.run_dir, &failed_id))
                .await?
                .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?
        }
    };

    let serial_log_path = session_dir.join("serial.log");
    let process_log_path = session_dir.join("process.log");

    // Bounded and rotation-aware. `serial.log` is guest-controlled console
    // output written through `CappedLogWriter`, so it both rotates and can be
    // arbitrarily large -- reading the whole bare file lost the rotated slice
    // and let the guest choose the allocation.
    let (serial_logs, process_logs) = tokio::task::spawn_blocking(move || {
        let serial = capsem_foundation::telemetry::read_log_tail(&serial_log_path, SESSION_LOG_TAIL_MAX_BYTES);
        let process = capsem_foundation::telemetry::read_log_tail(&process_log_path, SESSION_LOG_TAIL_MAX_BYTES);
        (serial, process)
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("log read failed: {e}")))?;

    Ok(Json(LogsResponse {
        logs: serial_logs.as_deref().unwrap_or("").to_string(),
        serial_logs,
        process_logs,
    }))
}

/// `GET /panics?since=30m&limit=20` -- structured panic + backtrace
/// extractor across all host log files. Returns JSON array. Used by the
/// `capsem_panics` MCP tool.
pub(crate) async fn handle_panics(
    State(state): State<Arc<ServiceState>>,
    axum::extract::Query(params): axum::extract::Query<TriageQuery>,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    let since_unix = params
        .since
        .as_deref()
        .and_then(triage::parse_since)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let limit = params.limit.unwrap_or(20).min(200);

    // Every host log is read in full; off the worker.
    let mut all_panics = state
        .off_worker(move |state| {
            let home = capsem_foundation::paths::capsem_home();
            let mut all_panics: Vec<triage::PanicEvent> = Vec::new();
            for binary in ["service", "mcp", "gateway", "tray"] {
                if let Some(path) = triage::host_log_path(&state.run_dir, binary) {
                    all_panics.extend(triage::scan_panics_in_file(
                        &path,
                        &format!("capsem-{binary}"),
                        since_unix,
                    ));
                }
            }
            if let Some(path) = triage::latest_app_log(&home) {
                all_panics.extend(triage::scan_panics_in_file(&path, "capsem-app", since_unix));
            }
            all_panics
        })
        .await?;

    all_panics.truncate(limit);
    Ok(axum::Json(serde_json::json!({ "panics": all_panics })))
}

/// `GET /triage?id=<vm>&since=30m&limit=20` -- ranked summary of recent
/// panics, errors, and slow ops across host logs (and, when `id` is
/// provided, session.db error rows). Used by the `capsem_triage` MCP
/// tool.
pub(crate) async fn handle_triage(
    State(state): State<Arc<ServiceState>>,
    axum::extract::Query(params): axum::extract::Query<TriageQuery>,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    let since_str = params.since.clone().unwrap_or_else(|| "30m".to_string());
    let since_unix = triage::parse_since(&since_str)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let limit = params.limit.unwrap_or(20).min(200);

    // Every host log is read in full; off the worker.
    let (mut panics, mut errors, mut slow_ops) = state
        .off_worker(move |state| {
            let home = capsem_foundation::paths::capsem_home();
            let mut panics: Vec<triage::PanicEvent> = Vec::new();
            let mut errors: Vec<triage::ErrorEvent> = Vec::new();
            let mut slow_ops: Vec<triage::SlowOpEvent> = Vec::new();
            for binary in ["service", "mcp", "gateway", "tray"] {
                if let Some(path) = triage::host_log_path(&state.run_dir, binary) {
                    let bin_label = format!("capsem-{binary}");
                    panics.extend(triage::scan_panics_in_file(&path, &bin_label, since_unix));
                    errors.extend(triage::scan_errors_in_file(&path, &bin_label, since_unix, limit));
                    slow_ops.extend(triage::scan_slow_ops_in_file(&path, &bin_label, since_unix, 500));
                }
            }
            if let Some(path) = triage::latest_app_log(&home) {
                panics.extend(triage::scan_panics_in_file(&path, "capsem-app", since_unix));
                errors.extend(triage::scan_errors_in_file(&path, "capsem-app", since_unix, limit));
            }
            (panics, errors, slow_ops)
        })
        .await?;

    panics.truncate(limit);
    errors.truncate(limit);
    slow_ops.truncate(limit);

    // When `id` is set, add session-scoped error signals from the canonical
    // session ledger. The future DB-owned mem layer can make this fast; the
    // service route does not own a separate logged-data copy.
    let session_block = if let Some(ref vm_id) = params.id {
        triage_for_vm(&state, vm_id, limit).await?
    } else {
        serde_json::json!({})
    };

    // Build a deterministic ranked-list of the highest-blast-radius items
    // first: panics > unhandled-enum warns > slow_op events > everything else.
    let mut rank: Vec<String> = Vec::new();
    for p in panics.iter().take(5) {
        rank.push(format!(
            "panic {} in {} at {} -- {}",
            p.ts.as_str().chars().take(19).collect::<String>(),
            p.binary,
            p.location.clone().unwrap_or_else(|| "?".into()),
            p.message.chars().take(120).collect::<String>(),
        ));
    }
    for e in errors.iter().filter(|e| e.target.as_deref() == Some("ipc")).take(3) {
        rank.push(format!(
            "ipc-warn {} in {} -- {}",
            e.ts.as_str().chars().take(19).collect::<String>(),
            e.binary,
            e.message.chars().take(120).collect::<String>(),
        ));
    }
    for s in slow_ops.iter().take(3) {
        rank.push(format!(
            "slow_op {} {} {}ms in {}",
            s.ts.as_str().chars().take(19).collect::<String>(),
            s.op,
            s.duration_ms,
            s.binary,
        ));
    }

    let out = serde_json::json!({
        "since": since_str,
        "session_id": params.id,
        "host": {
            "panics": panics,
            "errors": errors,
            "slow_ops": slow_ops,
        },
        "session": session_block,
        "rank": rank,
    });
    Ok(axum::Json(out))
}

pub(crate) async fn session_db_triage(
    vm_id: &str,
    db: &capsem_logger::DbHandle,
    db_path: &std::path::Path,
    limit: usize,
) -> anyhow::Result<serde_json::Value> {
    db.ready()
        .await
        .map_err(|error| anyhow!("session triage ledger is not ready for {vm_id}: {error}"))?;
    let denied_net_sql = format!(
        "SELECT timestamp, domain, decision, status_code, duration_ms \
         FROM net_events WHERE decision = 'denied' OR status_code >= 500 \
         ORDER BY timestamp DESC LIMIT {limit}"
    );
    let tool_errors_sql = format!(
        "SELECT timestamp, server_name, method, decision, policy_mode, policy_action, \
                policy_rule, policy_reason, error_message, duration_ms \
         FROM tool_calls \
         WHERE origin IN ('native', 'mcp', 'builtin', 'local') \
           AND (decision IN ('denied','error') OR error_message IS NOT NULL) \
         ORDER BY timestamp DESC LIMIT {limit}"
    );
    let exec_failures_sql = format!(
        "SELECT timestamp, exec_id, command, exit_code, duration_ms \
         FROM exec_events WHERE exit_code IS NOT NULL AND exit_code != 0 \
         ORDER BY timestamp DESC LIMIT {limit}"
    );

    async fn read_query(
        db: &capsem_logger::DbHandle,
        vm_id: &str,
        db_path: &std::path::Path,
        query_name: &str,
        sql: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let raw = db.query(sql, &[]).await.map_err(|error| {
            error!(
                vm_id,
                query_name,
                db_path = %db_path.display(),
                error = %error,
                "session triage ledger query failed"
            );
            anyhow!("session triage query {query_name} failed: {error}")
        })?;
        serde_json::from_str(&raw).map_err(|error| {
            error!(
                vm_id,
                query_name,
                db_path = %db_path.display(),
                error = %error,
                "session triage ledger query returned invalid JSON"
            );
            anyhow!("session triage query {query_name} returned invalid JSON: {error}")
        })
    }

    let denied_net_v = read_query(db, vm_id, db_path, "denied_net", &denied_net_sql).await?;
    let tool_errors_v = read_query(db, vm_id, db_path, "tool_errors", &tool_errors_sql).await?;
    let exec_failures_v = read_query(db, vm_id, db_path, "exec_failures", &exec_failures_sql).await?;

    Ok(serde_json::json!({
        "denied_net": denied_net_v,
        "tool_errors": tool_errors_v,
        "exec_failures": exec_failures_v,
    }))
}

pub(crate) fn limit_columnar_query_json(value: &serde_json::Value, limit: usize) -> serde_json::Value {
    let columns = value.get("columns").cloned().unwrap_or_else(|| json!([]));
    let rows = value
        .get("rows")
        .and_then(|value| value.as_array())
        .map(|rows| rows.iter().take(limit).cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    json!({
        "columns": columns,
        "rows": rows,
    })
}

pub(crate) fn limit_triage_session_block(value: &serde_json::Value, limit: usize) -> serde_json::Value {
    json!({
        "denied_net": limit_columnar_query_json(&value["denied_net"], limit),
        "tool_errors": limit_columnar_query_json(&value["tool_errors"], limit),
        "exec_failures": limit_columnar_query_json(&value["exec_failures"], limit),
    })
}

pub(crate) async fn triage_for_vm(
    state: &ServiceState,
    vm_id: &str,
    limit: usize,
) -> Result<serde_json::Value, AppError> {
    let session_dir = match resolve_session_dir(state, vm_id) {
        Ok(session_dir) => session_dir,
        Err(_) => {
            return Ok(json!({ "missing": true, "reason": "session not found" }));
        }
    };
    let db_path = session_db_path_for_session_dir(&session_dir);
    if !db_path.exists() {
        return Ok(json!({ "missing": true, "reason": "session not found" }));
    }
    let db = open_ready_session_db(state, vm_id, "triage", &db_path).await?;
    let session = session_db_triage(vm_id, &db, &db_path, limit).await.map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read triage ledger for {vm_id}: {error}"),
        )
    })?;
    Ok(limit_triage_session_block(&session, limit))
}

#[derive(Deserialize, Debug, Default)]
pub(crate) struct TriageQuery {
    /// Lookback window. Default "30m". Accepts "5m", "1h", "24h", or
    /// RFC3339 ("2026-05-02T17:30:00Z").
    since: Option<String>,
    /// Max items per category. Default 20, capped at 200.
    limit: Option<usize>,
    /// Optional session id (reserved for the future session.db query).
    id: Option<String>,
}

/// `GET /host-logs/{name}?grep=&tail=&max_bytes=` -- read a host-side log
/// file by symbolic name. Hard-coded allowlist (no path traversal). Used
/// by the `capsem_host_logs` MCP tool (T3) but the endpoint already lands
/// in this commit so a future T3 sub-sprint can wire the MCP tool without
/// touching the service.
pub(crate) async fn handle_host_logs(
    State(state): State<Arc<ServiceState>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<HostLogsQuery>,
) -> Result<String, AppError> {
    let path = if name == "app" {
        state
            .off_worker(|_| triage::latest_app_log(&capsem_foundation::paths::capsem_home()))
            .await?
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, "no app log found".into()))?
    } else {
        triage::host_log_path(&state.run_dir, &name)
            .ok_or_else(|| AppError(StatusCode::BAD_REQUEST, format!("unknown log name: {name}")))?
    };
    let max_bytes = params.max_bytes.unwrap_or(100 * 1024).min(5 * 1024 * 1024);
    // `service.log` names a daily-rotated stream, so opening that exact name
    // returns nothing the moment it has rotated -- this endpoint reported an
    // empty log for a service that was writing normally. Reading through the
    // stream reader also removes the fourth hand-rolled copy of seek-from-end
    // and trim-the-partial-line in this crate.
    let text = tokio::task::spawn_blocking(move || {
        capsem_foundation::telemetry::read_log_tail(&path, max_bytes as usize).unwrap_or_default()
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("log read failed: {e}")))?;

    // Apply grep + tail post-filters here so the wire surface to the
    // capsem_host_logs MCP tool can avoid two round-trips.
    let mut text = text;
    if let Some(pat) = &params.grep {
        text = text.lines().filter(|l| l.contains(pat)).collect::<Vec<_>>().join("\n");
    }
    if let Some(n) = params.tail {
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(n);
        text = lines[start..].join("\n");
    }
    Ok(text)
}

#[derive(Deserialize, Debug, Default)]
pub(crate) struct HostLogsQuery {
    grep: Option<String>,
    tail: Option<usize>,
    max_bytes: Option<u64>,
}

pub(crate) async fn handle_service_logs(State(state): State<Arc<ServiceState>>) -> Result<String, AppError> {
    let log_path = state.run_dir.join("service.log");

    let text = tokio::task::spawn_blocking(move || -> Result<String, String> {
        // `service.log` names a daily-rotated stream, not a file. Resolution
        // and tailing live in one place so every consumer sees the same log.
        capsem_foundation::telemetry::read_log_tail(&log_path, 100 * 1024)
            .ok_or_else(|| format!("no log files in stream {}", log_path.display()))
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("log read failed: {e}")))?
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(text)
}
