// All SQL query constants for the stats/analytics views.
// Queries target the session DB (web.db) unless noted.

// -- Stats bar (polled every 2s) ------------------------------------------

export const MODEL_STATS_SQL = `
  SELECT
    COALESCE(SUM(input_tokens), 0) as total_input_tokens,
    COALESCE(SUM(output_tokens), 0) as total_output_tokens,
    COALESCE(SUM(estimated_cost_usd), 0.0) as total_cost,
    COUNT(*) as call_count
  FROM model_calls
`;

export const TOOL_COUNT_SQL = `
  SELECT
    (SELECT COUNT(*) FROM tool_calls WHERE origin = 'native')
  + (SELECT COUNT(*) FROM mcp_calls WHERE tool_name IS NOT NULL) as cnt
`;

// -- Models tab (trace viewer) --------------------------------------------

export const TRACES_SQL = `
  WITH top_traces AS (
    SELECT trace_id, MAX(id) as max_id
    FROM model_calls
    WHERE trace_id IS NOT NULL
    GROUP BY trace_id
    ORDER BY max_id DESC
    LIMIT ?
  )
  SELECT
    t.trace_id,
    MIN(mc.timestamp) as started_at,
    (SELECT provider FROM model_calls m2 WHERE m2.trace_id = t.trace_id ORDER BY m2.id ASC LIMIT 1) as provider,
    (SELECT model FROM model_calls m3 WHERE m3.trace_id = t.trace_id ORDER BY m3.id ASC LIMIT 1) as model,
    COUNT(mc.id) as call_count,
    COALESCE(SUM(COALESCE(mc.input_tokens, 0)), 0) as total_input_tokens,
    COALESCE(SUM(COALESCE(mc.output_tokens, 0)), 0) as total_output_tokens,
    COALESCE(SUM(mc.duration_ms), 0) as total_duration_ms,
    COALESCE(SUM(mc.estimated_cost_usd), 0.0) as total_cost,
    (SELECT COUNT(*) FROM tool_calls tc
     JOIN model_calls mc2 ON tc.model_call_id = mc2.id
     WHERE mc2.trace_id = t.trace_id) as total_tool_calls,
    (SELECT stop_reason FROM model_calls m4 WHERE m4.trace_id = t.trace_id ORDER BY m4.id DESC LIMIT 1) as stop_reason
  FROM top_traces t
  JOIN model_calls mc ON mc.trace_id = t.trace_id
  GROUP BY t.trace_id
  HAVING total_input_tokens + total_output_tokens > 0
  ORDER BY t.max_id DESC
`;

export const TRACE_DETAIL_SQL = `
  SELECT id, timestamp, provider, model, thinking_content, text_content,
         input_tokens, output_tokens, duration_ms, estimated_cost_usd, stop_reason,
         request_body_preview, system_prompt_preview, messages_count, tools_count
  FROM model_calls
  WHERE trace_id = ?
  ORDER BY id ASC
`;

export const TRACE_TOOL_CALLS_SQL = `
  SELECT tc.id, tc.model_call_id, tc.call_index, tc.call_id, tc.tool_name, tc.arguments, tc.origin
  FROM tool_calls tc
  JOIN model_calls mc ON tc.model_call_id = mc.id
  WHERE mc.trace_id = ?
  ORDER BY tc.model_call_id, tc.call_index
`;

export const TRACE_TOOL_RESPONSES_SQL = `
  SELECT tr.model_call_id, tr.call_id, tr.content_preview, tr.is_error
  FROM tool_responses tr
  JOIN model_calls mc ON tr.model_call_id = mc.id
  WHERE mc.trace_id = ?
`;

// -- Tools tab (charts) -----------------------------------------------------

export const TOOLS_STATS_SQL = `
  SELECT
    (SELECT COUNT(*) FROM tool_calls WHERE origin = 'native') + (SELECT COUNT(*) FROM mcp_calls) as total,
    (SELECT COUNT(*) FROM tool_calls WHERE origin = 'native') as native,
    (SELECT COUNT(*) FROM mcp_calls) as mcp,
    (SELECT COUNT(*) FROM mcp_calls WHERE decision = 'allowed') as allowed,
    (SELECT COUNT(*) FROM mcp_calls WHERE decision != 'allowed') as denied
`;

