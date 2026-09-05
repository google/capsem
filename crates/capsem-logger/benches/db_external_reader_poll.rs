//! What a service route pays to read a ledger another process writes.
//!
//! The service's session handles are external readers: capsem-process owns
//! the writes and disk is the boundary. Before each query the reader brings
//! its memory tables up to date with disk. Two cases matter: a poll when
//! nothing was committed since the last one (the common UI case) and a poll
//! right after a commit. Both used to copy every hot table from disk; the
//! first now copies nothing and the second only the new rows.

use std::time::{Duration, SystemTime};

use capsem_logger::{DbHandle, DbWriter, Decision, DnsEvent, WriteOp};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

const ROWS: usize = 20_000;

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

struct Ledger {
    _dir: tempfile::TempDir,
    writer: DbWriter,
    reader: DbHandle,
    next: usize,
}

fn seeded_ledger(rt: &tokio::runtime::Runtime) -> Ledger {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session.db");
    let writer = DbWriter::open(&path, 512).expect("writer");
    rt.block_on(async {
        for idx in 0..ROWS {
            writer.write(dns_event(idx)).await;
        }
        writer.flush().await;
    });
    let reader = DbHandle::open_external_reader(&path).expect("external reader");
    rt.block_on(reader.ready()).expect("ready");
    Ledger {
        _dir: dir,
        writer,
        reader,
        next: ROWS,
    }
}

const POLL_SQL: &str = "SELECT COUNT(*) FROM dns_events";

fn external_reader_poll(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let mut ledger = seeded_ledger(&rt);

    c.bench_function("external_reader_poll_idle_20k", |b| {
        b.iter(|| rt.block_on(ledger.reader.query(POLL_SQL, &[])).expect("query"));
    });

    c.bench_function("external_reader_poll_after_write_20k", |b| {
        b.iter_batched(
            || {
                ledger.next += 1;
                let event = dns_event(ledger.next);
                rt.block_on(async {
                    ledger.writer.write(event).await;
                    ledger.writer.flush().await;
                });
            },
            |()| rt.block_on(ledger.reader.query(POLL_SQL, &[])).expect("query"),
            BatchSize::PerIteration,
        );
    });
}

criterion_group!(benches, external_reader_poll);
criterion_main!(benches);
