//! `GET /stats`: totals across every session, from `main.db`.
//!
//! This is the one aggregate the service still runs, and it runs over the
//! session index -- one row per session, filled from each session's counter
//! snapshot when it stops -- never over a session ledger.

use super::*;

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
                   storage_mode, rootfs_hash,
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

pub(crate) async fn read_stats_response_from_main_db_handle(state: &ServiceState) -> Result<Vec<u8>, AppError> {
    let db_path = state.main_db_path();
    let db = &state.profile_mutation_db;
    let db_epoch = db.read_cache_epoch(capsem_logger::ReadCacheDomain::SessionSummary);
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
            *state.stats_response_cache.lock().unwrap() = Some(CachedLedgerResponse {
                db_epoch,
                bytes: bytes.clone(),
            });
            Ok(bytes)
        }
        serde_json::Value::Object(_) => {
            let bytes = serde_json::to_vec(payload)
                .map_err(|error| main_ledger_route_error("stats", "serialize response payload", &db_path, error))?;
            *state.stats_response_cache.lock().unwrap() = Some(CachedLedgerResponse {
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
