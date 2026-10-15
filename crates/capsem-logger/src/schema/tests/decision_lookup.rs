use super::*;
use crate::schema::memory_sync::COPY_SECURITY_DECISION_OCCURRENCES;
use rusqlite::StatementStatus;

const DISTINCT_SHAPES: usize = 1024;
const NULLABLE_VALUES: [Option<&str>; 6] = [None, Some(""), Some("N"), Some("S"), Some("a\0b"), Some("été")];

fn seed_decisions(conn: &Connection) {
    let mut insert = conn
        .prepare(
            "INSERT INTO mem.security_decision_events (
                timestamp_unix_ms, event_id, event_type, stage, actor, rule_id, plugin_id,
                previous_decision, requested_decision, effective_decision, reason, trace_id, turn_id
             ) VALUES (?1, ?2, 'http.request', 'rule', 'rules.http', ?3, ?4,
                       'allow', 'block', 'block', ?5, ?6, ?7)",
        )
        .unwrap();
    for idx in 0..DISTINCT_SHAPES {
        let rule_id = NULLABLE_VALUES[idx % NULLABLE_VALUES.len()];
        let plugin_id = NULLABLE_VALUES[(idx / NULLABLE_VALUES.len()) % NULLABLE_VALUES.len()];
        let reason = if idx < NULLABLE_VALUES.len() {
            NULLABLE_VALUES[idx].map(str::to_owned)
        } else {
            Some(format!("reason-{idx}: été\0tail"))
        };
        insert
            .execute(rusqlite::params![
                idx as i64,
                format!("{idx:012x}"),
                rule_id,
                plugin_id,
                reason,
                format!("trace-{idx}"),
                format!("turn-{idx}")
            ])
            .unwrap();
    }
}

fn database(name: &str) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    create_memory_tables(&conn, &memory_uri_for_name(name)).unwrap();
    seed_decisions(&conn);
    conn
}

#[test]
fn occurrence_copy_uses_complete_decision_shape_index() {
    let conn = database("occurrence_copy_uses_complete_decision_shape_index");
    // Populate the real indexed run table independently of the lookup under
    // test. Each reason is distinct within the same event/stage/actor bucket.
    conn.execute_batch(
        "INSERT INTO main.security_decision_runs (
            event_type, stage, actor, rule_id, plugin_id,
            previous_decision, requested_decision, effective_decision, reason,
            count, first_timestamp_unix_ms, last_timestamp_unix_ms
         ) SELECT event_type, stage, actor, rule_id, plugin_id,
                  previous_decision, requested_decision, effective_decision, reason,
                  1, timestamp_unix_ms, timestamp_unix_ms
           FROM mem.security_decision_events",
    )
    .unwrap();
    let plan = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {COPY_SECURITY_DECISION_OCCURRENCES}"))
        .unwrap()
        .query_map([0], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut copy = conn.prepare(COPY_SECURITY_DECISION_OCCURRENCES).unwrap();
    assert_eq!(copy.execute([0]).unwrap(), DISTINCT_SHAPES);
    let steps = copy.get_status(StatementStatus::VmStep);
    println!("decision occurrence copy: {steps} VM steps; {plan:?}");
    // A prefix-only seek scans the growing decision bucket for every row.
    // Count actual SQLite instructions instead of imposing a noisy wall clock.
    // Allow roughly twice the indexed copy's per-row work for SQLite changes.
    assert!(
        steps < (DISTINCT_SHAPES * 512) as i32,
        "occurrence copy must seek the entire shape, not scan matching event/stage/actor rows: {steps} steps; {plan:?}"
    );
    let lookup = plan
        .iter()
        .find(|line| line.contains("idx_security_decision_runs_shape"))
        .expect("the actual correlated copy uses the decision shape index");
    assert_eq!(lookup.matches("<expr>=?").count(), 3, "{lookup}");
    assert!(lookup.contains("effective_decision=?"), "{lookup}");
}

#[test]
fn indexed_decision_copy_preserves_nullable_metadata_and_atomic_counts() {
    let mut conn = database("indexed_decision_copy_preserves_nullable_metadata_and_atomic_counts");
    let tables = ["security_decision_events"];
    let watermarks = initial_memory_flush_watermarks(&conn, tables).unwrap();
    {
        let tx = conn.transaction().unwrap();
        flush_memory_tables_to_disk(&tx, tables, &watermarks).unwrap();
        tx.rollback().unwrap();
    }
    assert_eq!(
        max_table_id(&conn, MEMORY_SCHEMA, tables[0]).unwrap(),
        DISTINCT_SHAPES as i64
    );
    assert_eq!(max_table_id(&conn, "main", tables[0]).unwrap(), 0);
    assert_eq!(max_table_id(&conn, "main", "security_decision_runs").unwrap(), 0);

    // Keep the expected metadata in a separate fixture table before the real
    // transaction compacts and removes the memory occurrences.
    conn.execute_batch("CREATE TEMP TABLE expected AS SELECT * FROM mem.security_decision_events")
        .unwrap();
    let tx = conn.transaction().unwrap();
    let advanced = flush_memory_tables_to_disk(&tx, tables, &watermarks).unwrap();
    tx.commit().unwrap();
    assert_eq!(advanced[tables[0]], DISTINCT_SHAPES as i64);
    assert_eq!(max_table_id(&conn, MEMORY_SCHEMA, tables[0]).unwrap(), 0);
    let identical: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM main.security_decision_events AS event
             JOIN main.security_decision_runs AS run ON run.id = event.run_id
             JOIN expected ON expected.id = event.id
             WHERE event.event_id = expected.event_id
               AND event.timestamp_unix_ms = expected.timestamp_unix_ms
               AND event.event_type = expected.event_type
               AND event.trace_id = expected.trace_id AND event.turn_id = expected.turn_id
               AND run.event_type = expected.event_type
               AND run.stage = expected.stage AND run.actor = expected.actor
               AND run.rule_id IS expected.rule_id AND run.plugin_id IS expected.plugin_id
               AND run.previous_decision = expected.previous_decision
               AND run.requested_decision = expected.requested_decision
               AND run.effective_decision = expected.effective_decision
               AND run.reason IS expected.reason
               AND run.count = 1 AND run.first_timestamp_unix_ms = expected.timestamp_unix_ms
               AND run.last_timestamp_unix_ms = expected.timestamp_unix_ms",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(identical, DISTINCT_SHAPES as i64);
    // Repeating a successful flush neither duplicates occurrences nor counts.
    flush_memory_tables_to_disk(&conn, tables, &advanced).unwrap();
    let totals: (i64, i64) = conn
        .query_row("SELECT COUNT(*), SUM(count) FROM security_decision_runs", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(totals, (DISTINCT_SHAPES as i64, DISTINCT_SHAPES as i64));
}
