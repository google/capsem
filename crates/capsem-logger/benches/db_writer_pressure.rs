use std::time::{Duration, SystemTime};

use capsem_logger::{DbWriter, FileAction, FileEvent, WriteOp};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};

#[path = "support/net_bodies.rs"]
mod net_bodies;

fn file_event(idx: usize) -> WriteOp {
    WriteOp::FileEvent(FileEvent {
        event_id: None,
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(idx as u64),
        action: FileAction::Read,
        path: format!("/root/bench/file-{idx}.txt"),
        size: Some(128),
        kind: capsem_logger::FileKind::File,
        trace_id: Some(format!("{idx:016x}")),
        credential_ref: None,
    })
}

fn bench_db_writer_bursts(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_writer_pressure");
    group.sample_size(10);
    // The 4096-event case needs about four seconds for ten samples. Keep a
    // rounded 20% margin so Criterion does not silently shorten the sample
    // count and make one run incomparable with the next.
    group.measurement_time(Duration::from_secs(5));

    for burst_size in [128usize, 1024usize, 4096usize] {
        group.throughput(Throughput::Elements(burst_size as u64));
        group.bench_with_input(format!("file_events_{burst_size}"), &burst_size, |bench, &burst| {
            bench.iter_batched(
                || {
                    let dir = tempfile::tempdir().expect("create temp db dir");
                    let db_path = dir.path().join("session.db");
                    let writer = DbWriter::open(&db_path, burst.max(128)).expect("open DbWriter");
                    let ops = (0..burst).map(file_event).collect::<Vec<_>>();
                    (dir, writer, ops)
                },
                |(_dir, writer, ops)| {
                    for op in ops {
                        writer.write_blocking(op);
                    }
                    writer.shutdown_blocking();
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

/// The same burst shape with every event archiving two bodies, a quarter of
/// them repeats of an earlier exchange: the writer's archive path end to end
/// -- staging, compression, the duplicate lookup, the segment flushes and
/// their index rows.
fn bench_db_writer_body_bursts(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_writer_pressure");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(8));

    for burst_size in [256usize, 1024usize] {
        group.throughput(Throughput::Elements(burst_size as u64));
        group.bench_with_input(format!("net_bodies_{burst_size}"), &burst_size, |bench, &burst| {
            bench.iter_batched(
                || {
                    let dir = tempfile::tempdir().expect("create temp db dir");
                    let db_path = dir.path().join("session.db");
                    let writer = DbWriter::open(&db_path, burst.max(128)).expect("open DbWriter");
                    let ops = (0..burst).map(net_bodies::net_event).collect::<Vec<_>>();
                    (dir, writer, ops)
                },
                |(_dir, writer, ops)| {
                    for op in ops {
                        writer.write_blocking(op);
                    }
                    writer.shutdown_blocking();
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(benches, bench_db_writer_bursts, bench_db_writer_body_bursts);
criterion_main!(benches);
