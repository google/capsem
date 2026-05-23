//! Anthropic interpreter microbench. Measures the SSE parse +
//! AnthropicStreamParserWithState pipeline end-to-end on a representative
//! tool-use response (the most expensive shape -- text + tool_use +
//! input_json_delta accumulation).

use capsem_core::net::ai_traffic::events::{collect_summary, ProviderStreamParser};
use capsem_core::net::interpreters::anthropic_interpreter::AnthropicStreamParserWithState;
use capsem_network_engine::sse_parser::SseParser;
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

const TOOL_USE_RESPONSE: &[u8] = b"\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_b\",\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":100,\"output_tokens\":1}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"I'll check the weather.\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_b1\",\"name\":\"get_weather\",\"input\":{}}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\": \\\"NYC\\\"}\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":50}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

fn bench_anthropic(c: &mut Criterion) {
    let mut group = c.benchmark_group("anthropic_interpreter");
    group.throughput(Throughput::Bytes(TOOL_USE_RESPONSE.len() as u64));
    group.bench_function("tool_use_full_pipeline", |b| {
        b.iter(|| {
            let mut sse = SseParser::new();
            let mut interp = AnthropicStreamParserWithState::new();
            let mut events = Vec::new();
            for chunk in black_box(TOOL_USE_RESPONSE).chunks(4096) {
                for sse_evt in sse.feed(chunk) {
                    events.extend(interp.parse_event(&sse_evt));
                }
            }
            let summary = collect_summary(&events);
            black_box(summary);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_anthropic);
criterion_main!(benches);
