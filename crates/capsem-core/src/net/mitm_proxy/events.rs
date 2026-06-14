//! Layered event ladder for the hook pipeline.
//!
//! Three abstraction levels. Hooks subscribe to whichever level fits
//! their job; parser hooks at L1 emit L2 events; interpreter hooks at
//! L2 emit L3 events. The layering is enforced by `EventLayer::try_from`
//! so the pipeline can statically prevent re-emit cycles (an L3 hook
//! cannot emit L1/L2; an L2 hook cannot emit L1).
//!
//! Mutation policy this sprint: hooks may mutate L1 byte/header
//! contents in place via `&mut` references; L2/L3 mutations are allowed
//! by the trait but parsers do not yet implement wire-level
//! re-serialization, so an L3 mutation only changes what later hooks
//! see, not what reaches the guest. Future security-engine sprint
//! widens this without trait changes.

#![allow(dead_code)]

use bytes::Bytes;

use crate::net::parsers::sse_parser::SseEvent;

/// Stateless placeholder shapes for L2/L3 event payloads that ship in
/// later phases (T3 DNS, T4 MCP). Defined here so the pipeline + hook
/// trait have stable variants to dispatch on without circular module
/// dependencies.
pub mod payloads {
    /// L2 -- a parsed JSON-RPC message (request, notification, or
    /// response). Real shape lands in T4.
    #[derive(Debug, Default, Clone)]
    pub struct JsonRpcMessage {
        pub method: Option<String>,
        pub id: Option<String>,
        pub raw: Vec<u8>,
    }

    /// L2 -- a DNS query as parsed by the resolver (T3).
    #[derive(Debug, Default, Clone)]
    pub struct DnsQuery {
        pub qname: String,
        pub qtype: u16,
    }

    /// L2 -- a DNS answer about to be returned to the guest (T3).
    #[derive(Debug, Default, Clone)]
    pub struct DnsAnswer {
        pub rcode: u8,
        pub answers_count: u16,
    }

    /// L3 -- summary of a model call's request shape (provider, model,
    /// system prompt presence, etc). Real shape uses
    /// `ai_traffic::request_parser::RequestMetadata` once we wire it.
    #[derive(Debug, Default, Clone)]
    pub struct AiRequestSummary {
        pub provider: String,
        pub model: Option<String>,
    }

    /// L3 -- a single delta from a streaming AI response (token, tool
    /// argument fragment, thinking chunk).
    #[derive(Debug, Default, Clone)]
    pub struct AiStreamDelta {
        pub kind: String,
        pub bytes_added: usize,
    }

    /// L3 -- summary of an MCP call seen on the wire (T4).
    #[derive(Debug, Default, Clone)]
    pub struct McpCallSummary {
        pub method: String,
        pub tool_name: Option<String>,
        pub trace_id: Option<String>,
    }
}

pub use payloads::{
    AiRequestSummary, AiStreamDelta, DnsAnswer, DnsQuery, JsonRpcMessage, McpCallSummary,
};

/// View of an L2-classified HTTP request (method, path, headers, body
/// preview). Built from a fully-buffered request. Hooks that need
/// per-chunk streaming use the L1 `RawRequestChunk` variant instead.
#[derive(Debug, Default)]
pub struct HttpRequestView {
    pub method: String,
    pub path: String,
    pub host: Option<String>,
    pub body_preview: Vec<u8>,
}

