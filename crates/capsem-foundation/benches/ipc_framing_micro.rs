use std::time::Instant;

use capsem_proto::ipc::ServiceToProcess;

const SMALL_ITERS: usize = 1_000_000;
const LARGE_ITERS: usize = 100;

fn measure(name: &str, iterations: usize, mut operation: impl FnMut()) {
    let started = Instant::now();
    for _ in 0..iterations {
        operation();
    }
    let elapsed = started.elapsed();
    println!(
        "| {name} | {iterations} | {:.3} | {:.0} |",
        elapsed.as_secs_f64() * 1000.0,
        iterations as f64 / elapsed.as_secs_f64()
    );
}

fn main() {
    let ping = ServiceToProcess::Ping;
    let file = ServiceToProcess::WriteFile {
        id: 7,
        path: "/root/bench.bin".into(),
        data: vec![0xff; 1024 * 1024],
    };
    let current_file = rmp_serde::to_vec_named(&file).unwrap();

    println!("ipc/framing microbench");
    println!("bounded MessagePack 1 MiB frame bytes: {}", current_file.len() + 4);
    println!("| bench | iterations | elapsed ms | ops/sec |");
    println!("|---|---:|---:|---:|");
    measure("bounded_msgpack_ping_encode", SMALL_ITERS, || {
        std::hint::black_box(rmp_serde::to_vec_named(std::hint::black_box(&ping)).unwrap());
    });
    let current_ping = rmp_serde::to_vec_named(&ping).unwrap();
    measure("bounded_msgpack_ping_decode", SMALL_ITERS, || {
        let _: ServiceToProcess =
            std::hint::black_box(rmp_serde::from_slice(std::hint::black_box(&current_ping)).unwrap());
    });
    measure("bounded_msgpack_file_1m_roundtrip", LARGE_ITERS, || {
        let bytes = rmp_serde::to_vec_named(std::hint::black_box(&file)).unwrap();
        let _: ServiceToProcess = rmp_serde::from_slice(std::hint::black_box(&bytes)).unwrap();
    });
}
