use super::*;

/// Unified recorded activity. DbHandle owns query execution and caching.
pub(crate) async fn handle_timeline(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<api::TimelineQuery>,
) -> Result<Json<api::TimelineResponse>, AppError> {
    use api::TimelineLayer::{Exec, Fs, Model, Net, Tool};
    let layers = params.layers.unwrap_or_else(|| vec![Exec, Tool, Net, Fs, Model]);
    if layers.is_empty() {
        return Err(AppError(StatusCode::BAD_REQUEST, "no layers selected".into()));
    }
    let cutoff = params
        .since
        .as_deref()
        .map(|since| {
            triage::parse_since(since)
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| secs_to_rfc3339(duration.as_secs()))
                .ok_or_else(|| {
                    AppError(
                        StatusCode::BAD_REQUEST,
                        "since must be a duration or RFC3339 timestamp".into(),
                    )
                })
        })
        .transpose()?;
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    let db = open_ready_session_db(&state, &id, "timeline", &db_path).await?;
    let rows = query_route_typed_rows::<api::TimelineEvent>(
        &id,
        "timeline",
        "events",
        &db_path,
        &db,
        &timeline_base_sql(),
        &[],
    )
    .await?;
    let events = rows
        .into_iter()
        .filter(|event| layers.contains(&event.layer))
        .filter(|event| {
            params
                .trace_id
                .as_deref()
                .is_none_or(|trace_id| event.trace_id.as_deref() == Some(trace_id) || event.trace_id.is_none())
        })
        .filter(|event| {
            cutoff
                .as_deref()
                .is_none_or(|cutoff| event.timestamp.as_str() >= cutoff)
        })
        .take(params.limit.unwrap_or(200).min(2000))
        .collect();
    Ok(Json(api::TimelineResponse { events }))
}

const TIMELINE_RECOVERY_LIMIT: usize = 50_000;

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