/// All events the pipeline can dispatch.
///
/// Variants carry `&'a mut` so hooks can mutate in place. Pattern
/// matching with `&mut Event<'_>` borrows the inner reference; the
/// borrow is released when the hook returns.
pub enum Event<'a> {
    // ── L1: raw transport ────────────────────────────────────────
    /// Mutable parsed HTTP request head -- method, URI, headers.
    RawRequestHead(&'a mut http::request::Parts),
    /// Mutable raw request body chunk. Hooks may rewrite the chunk
    /// in place; length changes are allowed (the underlying `Bytes`
    /// can be replaced via `*chunk = new_bytes`).
    RawRequestChunk(&'a mut Bytes),
    RawRequestEnd,

    /// Mutable parsed HTTP response head -- status, headers.
    RawResponseHead(&'a mut http::response::Parts),
    /// Mutable raw response body chunk. Same semantics as
    /// `RawRequestChunk`.
    RawResponseChunk(&'a mut Bytes),
    RawResponseEnd,

    // ── L2: protocol-classified (emitted by parser hooks) ────────
    HttpRequest(&'a mut HttpRequestView),
    SseEvent(&'a mut SseEvent),
    JsonRpcMessage(&'a mut JsonRpcMessage),
    DnsQuery(&'a mut DnsQuery),
    DnsAnswer(&'a mut DnsAnswer),

    // ── L3: semantic (emitted by interpreter hooks) ──────────────
    AiRequestStart(&'a mut AiRequestSummary),
    AiResponseChunk(&'a mut AiStreamDelta),
    AiCallEnd(&'a mut Box<capsem_logger::ModelCall>),
    McpCall(&'a mut McpCallSummary),
}

/// Discrete kinds backing `EventMask`. Order is stable -- the kind's
/// numeric value is its bit position in the mask. Adding a kind goes
/// at the end and bumps `KIND_COUNT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EventKind {
    RawRequestHead = 0,
    RawRequestChunk = 1,
    RawRequestEnd = 2,
    RawResponseHead = 3,
    RawResponseChunk = 4,
    RawResponseEnd = 5,
    HttpRequest = 6,
    SseEvent = 7,
    JsonRpcMessage = 8,
    DnsQuery = 9,
    DnsAnswer = 10,
    AiRequestStart = 11,
    AiResponseChunk = 12,
    AiCallEnd = 13,
    McpCall = 14,
}

pub const KIND_COUNT: usize = 15;

/// Every `EventKind` in declaration order. `ALL_KINDS[k as usize] == k`.
pub const ALL_KINDS: [EventKind; KIND_COUNT] = [
    EventKind::RawRequestHead,
    EventKind::RawRequestChunk,
    EventKind::RawRequestEnd,
    EventKind::RawResponseHead,
    EventKind::RawResponseChunk,
    EventKind::RawResponseEnd,
    EventKind::HttpRequest,
    EventKind::SseEvent,
    EventKind::JsonRpcMessage,
    EventKind::DnsQuery,
    EventKind::DnsAnswer,
    EventKind::AiRequestStart,
    EventKind::AiResponseChunk,
    EventKind::AiCallEnd,
    EventKind::McpCall,
];

impl EventKind {
    /// Layer this event lives at. Used for cycle prevention: a hook
    /// running at layer N can only `ctx.emit()` events at layer > N.
    pub fn layer(self) -> EventLayer {
        match self {
            Self::RawRequestHead
            | Self::RawRequestChunk
            | Self::RawRequestEnd
            | Self::RawResponseHead
            | Self::RawResponseChunk
            | Self::RawResponseEnd => EventLayer::L1,
            Self::HttpRequest
            | Self::SseEvent
            | Self::JsonRpcMessage
            | Self::DnsQuery
            | Self::DnsAnswer => EventLayer::L2,
            Self::AiRequestStart | Self::AiResponseChunk | Self::AiCallEnd | Self::McpCall => {
                EventLayer::L3
            }
        }
    }
}

/// Three-tier event layering. Strictly ordered: `L1 < L2 < L3`. A hook
/// dispatched at layer N can emit events at any layer > N via
/// `ctx.emit()`. Attempting to emit at layer ≤ N is rejected by the
/// pipeline -- this prevents re-emit cycles statically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventLayer {
    L1 = 1,
    L2 = 2,
    L3 = 3,
}

impl<'a> Event<'a> {
    /// The discrete kind of this event. Cheap (single match arm).
    pub fn kind(&self) -> EventKind {
        match self {
            Self::RawRequestHead(_) => EventKind::RawRequestHead,
            Self::RawRequestChunk(_) => EventKind::RawRequestChunk,
            Self::RawRequestEnd => EventKind::RawRequestEnd,
            Self::RawResponseHead(_) => EventKind::RawResponseHead,
            Self::RawResponseChunk(_) => EventKind::RawResponseChunk,
            Self::RawResponseEnd => EventKind::RawResponseEnd,
            Self::HttpRequest(_) => EventKind::HttpRequest,
            Self::SseEvent(_) => EventKind::SseEvent,
            Self::JsonRpcMessage(_) => EventKind::JsonRpcMessage,
            Self::DnsQuery(_) => EventKind::DnsQuery,
            Self::DnsAnswer(_) => EventKind::DnsAnswer,
            Self::AiRequestStart(_) => EventKind::AiRequestStart,
            Self::AiResponseChunk(_) => EventKind::AiResponseChunk,
            Self::AiCallEnd(_) => EventKind::AiCallEnd,
            Self::McpCall(_) => EventKind::McpCall,
        }
    }

    /// Layer this event lives at.
    pub fn layer(&self) -> EventLayer {
        self.kind().layer()
    }
}

/// Bitset over `EventKind`. Hooks declare `interest()` as an
/// `EventMask` so the dispatcher can skip them in O(1) for events
/// they don't care about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventMask(u32);

impl EventMask {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn all() -> Self {
        Self((1u32 << KIND_COUNT) - 1)
    }

    pub const fn single(kind: EventKind) -> Self {
        Self(1u32 << (kind as u8))
    }

    pub const fn or(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, kind: EventKind) -> bool {
        (self.0 & (1u32 << (kind as u8))) != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for EventMask {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.or(rhs)
    }
}

impl From<EventKind> for EventMask {
    fn from(k: EventKind) -> Self {
        Self::single(k)
    }
}

#[cfg(test)]
mod tests;
