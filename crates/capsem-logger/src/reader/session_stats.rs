//! The compact session aggregates behind `GET /vms/{id}/stats/summary`.
//!
//! This is a polled route: the TUI and the desktop UI ask for it on a timer
//! for every running VM, and the service reads the ledger file rather than a
//! RAM mirror of it. So each statement here is named, and
//! `reader/tests/query_plan.rs` runs `EXPLAIN QUERY PLAN` over these exact
//! strings and fails on any full table scan. Naming them is what makes that
//! guard honest: a test that retyped the SQL would go on passing while the
//! query it guards drifted away from it.
//!
//! `net_events` and `model_calls` are the two widest tables in the ledger --
//! headers, body previews, assistant text -- so the difference between a table
//! scan and a covering index scan here is the difference between reading every
//! captured byte of a session and reading the handful of counters this route
//! actually sums.

use std::collections::BTreeMap;

use super::{DbReader, SessionStats, TOOL_CALL_LEDGER_FILTER};

/// Request counts, decision split, and byte totals over the network ledger.
///
/// Covered by `idx_net_events_decision_bytes`.
pub(crate) const NET_TOTALS_SQL: &str = "SELECT
                    COUNT(*),
                    COALESCE(SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN decision = 'error' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(bytes_sent), 0),
                    COALESCE(SUM(bytes_received), 0)
                 FROM net_events";

/// Call count, token and cost totals, and the merged per-key usage details.
///
/// The outer aggregate is covered by `idx_model_calls_usage_totals`; the
/// `json_each` subquery walks `idx_model_calls_usage_details`, which is
/// partial on `usage_details IS NOT NULL` -- exactly the rows it wants.
pub(crate) const MODEL_TOTALS_SQL: &str = "SELECT
                    COUNT(*),
                    COALESCE(SUM(COALESCE(input_tokens, 0)), 0),
                    COALESCE(SUM(COALESCE(output_tokens, 0)), 0),
                    COALESCE(SUM(duration_ms), 0),
                    COALESCE(SUM(estimated_cost_usd), 0.0),
                    (SELECT json_group_object(je.key, je.total) FROM (
                        SELECT je.key, SUM(je.value) as total
                        FROM model_calls mc2, json_each(mc2.usage_details) je
                        WHERE mc2.usage_details IS NOT NULL
                        GROUP BY je.key
                    ) je)
                 FROM model_calls";

/// How many tool calls this session made, counting only the origins the
/// ledger presents as tool calls.
///
/// Covered by `idx_tool_calls_origin`, which turns the filter into an index
/// seek per origin rather than a scan of a table whose rows carry arguments
/// and response previews.
pub(crate) fn tool_calls_sql() -> String {
    format!("SELECT COUNT(*) FROM tool_calls WHERE {TOOL_CALL_LEDGER_FILTER}")
}

/// The three statements as one batch for `DbHandle::query_many`.
///
/// One batch, not three queries, and not a request of its own: the handle
/// caches a batch whole and keyed by its statements, so an idle session's
/// `stats/summary` poll is answered without the reader thread touching the
/// file. A private worker request would have needed its own copy of the
/// freshness protocol -- observe `data_version`, decide, commit the
/// observation -- and a second copy of that is a second chance to get it
/// wrong.
pub(crate) fn session_stats_batch() -> Vec<(String, Vec<serde_json::Value>)> {
    vec![
        (NET_TOTALS_SQL.to_string(), Vec::new()),
        (MODEL_TOTALS_SQL.to_string(), Vec::new()),
        (tool_calls_sql(), Vec::new()),
    ]
}

/// The single row of a `{"columns":[...],"rows":[[...]]}` result.
///
/// Each of these statements is an unfiltered aggregate, so SQLite returns one
/// row even for an empty ledger. No row at all is a broken result, not a
/// session that recorded nothing, and says so rather than reporting zeros.
fn aggregate_row(raw: &str, label: &str) -> Result<Vec<serde_json::Value>, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|error| format!("{label} returned invalid json: {error}"))?;
    parsed
        .get("rows")
        .and_then(serde_json::Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(serde_json::Value::as_array)
        .cloned()
        .ok_or_else(|| format!("{label} returned no aggregate row"))
}

fn count_at(row: &[serde_json::Value], index: usize, label: &str) -> Result<u64, String> {
    row.get(index)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("{label} column {index} is not a count"))
}

impl SessionStats {
    /// Read the aggregates back out of one `session_stats_batch` result.
    ///
    /// This is the only place the columns are named, so the direct reader and
    /// the cached handle cannot drift into reporting different numbers for the
    /// same ledger.
    pub(crate) fn from_query_batch(raw: &[String]) -> Result<Self, String> {
        let [net, model, tools] = raw else {
            return Err(format!(
                "session stats batch returned {} results, expected 3",
                raw.len()
            ));
        };
        let net = aggregate_row(net, "net totals")?;
        let model = aggregate_row(model, "model totals")?;
        let tools = aggregate_row(tools, "tool call count")?;
        let total_usage_details: BTreeMap<String, u64> = model
            .get(5)
            .and_then(serde_json::Value::as_str)
            .and_then(|merged| serde_json::from_str(merged).ok())
            .unwrap_or_default();
        Ok(Self {
            net_total: count_at(&net, 0, "net totals")?,
            net_allowed: count_at(&net, 1, "net totals")?,
            net_denied: count_at(&net, 2, "net totals")?,
            net_error: count_at(&net, 3, "net totals")?,
            net_bytes_sent: count_at(&net, 4, "net totals")?,
            net_bytes_received: count_at(&net, 5, "net totals")?,
            model_call_count: count_at(&model, 0, "model totals")?,
            total_input_tokens: count_at(&model, 1, "model totals")?,
            total_output_tokens: count_at(&model, 2, "model totals")?,
            total_usage_details,
            total_model_duration_ms: count_at(&model, 3, "model totals")?,
            total_tool_calls: count_at(&tools, 0, "tool call count")?,
            total_estimated_cost_usd: model
                .get(4)
                .and_then(serde_json::Value::as_f64)
                .ok_or_else(|| "model totals column 4 is not a cost".to_string())?,
        })
    }
}

impl DbReader {
    /// Compute aggregate session statistics from all tables.
    ///
    /// The handle reads these through `query_many` so an unchanged ledger is
    /// answered from its cache; this runs the same statements directly, for
    /// callers that already hold a reader.
    pub fn session_stats(&self) -> Result<SessionStats, String> {
        let raw = session_stats_batch()
            .iter()
            .map(|(sql, params)| self.query_raw_with_params(sql, params))
            .collect::<Result<Vec<String>, String>>()?;
        SessionStats::from_query_batch(&raw)
    }
}
