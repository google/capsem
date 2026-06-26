use std::time::Instant;

use capsem_proto::{
    decode_dns_request, decode_guest_msg, decode_host_msg, decode_mcp_frame_body,
    encode_dns_request, encode_guest_msg, encode_host_msg, encode_mcp_frame, DnsRequest,
    GuestToHost, HostToGuest,
};

const ITERS: usize = 1_000_000;

#[derive(Debug)]
struct Measurement {
    name: &'static str,
    iterations: usize,
    elapsed_ms: f64,
    ops_per_sec: f64,
}

fn measure(name: &'static str, iterations: usize, mut f: impl FnMut()) -> Measurement {
    let started = Instant::now();
    for _ in 0..iterations {
        f();
    }
    let elapsed = started.elapsed();
    let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    Measurement {
        name,
        iterations,
        elapsed_ms,
        ops_per_sec: iterations as f64 / elapsed.as_secs_f64(),
    }
}

fn main() {
    let host_frame = encode_host_msg(&HostToGuest::Ping { epoch_secs: 123 }).unwrap();
    let host_payload = &host_frame[4..];

    let guest_frame = encode_guest_msg(&GuestToHost::FileModified {
        path: "/root/bench/file.txt".to_string(),
        size: 128,
    })
    .unwrap();
    let guest_payload = &guest_frame[4..];

    let dns_request = DnsRequest {
        raw: vec![0xab; 96],
        proto: "udp".to_string(),
        process_name: Some("bench".to_string()),
    };
    let dns_frame = encode_dns_request(&dns_request).unwrap();
    let dns_payload = &dns_frame[4..];

    let mcp_frame = encode_mcp_frame(
        7,
        0,
        "bench-agent",
        br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"write_file","arguments":{"path":"poem.md"}}}"#,
    )
    .unwrap();
    let mcp_body = &mcp_frame[4..];

    let results = [
        measure("host_control_decode_payload_1m", ITERS, || {
            std::hint::black_box(decode_host_msg(std::hint::black_box(host_payload)).unwrap());
        }),
        measure("guest_control_decode_payload_1m", ITERS, || {
            std::hint::black_box(decode_guest_msg(std::hint::black_box(guest_payload)).unwrap());
        }),
        measure("dns_msgpack_decode_payload_1m", ITERS, || {
            std::hint::black_box(decode_dns_request(std::hint::black_box(dns_payload)).unwrap());
        }),
        measure("mcp_frame_decode_body_1m", ITERS, || {
            std::hint::black_box(decode_mcp_frame_body(std::hint::black_box(mcp_body)).unwrap());
        }),
        measure("dns_msgpack_encode_decode_roundtrip_1m", ITERS, || {
            let frame = encode_dns_request(std::hint::black_box(&dns_request)).unwrap();
            std::hint::black_box(decode_dns_request(&frame[4..]).unwrap());
        }),
    ];

    println!("vsock/framing microbench");
    println!("| bench | iterations | elapsed ms | ops/sec |");
    println!("|---|---:|---:|---:|");
    for result in results {
        println!(
            "| {} | {} | {:.3} | {:.0} |",
            result.name, result.iterations, result.elapsed_ms, result.ops_per_sec
        );
    }
}
