//! Pipeline-shape microbench placeholder. The Hook trait + dispatch
//! ships in T1; for T0 this bench measures the cost of:
//!
//! 1. Registering all metric descriptors (so a casual rename of a
//!    metric name surfaces as a CI failure here).
//! 2. Allocating + dropping a no-op recorder, the cheapest possible
//!    measurement of the metrics facade overhead per request.
//!
//! Once T1 lands, this bench gains real Hook dispatch measurements:
//! empty-pipeline overhead, dispatch with N registered hooks, and the
//! `ctx.emit()` recursion depth.

use capsem_core::net::mitm_proxy::metrics as mitm_metrics;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use metrics::counter;

fn bench_describe(c: &mut Criterion) {
    c.bench_function("metrics_describe_all", |b| {
        b.iter(|| {
            mitm_metrics::describe_all();
        });
    });
}

fn bench_counter_emit(c: &mut Criterion) {
    c.bench_function("counter_emit_no_recorder", |b| {
        b.iter(|| {
            // No recorder is installed in the bench harness, so the
            // counter resolves to a single relaxed atomic add against
            // the global no-op recorder. This is the per-request cost
            // hooks pay until T5 wires an exporter.
            counter!(black_box("mitm.requests_total"), "decision" => "allow").increment(1);
        });
    });
}

criterion_group!(benches, bench_describe, bench_counter_emit);
criterion_main!(benches);
