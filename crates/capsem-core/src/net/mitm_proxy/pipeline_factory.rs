use std::sync::Arc;

use super::{decompression_hook, interpreter_hook, pipeline, sse_parser_hook, telemetry_hook};

/// Build the default (empty) hook pipeline. T1 slices 2 + 3 will
/// extend this to register the production hook set; until then the
/// pipeline is wired through `MitmProxyConfig` but no dispatch
/// happens from `handle_request`.
pub fn make_default_pipeline() -> Arc<pipeline::Pipeline> {
    Arc::new(pipeline::Pipeline::builder().build())
}

/// Build the production hook pipeline. Registers the full sync ChunkHook chain
/// (decompression -> SSE parse -> provider interpreters -> telemetry).
///
/// All four ChunkHook stages are pure-sync: per-chunk work runs
/// inline from `poll_frame` with no `.await`, no channel hop, no
/// async wrapper. Header mutations needed for decompression
/// (Content-Encoding / Content-Length strip) happen inline in
/// `handle_request` before chunk dispatch begins -- the chunk hooks
/// themselves never see the head.
pub fn make_production_pipeline(
    telemetry: Arc<telemetry_hook::TelemetryDeps>,
) -> Arc<pipeline::Pipeline> {
    let policy = Arc::new(tokio::sync::RwLock::new(Arc::new(
        crate::net::policy::PolicyConfig::default(),
    )));
    make_production_pipeline_with_policy(policy, telemetry)
}

pub fn make_production_pipeline_with_policy(
    _policy: Arc<tokio::sync::RwLock<Arc<crate::net::policy::PolicyConfig>>>,
    telemetry: Arc<telemetry_hook::TelemetryDeps>,
) -> Arc<pipeline::Pipeline> {
    let p = pipeline::Pipeline::builder()
        // Chunk-hook order is load-bearing:
        //   1. DecompressionHook -- gzip detection on first chunk's
        //      magic; subsequent chunks fed through flate2::Decompress.
        //   2. SseParserHook -- needs decompressed bytes for AI
        //      domains.
        //   3. Interpreter hooks -- drain SseParserHook's queue and
        //      build LlmEvents. Three providers; only the matching
        //      one runs.
        //   4. TelemetryHook -- counts response bytes, captures
        //      preview, fires NetEvent + optional ModelCall on
        //      on_response_end.
        .register_chunk(Arc::new(decompression_hook::DecompressionHook::new()))
        .register_chunk(Arc::new(sse_parser_hook::SseParserHook::new()))
        .register_chunk(Arc::new(interpreter_hook::AnthropicInterpreterHook::new()))
        .register_chunk(Arc::new(interpreter_hook::OpenAiInterpreterHook::new()))
        .register_chunk(Arc::new(interpreter_hook::GoogleInterpreterHook::new()))
        .register_chunk(Arc::new(telemetry_hook::TelemetryHook::new(telemetry)))
        .build();
    Arc::new(p)
}
