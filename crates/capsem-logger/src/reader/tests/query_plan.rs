//! The polled session aggregates must run on indexes, not on table scans.
//!
//! `GET /vms/{id}/stats/summary` is asked for on a timer, per VM, by both the
//! TUI and the desktop UI, and the service answers it by reading the ledger
//! file -- there is no RAM mirror of the hot tables any more. So a full scan
//! here is not a one-off cost paid by one request; it is every poll of every
//! running session reading every captured byte of `net_events` and
//! `model_calls`, the two widest tables in the ledger.
//!
//! This guard asks SQLite how it intends to run each statement and fails on a
//! `SCAN` of a ledger table that names no index. It runs against the same
//! reader shape the service uses -- `open_disk_only`, reading `main` through
//! WAL -- because the mirrored in-process reader resolves different tables and
//! would answer a different question.

use rusqlite::Connection;

use super::super::session_stats::{tool_calls_sql, MODEL_TOTALS_SQL, NET_TOTALS_SQL};
use super::super::DbReader;
use crate::schema;

/// Enough rows that a scan is a decision SQLite would regret, and few enough
/// that the fixture builds in well under a second.
const ROWS_PER_TABLE: usize = 2_500;

/// The tables a plan is allowed to touch, and therefore the ones a bare
/// `SCAN` of is a finding. A name that is not a ledger table -- a co-routine,
/// a subquery result, `json_each` -- is not this guard's business.
const LEDGER_TABLES: &[&str] = &[
    "net_events",
    "model_calls",
    "tool_calls",
    "security_rule_events",
    "substitution_events",
];

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
        conn.execute(
            "INSERT INTO security_rule_events (timestamp_unix_ms, event_id, event_type, rule_id,
                                               rule_action, detection_level, rule_json)
             VALUES (?, ?, 'model.call', ?, ?, ?, '{}')",
            rusqlite::params![
                1_789_000_000_000_i64 + row as i64,
                format!("{:012x}", row),
                format!("profiles.rules.r{}", row % 17),
                ["allow", "ask", "block"][row % 3],
                ["none", "low", "high"][row % 3],
            ],
        )
        .expect("insert security rule event");
        conn.execute(
            "INSERT INTO substitution_events (timestamp, substitution_ref, material_class, provider, outcome,
                                             source, algorithm)
             VALUES ('2026-01-01T00:00:00Z', ?, 'credential', 'anthropic', 'injected', 'proxy', 'blake3')",
            rusqlite::params![format!("credential:blake3:{:064x}", row % 11)],
        )
        .expect("insert substitution event");
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

/// Fail naming every plan node that scans a ledger table with no index.
///
/// The message carries the whole plan, because the useful thing to know when
/// this fires is not that some line was bad but which index the statement
/// stopped being able to use.
fn assert_no_unindexed_scan(label: &str, plan: &[String]) {
    let offenders: Vec<&String> = plan
        .iter()
        .filter(|node| {
            let Some(rest) = node.trim().strip_prefix("SCAN ") else {
                return false;
            };
            let table = rest.split_whitespace().next().unwrap_or_default();
            LEDGER_TABLES.contains(&table) && !rest.contains("USING INDEX") && !rest.contains("USING COVERING INDEX")
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "{label} scans a ledger table with no index: {offenders:?}\nfull plan:\n  {}\n\
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

/// The security ledger's own aggregates, behind `GET /vms/{id}/security/status`.
///
/// The SQL these run is owned by `capsem-service`, which owns the route's
/// query intent; `capsem-service`'s own
/// `security_status_aggregates_run_on_indexes` guards those exact strings.
/// What is guarded here is the shape the ledger offers them: the indexes on
/// `security_rule_events` that let a grouped count be answered from an index,
/// and the correlated "latest match per rule" lookup be a seek rather than a
/// scan per group.
#[test]
fn security_rule_aggregates_run_on_indexes() {
    let (_dir, reader) = ledger_with_rows();
    for (label, sql) in [
        (
            "by action",
            "SELECT rule_action, COUNT(*) FROM security_rule_events GROUP BY rule_action".to_string(),
        ),
        (
            "by level",
            "SELECT detection_level, COUNT(*) FROM security_rule_events GROUP BY detection_level".to_string(),
        ),
        (
            "latest per rule",
            "SELECT sre.rule_id, (SELECT latest.event_id FROM security_rule_events latest \
             WHERE latest.rule_id = sre.rule_id AND latest.rule_action = sre.rule_action \
             AND latest.detection_level = sre.detection_level \
             ORDER BY latest.timestamp_unix_ms DESC, latest.id DESC LIMIT 1) \
             FROM security_rule_events sre GROUP BY sre.rule_id, sre.rule_action, sre.detection_level"
                .to_string(),
        ),
    ] {
        assert_no_unindexed_scan(label, &plan(&reader, &sql));
    }
}
