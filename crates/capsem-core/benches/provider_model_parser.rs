//! Provider model-response parser microbenchmarks for the MITM security spine.
//!
//! These measure the pure OpenAI SSE -> provider events -> canonical summary
//! path used before model-response CEL enforcement. They intentionally avoid
//! network I/O and DB writes so the numbers stay fast and attributable.

use std::io::{Read, Write};

use capsem_core::net::interpreters::openai_interpreter::OpenAiStreamParser;
use capsem_network_engine::model_stream::{collect_summary, ProviderStreamParser};
use capsem_network_engine::sse_parser::SseParser;
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

fn openai_text_response(model: &str, content: &str) -> Vec<u8> {
    format!(
        "data: {{\"id\":\"chatcmpl-bench\",\"model\":\"{model}\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{content}\"}},\"finish_reason\":null}}]}}\n\n\
data: {{\"id\":\"chatcmpl-bench\",\"model\":\"{model}\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n\
data: [DONE]\n\n"
    )
    .into_bytes()
}

fn openai_text_fragments_response(model: &str, fragments: &[&str]) -> Vec<u8> {
    let mut response = String::new();
    for fragment in fragments {
        let content = serde_json::to_string(fragment).unwrap();
        response.push_str(&format!(
            "data: {{\"id\":\"chatcmpl-bench\",\"model\":\"{model}\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{content}}},\"finish_reason\":null}}]}}\n\n"
        ));
    }
    response.push_str(&format!(
        "data: {{\"id\":\"chatcmpl-bench\",\"model\":\"{model}\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n\
data: [DONE]\n\n"
    ));
    response.into_bytes()
}

fn openai_tool_call_response(model: &str) -> Vec<u8> {
    let tool_name = serde_json::to_string("mcp__filesystem__read_file").unwrap();
    let arguments = serde_json::to_string(r#"{"path":"/workspace/secret.txt"}"#).unwrap();
    format!(
        "data: {{\"id\":\"chatcmpl-bench\",\"model\":\"{model}\",\"choices\":[{{\"index\":0,\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{{\"name\":{tool_name},\"arguments\":{arguments}}}}}]}},\"finish_reason\":null}}]}}\n\n\
data: {{\"id\":\"chatcmpl-bench\",\"model\":\"{model}\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\n\
data: [DONE]\n\n"
    )
    .into_bytes()
}

fn malformed_openai_response() -> Vec<u8> {
    b"data: {\"id\":\"chatcmpl-bench\",\"choices\":[{\"delta\":{\"content\":\"malformed-model-needle\"}}\n\n\
data: [DONE]\n\n"
        .to_vec()
}

fn gzip_bytes(body: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(body).unwrap();
    encoder.finish().unwrap()
}

fn gunzip_bytes(body: &[u8]) -> Vec<u8> {
    let mut decoder = GzDecoder::new(body);
    let mut decoded = Vec::new();
    decoder.read_to_end(&mut decoded).unwrap();
    decoded
}

fn parse_openai_summary(body: &[u8]) -> usize {
    let mut sse = SseParser::new();
    let mut parser = OpenAiStreamParser::new();
    let mut llm_events = Vec::new();

    for event in sse.feed(body) {
        llm_events.extend(parser.parse_event(&event));
    }
    if let Some(event) = sse.flush() {
        llm_events.extend(parser.parse_event(&event));
    }

    let summary = collect_summary(&llm_events);
    summary.text.len()
        + summary.thinking.len()
        + summary.tool_calls.len()
        + usize::from(summary.stop_reason.is_some())
}

fn bench_openai_provider_parser(c: &mut Criterion) {
    let single = openai_text_response("gpt-test", "single-frame-model-needle");
    let multiframe = openai_text_fragments_response(
        "gpt-test",
        &[
            "streamed-",
            "model-",
            "needle-",
            "with-",
            "several-",
            "provider-",
            "frames",
        ],
    );
    let malformed = malformed_openai_response();
    let tool_call = openai_tool_call_response("gpt-test");
    let gzipped_multiframe = gzip_bytes(&multiframe);

    let mut group = c.benchmark_group("provider_model_parser_openai");
    for (name, body) in [
        ("single_frame_text", &single),
        ("multiframe_text", &multiframe),
        ("malformed_unknown_only", &malformed),
        ("provider_tool_call", &tool_call),
    ] {
        group.throughput(Throughput::Bytes(body.len() as u64));
        group.bench_function(name, |b| {
            b.iter(|| black_box(parse_openai_summary(black_box(body))));
        });
    }

    group.throughput(Throughput::Bytes(gzipped_multiframe.len() as u64));
    group.bench_function("gzip_decode_then_parse_multiframe_text", |b| {
        b.iter(|| {
            let decoded = gunzip_bytes(black_box(&gzipped_multiframe));
            black_box(parse_openai_summary(black_box(&decoded)))
        });
    });
    group.finish();
}

criterion_group!(benches, bench_openai_provider_parser);
criterion_main!(benches);
