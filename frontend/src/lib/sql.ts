// Centralized SQL queries for per-session analytics.
// All queries run against info.db via queryDb() / queryAll() / queryOne().

/** Aggregate counts from net_events. */
export const NET_STATS_SQL = `
  SELECT COUNT(*) AS net_total,
    COALESCE(SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END), 0) AS net_allowed,
    COALESCE(SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END), 0) AS net_denied,
    COALESCE(SUM(CASE WHEN decision = 'error' THEN 1 ELSE 0 END), 0) AS net_error,
    COALESCE(SUM(bytes_sent), 0) AS net_bytes_sent,
    COALESCE(SUM(bytes_received), 0) AS net_bytes_received
  FROM net_events`;

/** Top domains with allowed/denied breakdown. */
export const TOP_DOMAINS_SQL = `
  SELECT domain, COUNT(*) AS count,
    SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END) AS allowed,
    SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END) AS denied
  FROM net_events GROUP BY domain ORDER BY count DESC LIMIT 10`;

/** Time buckets (6-minute windows) for requests-over-time chart. */
export const NET_TIME_BUCKETS_SQL = `
  SELECT substr(timestamp, 1, 16) || ':00Z' AS bucket_start,
    SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END) AS allowed,
    SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END) AS denied
  FROM net_events GROUP BY bucket_start ORDER BY bucket_start`;

/** Per-provider token usage from model_calls. */
export const PROVIDER_USAGE_SQL = `
  SELECT provider, COUNT(*) AS call_count,
    COALESCE(SUM(COALESCE(input_tokens, 0)), 0) AS total_input_tokens,
    COALESCE(SUM(COALESCE(output_tokens, 0)), 0) AS total_output_tokens,
    COALESCE(SUM(duration_ms), 0) AS total_duration_ms,
    COALESCE(SUM(estimated_cost_usd), 0.0) AS total_estimated_cost_usd
  FROM model_calls GROUP BY provider ORDER BY COUNT(*) DESC`;

/** Top tools by usage count. */
export const TOOL_USAGE_SQL = `
  SELECT tool_name, COUNT(*) AS count
  FROM tool_calls GROUP BY tool_name ORDER BY count DESC LIMIT 20`;

/** Aggregate model stats (tokens, cost). */
export const MODEL_STATS_SQL = `
  SELECT COUNT(*) AS model_call_count,
    COALESCE(SUM(COALESCE(input_tokens, 0)), 0) AS total_input_tokens,
    COALESCE(SUM(COALESCE(output_tokens, 0)), 0) AS total_output_tokens,
    COALESCE(SUM(duration_ms), 0) AS total_model_duration_ms,
    COALESCE(SUM(estimated_cost_usd), 0.0) AS total_estimated_cost_usd
  FROM model_calls`;

/** Total tool call count. */
export const TOOL_COUNT_SQL = `SELECT COUNT(*) AS count FROM tool_calls`;

/** MCP call decision counts. */
export const MCP_STATS_SQL = `
  SELECT COUNT(*) AS total,
    COALESCE(SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END), 0) AS allowed,
    COALESCE(SUM(CASE WHEN decision = 'warned' THEN 1 ELSE 0 END), 0) AS warned,
    COALESCE(SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END), 0) AS denied,
    COALESCE(SUM(CASE WHEN decision = 'error' THEN 1 ELSE 0 END), 0) AS errored
  FROM mcp_calls`;

/** MCP calls grouped by server. */
export const MCP_BY_SERVER_SQL = `
  SELECT server_name, COUNT(*) AS count,
    SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END) AS denied,
    SUM(CASE WHEN decision = 'warned' THEN 1 ELSE 0 END) AS warned
  FROM mcp_calls GROUP BY server_name ORDER BY count DESC`;

/** File event action counts. */
export const FILE_STATS_SQL = `
  SELECT COUNT(*) AS total,
    COALESCE(SUM(CASE WHEN action = 'created' THEN 1 ELSE 0 END), 0) AS created,
    COALESCE(SUM(CASE WHEN action = 'modified' THEN 1 ELSE 0 END), 0) AS modified,
    COALESCE(SUM(CASE WHEN action = 'deleted' THEN 1 ELSE 0 END), 0) AS deleted
  FROM fs_events`;

/** Average latency from net_events. */
export const AVG_LATENCY_SQL = `
  SELECT COALESCE(CAST(AVG(duration_ms) AS INTEGER), 0) AS avg_latency
  FROM net_events`;

/** HTTP method distribution from net_events. */
export const METHOD_DIST_SQL = `
  SELECT COALESCE(method, 'CONNECT') AS method, COUNT(*) AS count
  FROM net_events GROUP BY method ORDER BY count DESC`;

/** Process distribution from net_events. */
export const PROCESS_DIST_SQL = `
  SELECT COALESCE(process_name, 'unknown') AS process_name, COUNT(*) AS count
  FROM net_events GROUP BY process_name ORDER BY count DESC LIMIT 8`;

// ---------------------------------------------------------------------------
// Parameterized queries used by db.ts (replacing per-command IPC wrappers)
// ---------------------------------------------------------------------------

