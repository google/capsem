//! The polled session aggregates must run on indexes, not on table scans.
//!
//! `GET /vms/{id}/stats/summary` is asked for on a timer, per VM, by both the
//! TUI and the desktop UI, and the service answers it by reading the ledger
//! file -- there is no RAM mirror of the hot tables any more. So a full scan
//! here is not a one-off cost paid by one request; it is every poll of every
//! running session reading every captured byte of `net_events` and
//! `model_calls`, the two widest tables in the ledger.
//!
//! This guard asks SQLite how it intends to run each statement and fails on
//! any `SCAN` that names no index. It runs against the same reader shape the
//! service uses -- `open_disk_only`, reading `main` through WAL -- because the
//! mirrored in-process reader resolves different tables and would answer a
//! different question.
//!
//! `security/status`'s aggregates are not guarded here. Their SQL belongs to
//! `capsem-service`, which owns the route's query intent, and
//! `security_status_aggregates_run_on_indexes` there runs the exact strings
//! the route sends. A copy of them here would be a second guard over
//! statements that are not the ones in production, and would go on passing
//! while the real ones drifted.

use rusqlite::Connection;

use super::super::session_stats::{tool_calls_sql, MODEL_TOTALS_SQL, NET_TOTALS_SQL};
use super::super::DbReader;
use crate::schema;

/// Enough rows that a scan is a decision SQLite would regret, and few enough
/// that the fixture builds in well under a second.
const ROWS_PER_TABLE: usize = 2_500;

fn ledger_with_rows() -> (tempfile::TempDir, DbReader) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session.db");
    let conn = Connection::open(&path).expect("open ledger");
    schema::apply_pragmas(&conn).expect("pragmas");
    schema::create_tables(&conn).expect("create tables");
    conn.execute_batch("BEGIN").expect("begin");
    for row in 0..ROWS_PER_TABLE {
        let decision = ["allowed", "denied", "error"][row % 3];
        conn.execute(
            "INSERT INTO net_events (timestamp, domain, port, decision, bytes_sent, bytes_received, duration_ms)
             VALUES ('2026-01-01T00:00:00Z', 'example.com', 443, ?, ?, ?, 1)",
            rusqlite::params![decision, row as i64, row as i64],
        )
        .expect("insert net event");
        conn.execute(
            "INSERT INTO model_calls (timestamp, provider, method, path, input_tokens, output_tokens,
                                      duration_ms, estimated_cost_usd, usage_details, text_content)
             VALUES ('2026-01-01T00:00:00Z', 'anthropic', 'POST', '/v1/messages', ?, ?, 10, 0.001, ?, ?)",
            rusqlite::params![
                row as i64,
                row as i64,
                if row % 2 == 0 { Some(r#"{"thinking":7}"#) } else { None },
                "x".repeat(512),
            ],
        )
        .expect("insert model call");
        conn.execute(
            "INSERT INTO tool_calls (timestamp, call_index, call_id, tool_name, origin)
             VALUES ('2026-01-01T00:00:00Z', ?, 'call', 'Read', ?)",
            rusqlite::params![row as i64, ["native", "mcp", "builtin", "local"][row % 4]],
        )
        .expect("insert tool call");
    }
    conn.execute_batch("COMMIT").expect("commit");
    drop(conn);
    let reader = DbReader::open_disk_only(&path).expect("open disk-only reader");
    (dir, reader)
}

/// `EXPLAIN QUERY PLAN` for one statement, one line per plan node.
fn plan(reader: &DbReader, sql: &str) -> Vec<String> {
    let mut stmt = reader
        .conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare plan");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(3))
        .expect("query plan")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("read plan");
    rows
}

/// Every `SCAN` in a plan that reads a table without an index.
///
/// Judged on the node, not on a list of table names: SQLite reports a table
/// by the alias the statement gives it, so a name list is blind to exactly
/// the aliased subqueries -- `mc2` in the model totals -- that are easiest to
/// break. What is exempt is what is not a table at all: a co-routine or
/// materialized subquery the plan itself declared, a virtual table such as
/// `json_each`, and the constant row of a table-less `SELECT`.
fn unindexed_scans(plan: &[String]) -> Vec<&String> {
    let derived: Vec<&str> = plan
        .iter()
        .filter_map(|node| {
            let node = node.trim();
            node.strip_prefix("CO-ROUTINE ")
                .or_else(|| node.strip_prefix("MATERIALIZE "))
                .and_then(|rest| rest.split_whitespace().next())
        })
        .collect();
    plan.iter()
        .filter(|node| {
            let Some(rest) = node.trim().strip_prefix("SCAN ") else {
                return false;
            };
            let target = rest.split_whitespace().next().unwrap_or_default();
            !rest.contains("USING INDEX")
                && !rest.contains("USING COVERING INDEX")
                && !rest.contains("VIRTUAL TABLE")
                && rest != "CONSTANT ROW"
                && !derived.contains(&target)
        })
        .collect()
}

/// Fail naming every plan node that scans a table with no index.
///
/// The message carries the whole plan, because the useful thing to know when
/// this fires is not that some line was bad but which index the statement
/// stopped being able to use.
fn assert_no_unindexed_scan(label: &str, plan: &[String]) {
    let offenders = unindexed_scans(plan);
    assert!(
        offenders.is_empty(),
        "{label} scans a table with no index: {offenders:?}\nfull plan:\n  {}\n\
         This route is polled per VM on a timer and the service reads the file, so a scan here \
         is every poll of every running session reading every captured byte of that table. \
         Add the index the plan wants in schema/ddl.rs, or -- if the aggregate genuinely cannot \
         be indexed -- say so here with the reason.",
        plan.join("\n  ")
    );
}

#[test]
fn session_stats_aggregates_run_on_indexes() {
    let (_dir, reader) = ledger_with_rows();
    for (label, sql) in [
        ("net totals", NET_TOTALS_SQL.to_string()),
        ("model totals", MODEL_TOTALS_SQL.to_string()),
        ("tool call count", tool_calls_sql()),
    ] {
        assert_no_unindexed_scan(label, &plan(&reader, &sql));
    }
}

/// The rule itself, against plans SQLite really produced.
///
/// The model-totals plan below is the one it printed with
/// `idx_model_calls_usage_details` dropped: the aliased `mc2` scan is the
/// finding, and the co-routine `je` and the `json_each` virtual table are not.
/// A guard that matched table names passed this plan.
#[test]
fn an_aliased_scan_is_a_finding_and_a_derived_one_is_not() {
    let plan: Vec<String> = [
        "SCAN model_calls USING COVERING INDEX idx_model_calls_usage_totals",
        "SCALAR SUBQUERY 2",
        "CO-ROUTINE je",
        "SCAN mc2",
        "SCAN je VIRTUAL TABLE INDEX 1:",
        "USE TEMP B-TREE FOR GROUP BY",
        "SCAN je",
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(unindexed_scans(&plan), vec!["SCAN mc2"]);
}
