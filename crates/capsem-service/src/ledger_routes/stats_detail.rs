//! Stats detail query intent; DbHandle owns execution and read caching.
use super::*;
use std::collections::BTreeMap;
mod interactions;

pub(super) const STATS_DETAIL_MODEL_STATS_SQL: &str = r#"
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

pub(crate) async fn read_stats_detail_payload_from_session_db(
    state: &ServiceState,
    vm_id: &str,
    db_path: &StdPath,
) -> Result<api::VmStatsDetailResponse, AppError> {
    let db = open_ready_session_db(state, vm_id, "stats_detail", db_path).await?;
    let bodies: Vec<api::EventBody> =
        query_rows(vm_id, db_path, &db, "body_blobs", STATS_DETAIL_BODY_BLOBS_SQL).await?;
    let mut body_blobs: BTreeMap<String, Vec<api::EventBody>> = BTreeMap::new();
    for body in bodies {
        body_blobs.entry(body.event_id.clone()).or_default().push(body);
    }
    Ok(api::VmStatsDetailResponse {
        interactions: interactions::read_interactions(vm_id, db_path, &db, &body_blobs).await?,
        model_stats: query_rows(vm_id, db_path, &db, "model_stats", STATS_DETAIL_MODEL_STATS_SQL).await?,
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
