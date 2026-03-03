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
  SELECT COUNT(*) as cnt FROM tool_calls
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
         input_tokens, output_tokens, duration_ms, estimated_cost_usd, stop_reason
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
           tc.arguments, NULL as response_preview,
           NULL as error_message, 'native' as source
    FROM tool_calls tc
    JOIN model_calls mc ON tc.model_call_id = mc.id
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
           tc.arguments, NULL as response_preview,
           NULL as error_message, 'native' as source
    FROM tool_calls tc
    JOIN model_calls mc ON tc.model_call_id = mc.id
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

// -- Network tab -----------------------------------------------------------

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

// -- Files tab -------------------------------------------------------------

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
