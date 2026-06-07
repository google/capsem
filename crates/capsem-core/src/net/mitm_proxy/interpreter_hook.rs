//! Provider-specific interpreter `ChunkHook`s that consume parsed
//! `SseEvent`s from [`super::sse_parser_hook::SseEventStream`] and emit
//! provider-agnostic `LlmEvent`s into a shared [`LlmEventStream`] slot.
//!
//! T1 slice 6. Three concrete hooks (Anthropic / OpenAI / Google),
//! each gating on the model protocol resolved by MITM from the live
//! endpoint registry. Only the matching hook does work for a given
//! connection; the other two short-circuit before touching state.
//! Together they replace the inline parsing in `ai_traffic::ai_body::AiResponseBody`.
//!
//! Slot ownership:
//! - `SseEventStream` (owned by `SseParserHook`): producer-only here.
//!   The matching interpreter `drain(..)`s it on every chunk so the
//!   queue never grows unboundedly.
//! - `LlmEventStream`: shared across all three interpreter hooks
//!   (keyed by Rust type, so the slot map collapses them onto one
//!   value). Cumulative log of LlmEvents for the response.
//!   Downstream `TelemetryHook` reads from here at `on_response_end`.

#![allow(dead_code)]

use bytes::Bytes;

use super::hooks::{ChunkCtx, ChunkHook, ConnMeta};
use super::sse_parser_hook::SseEventStream;
use crate::net::ai_traffic::events::{LlmEvent, ProviderStreamParser};
use crate::net::ai_traffic::provider::ProviderKind;
use crate::net::interpreters::anthropic_interpreter::AnthropicStreamParserWithState;
use crate::net::interpreters::google_interpreter::GoogleStreamParser;
use crate::net::interpreters::openai_interpreter::OpenAiStreamParser;

/// Per-request shared accumulator of provider-agnostic `LlmEvent`s.
/// All three interpreter hooks write to the same slot (only one
/// matches per connection); the upcoming `TelemetryHook` reads it.
#[derive(Default)]
pub struct LlmEventStream {
    pub events: Vec<LlmEvent>,
    /// Provider that owns this stream, set the first time the matching
    /// interpreter runs. None for non-AI traffic.
    pub provider: Option<ProviderKind>,
}

fn conn_matches_provider(conn: &ConnMeta, provider: ProviderKind) -> bool {
    conn.ai_provider == Some(provider)
}

/// Run an interpreter pass: drain `SseEventStream`, parse via the
/// hook's parser, push resulting `LlmEvent`s onto `LlmEventStream`.
///
/// `take_parser` / `put_parser` bracket the parser borrow so the slot
/// map is free for the SSE/LLM slot accesses inside. We can't hold a
/// `&mut state.parser` across the SSE/LLM slot calls because both go
/// through `ctx.state` (single-borrow on the map at a time).
fn run<P, Take, Put>(
    ctx: &mut ChunkCtx<'_>,
    kind: ProviderKind,
    mut take_parser: Take,
    mut put_parser: Put,
) where
    P: ProviderStreamParser + Default,
    Take: FnMut(&mut ChunkCtx<'_>) -> P,
    Put: FnMut(&mut ChunkCtx<'_>, P),
{
    let drained: Vec<_> = {
        let stream = ctx.state::<SseEventStream>(SseEventStream::default);
        if stream.events.is_empty() {
            return;
        }
        stream.events.drain(..).collect()
    };
    let mut parser = take_parser(ctx);
    let mut llm_batch: Vec<LlmEvent> = Vec::new();
    for sse in &drained {
        llm_batch.extend(parser.parse_event(sse));
    }
    put_parser(ctx, parser);
    if llm_batch.is_empty() {
        return;
    }
    let out = ctx.state::<LlmEventStream>(LlmEventStream::default);
    if out.provider.is_none() {
        out.provider = Some(kind);
    }
    out.events.extend(llm_batch);
}

// ── Anthropic ────────────────────────────────────────────────────

#[derive(Default)]
struct AnthropicSlot(AnthropicStreamParserWithState);

pub struct AnthropicInterpreterHook;

impl AnthropicInterpreterHook {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AnthropicInterpreterHook {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkHook for AnthropicInterpreterHook {
    fn name(&self) -> &'static str {
        "interpreter_anthropic"
    }

    fn on_response_chunk(&self, _chunk: &mut Bytes, ctx: &mut ChunkCtx<'_>) {
        if !conn_matches_provider(ctx.conn(), ProviderKind::Anthropic) {
            return;
        }
        run::<AnthropicStreamParserWithState, _, _>(
            ctx,
            ProviderKind::Anthropic,
            |c| std::mem::take(&mut c.state::<AnthropicSlot>(AnthropicSlot::default).0),
            |c, p| c.state::<AnthropicSlot>(AnthropicSlot::default).0 = p,
        );
    }

    fn on_response_end(&self, ctx: &mut ChunkCtx<'_>) {
        self.on_response_chunk(&mut Bytes::new(), ctx);
    }
}

// ── OpenAI ───────────────────────────────────────────────────────

#[derive(Default)]
struct OpenAiSlot(OpenAiStreamParser);

pub struct OpenAiInterpreterHook;

impl OpenAiInterpreterHook {
    pub fn new() -> Self {
        Self
    }
}

impl Default for OpenAiInterpreterHook {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkHook for OpenAiInterpreterHook {
    fn name(&self) -> &'static str {
        "interpreter_openai"
    }

    fn on_response_chunk(&self, _chunk: &mut Bytes, ctx: &mut ChunkCtx<'_>) {
        if !conn_matches_provider(ctx.conn(), ProviderKind::OpenAi) {
            return;
        }
        run::<OpenAiStreamParser, _, _>(
            ctx,
            ProviderKind::OpenAi,
            |c| std::mem::take(&mut c.state::<OpenAiSlot>(OpenAiSlot::default).0),
            |c, p| c.state::<OpenAiSlot>(OpenAiSlot::default).0 = p,
        );
    }

    fn on_response_end(&self, ctx: &mut ChunkCtx<'_>) {
        self.on_response_chunk(&mut Bytes::new(), ctx);
    }
}

// ── Google ───────────────────────────────────────────────────────

#[derive(Default)]
struct GoogleSlot(GoogleStreamParser);

pub struct GoogleInterpreterHook;

impl GoogleInterpreterHook {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GoogleInterpreterHook {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkHook for GoogleInterpreterHook {
    fn name(&self) -> &'static str {
        "interpreter_google"
    }

    fn on_response_chunk(&self, _chunk: &mut Bytes, ctx: &mut ChunkCtx<'_>) {
        if !conn_matches_provider(ctx.conn(), ProviderKind::Google) {
            return;
        }
        run::<GoogleStreamParser, _, _>(
            ctx,
            ProviderKind::Google,
            |c| std::mem::take(&mut c.state::<GoogleSlot>(GoogleSlot::default).0),
            |c, p| c.state::<GoogleSlot>(GoogleSlot::default).0 = p,
        );
    }

    fn on_response_end(&self, ctx: &mut ChunkCtx<'_>) {
        self.on_response_chunk(&mut Bytes::new(), ctx);
    }
}

#[cfg(test)]
mod tests;