/** Recent network events. Bind: [limit]. */
export const NET_EVENTS_SQL = `
  SELECT CAST(strftime('%s', timestamp) AS INTEGER) AS timestamp,
         domain, port, decision, process_name, pid,
         method, path, query, status_code,
         bytes_sent, bytes_received, duration_ms, matched_rule,
         request_headers, response_headers,
         request_body_preview, response_body_preview, conn_type
  FROM net_events ORDER BY id DESC LIMIT ?`;

/** Network events filtered by search. Bind: [search, search, search, search, limit]. */
export const NET_EVENTS_SEARCH_SQL = `
  SELECT CAST(strftime('%s', timestamp) AS INTEGER) AS timestamp,
         domain, port, decision, process_name, pid,
         method, path, query, status_code,
         bytes_sent, bytes_received, duration_ms, matched_rule,
         request_headers, response_headers,
         request_body_preview, response_body_preview, conn_type
  FROM net_events
  WHERE domain LIKE '%' || ? || '%'
     OR path LIKE '%' || ? || '%'
     OR method LIKE '%' || ? || '%'
     OR matched_rule LIKE '%' || ? || '%'
  ORDER BY id DESC LIMIT ?`;

/** Recent model calls. Bind: [limit]. */
export const MODEL_CALLS_SQL = `
  SELECT id, CAST(strftime('%s', timestamp) AS INTEGER) AS timestamp,
         provider, model, process_name, pid,
         method, path, stream, system_prompt_preview,
         messages_count, tools_count, request_bytes,
         request_body_preview, message_id, status_code,
         text_content, thinking_content, stop_reason,
         input_tokens, output_tokens, usage_details, duration_ms,
         response_bytes, estimated_cost_usd, trace_id
  FROM model_calls ORDER BY id DESC LIMIT ?`;

/** Model calls filtered by search. Bind: [search, search, search, limit]. */
export const MODEL_CALLS_SEARCH_SQL = `
  SELECT id, CAST(strftime('%s', timestamp) AS INTEGER) AS timestamp,
         provider, model, process_name, pid,
         method, path, stream, system_prompt_preview,
         messages_count, tools_count, request_bytes,
         request_body_preview, message_id, status_code,
         text_content, thinking_content, stop_reason,
         input_tokens, output_tokens, usage_details, duration_ms,
         response_bytes, estimated_cost_usd, trace_id
  FROM model_calls
  WHERE provider LIKE '%' || ? || '%'
     OR model LIKE '%' || ? || '%'
     OR stop_reason LIKE '%' || ? || '%'
  ORDER BY id DESC LIMIT ?`;

/** Tool calls for model call IDs. Bind: no params -- caller must inline IDs.
 *  Use with template literal: TOOL_CALLS_FOR_SQL(ids). */
export function toolCallsForSql(ids: number[]): string {
  if (ids.length === 0) return 'SELECT 1 WHERE 0'; // no-op
  return `SELECT model_call_id, call_index, call_id, tool_name, arguments
          FROM tool_calls WHERE model_call_id IN (${ids.join(',')}) ORDER BY call_index`;
}

/** Tool responses for model call IDs. Same pattern as toolCallsForSql. */
export function toolResponsesForSql(ids: number[]): string {
  if (ids.length === 0) return 'SELECT 1 WHERE 0';
  return `SELECT model_call_id, call_id, content_preview, is_error
          FROM tool_responses WHERE model_call_id IN (${ids.join(',')})`;
}

/** Recent traces (grouped agent turns). Bind: [limit]. */
export const TRACES_SQL = `
  WITH top_traces AS (
    SELECT trace_id, MAX(id) AS max_id
    FROM model_calls WHERE trace_id IS NOT NULL
    GROUP BY trace_id ORDER BY MAX(id) DESC LIMIT ?
  )
  SELECT t.trace_id,
         MIN(mc.timestamp) AS started_at,
         MAX(mc.timestamp) AS ended_at,
         (SELECT provider FROM model_calls m2 WHERE m2.trace_id = t.trace_id ORDER BY m2.id ASC LIMIT 1) AS provider,
         (SELECT model FROM model_calls m3 WHERE m3.trace_id = t.trace_id ORDER BY m3.id ASC LIMIT 1) AS model,
         COUNT(*) AS call_count,
         COALESCE(SUM(COALESCE(mc.input_tokens, 0)), 0) AS total_input_tokens,
         COALESCE(SUM(COALESCE(mc.output_tokens, 0)), 0) AS total_output_tokens,
         (SELECT json_group_object(je.key, je.total) FROM (
             SELECT je.key, SUM(je.value) AS total
             FROM model_calls mc6, json_each(mc6.usage_details) je
             WHERE mc6.trace_id = t.trace_id AND mc6.usage_details IS NOT NULL
             GROUP BY je.key
         ) je) AS total_usage_details,
         COALESCE(SUM(mc.duration_ms), 0) AS total_duration_ms,
         COALESCE(SUM(mc.estimated_cost_usd), 0.0) AS total_estimated_cost_usd,
         (SELECT COUNT(*) FROM tool_calls tc2 JOIN model_calls mc2 ON tc2.model_call_id = mc2.id WHERE mc2.trace_id = t.trace_id) AS total_tool_calls,
         (SELECT stop_reason FROM model_calls m4 WHERE m4.trace_id = t.trace_id ORDER BY m4.id DESC LIMIT 1) AS stop_reason,
         (SELECT system_prompt_preview FROM model_calls m5 WHERE m5.trace_id = t.trace_id ORDER BY m5.id ASC LIMIT 1) AS system_prompt_preview
  FROM top_traces t
  JOIN model_calls mc ON mc.trace_id = t.trace_id
  GROUP BY t.trace_id
  ORDER BY t.max_id DESC`;

