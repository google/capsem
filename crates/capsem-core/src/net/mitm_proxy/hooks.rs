//! The `Hook` trait + supporting types: `HookOutcome`, `StopAction`,
//! `HookCtx`. The dispatcher itself lives in `pipeline.rs`.
//!
//! Single trait, layered events. Parsers, interpreters, policy, and
//! telemetry are all `Hook` impls subscribing to whichever
//! `EventKind`s their job needs. See `pipeline::Pipeline` for the
//! dispatch loop and cycle-prevention rules.

#![allow(dead_code)]

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http_body_util::combinators::BoxBody;
use hyper::body::Bytes;

use super::events::{Event, EventKind, EventLayer, EventMask};
use super::protocol::Protocol;
use crate::net::ai_traffic::provider::ProviderKind;

/// Outcome of a single `Hook::on_event` call.
///
/// `Continue`: dispatch proceeds to the next hook for this event.
/// `Rewrote`: same as `Continue`, but the hook signals it mutated the
///   event in place (used for tracing + metrics labels; doesn't change
///   dispatch behavior).
/// `Stop`: short-circuit. Remaining hooks for this event are skipped
///   and the connection takes the `StopAction` (drop, reject HTTP,
///   reject DNS).
pub enum HookOutcome {
    Continue,
    Rewrote,
    Stop(StopAction),
}

/// What the connection should do when a hook returns `Stop`.
pub enum StopAction {
    /// Silently drop this request/connection. No response sent to the
    /// guest. Used for DNS NXDOMAIN-style refusals on the request path.
    Drop,
    /// Synthesize this HTTP response back to the guest, skipping
    /// upstream entirely.
    Reject(http::Response<BoxBody<Bytes, anyhow::Error>>),
    /// Reject a DNS query with this rcode (T3 wires the resolver).
    DnsReject(u16),
}

/// Per-connection scratch space passed to every hook. Carries the
/// shared logger handle, the current event's layer (for cycle
/// prevention on `emit`), and a typed slot map so a hook can persist
/// state across multiple `on_event` calls within one connection (e.g.,
/// a streaming regex substituter holding its carry-over buffer between
/// `RawRequestChunk` events).
pub struct HookCtx<'pipe> {
    pub(super) layer: EventLayer,
    pub(super) emitter: &'pipe mut dyn DynEmitter,
    pub(super) state: &'pipe mut HookState,
    pub(super) trace_id: Option<String>,
    pub(super) conn: &'pipe ConnMeta,
}

/// Per-connection metadata that hooks can read but not mutate. Set
/// once by `handle_inner` after TLS termination + ClientHello SNI
/// extraction; visible to every hook for the lifetime of the
/// connection.
#[derive(Debug, Clone, Default)]
pub struct ConnMeta {
    /// Domain extracted from the TLS ClientHello SNI (or the HTTP
    /// `Host` header for plain HTTP, in T2). `<unknown>` if neither
    /// source produced one.
    pub domain: String,
    /// Best-effort process name from the `\0CAPSEM_META:` prefix the
    /// guest agent prepends to each connection. None if absent or
    /// not parseable.
    pub process_name: Option<String>,
    /// Upstream port. 443 for TLS today; T2 plain HTTP brings 80
    /// and the configurable allowlist.
    pub port: u16,
    /// Wire-protocol classification from the first-byte sniff.
    /// `Tls` for HTTPS (the original path); `Http` for plain HTTP/1.1
    /// (T2.2 wires the handler). Hooks that label metrics or branch
    /// on transport read this; pre-T2 fixtures and `Default` use
    /// `Unknown`.
    pub protocol: Protocol,
    /// AI provider classification when known independently of the domain.
    /// Normal provider domains still infer this from `domain`; local
    /// OpenAI-compatible servers and direct test fixtures can set it
    /// explicitly so response parsers and telemetry use the same provider
    /// decision as the enforcement path.
    pub ai_provider: Option<ProviderKind>,
}

impl<'pipe> HookCtx<'pipe> {
    /// Layer of the event currently being dispatched. A hook may only
    /// `emit()` events at a strictly higher layer.
    pub fn layer(&self) -> EventLayer {
        self.layer
    }

