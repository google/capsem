//! Hook dispatcher.
//!
//! `Pipeline::register` builds a sorted plan (one Vec per `EventKind`)
//! at registration time so per-event dispatch is a single
//! `Vec::iter()` over the matching hooks. `Pipeline::dispatch` runs
//! the loop and short-circuits on `HookOutcome::Stop`.
//!
//! `HookCtx::emit` re-enters `Pipeline::dispatch` recursively with a
//! synthesized event. The `EmitError::CycleAttempt` check inside
//! `HookCtx::emit` rejects emissions to layers ≤ the current hook's
//! layer, so an L3 hook cannot trigger L1/L2 dispatch and a recursion
//! depth bound (3 = L1->L2->L3) is structural rather than
//! runtime-enforced.

#![allow(dead_code)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::events::{Event, ALL_KINDS, KIND_COUNT};
use super::hooks::{
    ArcChunkHook, ArcHook, ChunkCtx, ConnMeta, DynEmitter, EmitError, HookCtx, HookOutcome,
    HookState, Registration, StopAction,
};
use super::metrics as m;
use bytes::Bytes;
use std::time::Instant;
use tracing::{debug, trace, Instrument};

/// Outcome of a full pipeline dispatch (potentially many hooks +
/// emitted children).
pub enum DispatchOutcome {
    /// Every matching hook returned `Continue` or `Rewrote`.
    Completed,
    /// A hook returned `Stop(StopAction)`. Caller acts on it.
    Stopped(StopAction),
}

/// Builder for a `Pipeline`. Each `register` call assigns the next
/// registration index so ties on priority break in registration order.
pub struct PipelineBuilder {
    hooks: Vec<Registration>,
    chunk_hooks: Vec<ArcChunkHook>,
    next_index: usize,
}

impl Default for PipelineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl PipelineBuilder {
    pub fn new() -> Self {
        Self {
            hooks: Vec::new(),
            chunk_hooks: Vec::new(),
            next_index: 0,
        }
    }

    pub fn register(mut self, hook: ArcHook) -> Self {
        let priority = hook.priority();
        let interest = hook.interest();
        let registration_order = self.next_index;
        self.next_index += 1;
        self.hooks.push(Registration {
            hook,
            priority,
            registration_order,
            interest,
        });
        self
    }

    /// Register a sync per-chunk hook. ChunkHooks run in registration
    /// order on every body chunk -- no priority sort. The body wrapper
    /// invokes them inline from `poll_frame`, so total per-chunk cost
    /// is the sum of every registered ChunkHook's work.
    pub fn register_chunk(mut self, hook: ArcChunkHook) -> Self {
        self.chunk_hooks.push(hook);
        self
    }

    pub fn build(self) -> Pipeline {
        // Sort by (priority asc, registration_order asc) once.
        let mut hooks = self.hooks;
        hooks.sort_by_key(|r| (r.priority, r.registration_order));

        // Pre-build a per-kind plan: for each EventKind, the indices
        // (into hooks[]) of registrations whose interest mask covers it.
        // O(KIND_COUNT * hooks.len()) once at build time; O(1) lookup
        // per dispatch.
        let mut by_kind: [Vec<usize>; KIND_COUNT] = std::array::from_fn(|_| Vec::new());
        for (idx, reg) in hooks.iter().enumerate() {
            for (slot, kind) in by_kind.iter_mut().zip(ALL_KINDS.iter().copied()) {
                if reg.matches(kind) {
                    slot.push(idx);
                }
            }
        }

        Pipeline {
            hooks: Arc::new(hooks),
            by_kind: Arc::new(by_kind),
            chunk_hooks: Arc::new(self.chunk_hooks),
        }
    }
}

/// Immutable hook execution plan. Cheap to clone -- shares its inner
/// `Arc`s. Each MITM connection holds one (per-connection state lives
/// in `HookCtx`, not here).
#[derive(Clone)]
pub struct Pipeline {
    hooks: Arc<Vec<Registration>>,
    by_kind: Arc<[Vec<usize>; KIND_COUNT]>,
    chunk_hooks: Arc<Vec<ArcChunkHook>>,
}

impl Pipeline {
    pub fn builder() -> PipelineBuilder {
        PipelineBuilder::new()
    }

    /// True if any chunk hooks are registered. Body wrappers can use
    /// this to skip the per-chunk iteration entirely when no hook
    /// cares (the common case in tests + the empty pipeline).
    pub fn has_chunk_hooks(&self) -> bool {
        !self.chunk_hooks.is_empty()
    }

    /// Iterate every registered ChunkHook for a request body chunk.
    /// Sync; runs inline. Pass-through to each hook's
    /// `on_request_chunk`.
    ///
    /// Per-chunk observability: every hook gets its
    /// `mitm.hook_invocations_total{hook}` counter incremented and a
    /// `mitm.hook_duration_ms{hook}` histogram sample recorded. No
    /// per-chunk span -- that would be cost-prohibitive on streaming
    /// bodies. Dump per-chunk traces with
    /// `RUST_LOG=mitm.hook.chunk=trace`; otherwise silent.
    pub fn dispatch_request_chunk(
        &self,
        chunk: &mut Bytes,
        state: &mut HookState,
        conn: &ConnMeta,
        trace_id: Option<&str>,
    ) {
        if self.chunk_hooks.is_empty() {
            return;
        }
        let mut ctx = ChunkCtx {
            state,
            conn,
            trace_id,
        };
        for hook in self.chunk_hooks.iter() {
            self.run_chunk(hook.as_ref(), "request", &mut ctx, |h, c| {
                h.on_request_chunk(chunk, c)
            });
        }
    }

