//! What the polled session-stats read costs as a ledger grows.
//!
//! `stats/summary`, `/info`, `/vms/list` and the status routes read a
//! session's totals on a timer. They used to aggregate the ledger's tables
//! on every poll after a commit; they now read the writer's counter snapshot
//! by primary key (#223). Measured at 20k and 1M rows of network, model and
//! tool-call traffic, idle (nothing committed since the last poll) and right
//! after a commit (the handle's cache has to go back to the file). A flat
//! line across sizes is the claim.
//!
//! The same file measures the pre-#223 aggregate when its one poll call is
//! switched to `session_stats()` on a commit that still has it.

use std::path::Path;
use std::time::{Duration, SystemTime};

use capsem_logger::{DbHandle, DbWriter, Decision, NetEvent, WriteOp};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use rusqlite::{params, Connection};

const SIZES: &[(usize, &str)] = &[(20_000, "20k"), (1_000_000, "1m")];

fn net_event(idx: usize) -> WriteOp {
    WriteOp::NetEvent(NetEvent {
        event_id: Some(format!("{idx:012x}")),
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(idx as u64),
        domain: "stats.example".into(),
        port: 443,
        decision: Decision::Allowed,
        process_name: None,
        pid: None,
        method: Some("GET".into()),
        path: Some("/".into()),
        query: None,
        status_code: Some(200),
        bytes_sent: 10,
        bytes_received: 20,
        duration_ms: 1,
        matched_rule: None,
        request_headers: None,
        response_headers: None,
        request_body: None,
        response_body: None,
        conn_type: None,
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: None,
        credential_ref: None,
    })
}

/// Seed `rows` rows split across the three tables the old summary summed,
/// straight into the file while no writer holds it. Setup only: a million
/// rows through the writer queue would measure the writer, not the poll.
fn seed(path: &Path, rows: usize) {
    // A writer creates the ledger whole -- schema, archive generation and
    // lock -- so the rows seeded below land in a ledger any writer reopens.
    DbWriter::open(path, 1).expect("create ledger").shutdown_blocking();
    let mut conn = Connection::open(path).expect("open seed db");
    let tx = conn.transaction().expect("seed transaction");
    for idx in 0..rows {
        let id = format!("{idx:012x}");
        match idx % 3 {
            0 => tx.execute(
                "INSERT INTO net_events (event_id, timestamp, domain, port, decision, bytes_sent, bytes_received)
                 VALUES (?1, '1970-01-01T00:00:00Z', 'stats.example', 443, 'allowed', 10, 20)",
                params![id],
            ),
            1 => tx.execute(
                "INSERT INTO model_calls (event_id, timestamp, provider, model, method, path, input_tokens,
                                          output_tokens, estimated_cost_usd, usage_details)
                 VALUES (?1, '1970-01-01T00:00:00Z', 'anthropic', 'm', 'POST', '/v1/messages', 100, 20, 0.001,
                         '{\"thinking\":3}')",
                params![id],
            ),
            _ => tx.execute(
                "INSERT INTO tool_calls (event_id, call_index, call_id, tool_name, origin)
                 VALUES (?1, 0, ?1, 'bash', 'native')",
                params![id],
            ),
        }
        .expect("insert seed row");
    }
    tx.commit().expect("commit seed rows");
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("checkpoint seed rows");
}

fn session_stats_poll(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let mut group = c.benchmark_group("session_stats_poll");
    group.sample_size(20);
    for &(rows, label) in SIZES {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.db");
        seed(&path, rows);
        let writer = DbWriter::open(&path, 512).expect("writer");
        let reader = DbHandle::open_external_reader(&path).expect("external reader");
        rt.block_on(reader.ready()).expect("ready");
        let mut next = rows;
        let poll = || rt.block_on(reader.ledger_counters()).expect("stats");

        group.bench_function(format!("idle_{label}"), |b| b.iter(poll));
        group.bench_function(format!("after_write_{label}"), |b| {
            b.iter_batched(
                || {
                    next += 1;
                    rt.block_on(async {
                        writer.write(net_event(next)).await;
                        writer.flush().await;
                    });
                },
                |()| poll(),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, session_stats_poll);
criterion_main!(benches);