    /// Trace id (lower 16 hex of the W3C trace_id) associated with
    /// this connection, if any. Hooks that emit telemetry use this
    /// for correlation.
    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    /// Per-connection metadata: domain, process name, port. Set once
    /// by `handle_inner` after SNI extraction (or Host header parse
    /// for plain HTTP). Read-only to hooks.
    pub fn conn(&self) -> &ConnMeta {
        self.conn
    }

    /// Emit an event for downstream hooks at a higher layer. The
    /// pipeline runs the dispatch loop again for this event against
    /// hooks whose `interest()` mask contains its kind.
    ///
    /// Returns `Err(EmitError::CycleAttempt)` if the event's layer is
    /// not strictly greater than the current `ctx.layer()` -- this
    /// statically prevents re-emit cycles (an L3 hook cannot emit L1
    /// or L2; an L2 hook cannot emit L1). Returns `Err(EmitError::Stop)`
    /// if a downstream hook short-circuited the synthesized event;
    /// the original hook treats it however it sees fit.
    pub async fn emit<'a>(&mut self, ev: Event<'a>) -> Result<(), EmitError> {
        let target_layer = ev.layer();
        if target_layer <= self.layer {
            return Err(EmitError::CycleAttempt {
                from: self.layer,
                to: target_layer,
            });
        }
        self.emitter.emit(ev).await
    }

    /// Borrow a typed state slot for this hook within this connection.
    /// Inserted on first access via `init`. Returned reference is
    /// scoped to this `on_event` call.
    pub fn state<T: Send + Sync + 'static>(&mut self, init: impl FnOnce() -> T) -> &mut T {
        let key = TypeId::of::<T>();
        let entry = self
            .state
            .map
            .entry(key)
            .or_insert_with(|| Box::new(init()));
        // SAFETY: the TypeId key uniquely identifies T's storage; we
        // inserted a Box<T> for this key, so downcasting is sound.
        entry
            .downcast_mut::<T>()
            .expect("HookCtx::state type punned")
    }
}

/// Per-connection typed slot map. Each hook stores its scratch state
/// keyed by Rust type. Slots survive across all `on_event` calls for
/// the lifetime of one connection.
///
/// Slot values must be `Send + Sync` so that body wrappers carrying a
/// `HookState` (e.g., `ChunkDispatchBody`) can themselves be `Sync`
/// -- a requirement hyper imposes on `Body::boxed()`.
#[derive(Default)]
pub struct HookState {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl HookState {
    /// Borrow a typed slot if one was inserted for `T`. Used by tests
    /// and telemetry to inspect the hook's accumulated state after a
    /// dispatch finishes.
    pub fn peek<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref())
    }

    /// Insert / replace a typed slot. Used by `handle_request` to seed
    /// per-request context (e.g. `TelemetryRequestContext`) into the
    /// `ChunkDispatchBody`'s `HookState` before serving, so the
    /// matching `ChunkHook` can read it back at end-of-stream.
    pub fn set<T: Send + Sync + 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(value));
    }
}

/// Errors `ctx.emit` can return.
#[derive(Debug)]
pub enum EmitError {
    /// The hook tried to emit at a layer ≤ its own. Statically prevents
    /// re-emit cycles.
    CycleAttempt { from: EventLayer, to: EventLayer },
    /// A downstream hook short-circuited the dispatched event.
    Stop,
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CycleAttempt { from, to } => write!(
                f,
                "hook at layer {from:?} tried to emit at layer {to:?}; only strictly higher layers are allowed",
            ),
            Self::Stop => write!(f, "downstream hook short-circuited the emitted event"),
        }
    }
}

impl std::error::Error for EmitError {}

/// Erased pipeline-internal interface: lets `HookCtx::emit` re-enter
/// dispatch without `Pipeline` being a generic type parameter on
/// `HookCtx`.
pub(super) trait DynEmitter: Send {
    fn emit<'a, 'b>(
        &'b mut self,
        ev: Event<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<(), EmitError>> + Send + 'b>>
    where
        'a: 'b;
}

/// The single hook trait. Every parser, interpreter, policy check,
/// telemetry observer, and (later) credential rewriter implements this.
///
/// Hooks declare:
/// - `name()`: stable identifier shown in logs / metrics labels.
/// - `interest()`: the `EventMask` of kinds this hook cares about. The
///   dispatcher skips it for everything else (O(1) bitset check, zero
///   per-event overhead for non-matching kinds).
/// - `priority()`: stable sort key within an event kind. Lower runs
///   earlier. Ties break by registration order. Default 0.
///
/// `on_event` runs inside an `#[instrument]` span installed by the
/// pipeline (target = `mitm.<area>`, fields include hook name +
/// decision); hooks need not add their own.
pub trait Hook: Send + Sync {
    fn name(&self) -> &'static str;
    fn interest(&self) -> EventMask;
    fn priority(&self) -> i32 {
        0
    }