    /// Iterate every registered ChunkHook for a response body chunk.
    pub fn dispatch_response_chunk(
        &self,
        chunk: &mut Bytes,
        state: &mut HookState,
        conn: &ConnMeta,
        trace_id: Option<&str>,
    ) {
        if self.chunk_hooks.is_empty() {
            return;
        }
        let mut ctx = ChunkCtx {
            state,
            conn,
            trace_id,
        };
        for hook in self.chunk_hooks.iter() {
            self.run_chunk(hook.as_ref(), "response", &mut ctx, |h, c| {
                h.on_response_chunk(chunk, c)
            });
        }
    }

    /// Notify ChunkHooks the request body has finished.
    pub fn dispatch_request_end(
        &self,
        state: &mut HookState,
        conn: &ConnMeta,
        trace_id: Option<&str>,
    ) {
        if self.chunk_hooks.is_empty() {
            return;
        }
        let mut ctx = ChunkCtx {
            state,
            conn,
            trace_id,
        };
        for hook in self.chunk_hooks.iter() {
            self.run_chunk(hook.as_ref(), "request_end", &mut ctx, |h, c| {
                h.on_request_end(c)
            });
        }
    }

    /// Notify ChunkHooks the response body has finished.
    pub fn dispatch_response_end(
        &self,
        state: &mut HookState,
        conn: &ConnMeta,
        trace_id: Option<&str>,
    ) {
        if self.chunk_hooks.is_empty() {
            return;
        }
        let mut ctx = ChunkCtx {
            state,
            conn,
            trace_id,
        };
        for hook in self.chunk_hooks.iter() {
            self.run_chunk(hook.as_ref(), "response_end", &mut ctx, |h, c| {
                h.on_response_end(c)
            });
        }
    }

    /// Common timing + counter wrapper for sync chunk-hook calls. Cheap
    /// (one Instant::now + one closure call) so it can run on the
    /// per-chunk hot path without burning the budget.
    fn run_chunk<F>(
        &self,
        hook: &dyn super::hooks::ChunkHook,
        kind: &'static str,
        ctx: &mut ChunkCtx<'_>,
        f: F,
    ) where
        F: FnOnce(&dyn super::hooks::ChunkHook, &mut ChunkCtx<'_>),
    {
        let name = hook.name();
        ::metrics::counter!(m::HOOK_INVOCATIONS_TOTAL, "hook" => name).increment(1);
        let started = Instant::now();
        f(hook, ctx);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        ::metrics::histogram!(m::HOOK_DURATION_MS, "hook" => name).record(elapsed_ms);
        trace!(
            target: "mitm.hook.chunk",
            hook = name,
            kind = kind,
            duration_ms = elapsed_ms,
            "chunk hook invoked",
        );
    }

    /// Dispatch one event through every matching hook (in priority
    /// order). Each hook's `on_event` future is awaited in sequence
    /// because they share `&mut Event<'_>`.
    pub async fn dispatch<'a>(
        &self,
        mut ev: Event<'a>,
        ctx_state: &mut HookState,
        trace_id: Option<String>,
        conn: &ConnMeta,
    ) -> DispatchOutcome {
        let kind = ev.kind();
        let layer = ev.layer();
        let state_ptr = StatePtr(ctx_state as *mut HookState);
        let conn_ptr = ConnPtr(conn as *const ConnMeta);

        let plan = self.by_kind[kind as usize].clone();
        for reg_idx in plan {
            let hook_arc = self.hooks[reg_idx].hook.clone();
            let hook_name = hook_arc.name();

            // Build a fresh HookCtx + PipelineEmitter for each hook call.
            // Both reference the same state_ptr so emit() reaches the
            // same state map; the inner pipeline.dispatch reborrows
            // through the StatePtr.
            let mut emitter = PipelineEmitter {
                pipeline: self.clone(),
                state_ptr,
                conn_ptr,
                trace_id: trace_id.clone(),
            };
            // SAFETY: state_ptr -- single-task access; we re-deref each
            // iteration so no cross-await aliasing of &mut HookState
            // occurs.
            let state_ref: &mut HookState = unsafe { &mut *state_ptr.0 };
            let conn_ref: &ConnMeta = unsafe { &*conn_ptr.0 };
            let mut ctx = HookCtx {
                layer,
                emitter: &mut emitter,
                state: state_ref,
                trace_id: trace_id.clone(),
                conn: conn_ref,
            };

            // Dispatch contract: every hook invocation runs inside its
            // own span (target = "mitm.hook"), gets counted (counter
            // mitm.hook_invocations_total{hook}), and gets timed
            // (histogram mitm.hook_duration_ms{hook}). The `decision`
            // field on the span is recorded after the future resolves
            // so triage tooling can see what each hook returned.
            // `on_enter` is logged at trace! so RUST_LOG=mitm.hook=trace
            // surfaces the entry-exit pair without flooding info.
            ::metrics::counter!(m::HOOK_INVOCATIONS_TOTAL, "hook" => hook_name).increment(1);
            let span = tracing::info_span!(
                target: "mitm.hook",
                "hook",
                hook = hook_name,
                kind = ?kind,
                layer = ?layer,
                decision = tracing::field::Empty,
                duration_ms = tracing::field::Empty,
            );
            let outcome = {
                let _enter = span.enter();
                trace!(target: "mitm.hook", hook = hook_name, "on_enter");
                let started = Instant::now();
                let outcome = hook_arc
                    .on_event(&mut ev, &mut ctx)
                    .instrument(span.clone())
                    .await;
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                ::metrics::histogram!(m::HOOK_DURATION_MS, "hook" => hook_name).record(elapsed_ms);
                let decision_str = match &outcome {
                    HookOutcome::Continue => "continue",
                    HookOutcome::Rewrote => "rewrote",
                    HookOutcome::Stop(StopAction::Drop) => "stop_drop",
                    HookOutcome::Stop(StopAction::Reject(_)) => "stop_reject",
                    HookOutcome::Stop(StopAction::DnsReject(_)) => "stop_dns_reject",
                };
                span.record("decision", decision_str);
                span.record("duration_ms", elapsed_ms);
                trace!(
                    target: "mitm.hook",
                    hook = hook_name,
                    decision = decision_str,
                    duration_ms = elapsed_ms,
                    "on_exit"
                );
                outcome
            };

            match outcome {
                HookOutcome::Continue | HookOutcome::Rewrote => {}
                HookOutcome::Stop(action) => {
                    // Stop is the load-bearing event for triage --
                    // promote to debug! so it shows even at default
                    // RUST_LOG=info filtering of mitm.hook (info+
                    // would only surface if the user explicitly opted
                    // info span on every hook -- noisy).
                    debug!(
                        target: "mitm.hook.cause",
                        hook = hook_name,
                        kind = ?kind,
                        "hook short-circuited the pipeline"
                    );
                    return DispatchOutcome::Stopped(action);
                }
            }
        }
        DispatchOutcome::Completed
    }
}

