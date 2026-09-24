//! `GET /vms/{id}/history`: one page of exec and audit rows, newest first.
//!
//! The route used to load every exec and audit row, then filter, sort and
//! page in memory, so a poll cost as much as the session was long (#223).
//! Layer, search, order and page now run in SQL: each layer's arm walks its
//! table's timestamp index newest first and stops at `offset + limit` rows,
//! and the merge keeps the page.
//!
//! `total` without a search is the writer's counter snapshot, the numbers
//! `/history/counts` reports. With a search it is a COUNT over the matching
//! rows, which is a scan by nature until the ledger has full-text search.
//!
//! Search is `instr`, a literal, case-sensitive substring match: the
//! semantics the API documents and the in-memory filter had. `LIKE` would be
//! case-insensitive and read `%` and `_` as wildcards. The `details` column is
//! searched as the JSON object the response carries, built by the same
//! `json_object` call whose key order matches the API type's field order.

use super::*;

const EXEC_ROWS: &str = r#"
SELECT timestamp, 'exec' AS layer, command, exit_code, duration_ms,
       stdout_preview, stderr_preview,
       json_object(
           'source', source,
           'trace_id', trace_id,
           'process_name', process_name,
           'exec_id', exec_id
       ) AS details
FROM exec_events
"#;

const AUDIT_ROWS: &str = r#"
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
"#;

/// `?1` is the search text in both statements.
const SEARCH: &str = " WHERE instr(command, ?1) > 0 OR instr(stdout_preview, ?1) > 0 \
                      OR instr(stderr_preview, ?1) > 0 OR instr(details, ?1) > 0";

/// The one aggregate here: how many rows a search matches.
const SEARCH_TOTAL: &str = "SELECT COUNT(*) AS total FROM ({arms})";

/// The maximum rows the reader returns for one page, as the API caps it.
pub(crate) const PAGE_CAP: usize = 2000;

fn arms(layer: api::HistoryLayerFilter) -> Vec<&'static str> {
    [
        (api::HistoryLayer::Exec, EXEC_ROWS),
        (api::HistoryLayer::Audit, AUDIT_ROWS),
    ]
    .into_iter()
    .filter(|(arm, _)| layer.includes(*arm))
    .map(|(_, rows)| rows)
    .collect()
}

fn filtered(rows: &str, search: bool) -> String {
    let search = if search { SEARCH } else { "" };
    format!("SELECT * FROM ({rows}){search}")
}

/// One page: `?1` search (NULL and unused without one), `?2` rows per arm
/// (`offset + limit`), `?3` limit, `?4` offset.
pub(crate) fn page_sql(layer: api::HistoryLayerFilter, search: bool) -> String {
    let arms = arms(layer)
        .into_iter()
        .map(|rows| {
            format!(
                "SELECT * FROM ({} ORDER BY timestamp DESC LIMIT ?2)",
                filtered(rows, search)
            )
        })
        .collect::<Vec<_>>();
    format!(
        "{} ORDER BY timestamp DESC LIMIT ?3 OFFSET ?4",
        arms.join(" UNION ALL ")
    )
}

pub(crate) fn search_total_sql(layer: api::HistoryLayerFilter) -> String {
    let arms = arms(layer)
        .into_iter()
        .map(|rows| filtered(rows, true))
        .collect::<Vec<_>>();
    SEARCH_TOTAL.replace("{arms}", &arms.join(" UNION ALL "))
}

fn sql_integer(value: usize) -> serde_json::Value {
    json!(i64::try_from(value).unwrap_or(i64::MAX))
}

pub(crate) async fn history_page(
    state: &ServiceState,
    vm_id: &str,
    params: &api::HistoryQuery,
) -> Result<api::HistoryResponse, AppError> {
    let session_dir = resolve_session_dir(state, vm_id)?;
    let db_path = session_dir.join("session.db");
    let db = open_ready_session_db(state, vm_id, "history", &db_path).await?;
    let limit = params.limit.min(PAGE_CAP);
    let search = params.search.as_deref();
    let total = match search {
        None => {
            let counters = db
                .ledger_counters()
                .await
                .map_err(|error| ledger_route_error(vm_id, "history", "counters", &db_path, error))?;
            [
                (api::HistoryLayer::Exec, counters.exec.started),
                (api::HistoryLayer::Audit, counters.audit.events),
            ]
            .into_iter()
            .filter(|(layer, _)| params.layer.includes(*layer))
            .map(|(_, count)| count)
            .sum()
        }
        Some(search) => {
            let rows = query_route_objects(
                vm_id,
                "history",
                "search total",
                &db_path,
                &db,
                &search_total_sql(params.layer),
                &[json!(search)],
            )
            .await?;
            rows.first()
                .and_then(|row| row.get("total"))
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| ledger_route_error(vm_id, "history", "search total", &db_path, "missing total"))?
        }
    };
    let commands = if limit == 0 || u64::try_from(params.offset).map_or(true, |offset| offset >= total) {
        Vec::new()
    } else {
        let rows = query_route_objects(
            vm_id,
            "history",
            "entries",
            &db_path,
            &db,
            &page_sql(params.layer, search.is_some()),
            &[
                search.map_or(serde_json::Value::Null, |search| json!(search)),
                sql_integer(params.offset.saturating_add(limit)),
                sql_integer(limit),
                sql_integer(params.offset),
            ],
        )
        .await?;
        rows.into_iter()
            .map(|row| decode_entry(vm_id, &db_path, row))
            .collect::<Result<Vec<_>, AppError>>()?
    };
    let has_more = (params.offset.saturating_add(commands.len()) as u64) < total;
    Ok(api::HistoryResponse {
        commands,
        total,
        has_more,
    })
}

fn decode_entry(vm_id: &str, db_path: &StdPath, mut row: serde_json::Value) -> Result<api::HistoryEntry, AppError> {
    let details = row
        .get_mut("details")
        .ok_or_else(|| ledger_route_error(vm_id, "history", "entry details", db_path, "missing details"))?;
    if let serde_json::Value::String(text) = details {
        *details = serde_json::from_str(text)
            .map_err(|error| ledger_route_error(vm_id, "history", "parse entry details", db_path, error))?;
    }
    serde_json::from_value::<api::HistoryEntry>(row)
        .map_err(|error| ledger_route_error(vm_id, "history", "decode entry", db_path, error))
}
