//! SSE parser microbench. Measures bytes/sec through `SseParser::feed`
//! across realistic chunk sizes (4KB, 64KB, 1MB) on a captured Anthropic
//! event-stream corpus.
//!
//! Pre-rewrite baseline lives at `benches/baselines/parser_sse-pre.txt`
//! (regenerate with `cargo bench -p capsem-core --bench parser_sse`).

use capsem_network_engine::sse_parser::SseParser;

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

const ANTHROPIC_EVENT: &[u8] = b"event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello, world from a benchmark line.\"}}\n\n";

fn make_corpus(target_bytes: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(target_bytes + ANTHROPIC_EVENT.len());
    while buf.len() < target_bytes {
        buf.extend_from_slice(ANTHROPIC_EVENT);
    }
    buf
}

fn bench_chunk_size(c: &mut Criterion, label: &str, total: usize, chunk: usize) {
    let corpus = make_corpus(total);
    let mut group = c.benchmark_group("sse_parser");
    group.throughput(Throughput::Bytes(corpus.len() as u64));
    group.bench_function(label, |b| {
        b.iter(|| {
            let mut parser = SseParser::new();
            let mut events = 0usize;
            for chunk_bytes in corpus.chunks(chunk) {
                events += parser.feed(black_box(chunk_bytes)).len();
            }
            black_box(events);
        });
    });
    group.finish();
}

fn bench_sse(c: &mut Criterion) {
    bench_chunk_size(c, "1MB_in_4KB_chunks", 1024 * 1024, 4096);
    bench_chunk_size(c, "1MB_in_64KB_chunks", 1024 * 1024, 65536);
    bench_chunk_size(c, "1MB_in_1MB_chunk", 1024 * 1024, 1024 * 1024);
}

criterion_group!(benches, bench_sse);
criterion_main!(benches);