export const TOOLS_TOP_TOOLS_SQL = `
  SELECT tool_name, cnt, source FROM (
    SELECT tc.tool_name, COUNT(*) as cnt, 'native' as source
    FROM tool_calls tc
    WHERE tc.origin = 'native'
    GROUP BY tc.tool_name
    UNION ALL
    SELECT tool_name, COUNT(*) as cnt, 'mcp' as source
    FROM mcp_calls
    WHERE tool_name IS NOT NULL
    GROUP BY tool_name
  )
  ORDER BY cnt DESC
  LIMIT 10
`;

export const TOOLS_TOP_SERVERS_SQL = `
  SELECT server_name, COUNT(*) as cnt
  FROM mcp_calls
  GROUP BY server_name
  ORDER BY cnt DESC
  LIMIT 8
`;

export const TOOLS_OVER_TIME_SQL = `
  WITH all_calls AS (
    SELECT mc.timestamp, 'native' as source
    FROM tool_calls tc
    JOIN model_calls mc ON tc.model_call_id = mc.id
    WHERE tc.origin = 'native'
    UNION ALL
    SELECT timestamp, 'mcp' as source
    FROM mcp_calls
  ),
  numbered AS (
    SELECT source,
      (ROW_NUMBER() OVER (ORDER BY timestamp) - 1) / 5 as bucket
    FROM all_calls
  )
  SELECT bucket,
    SUM(CASE WHEN source = 'native' THEN 1 ELSE 0 END) as native,
    SUM(CASE WHEN source = 'mcp' THEN 1 ELSE 0 END) as mcp
  FROM numbered
  GROUP BY bucket
  ORDER BY bucket
`;

// -- Tools tab (unified native + MCP) ----------------------------------------

export const TOOLS_UNIFIED_SQL = `
  SELECT timestamp, process_name, server_name, tool_name, method,
         decision, duration_ms, bytes, arguments, response_preview,
         error_message, source
  FROM (
    SELECT mc.timestamp, NULL as process_name, 'local' as server_name,
           tc.tool_name, NULL as method, 'allowed' as decision,
           mc.duration_ms,
           COALESCE(LENGTH(tc.arguments), 0) as bytes,
           tc.arguments, tr.content_preview as response_preview,
           NULL as error_message, 'native' as source
    FROM tool_calls tc
    JOIN model_calls mc ON tc.model_call_id = mc.id
    LEFT JOIN tool_responses tr ON tc.call_id = tr.call_id
    WHERE tc.origin = 'native'
    UNION ALL
    SELECT timestamp, process_name, server_name, tool_name, method,
           decision, duration_ms,
           COALESCE(LENGTH(request_preview), 0) + COALESCE(LENGTH(response_preview), 0) as bytes,
           request_preview as arguments, response_preview,
           error_message, 'mcp' as source
    FROM mcp_calls
  )
  ORDER BY timestamp DESC
`;

export const TOOLS_UNIFIED_SEARCH_SQL = `
  SELECT timestamp, process_name, server_name, tool_name, method,
         decision, duration_ms, bytes, arguments, response_preview,
         error_message, source
  FROM (
    SELECT mc.timestamp, NULL as process_name, 'local' as server_name,
           tc.tool_name, NULL as method, 'allowed' as decision,
           mc.duration_ms,
           COALESCE(LENGTH(tc.arguments), 0) as bytes,
           tc.arguments, tr.content_preview as response_preview,
           NULL as error_message, 'native' as source
    FROM tool_calls tc
    JOIN model_calls mc ON tc.model_call_id = mc.id
    LEFT JOIN tool_responses tr ON tc.call_id = tr.call_id
    WHERE tc.origin = 'native'
    UNION ALL
    SELECT timestamp, process_name, server_name, tool_name, method,
           decision, duration_ms,
           COALESCE(LENGTH(request_preview), 0) + COALESCE(LENGTH(response_preview), 0) as bytes,
           request_preview as arguments, response_preview,
           error_message, 'mcp' as source
    FROM mcp_calls
  )
  WHERE tool_name LIKE ? OR method LIKE ? OR server_name LIKE ? OR process_name LIKE ?
  ORDER BY timestamp DESC
`;

// -- AI tab (charts) -------------------------------------------------------

export const AI_USAGE_PER_PROVIDER_SQL = `
  SELECT provider,
    COALESCE(SUM(input_tokens), 0) as input_tokens,
    COALESCE(SUM(output_tokens), 0) as output_tokens,
    COALESCE(SUM(estimated_cost_usd), 0.0) as cost,
    COUNT(*) as call_count
  FROM model_calls
  GROUP BY provider
  ORDER BY cost DESC
`;

