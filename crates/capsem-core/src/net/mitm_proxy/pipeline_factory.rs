//! Production hook pipeline assembly.

use std::sync::Arc;

use crate::net::ai_traffic::TraceHintSink;

use super::{decompression_hook, interpreter_hook, pipeline, sse_parser_hook, telemetry_hook};

/// Build the production hook pipeline. All chunk stages are synchronous;
/// header mutations needed for decompression happen before chunk dispatch.
pub fn make_production_pipeline(telemetry: Arc<telemetry_hook::TelemetryDeps>) -> Arc<pipeline::Pipeline> {
    make(telemetry_hook::TelemetryHook::new(telemetry))
}

pub fn make_production_pipeline_with_trace_hints(
    telemetry: Arc<telemetry_hook::TelemetryDeps>,
    trace_hints: Arc<dyn TraceHintSink>,
) -> Arc<pipeline::Pipeline> {
    make(telemetry_hook::TelemetryHook::new(telemetry).with_trace_hint_sink(trace_hints))
}

fn make(telemetry: telemetry_hook::TelemetryHook) -> Arc<pipeline::Pipeline> {
    // Order is load-bearing: decompress, parse SSE, interpret provider
    // events, then record the completed request and optional model call.
    Arc::new(
        pipeline::Pipeline::builder()
            .register_chunk(Arc::new(decompression_hook::DecompressionHook::new()))
            .register_chunk(Arc::new(sse_parser_hook::SseParserHook::new()))
            .register_chunk(Arc::new(interpreter_hook::AnthropicInterpreterHook::new()))
            .register_chunk(Arc::new(interpreter_hook::OpenAiInterpreterHook::new()))
            .register_chunk(Arc::new(interpreter_hook::GoogleInterpreterHook::new()))
            .register_chunk(Arc::new(telemetry))
            .build(),
    )
}
