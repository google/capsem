//! Stats detail query intent; DbHandle owns execution and read caching.
use super::bodies::{STATS_DETAIL_BODY_BLOBS_SQL, STATS_DETAIL_PROCESS_EVENTS_LIMIT};
use super::*;
use std::collections::BTreeMap;
pub(crate) mod interactions;

pub(crate) const STATS_DETAIL_MODEL_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, provider, model, method, path, status_code,
       input_tokens, output_tokens, duration_ms, response_bytes,
       stop_reason, trace_id, credential_ref
FROM model_calls
ORDER BY id DESC
LIMIT 200
"#;

/// The newest 200 listed tool calls, newest first, with their model call and
/// response joined on.
///
/// `tool_calls` drives the joins in a reverse rowid walk that stops at 200
/// rows, each probing its model call by rowid and its response by
/// `idx_tool_responses_call_id`. `NOT INDEXED` keeps the planner off
/// `idx_tool_calls_origin`, which it would otherwise pick for the `IN` list and
/// then sort every tool row of the session back into id order to find the
/// newest -- a cost that grows with the ledger on every stats poll.
/// Insertion order is newest first; a timestamp is not, since a call recorded
/// without one borrows its model call's.
///
/// `arguments` is cut to [`STATS_DETAIL_FIELD_CHARS`], bound as `?1`: a model's
/// tool arguments are stored whole, and a file-writing agent's run to megabytes
/// each, so the list carried up to 200 of them on every poll. `bytes` still
/// measures the whole call, so a client can tell a cut argument from a short one.
pub(crate) const STATS_DETAIL_TOOL_EVENTS_SQL: &str = r#"
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
       substr(tc.arguments, 1, ?1) AS arguments,
       COALESCE(tc.response_preview, tr.content_preview) AS response_preview,
       tr.event_id AS response_event_id,
       tc.error_message,
       tc.origin AS source,
       COALESCE(tc.credential_ref, tr.credential_ref) AS credential_ref
FROM tool_calls AS tc NOT INDEXED
LEFT JOIN model_calls mc ON tc.model_call_id = mc.id
LEFT JOIN tool_responses tr ON tc.call_id = tr.call_id
WHERE tc.origin IN ('model', 'native', 'mcp', 'builtin', 'local', 'mcp_proxy')
ORDER BY tc.id DESC
LIMIT 200
"#;

pub(crate) const STATS_DETAIL_HTTP_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, domain, port, method, path, query, status_code,
       decision, duration_ms, bytes_sent, bytes_received, matched_rule, policy_rule,
       trace_id, credential_ref, request_headers, response_headers
FROM net_events
ORDER BY id DESC
LIMIT 200
"#;

pub(crate) const STATS_DETAIL_DNS_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, qname, qtype, qclass, rcode, decision,
       matched_rule, policy_rule, source_proto, process_name,
       upstream_resolver_ms, trace_id, credential_ref
FROM dns_events
ORDER BY id DESC
LIMIT 200
"#;

pub(crate) const STATS_DETAIL_FILE_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, action, path, size, trace_id, credential_ref
FROM fs_events
ORDER BY id DESC
LIMIT 200
"#;

pub(crate) const STATS_DETAIL_PROCESS_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, exec_id, command, exit_code, duration_ms,
       stdout_bytes, stderr_bytes, source, process_name, pid, trace_id,
       credential_ref
FROM exec_events
ORDER BY id DESC
LIMIT 100
"#;

pub(crate) const STATS_DETAIL_AUDIT_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, pid, ppid, uid, exe, comm, argv, cwd,
       exit_code, session_id, tty, audit_id, exec_event_id, parent_exe,
       trace_id, credential_ref
FROM audit_events
ORDER BY id DESC
LIMIT 100
"#;

pub(crate) const STATS_DETAIL_CREDENTIAL_EVENTS_SQL: &str = r#"
SELECT event_id, timestamp, material_class, source, event_type,
       event_type AS origin, outcome AS verb, provider,
       trace_id, context_json
FROM substitution_events
ORDER BY id DESC
LIMIT 100
"#;

/// How many characters of one free-text field a listed row carries: a tool
/// call's arguments, or a reasoning block's text.
///
/// The ledger stores both whole -- they have no archived body of their own to
/// point at -- so a file-writing agent's calls run to megabytes each, and the
/// list carried 200 of them, twice, on every poll. A cut field says so: an
/// interaction payload reports `truncated`, and a tool event keeps measuring
/// the whole call in `bytes`.
pub(crate) const STATS_DETAIL_FIELD_CHARS: usize = 16 * 1024;

/// SQLite's integer flags, which the API types as booleans.
const BOOLEAN_FLAGS: [&str; 4] = [
    "model_parent_missing",
    "truncated",
    "arguments_truncated",
    "content_truncated",
];