/** Recent MCP calls. Bind: [limit]. */
export const MCP_CALLS_SQL = `
  SELECT CAST(strftime('%s', timestamp) AS INTEGER) AS timestamp,
         server_name, method, tool_name, request_id,
         request_preview, response_preview, decision,
         duration_ms, error_message, process_name
  FROM mcp_calls ORDER BY id DESC LIMIT ?`;

/** MCP calls filtered by search. Bind: [search, search, search, limit]. */
export const MCP_CALLS_SEARCH_SQL = `
  SELECT CAST(strftime('%s', timestamp) AS INTEGER) AS timestamp,
         server_name, method, tool_name, request_id,
         request_preview, response_preview, decision,
         duration_ms, error_message, process_name
  FROM mcp_calls
  WHERE server_name LIKE '%' || ? || '%'
     OR tool_name LIKE '%' || ? || '%'
     OR method LIKE '%' || ? || '%'
  ORDER BY id DESC LIMIT ?`;

/** Recent file events. Bind: [limit]. */
export const FILE_EVENTS_SQL = `
  SELECT CAST(strftime('%s', timestamp) AS INTEGER) AS timestamp,
         action, path, size
  FROM fs_events ORDER BY id DESC LIMIT ?`;

/** File events filtered by search. Bind: [search, limit]. */
export const FILE_EVENTS_SEARCH_SQL = `
  SELECT CAST(strftime('%s', timestamp) AS INTEGER) AS timestamp,
         action, path, size
  FROM fs_events WHERE path LIKE '%' || ? || '%'
  ORDER BY id DESC LIMIT ?`;

// ---------------------------------------------------------------------------
// main.db queries (cross-session, db: 'main')
// ---------------------------------------------------------------------------

/** Session history. Bind: [limit]. */
export const SESSION_HISTORY_SQL = `
  SELECT id, mode, command, status, created_at, stopped_at,
         scratch_disk_size_gb, ram_bytes,
         total_requests, allowed_requests, denied_requests,
         total_input_tokens, total_output_tokens, total_estimated_cost,
         total_tool_calls, total_mcp_calls, total_file_events,
         compressed_size_bytes, vacuumed_at
  FROM sessions ORDER BY created_at DESC LIMIT ?`;

/** Global aggregated stats. No params. */
export const GLOBAL_STATS_SQL = `
  SELECT COUNT(*) AS total_sessions,
         COALESCE(SUM(total_input_tokens), 0) AS total_input_tokens,
         COALESCE(SUM(total_output_tokens), 0) AS total_output_tokens,
         COALESCE(SUM(total_estimated_cost), 0.0) AS total_estimated_cost,
         COALESCE(SUM(total_tool_calls), 0) AS total_tool_calls,
         COALESCE(SUM(total_mcp_calls), 0) AS total_mcp_calls,
         COALESCE(SUM(total_file_events), 0) AS total_file_events,
         COALESCE(SUM(total_requests), 0) AS total_requests,
         COALESCE(SUM(allowed_requests), 0) AS total_allowed,
         COALESCE(SUM(denied_requests), 0) AS total_denied
  FROM sessions`;

/** Top AI providers by call count. Bind: [limit]. */
export const TOP_PROVIDERS_SQL = `
  SELECT provider, SUM(call_count) AS call_count,
         SUM(input_tokens) AS input_tokens,
         SUM(output_tokens) AS output_tokens,
         SUM(estimated_cost) AS estimated_cost,
         SUM(total_duration_ms) AS total_duration_ms
  FROM ai_usage GROUP BY provider ORDER BY SUM(call_count) DESC LIMIT ?`;

/** Top tools by call count. Bind: [limit]. */
export const TOP_TOOLS_SQL = `
  SELECT tool_name, SUM(call_count) AS call_count,
         SUM(total_bytes) AS total_bytes,
         SUM(total_duration_ms) AS total_duration_ms
  FROM tool_usage GROUP BY tool_name ORDER BY SUM(call_count) DESC LIMIT ?`;

/** Top MCP tools by call count. Bind: [limit]. */
export const TOP_MCP_TOOLS_SQL = `
  SELECT tool_name, server_name,
         SUM(call_count) AS call_count,
         SUM(total_bytes) AS total_bytes,
         SUM(total_duration_ms) AS total_duration_ms
  FROM mcp_usage GROUP BY tool_name ORDER BY SUM(call_count) DESC LIMIT ?`;
