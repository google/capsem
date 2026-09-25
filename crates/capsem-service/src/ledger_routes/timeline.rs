use api::TimelineLayer;

use super::*;

/// Unified recorded activity, oldest first from `since`.
///
/// Each requested layer reads its own window -- rows at or after the cutoff,
/// in its table's timestamp order, at most `limit` of them -- and the merge
/// keeps the oldest `limit`. The route used to read the oldest rows of the
/// whole ledger and filter them in memory, so on a long session a recent
/// `since` found nothing.
pub(crate) async fn handle_timeline(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<api::TimelineQuery>,
) -> Result<Json<api::TimelineResponse>, AppError> {
    let requested = params.layers.unwrap_or_else(|| LAYERS.to_vec());
    if requested.is_empty() {
        return Err(AppError(StatusCode::BAD_REQUEST, "no layers selected".into()));
    }
    let layers: Vec<TimelineLayer> = LAYERS.into_iter().filter(|layer| requested.contains(layer)).collect();
    let cutoff = params
        .since
        .as_deref()
        .map(|since| {
            triage::parse_since(since)
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| ledger_cutoff(duration.as_secs()))
                .ok_or_else(|| {
                    AppError(
                        StatusCode::BAD_REQUEST,
                        "since must be a duration or RFC3339 timestamp".into(),
                    )
                })
        })
        .transpose()?
        .unwrap_or_default();
    // A tool call that never recorded its time reads as the epoch, and the
    // filter has always compared that displayed time: an epoch cutoff keeps it.
    let tool_cutoff = if cutoff.as_str() <= UNDATED_TOOL_CALL {
        String::new()
    } else {
        cutoff.clone()
    };
    let limit = params.limit.unwrap_or(200).min(2000);
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    let db = open_ready_session_db(&state, &id, "timeline", &db_path).await?;
    let events = query_route_typed_rows::<api::TimelineEvent>(
        &id,
        "timeline",
        "events",
        &db_path,
        &db,
        &timeline_sql(&layers),
        &[
            json!(limit),
            json!(cutoff),
            json!(tool_cutoff),
            params
                .trace_id
                .map_or(serde_json::Value::Null, serde_json::Value::String),
        ],
    )
    .await?;
    Ok(Json(api::TimelineResponse { events }))
}

const LAYERS: [TimelineLayer; 5] = [
    TimelineLayer::Exec,
    TimelineLayer::Tool,
    TimelineLayer::Net,
    TimelineLayer::Fs,
    TimelineLayer::Model,
];

/// How a tool call without a recorded time is displayed and ordered.
const UNDATED_TOOL_CALL: &str = "1970-01-01T00:00:00Z";

/// Rows a layer keeps: at or after the cutoff, and in the requested trace or
/// in none. The cutoff is `''` without `since`, which every timestamp passes.
const EXEC_WINDOW: &str = "SELECT timestamp, 'exec' AS layer, exec_id AS ref, command AS summary, \
     exit_code AS status, duration_ms, trace_id FROM exec_events \
     WHERE timestamp >= ?2 AND (?4 IS NULL OR trace_id = ?4 OR trace_id IS NULL) \
     ORDER BY timestamp ASC LIMIT ?1";

/// The tool window filters and orders on the stored column, which its index
/// serves; an empty one sorts first, where the epoch it displays as would.
/// `+tc.origin` keeps the planner on the timestamp index: nearly every call
/// has one of these origins, so the origin index would only add a sort.
const TOOL_WINDOW: &str = "SELECT COALESCE(NULLIF(tc.timestamp, ''), '1970-01-01T00:00:00Z') AS timestamp, \
     'tool' AS layer, tc.event_id AS ref, \
     COALESCE(tc.server_name, tc.origin) || '/' || tc.tool_name || COALESCE(' (call_id=' || tc.call_id || ')', '') AS summary, \
     tc.decision AS status, tc.duration_ms AS duration_ms, tc.trace_id AS trace_id \
     FROM tool_calls tc \
     WHERE +tc.origin IN ('model', 'native', 'mcp', 'builtin', 'local', 'mcp_proxy') \
     AND tc.timestamp >= ?3 AND (?4 IS NULL OR tc.trace_id = ?4 OR tc.trace_id IS NULL) \
     ORDER BY tc.timestamp ASC LIMIT ?1";

const NET_WINDOW: &str = "SELECT timestamp, 'net' AS layer, id AS ref, \
     COALESCE(method, 'GET') || ' ' || domain || COALESCE(path, '') AS summary, \
     status_code AS status, duration_ms, trace_id FROM net_events \
     WHERE timestamp >= ?2 AND (?4 IS NULL OR trace_id = ?4 OR trace_id IS NULL) \
     ORDER BY timestamp ASC LIMIT ?1";

const FS_WINDOW: &str = "SELECT timestamp, 'fs' AS layer, id AS ref, action || ' ' || path AS summary, \
     NULL AS status, NULL AS duration_ms, trace_id FROM fs_events \
     WHERE timestamp >= ?2 AND (?4 IS NULL OR trace_id = ?4 OR trace_id IS NULL) \
     ORDER BY timestamp ASC LIMIT ?1";

const MODEL_WINDOW: &str = "SELECT timestamp, 'model' AS layer, id AS ref, \
     provider || '/' || COALESCE(model, '?') AS summary, \
     status_code AS status, duration_ms, trace_id FROM model_calls \
     WHERE timestamp >= ?2 AND (?4 IS NULL OR trace_id = ?4 OR trace_id IS NULL) \
     ORDER BY timestamp ASC LIMIT ?1";

/// The requested layers' windows, merged oldest first. Parameters: `?1`
/// limit, `?2` cutoff, `?3` tool cutoff, `?4` trace id or NULL. Every window
/// names `?4`, so the statement always takes all four.
pub(crate) fn timeline_sql(layers: &[TimelineLayer]) -> String {
    let windows = layers
        .iter()
        .map(|layer| {
            let window = match layer {
                TimelineLayer::Exec => EXEC_WINDOW,
                TimelineLayer::Tool => TOOL_WINDOW,
                TimelineLayer::Net => NET_WINDOW,
                TimelineLayer::Fs => FS_WINDOW,
                TimelineLayer::Model => MODEL_WINDOW,
            };
            format!("SELECT * FROM ({window})")
        })
        .collect::<Vec<_>>();
    format!("{} ORDER BY timestamp ASC LIMIT ?1", windows.join(" UNION ALL "))
}

/// A cutoff spelled like a ledger timestamp, to the microsecond. Spelled to
/// the second, `12:00:00Z` sorts after every `12:00:00.xxxxxxZ` row and the
/// cutoff's own second was dropped.
pub(super) fn ledger_cutoff(secs: u64) -> String {
    // Pure-stdlib RFC3339 (UTC); capsem-service stays free of `chrono`.
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
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.000000Z")
}