/// Decode one stats detail result into the rows the API types.
///
/// SQLite's integer flags are an implementation detail, not the JSON API.
/// Corrupt flags are rejected rather than read as true for every nonzero.
pub(super) fn decode_rows<T: DeserializeOwned>(
    vm_id: &str,
    db_path: &StdPath,
    query_name: &str,
    rows: capsem_logger::ledger_protocol::LedgerRows,
) -> Result<Vec<T>, AppError> {
    ledger_rows_to_objects(rows)
        .into_iter()
        .map(|mut row| {
            for name in BOOLEAN_FLAGS {
                if let Some(flag) = row.get_mut(name) {
                    *flag = match flag.as_i64() {
                        Some(0) => json!(false),
                        Some(1) => json!(true),
                        _ => {
                            return Err(ledger_route_error(
                                vm_id,
                                "stats_detail",
                                query_name,
                                db_path,
                                format!("invalid boolean flag {name}"),
                            ))
                        }
                    };
                }
            }
            serde_json::from_value(row)
                .map_err(|error| ledger_route_error(vm_id, "stats_detail", query_name, db_path, error))
        })
        .collect()
}

/// Every statement one stats detail read runs, named, in the order the route
/// decodes their results.
pub(crate) fn stats_detail_statements() -> [(&'static str, &'static str, Vec<serde_json::Value>); 11] {
    let field = || vec![json!(STATS_DETAIL_FIELD_CHARS)];
    [
        (
            "body_blobs",
            STATS_DETAIL_BODY_BLOBS_SQL,
            vec![json!(STATS_DETAIL_PROCESS_EVENTS_LIMIT)],
        ),
        ("interaction_models", interactions::MODEL_ITEMS_SQL, field()),
        ("interaction_tools", interactions::TOOL_CALLS_SQL, field()),
        ("model_events", STATS_DETAIL_MODEL_EVENTS_SQL, Vec::new()),
        ("tool_events", STATS_DETAIL_TOOL_EVENTS_SQL, field()),
        ("http_events", STATS_DETAIL_HTTP_EVENTS_SQL, Vec::new()),
        ("dns_events", STATS_DETAIL_DNS_EVENTS_SQL, Vec::new()),
        ("file_events", STATS_DETAIL_FILE_EVENTS_SQL, Vec::new()),
        ("process_events", STATS_DETAIL_PROCESS_EVENTS_SQL, Vec::new()),
        ("audit_events", STATS_DETAIL_AUDIT_EVENTS_SQL, Vec::new()),
        ("credential_events", STATS_DETAIL_CREDENTIAL_EVENTS_SQL, Vec::new()),
    ]
}

/// The stats detail view, read as one batch.
///
/// The web app fetches the view when a user opens a session's stats, and on
/// Refresh; nothing polls it. Its statements go to the DB handle together, so
/// a fetch is one worker round trip -- the batch carries its own readiness
/// check -- and a batch over a ledger that has not moved is answered from the
/// handle's cache without executing anything. The model totals come from the
/// handle's counter snapshot in memory. It used to be a worker round trip, a
/// statement compile and an execution per list, on every fetch.
pub(crate) async fn read_stats_detail_payload_from_session_db(
    state: &ServiceState,
    vm_id: &str,
    db_path: &StdPath,
) -> Result<api::VmStatsDetailResponse, AppError> {
    let db = session_db(state, vm_id, "stats_detail", db_path).await?;
    let statements = stats_detail_statements();
    let raw = db
        .query(capsem_logger::ledger_protocol::LedgerQuery::StatsDetail)
        .await
        .map_err(|error| ledger_route_error(vm_id, "stats_detail", "query", db_path, error))?;
    let raw = <[capsem_logger::ledger_protocol::LedgerRows; 11]>::try_from(raw).map_err(|raw| {
        ledger_route_error(
            vm_id,
            "stats_detail",
            "query",
            db_path,
            format!("batch returned {} results, expected {}", raw.len(), statements.len()),
        )
    })?;
    let [body_blobs, models, tools, model_events, tool_events, http, dns, files, processes, audit, credentials] = raw;
    let name = |index: usize| statements[index].0;
    let bodies: Vec<api::EventBody> = decode_rows(vm_id, db_path, name(0), body_blobs)?;
    let mut body_blobs: BTreeMap<String, Vec<api::EventBody>> = BTreeMap::new();
    for body in bodies {
        body_blobs.entry(body.event_id.clone()).or_default().push(body);
    }
    let counters = db
        .ledger_counters()
        .await
        .map_err(|error| ledger_route_error(vm_id, "stats_detail", "counters", db_path, error))?;
    Ok(api::VmStatsDetailResponse {
        interactions: interactions::read_interactions(
            vm_id,
            db_path,
            decode_rows(vm_id, db_path, name(1), models)?,
            decode_rows(vm_id, db_path, name(2), tools)?,
            &body_blobs,
        )?,
        model_stats: super::activity::model_usage(&counters),
        model_events: decode_rows(vm_id, db_path, name(3), model_events)?,
        tool_events: decode_rows(vm_id, db_path, name(4), tool_events)?,
        http_events: decode_rows(vm_id, db_path, name(5), http)?,
        dns_events: decode_rows(vm_id, db_path, name(6), dns)?,
        file_events: decode_rows(vm_id, db_path, name(7), files)?,
        process_events: decode_rows(vm_id, db_path, name(8), processes)?,
        audit_events: decode_rows(vm_id, db_path, name(9), audit)?,
        credential_events: decode_rows(vm_id, db_path, name(10), credentials)?,
        body_blobs,
    })
}
