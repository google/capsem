//! The canonical session owner: parse, project and deliver updates, independent
//! of a browser renderer or guest transport. The existing Criterion collector
//! discovers this target and exports raw samples to capsem-bench.
use std::time::Duration;

use capsem_core::terminal::SessionTerminal;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn corpus(marker: char) -> Vec<u8> {
    format!(
        "\x1b[H{}",
        format!("\x1b[38;2;20;160;240m{marker} terminal 界 e\u{301} 🙂 benchmark\x1b[0m\r\n").repeat(40)
    )
    .into_bytes()
}

fn terminal(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_terminal");
    group.bench_function("coalesced_rgb_unicode", |b| {
        let owner = SessionTerminal::new(8);
        let (view, _, mut updates) = owner.subscribe();
        owner.focus(view, 120, 48).unwrap();
        updates.try_recv().unwrap();
        let inputs = [corpus('x'), corpus('y')];
        let mut which = 0;
        b.iter(|| {
            black_box(owner.publish(black_box(&inputs[which])));
            black_box(updates.try_recv().unwrap());
            which ^= 1;
        });
    });
    group.bench_function("single_cell_update", |b| {
        let owner = SessionTerminal::new(8);
        let (_, _, mut updates) = owner.subscribe();
        let mut which = 0;
        let inputs = [b"\x1b[Hx", b"\x1b[Hy"];
        b.iter(|| {
            black_box(owner.publish(black_box(inputs[which])));
            black_box(updates.try_recv().unwrap());
            which ^= 1;
        });
    });
    group.bench_function("attach_reconstruct", |b| {
        let owner = SessionTerminal::new(8);
        owner.publish(corpus('x'));
        b.iter(|| black_box(owner.subscribe()));
    });
    group.bench_function("resize_pair", |b| {
        let owner = SessionTerminal::new(8);
        let (view, _, _) = owner.subscribe();
        owner.focus(view, 120, 48).unwrap();
        owner.publish(corpus('x'));
        b.iter(|| {
            assert!(owner.resize(view, 60, 30).unwrap());
            assert!(owner.resize(view, 120, 48).unwrap());
        });
    });
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(2));
    targets = terminal
}
criterion_main!(benches);