    fn on_event<'a, 'b>(
        &'a self,
        ev: &'b mut Event<'_>,
        ctx: &'b mut HookCtx<'_>,
    ) -> Pin<Box<dyn Future<Output = HookOutcome> + Send + 'b>>
    where
        'a: 'b;
}

/// Convenience: `Arc<dyn Hook>` so the pipeline can hold heterogeneous
/// hook impls without the overhead of cloning.
pub type ArcHook = Arc<dyn Hook>;

/// Sync per-chunk callback companion to `Hook`. Body wrappers
/// (`TrackedBody`, `DecompressionHook`'s body, `SseParserHook`'s body
/// tap) iterate registered `ChunkHook`s synchronously inside
/// `poll_frame`, so per-byte work runs without an `.await` and
/// without a channel hop.
///
/// Why sync: per-chunk work is fundamentally CPU-bound byte
/// transformation -- decompression, regex match-and-replace,
/// streaming parsers -- none of which need an `.await`. Hooks that
/// genuinely need async per-chunk (rare) push to an `mpsc` from the
/// sync method and drain in their own task.
///
/// Per-connection state for a chunk hook lives in the same
/// `HookCtx` slot map the async hooks use, keyed by the chunk
/// hook's struct type.
pub trait ChunkHook: Send + Sync {
    fn name(&self) -> &'static str;

    /// Called once per request body chunk, in order. May rewrite the
    /// chunk in place (mutate the inner `Bytes`); may replace it with
    /// a freshly-allocated `Bytes` of different length. Length changes
    /// require the corresponding L1 head hook to have updated
    /// `Content-Length` / `Transfer-Encoding` accordingly.
    fn on_request_chunk(&self, _chunk: &mut Bytes, _ctx: &mut ChunkCtx<'_>) {}

    /// Called once per response body chunk, in order. Same semantics
    /// as `on_request_chunk` but on the response path.
    fn on_response_chunk(&self, _chunk: &mut Bytes, _ctx: &mut ChunkCtx<'_>) {}

    /// Called once when the request body finishes (after the last
    /// chunk). Mostly useful for parsers that need to flush
    /// accumulator state (e.g., SSE's trailing event without a
    /// terminating blank line).
    fn on_request_end(&self, _ctx: &mut ChunkCtx<'_>) {}

    /// Called once when the response body finishes.
    fn on_response_end(&self, _ctx: &mut ChunkCtx<'_>) {}
}

pub type ArcChunkHook = Arc<dyn ChunkHook>;

/// Per-connection sync context passed to every `ChunkHook` callback.
/// Cheap to construct each call -- it borrows the long-lived
/// per-connection state.
pub struct ChunkCtx<'a> {
    pub(super) state: &'a mut HookState,
    pub(super) conn: &'a ConnMeta,
    pub(super) trace_id: Option<&'a str>,
}

impl<'a> ChunkCtx<'a> {
    pub fn conn(&self) -> &ConnMeta {
        self.conn
    }

    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id
    }

    /// Borrow a typed slot for this hook within this connection.
    /// Same semantics as `HookCtx::state` -- inserted on first
    /// access, persists across all chunks in the connection.
    pub fn state<T: Send + Sync + 'static>(&mut self, init: impl FnOnce() -> T) -> &mut T {
        let key = TypeId::of::<T>();
        let entry = self
            .state
            .map
            .entry(key)
            .or_insert_with(|| Box::new(init()));
        entry
            .downcast_mut::<T>()
            .expect("ChunkCtx::state type punned")
    }
}

/// A registered hook plus its registration index (for stable ordering
/// when priorities tie).
pub(super) struct Registration {
    pub hook: ArcHook,
    pub priority: i32,
    pub registration_order: usize,
    pub interest: EventMask,
}

impl Registration {
    pub(super) fn matches(&self, kind: EventKind) -> bool {
        self.interest.contains(kind)
    }
}

#[cfg(test)]
mod tests;
