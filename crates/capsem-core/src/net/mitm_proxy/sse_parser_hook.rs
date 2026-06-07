//! `SseParserHook`: feeds the response body byte stream through the
//! shared `SseParser` and stashes parsed `SseEvent`s into a per-request
//! state slot for downstream interpreter hooks to consume.
//!
//! T1 slice 4 -- the first concrete `ChunkHook`. Drives one slot type:
//! [`SseEventStream`], a public producer/consumer queue keyed by Rust
//! type so a provider-specific interpreter hook (Anthropic / OpenAI /
//! Google, landing in the next slice) can drain new events on every
//! chunk.
//!
//! The hook gates internally: only connections whose runtime metadata
//! already carries a model protocol run the parser, so registering it
//! in the production pipeline is free for non-AI traffic.

#![allow(dead_code)]

use bytes::Bytes;

use super::hooks::{ChunkCtx, ChunkHook, ConnMeta};
use crate::net::ai_traffic::provider::ProviderKind;
use crate::net::parsers::sse_parser::{SseEvent, SseParser};

/// Per-request producer/consumer slot for parsed SSE events.
///
/// `SseParserHook` pushes new events here on every response chunk;
/// interpreter hooks running later in the same chunk pass drain them.
/// Cumulative across the response (consumers are expected to drain on
/// each pass to avoid double-processing).
#[derive(Default)]
pub struct SseEventStream {
    /// New events parsed since the last drain. Consumers should
    /// `events.drain(..)` and process them in order.
    pub events: Vec<SseEvent>,
}

/// Internal scratch slot: holds the running `SseParser` state across
/// chunks for one response. Kept private -- consumers read events via
/// [`SseEventStream`], not directly from the parser.
#[derive(Default)]
struct SseParserState {
    parser: SseParser,
    /// True once we've decided whether this connection is AI traffic.
    /// Cached so the per-chunk hot path doesn't repeat the domain
    /// match.
    is_ai: bool,
    /// Have we run the per-connection AI gating yet?
    initialized: bool,
}

fn conn_ai_provider(conn: &ConnMeta) -> Option<ProviderKind> {
    conn.ai_provider
}

/// `ChunkHook` that runs the shared `SseParser` over the response
/// body. No-op for non-AI domains.
pub struct SseParserHook;

impl SseParserHook {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SseParserHook {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkHook for SseParserHook {
    fn name(&self) -> &'static str {
        "sse_parser"
    }

    fn on_response_chunk(&self, chunk: &mut Bytes, ctx: &mut ChunkCtx<'_>) {
        // Read conn metadata before claiming a state slot -- the slot
        // borrow holds &mut on the slot map, which would otherwise
        // conflict with `ctx.conn()`'s shared borrow of the same ctx.
        let domain_is_ai = conn_ai_provider(ctx.conn()).is_some();
        // Two sequential state borrows: the parser slot (private) and
        // the public event-stream slot. Each `state::<T>()` call only
        // borrows the slot map for its T, so this composes cleanly.
        let parsed = {
            let pstate = ctx.state::<SseParserState>(SseParserState::default);
            if !pstate.initialized {
                pstate.is_ai = domain_is_ai;
                pstate.initialized = true;
            }
            if !pstate.is_ai {
                return;
            }
            pstate.parser.feed(chunk)
        };
        if parsed.is_empty() {
            return;
        }
        let stream = ctx.state::<SseEventStream>(SseEventStream::default);
        stream.events.extend(parsed);
    }

    fn on_response_end(&self, ctx: &mut ChunkCtx<'_>) {
        let trailing = {
            let pstate = ctx.state::<SseParserState>(SseParserState::default);
            if !pstate.is_ai {
                return;
            }
            pstate.parser.flush()
        };
        if let Some(ev) = trailing {
            let stream = ctx.state::<SseEventStream>(SseEventStream::default);
            stream.events.push(ev);
        }
    }
}

#[cfg(test)]
mod tests;
