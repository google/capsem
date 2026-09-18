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

impl DbReader {
    /// Compute aggregate session statistics from all tables.
    pub fn session_stats(&self) -> rusqlite::Result<SessionStats> {
        // Net event aggregates.
        let (net_total, net_allowed, net_denied, net_error, net_bytes_sent, net_bytes_received) =
            self.conn.query_row(NET_TOTALS_SQL, [], |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                    row.get::<_, i64>(5)? as u64,
                ))
            })?;

        // Model call aggregates.
        let (
            model_call_count,
            total_input_tokens,
            total_output_tokens,
            total_model_duration_ms,
            total_estimated_cost_usd,
            usage_details_json,
        ) = self.conn.query_row(MODEL_TOTALS_SQL, [], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u64,
                row.get::<_, i64>(3)? as u64,
                row.get::<_, f64>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;

        let total_usage_details: BTreeMap<String, u64> = usage_details_json
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        // Total tool calls.
        let total_tool_calls: u64 = self
            .conn
            .query_row(&tool_calls_sql(), [], |row| row.get::<_, i64>(0).map(|n| n as u64))?;

        Ok(SessionStats {
            net_total,
            net_allowed,
            net_denied,
            net_error,
            net_bytes_sent,
            net_bytes_received,
            model_call_count,
            total_input_tokens,
            total_output_tokens,
            total_usage_details,
            total_model_duration_ms,
            total_tool_calls,
            total_estimated_cost_usd,
        })
    }
}
