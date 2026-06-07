use std::time::{Duration, SystemTime};

use capsem_logger::{DbWriter, FileAction, FileEvent, WriteOp};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};

fn file_event(idx: usize) -> WriteOp {
    WriteOp::FileEvent(FileEvent {
        event_id: None,
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(idx as u64),
        action: FileAction::Read,
        path: format!("/root/bench/file-{idx}.txt"),
        size: Some(128),
        trace_id: Some(format!("{idx:016x}")),
        credential_ref: None,
    })
}

fn bench_db_writer_bursts(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_writer_pressure");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));

    for burst_size in [128usize, 1024usize, 4096usize] {
        group.throughput(Throughput::Elements(burst_size as u64));
        group.bench_with_input(
            format!("file_events_{burst_size}"),
            &burst_size,
            |bench, &burst| {
                bench.iter_batched(
                    || {
                        let dir = tempfile::tempdir().expect("create temp db dir");
                        let db_path = dir.path().join("session.db");
                        let writer =
                            DbWriter::open(&db_path, burst.max(128)).expect("open DbWriter");
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
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_db_writer_bursts);
criterion_main!(benches);