export const AI_TOKENS_OVER_TIME_SQL = `
  WITH numbered AS (
    SELECT id, provider,
      COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0) as tokens,
      (ROW_NUMBER() OVER (ORDER BY id) - 1) / 5 as bucket
    FROM model_calls
  )
  SELECT bucket, provider, SUM(tokens) as tokens
  FROM numbered
  GROUP BY bucket, provider
  ORDER BY bucket, provider
`;

export const AI_TOKENS_OVER_TIME_BY_MODEL_SQL = `
  SELECT id as bucket,
    COALESCE(model, 'unknown') as model,
    COALESCE(provider, 'unknown') as provider,
    COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0) as tokens
  FROM model_calls
  WHERE COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0) > 0
  ORDER BY id
`;

export const AI_COST_OVER_TIME_SQL = `
  WITH numbered AS (
    SELECT id, provider,
      COALESCE(estimated_cost_usd, 0.0) as cost,
      (ROW_NUMBER() OVER (ORDER BY id) - 1) / 5 as bucket
    FROM model_calls
  )
  SELECT bucket, provider, SUM(cost) as cost
  FROM numbered
  GROUP BY bucket, provider
  ORDER BY bucket, provider
`;

export const AI_MODEL_USAGE_SQL = `
  SELECT model,
    COALESCE(provider, 'unknown') as provider,
    COALESCE(SUM(input_tokens), 0) as input_tokens,
    COALESCE(SUM(output_tokens), 0) as output_tokens,
    COALESCE(SUM(input_tokens), 0) + COALESCE(SUM(output_tokens), 0) as tokens,
    COALESCE(SUM(estimated_cost_usd), 0.0) as cost,
    COUNT(*) as call_count
  FROM model_calls
  GROUP BY model
  ORDER BY tokens DESC
`;

// -- Network tab (charts) --------------------------------------------------

export const NET_STATS_SQL = `
  SELECT
    COUNT(*) as total,
    SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END) as allowed,
    SUM(CASE WHEN decision != 'allowed' THEN 1 ELSE 0 END) as denied,
    COALESCE(AVG(duration_ms), 0) as avg_latency
  FROM net_events
`;

export const NET_REQUESTS_OVER_TIME_SQL = `
  WITH numbered AS (
    SELECT id, decision,
      (ROW_NUMBER() OVER (ORDER BY id) - 1) / 3 as bucket
    FROM net_events
  )
  SELECT bucket,
    SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END) as allowed,
    SUM(CASE WHEN decision != 'allowed' THEN 1 ELSE 0 END) as denied
  FROM numbered
  GROUP BY bucket
  ORDER BY bucket
`;

export const NET_METHODS_SQL = `
  SELECT COALESCE(method, 'CONNECT') as method, COUNT(*) as cnt
  FROM net_events
  GROUP BY method
  ORDER BY cnt DESC
`;

export const NET_TOP_DOMAINS_SQL = `
  SELECT domain,
    SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END) as allowed,
    SUM(CASE WHEN decision != 'allowed' THEN 1 ELSE 0 END) as denied
  FROM net_events
  GROUP BY domain
  ORDER BY COUNT(*) DESC
  LIMIT 8
`;

// -- Network tab (event list) ----------------------------------------------

export const NET_EVENTS_ALL_SQL = `
  SELECT id, timestamp, domain, port, decision, method, path, query,
         status_code, bytes_sent, bytes_received, duration_ms, matched_rule,
         request_headers, response_headers, request_body_preview, response_body_preview
  FROM net_events
  ORDER BY id DESC
`;

export const NET_EVENTS_SEARCH_SQL = `
  SELECT id, timestamp, domain, port, decision, method, path, query,
         status_code, bytes_sent, bytes_received, duration_ms, matched_rule,
         request_headers, response_headers, request_body_preview, response_body_preview
  FROM net_events
  WHERE domain LIKE ? OR path LIKE ? OR method LIKE ?
  ORDER BY id DESC
`;

// -- Files tab (charts) ----------------------------------------------------

export const FILE_STATS_SQL = `
  SELECT
    COUNT(*) as total,
    SUM(CASE WHEN action = 'created' THEN 1 ELSE 0 END) as created,
    SUM(CASE WHEN action = 'modified' THEN 1 ELSE 0 END) as modified,
    SUM(CASE WHEN action = 'deleted' THEN 1 ELSE 0 END) as deleted
  FROM fs_events
`;

