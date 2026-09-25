//! What a route pays to poll a ledger, idle and right after a commit.
//!
//! Two kinds of handle read a ledger. The service's session handles are
//! external readers: capsem-process owns the writes and disk is the boundary.
//! The service's own `main.db` and the network ledgers are owning handles:
//! the handle holds the writer. Both are measured at 20k rows and at 1M, the
//! size a long session reaches, because a poll whose cost grows with the
//! ledger is only visible on a large one.
//!
//! Two cases matter for each: a poll when nothing was committed since the last
//! one (the common UI case) and a poll right after a commit.

use std::path::Path;
use std::time::{Duration, SystemTime};

use capsem_logger::{DbHandle, DbWriter, Decision, DnsEvent, WriteOp};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use rusqlite::{params, Connection};

const SIZES: &[(usize, &str)] = &[(20_000, "20k"), (1_000_000, "1m")];

fn dns_event(idx: usize) -> WriteOp {
    WriteOp::DnsEvent(DnsEvent {
        event_id: Some(format!("{idx:012x}")),
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(idx as u64),
        qname: format!("poll-{idx}.example"),
        qtype: 1,
        qclass: 1,
        rcode: 0,
        answer_ip: Some("127.0.0.1".to_string()),
        decision: Decision::Allowed.as_str().to_string(),
        matched_rule: None,
        source_proto: Some("udp".to_string()),
        process_name: Some("reader-poll".to_string()),
        upstream_resolver_ms: 0,
        trace_id: Some(format!("{idx:016x}")),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        credential_ref: None,
    })
}

/// Seed `rows` DNS rows straight into the file while no writer holds it.
/// Setup only: a million rows through the writer queue would measure the
/// writer, not the poll.
fn seed_dns_rows(path: &Path, rows: usize) {
    // A writer creates the ledger whole -- schema, archive generation and
    // lock -- so the rows seeded below land in a ledger any writer reopens.
    DbWriter::open(path, 1).expect("create ledger").shutdown_blocking();
    let mut conn = Connection::open(path).expect("open seed db");
    let tx = conn.transaction().expect("seed transaction");
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO dns_events (
                    event_id, timestamp, qname, qtype, qclass, rcode, answer_ip,
                    decision, source_proto, process_name, upstream_resolver_ms, trace_id, turn_id
                 )
                 VALUES (?1, '1970-01-01T00:00:00Z', ?2, 1, 1, 0, '127.0.0.1', 'allowed', 'udp',
                         'reader-poll', 0, ?3, ?3)",
            )
            .expect("prepare seed insert");
        for idx in 0..rows {
            stmt.execute(params![
                format!("{idx:012x}"),
                format!("poll-{idx}.example"),
                format!("{idx:016x}"),
            ])
            .expect("insert seed row");
        }
    }
    tx.commit().expect("commit seed rows");
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("checkpoint seed rows");
}

const POLL_SQL: &str = "SELECT COUNT(*) FROM dns_events";

fn poll_batch() -> Vec<(String, Vec<serde_json::Value>)> {
    vec![(POLL_SQL.to_string(), Vec::new())]
}

fn external_reader_poll(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    for &(rows, label) in SIZES {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.db");
        seed_dns_rows(&path, rows);
        let writer = DbWriter::open(&path, 512).expect("writer");
        let reader = DbHandle::open_external_reader(&path).expect("external reader");
        rt.block_on(reader.ready()).expect("ready");
        let mut next = rows;

        c.bench_function(&format!("external_reader_poll_idle_{label}"), |b| {
            b.iter(|| rt.block_on(reader.query_many(poll_batch())).expect("query"));
        });

        c.bench_function(&format!("external_reader_poll_after_write_{label}"), |b| {
            b.iter_batched(
                || {
                    next += 1;
                    let event = dns_event(next);
                    rt.block_on(async {
                        writer.write(event).await;
                        writer.flush().await;
                    });
                },
                |()| rt.block_on(reader.query_many(poll_batch())).expect("query"),
                BatchSize::PerIteration,
            );
        });
    }
}

fn owning_handle_poll(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    for &(rows, label) in SIZES {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("main.db");
        seed_dns_rows(&path, rows);
        let handle = DbHandle::open(&path).expect("owning handle");
        rt.block_on(handle.ready()).expect("ready");
        let mut next = rows;

        c.bench_function(&format!("owning_handle_poll_idle_{label}"), |b| {
            b.iter(|| rt.block_on(handle.query_many(poll_batch())).expect("query"));
        });

        c.bench_function(&format!("owning_handle_poll_after_write_{label}"), |b| {
            b.iter_batched(
                || {
                    next += 1;
                    let event = dns_event(next);
                    rt.block_on(async {
                        handle.write(event).await.expect("write");
                        handle.flush().await.expect("flush");
                    });
                },
                |()| rt.block_on(handle.query_many(poll_batch())).expect("query"),
                BatchSize::PerIteration,
            );
        });
    }
}

/// What readiness costs a route, which is the part of a poll that must not
/// grow with the ledger: the first check a fresh handle makes, a repeated
/// check on a healthy ledger, and a repeated check on one whose schema is
/// broken -- failed readiness is retried, so its cost is paid every poll.
fn external_reader_ready(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    for &(rows, label) in SIZES {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.db");
        seed_dns_rows(&path, rows);

        c.bench_function(&format!("external_reader_ready_first_{label}"), |b| {
            b.iter(|| {
                let reader = DbHandle::open_external_reader(&path).expect("external reader");
                rt.block_on(reader.ready()).expect("ready");
            });
        });

        let reader = DbHandle::open_external_reader(&path).expect("external reader");
        rt.block_on(reader.ready()).expect("ready");
        c.bench_function(&format!("external_reader_ready_repeated_{label}"), |b| {
            b.iter(|| rt.block_on(reader.ready()).expect("ready"));
        });

        let broken = dir.path().join("broken.db");
        seed_dns_rows(&broken, rows);
        Connection::open(&broken)
            .expect("open broken db")
            .execute_batch("DROP TABLE profile_mutation_events;")
            .expect("drop a required table");
        let reader = DbHandle::open_external_reader(&broken).expect("external reader");
        c.bench_function(&format!("external_reader_ready_failed_{label}"), |b| {
            b.iter(|| rt.block_on(reader.ready()).expect_err("a broken schema is not ready"));
        });
    }
}

criterion_group!(benches, external_reader_poll, owning_handle_poll, external_reader_ready);
criterion_main!(benches);