/// `HookCtx::emit` plumbs back into `Pipeline::dispatch` through this
/// erased emitter. Held inside `HookCtx`, not stored on the hook.
struct PipelineEmitter {
    pipeline: Pipeline,
    state_ptr: StatePtr,
    conn_ptr: ConnPtr,
    trace_id: Option<String>,
}

/// `*mut HookState` wrapped in a Send + Sync marker. Safety contract:
/// the pointer is only dereferenced from the current async task. The
/// pipeline never spawns new tasks against the same state map, so the
/// pointer is single-threaded by construction even though the future
/// it lives in is `Send` (Send only requires the state to be safe to
/// move across threads, not concurrent access).
#[derive(Clone, Copy)]
struct StatePtr(*mut HookState);

unsafe impl Send for StatePtr {}
unsafe impl Sync for StatePtr {}

/// Same shape as `StatePtr` for the read-only ConnMeta. The pointee
/// outlives the dispatch call; never aliased mutably.
#[derive(Clone, Copy)]
struct ConnPtr(*const ConnMeta);

unsafe impl Send for ConnPtr {}
unsafe impl Sync for ConnPtr {}

impl DynEmitter for PipelineEmitter {
    fn emit<'a, 'b>(
        &'b mut self,
        ev: Event<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<(), EmitError>> + Send + 'b>>
    where
        'a: 'b,
    {
        let pipeline = self.pipeline.clone();
        let trace_id = self.trace_id.clone();
        // Materialize &mut HookState + &ConnMeta here (synchronously,
        // outside the async block) so the future never captures raw
        // pointers.
        // SAFETY: see `StatePtr` / `ConnPtr` -- single-task access;
        // the conn pointee outlives the dispatch call by construction.
        let state: &'b mut HookState = unsafe { &mut *self.state_ptr.0 };
        let conn: &'b ConnMeta = unsafe { &*self.conn_ptr.0 };
        Box::pin(async move {
            match pipeline.dispatch(ev, state, trace_id, conn).await {
                DispatchOutcome::Completed => Ok(()),
                DispatchOutcome::Stopped(_) => Err(EmitError::Stop),
            }
        })
    }
}

/// Helper for callers (and tests) that want to dispatch an event
/// without holding a long-lived `HookState` or building a `ConnMeta`
/// by hand. Defaults the `ConnMeta` to its `Default` (empty
/// domain, port=0, no process name).
pub async fn dispatch_one(
    pipeline: &Pipeline,
    ev: Event<'_>,
    trace_id: Option<String>,
) -> DispatchOutcome {
    let mut state = HookState::default();
    let conn = ConnMeta::default();
    pipeline.dispatch(ev, &mut state, trace_id, &conn).await
}

#[cfg(test)]
mod tests;