export const FILE_ACTIONS_SQL = `
  SELECT action, COUNT(*) as cnt
  FROM fs_events
  GROUP BY action
  ORDER BY cnt DESC
`;

export const FILE_EVENTS_OVER_TIME_SQL = `
  WITH numbered AS (
    SELECT id, action,
      (ROW_NUMBER() OVER (ORDER BY id) - 1) / 10 as bucket
    FROM fs_events
  )
  SELECT bucket, action, COUNT(*) as cnt
  FROM numbered
  GROUP BY bucket, action
  ORDER BY bucket, action
`;

// -- Files tab (event list) ------------------------------------------------

export const FILE_EVENTS_ALL_SQL = `
  SELECT id, timestamp, action, path, size
  FROM fs_events
  ORDER BY id DESC
`;

export const FILE_EVENTS_SEARCH_SQL = `
  SELECT id, timestamp, action, path, size
  FROM fs_events
  WHERE path LIKE ?
  ORDER BY id DESC
`;

// -- Snapshots tab -----------------------------------------------------------

export const SNAPSHOT_STATS_SQL = `
  SELECT
    COUNT(*) as total,
    SUM(CASE WHEN origin = 'auto' THEN 1 ELSE 0 END) as auto_count,
    SUM(CASE WHEN origin = 'manual' THEN 1 ELSE 0 END) as manual_count
  FROM snapshot_events
  WHERE id IN (SELECT MAX(id) FROM snapshot_events GROUP BY slot)
`;

export const SNAPSHOT_LIST_SQL = `
  SELECT s.id, s.timestamp, s.slot, s.origin, s.name, s.files_count,
    s.start_fs_event_id, s.stop_fs_event_id,
    (SELECT COUNT(*) FROM fs_events
     WHERE id > s.start_fs_event_id AND id <= s.stop_fs_event_id
     AND action = 'created') as created,
    (SELECT COUNT(*) FROM fs_events
     WHERE id > s.start_fs_event_id AND id <= s.stop_fs_event_id
     AND action = 'modified') as modified,
    (SELECT COUNT(*) FROM fs_events
     WHERE id > s.start_fs_event_id AND id <= s.stop_fs_event_id
     AND action = 'deleted') as deleted
  FROM snapshot_events s
  WHERE s.id IN (
    SELECT MAX(id) FROM snapshot_events GROUP BY slot
  )
  ORDER BY s.timestamp DESC
`;

// -- Inspector preset queries -----------------------------------------------

import type { PresetQuery } from './types';

export const PRESET_QUERIES: PresetQuery[] = [
  { label: 'Recent events', sql: 'SELECT timestamp, event_type, summary FROM event_log ORDER BY timestamp DESC LIMIT 20' },
  { label: 'HTTP requests', sql: 'SELECT method, url, status_code, decision, duration_ms FROM http_requests ORDER BY timestamp DESC LIMIT 20' },
  { label: 'Tool calls', sql: 'SELECT tool_name, server, duration_ms, timestamp FROM tool_calls ORDER BY timestamp DESC LIMIT 20' },
  { label: 'Model calls', sql: 'SELECT provider, model, input_tokens, output_tokens, estimated_cost_usd FROM model_calls ORDER BY timestamp DESC' },
  { label: 'File events', sql: 'SELECT path, operation, size_bytes, timestamp FROM file_events ORDER BY timestamp DESC LIMIT 20' },
];

/**
 * Validate that a SQL query is a SELECT statement (not INSERT/UPDATE/DELETE/DROP/etc).
 * Returns null if valid, or an error message if rejected.
 */
export function validateSelectOnly(sql: string): string | null {
  const trimmed = sql.trim();
  if (!trimmed) return 'Query is empty';

  const stripped = trimmed
    .replace(/--[^\n]*/g, '')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .trim();

  if (!stripped) return 'Query is empty (only comments)';

  if (!/^SELECT\b/i.test(stripped)) {
    return 'Only SELECT queries are allowed';
  }

  const dangerous = /\b(INSERT|UPDATE|DELETE|DROP|ALTER|CREATE|TRUNCATE|REPLACE|ATTACH|DETACH|PRAGMA)\b/i;
  if (dangerous.test(stripped)) {
    return 'Query contains forbidden keyword';
  }

  return null;
}
