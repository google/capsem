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
       tc.arguments,
       COALESCE(tc.response_preview, tr.content_preview) AS response_preview,
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

pub(super) async fn query_rows<T: DeserializeOwned>(
    vm_id: &str,
    db_path: &StdPath,
    db: &capsem_logger::DbHandle,
    query_name: &str,
    sql: &str,
) -> Result<Vec<T>, AppError> {
    let rows = query_route_objects(vm_id, "stats_detail", query_name, db_path, db, sql, &[]).await?;
    rows.into_iter()
        .map(|mut row| {
            // SQLite's integer flags are an implementation detail, not the JSON API.
            // Reject corrupt flags rather than silently turning every nonzero into true.
            for name in ["model_parent_missing", "truncated"] {
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

async fn query_rows_with_params<T: DeserializeOwned>(
    vm_id: &str,
    db_path: &StdPath,
    db: &capsem_logger::DbHandle,
    query_name: &str,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<Vec<T>, AppError> {
    let rows = query_route_objects(vm_id, "stats_detail", query_name, db_path, db, sql, params).await?;
    rows.into_iter()
        .map(|mut row| {
            if let Some(flag) = row.get_mut("truncated") {
                *flag = match flag.as_i64() {
                    Some(0) => json!(false),
                    Some(1) => json!(true),
                    _ => {
                        return Err(ledger_route_error(
                            vm_id,
                            "stats_detail",
                            query_name,
                            db_path,
                            "invalid boolean flag truncated",
                        ))
                    }
                };
            }
            serde_json::from_value(row)
                .map_err(|error| ledger_route_error(vm_id, "stats_detail", query_name, db_path, error))
        })
        .collect()
}

pub(crate) async fn read_stats_detail_payload_from_session_db(
    state: &ServiceState,
    vm_id: &str,
    db_path: &StdPath,
) -> Result<api::VmStatsDetailResponse, AppError> {
    let db = open_ready_session_db(state, vm_id, "stats_detail", db_path).await?;
    let bodies: Vec<api::EventBody> = query_rows_with_params(
        vm_id,
        db_path,
        &db,
        "body_blobs",
        STATS_DETAIL_BODY_BLOBS_SQL,
        &[json!(STATS_DETAIL_PROCESS_EVENTS_LIMIT)],
    )
    .await?;
    let mut body_blobs: BTreeMap<String, Vec<api::EventBody>> = BTreeMap::new();
    for body in bodies {
        body_blobs.entry(body.event_id.clone()).or_default().push(body);
    }
    Ok(api::VmStatsDetailResponse {
        interactions: interactions::read_interactions(vm_id, db_path, &db, &body_blobs).await?,
        model_stats: super::activity::model_usage(
            &db.ledger_counters()
                .await
                .map_err(|error| ledger_route_error(vm_id, "stats_detail", "counters", db_path, error))?,
        ),
        model_events: query_rows(vm_id, db_path, &db, "model_events", STATS_DETAIL_MODEL_EVENTS_SQL).await?,
        tool_events: query_rows(vm_id, db_path, &db, "tool_events", STATS_DETAIL_TOOL_EVENTS_SQL).await?,
        http_events: query_rows(vm_id, db_path, &db, "http_events", STATS_DETAIL_HTTP_EVENTS_SQL).await?,
        dns_events: query_rows(vm_id, db_path, &db, "dns_events", STATS_DETAIL_DNS_EVENTS_SQL).await?,
        file_events: query_rows(vm_id, db_path, &db, "file_events", STATS_DETAIL_FILE_EVENTS_SQL).await?,
        process_events: query_rows(vm_id, db_path, &db, "process_events", STATS_DETAIL_PROCESS_EVENTS_SQL).await?,
        audit_events: query_rows(vm_id, db_path, &db, "audit_events", STATS_DETAIL_AUDIT_EVENTS_SQL).await?,
        credential_events: query_rows(
            vm_id,
            db_path,
            &db,
            "credential_events",
            STATS_DETAIL_CREDENTIAL_EVENTS_SQL,
        )
        .await?,
        body_blobs,
    })
}
