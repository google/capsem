//! `security/status` is polled, so its SQL has to be SQL an index can serve.
//!
//! The statements are owned here, not in `capsem-logger` -- a route owns its
//! query intent -- so the guard that they stay indexed is owned here too,
//! running against the exact strings the route sends.
//!
//! "Uses some index" is not enough to hold on to: drop
//! `idx_security_rule_events_rule_stats` and the per-rule breakdown still
//! uses one -- `idx_security_rule_events_rule_id`, non-covering, with three
//! temp B-trees -- so each statement also names the covering index it has to
//! be answered from. An index nobody pins is one whose loss nobody notices,
//! and every index is paid for on every insert into this table.
//!
//! Rewriting one of these statements is the easy way to lose that shape. A
//! `GROUP BY` on a column with no index, or a correlated lookup whose
//! equalities stop lining up with the index's leading columns, both turn a
//! poll of a busy session into a scan of its whole security ledger -- and
//! neither shows up as anything but a slower page.

use crate::ledger_routes::security::security_stats_batch;

/// Enough rows that the plan is worth asking about, and few enough that the
/// fixture is written in a fraction of a second.
const SECURITY_ROWS: usize = 400;

#[tokio::test]
async fn security_status_aggregates_run_on_indexes() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(&session_dir).unwrap();
    let db_path = session_dir.join("session.db");

    let writer_path = db_path.clone();
    tokio::task::spawn_blocking(move || {
        let writer = capsem_logger::DbWriter::open(&writer_path, 64).unwrap();
        for row in 0..SECURITY_ROWS {
            writer.write_blocking(capsem_logger::WriteOp::SecurityRuleEvent(
                capsem_logger::SecurityRuleEvent::new(
                    1_789_000_000_000 + row as i64,
                    format!("{row:012x}"),
                    "model.call",
                    format!("profiles.rules.r{}", row % 13),
                    "{}",
                    "{}",
                )
                .with_rule_action(
                    [
                        capsem_logger::SecurityRuleAction::Allow,
                        capsem_logger::SecurityRuleAction::Ask,
                        capsem_logger::SecurityRuleAction::Block,
                    ][row % 3],
                )
                .with_detection_level(
                    [
                        capsem_logger::SecurityDetectionLevel::None,
                        capsem_logger::SecurityDetectionLevel::Low,
                        capsem_logger::SecurityDetectionLevel::High,
                    ][row % 3],
                ),
            ));
        }
        writer.shutdown_blocking();
    })
    .await
    .unwrap();

    let db = capsem_logger::DbHandle::open_external_reader(&db_path).unwrap();
    db.ready().await.unwrap();

    // Aggregates read the small counted-run index; only the latest-event
    // lookup reads occurrence rows. Pin both covering reads so a poll never
    // scans every occurrence in the session.
    const RUN_ACTION: &str = "idx_security_rule_runs_action";
    const RUN_EVENT_TYPE: &str = "idx_security_rule_runs_event_type";
    const OCCURRENCE_STATS: &str = "idx_security_rule_events_rule_stats";
    let pinned: [&[&str]; 5] = [
        &[RUN_ACTION],
        &[RUN_ACTION],
        &[RUN_EVENT_TYPE],
        &[RUN_ACTION],
        &[RUN_ACTION, OCCURRENCE_STATS],
    ];
    let batch = security_stats_batch();
    assert_eq!(
        batch.len(),
        pinned.len(),
        "pin an index for every security/status statement"
    );
    for ((sql, _), indexes) in batch.into_iter().zip(pinned) {
        let raw = db
            .query(&format!("EXPLAIN QUERY PLAN {sql}"), &[])
            .await
            .expect("read the query plan");
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let plan: Vec<String> = parsed["rows"]
            .as_array()
            .expect("plan rows")
            .iter()
            .map(|row| row[3].as_str().unwrap_or_default().to_string())
            .collect();
        // SQLite reports aliases (`sre`, `latest`) in the plan, so inspect
        // every table node rather than matching table names alone.
        let offenders: Vec<&String> = plan
            .iter()
            .filter(|node| {
                node.trim().starts_with("SCAN ")
                    && !node.contains("USING INDEX")
                    && !node.contains("USING COVERING INDEX")
            })
            .collect();
        assert!(
            offenders.is_empty(),
            "a security/status statement scans the whole security ledger: {offenders:?}\n\
             statement:{sql}\nfull plan:\n  {}\n\
             This route is polled per VM on a timer and the service reads session.db from disk, \
             so a scan here is every poll of every running session reading every matched rule it \
             ever recorded. Add the index the plan wants in capsem-logger's schema/ddl.rs, or \
             rewrite the statement so an existing one serves it.",
            plan.join("\n  ")
        );
        let table_nodes: Vec<&String> = plan
            .iter()
            .filter(|node| node.trim().starts_with("SCAN ") || node.trim().starts_with("SEARCH "))
            .collect();
        assert!(
            table_nodes.len() == indexes.len()
                && indexes.iter().all(|index| {
                    let covering = format!("USING COVERING INDEX {index}");
                    table_nodes.iter().any(|node| node.contains(&covering))
                }),
            "a security/status statement is no longer answered from {indexes:?}\n\
             statement:{sql}\nfull plan:\n  {}\n\
             Every table read here must be a covering read of that index. If the index was \
             dropped, or the statement no longer lines up with its columns, the plan falls back \
             to row lookups and sorts per poll of every running session.",
            plan.join("\n  ")
        );
    }
}
