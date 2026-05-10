//! JSON-RPC parser microbench placeholder. The real parser ships in T4
//! (mcp-protocol-aware-mitm); for T0 the bench measures `serde_json`
//! deserialization of representative MCP envelopes so a baseline exists
//! to regress against once the real parser hook lands.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use serde_json::Value;

const TOOLS_LIST_REQ: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
const TOOLS_CALL_REQ: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"fetch_http","arguments":{"url":"https://example.com","method":"GET"}}}"#;
const TOOLS_LIST_RESP: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"fetch_http","description":"Fetch an HTTP URL","inputSchema":{"type":"object","properties":{"url":{"type":"string"},"method":{"type":"string"}}}},{"name":"grep_http","description":"Grep an HTTP body","inputSchema":{"type":"object","properties":{"url":{"type":"string"},"pattern":{"type":"string"}}}}]}}"#;

fn bench_envelope(c: &mut Criterion, label: &str, payload: &'static str) {
    let mut group = c.benchmark_group("jsonrpc_parser_placeholder");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function(label, |b| {
        b.iter(|| {
            let v: Value = serde_json::from_str(black_box(payload)).unwrap();
            black_box(v);
        });
    });
    group.finish();
}

fn bench_jsonrpc(c: &mut Criterion) {
    bench_envelope(c, "tools_list_req", TOOLS_LIST_REQ);
    bench_envelope(c, "tools_call_req", TOOLS_CALL_REQ);
    bench_envelope(c, "tools_list_resp", TOOLS_LIST_RESP);
}

criterion_group!(benches, bench_jsonrpc);
criterion_main!(benches);
