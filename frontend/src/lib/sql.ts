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
