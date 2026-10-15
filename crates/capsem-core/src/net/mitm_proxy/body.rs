//! Body wrappers for the MITM pipeline.
//!
//! - `BodyStats`: per-request byte counter + body-capture buffer.
//!   Used by `TrackedBody` (request side) and read by
//!   `TelemetryHook` at end-of-stream via the seeded
//!   `TelemetryRequestContext`.
//! - `TrackedBody`: counts bytes flowing through any hyper Body and
//!   caps the capture buffer. Wraps the upstream request body.
//! - `ChunkDispatchBody`: drives the sync `ChunkHook` chain on every
//!   frame. Per-request `HookState` slot map can be pre-seeded via
//!   `seed::<T>()` so hooks read context (e.g.
//!   `TelemetryRequestContext`) at end-of-stream.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use hyper::body::Bytes;

use super::hooks::{ConnMeta, HookState};
use super::pipeline::Pipeline;

pub type ProxyBoxBody = http_body_util::combinators::BoxBody<Bytes, anyhow::Error>;

pub struct BodyStats {
    pub bytes: u64,
    pub preview: Vec<u8>,
    pub max_body_capture: usize,
}

impl BodyStats {
    pub fn new(max_body_capture: usize) -> Self {
        Self {
            bytes: 0,
            preview: Vec::new(),
            max_body_capture,
        }
    }
}

pin_project_lite::pin_project! {
    pub struct TrackedBody<B> {
        #[pin]
        inner: B,
        stats: Arc<Mutex<BodyStats>>,
        max_size: u64,
    }
}

impl<B> TrackedBody<B> {
    pub fn new(inner: B, stats: Arc<Mutex<BodyStats>>, max_size: u64) -> Self {
        Self { inner, stats, max_size }
    }
}

impl<B> hyper::body::Body for TrackedBody<B>
where
    B: hyper::body::Body,
    B::Error: Into<anyhow::Error>,
{
    type Data = B::Data;
    type Error = anyhow::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        let mut this = self.project();
        match this.inner.as_mut().poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let len = hyper::body::Buf::remaining(data) as u64;
                    let mut st = this.stats.lock().unwrap();
                    st.bytes += len;
                    if st.bytes > *this.max_size {
                        return Poll::Ready(Some(Err(anyhow::anyhow!("body exceeded maximum size"))));
                    }
                    if st.preview.len() < st.max_body_capture {
                        let to_copy = (st.max_body_capture - st.preview.len()).min(len as usize);
                        let chunk = hyper::body::Buf::chunk(data);
                        let to_copy = to_copy.min(chunk.len());
                        st.preview.extend_from_slice(&chunk[..to_copy]);
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e.into()))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}

/// Sync chunk-hook iteration as a hyper Body wrapper.
///
/// Wraps an inner Body and, for every successful data frame, calls
/// `Pipeline::dispatch_response_chunk` against the registered
/// `ChunkHook`s before passing the (possibly mutated) frame
/// downstream. Fires `dispatch_response_end` once when the inner
/// body returns `Poll::Ready(None)`.
///
/// Costs nothing when no chunk hooks are registered: the
/// `pipeline.has_chunk_hooks()` short-circuit skips the dispatch
/// helper entirely.
///
/// Per-connection chunk-hook state lives in the wrapper's owned
/// `HookState` (one per body, since a body is per-request and chunk
/// hooks rarely need cross-request state -- if they do, they can
/// stash through the connection-level cache via Arc handles in their
/// own struct).
pub struct ChunkDispatchBody<B> {
    inner: B,
    pipeline: Arc<Pipeline>,
    state: HookState,
    conn: ConnMeta,
    trace_id: Option<String>,
    end_dispatched: bool,
    pending_frame: Option<hyper::body::Frame<Bytes>>,
    end_tail: Option<Bytes>,
    pending_end: VecDeque<super::hooks::ChunkEndFuture>,
    preserve_size_hint: bool,
}

impl<B> ChunkDispatchBody<B> {
    pub fn new(inner: B, pipeline: Arc<Pipeline>, conn: ConnMeta, trace_id: Option<String>) -> Self {
        Self {
            inner,
            pipeline,
            state: HookState::default(),
            conn,
            trace_id,
            end_dispatched: false,
            pending_frame: None,
            end_tail: None,
            pending_end: VecDeque::new(),
            preserve_size_hint: true,
        }
    }

    /// Seed the per-request `HookState` with a typed value before
    /// serving begins. Used by `handle_request` to hand
    /// `TelemetryRequestContext` (or any future per-request seed) to
    /// chunk hooks that need it without expanding the constructor.
    pub fn seed<T: Send + Sync + 'static>(mut self, value: T) -> Self {
        self.state.set(value);
        self
    }

    /// Drop the inner body's exact size hint when chunk hooks can
    /// change body length. Without this, hyper may synthesize a stale
    /// `Content-Length` even after headers were stripped.
    pub fn without_size_hint(mut self) -> Self {
        self.preserve_size_hint = false;
        self
    }

    fn dispatch_response_end(&mut self) {
        if self.end_dispatched || !self.pipeline.has_chunk_hooks() {
            return;
        }
        let end = self
            .pipeline
            .dispatch_response_end(&mut self.state, &self.conn, self.trace_id.as_deref());
        self.end_dispatched = true;
        self.end_tail = Some(end.tail);
        self.pending_end = end.pending.into();
    }

    fn poll_response_end(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        while let Some(future) = self.pending_end.front_mut() {
            if Pin::new(future).poll(cx).is_pending() {
                return Poll::Pending;
            }
            self.pending_end.pop_front();
        }
        Poll::Ready(())
    }
}

impl<B> hyper::body::Body for ChunkDispatchBody<B>
where
    B: hyper::body::Body<Data = Bytes> + Unpin,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        let this = &mut *self;
        if this.pending_frame.is_some() {
            if this.poll_response_end(cx).is_pending() {
                return Poll::Pending;
            }
            return Poll::Ready(Some(Ok(this.pending_frame.take().expect("pending frame checked"))));
        }
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                let frame = if this.pipeline.has_chunk_hooks() {
                    if let Some(data) = frame.data_ref() {
                        // We have to detach the bytes to mutate them
                        // through ChunkHook, then rebuild a frame.
                        // hyper::body::Frame::data_ref returns &Self::Data;
                        // for Bytes that's just a clone of the Arc-backed
                        // buffer, so this is cheap.
                        let mut chunk: Bytes = data.clone();
                        this.pipeline.dispatch_response_chunk(
                            &mut chunk,
                            &mut this.state,
                            &this.conn,
                            this.trace_id.as_deref(),
                        );
                        hyper::body::Frame::data(chunk)
                    } else {
                        frame
                    }
                } else {
                    frame
                };
                // A fixed-length HTTP consumer can treat the final data frame
                // as completion without polling the body for a subsequent
                // `None`. Hold that frame until response-end work is accepted
                // so command-level DB flush barriers cannot race Drop cleanup.
                if this.pipeline.has_chunk_hooks() && this.inner.is_end_stream() {
                    this.dispatch_response_end();
                    if this.poll_response_end(cx).is_pending() {
                        this.pending_frame = Some(frame);
                        return Poll::Pending;
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => {
                this.dispatch_response_end();
                if this.poll_response_end(cx).is_pending() {
                    return Poll::Pending;
                }
                if let Some(tail) = this.end_tail.take().filter(|tail| !tail.is_empty()) {
                    return Poll::Ready(Some(Ok(hyper::body::Frame::data(tail))));
                }
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        (!self.pipeline.has_chunk_hooks()
            || (self.end_dispatched
                && self.pending_frame.is_none()
                && self.pending_end.is_empty()
                && self.end_tail.as_ref().is_none_or(Bytes::is_empty)))
            && self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        if self.preserve_size_hint {
            self.inner.size_hint()
        } else {
            hyper::body::SizeHint::default()
        }
    }
}

impl<B> Drop for ChunkDispatchBody<B> {
    fn drop(&mut self) {
        // Fallback: if the body was dropped before completion (e.g.
        // client disconnect), still notify hooks so they can flush
        // accumulator state (SSE parser's trailing event without a
        // terminating blank line, etc.).
        if !self.end_dispatched && self.pipeline.has_chunk_hooks() {
            self.state.set(super::telemetry_hook::ResponseAborted(true));
            self.dispatch_response_end();
        }
        if !self.pending_end.is_empty() {
            poll_or_spawn_chunk_end_futures(self.pending_end.drain(..));
        }
    }
}

pub(super) fn poll_or_spawn_chunk_end_futures(
    futures: impl IntoIterator<Item = super::hooks::ChunkEndFuture>,
) {
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    for mut future in futures {
        if Pin::new(&mut future).poll(&mut cx).is_pending() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(future);
            } else {
                tracing::warn!(
                    "response body dropped outside a Tokio runtime; pending chunk-end work could not run"
                );
            }
        }
    }
}

/// RAII guard armed inside `handle_request` before upstream dial/send/body
/// collection. If `handle_request` is cancelled/dropped before sealing a
/// response with `ChunkDispatchBody` (e.g. client 60s timeout or VM exit),
/// `Drop` dispatches `TelemetryHook::on_response_end` so `NetEvent` and
/// `ModelCall` are still recorded in `session.db`.
pub(super) struct InFlightRequestGuard {
    pipeline: Arc<Pipeline>,
    conn: ConnMeta,
    trace_id: Option<String>,
    disarmed: Arc<std::sync::atomic::AtomicBool>,
    req_ctx: Option<super::telemetry_hook::TelemetryRequestContext>,
}

impl InFlightRequestGuard {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_request(
        pipeline: Arc<Pipeline>,
        policy_snapshot: Arc<crate::net::proxy_engine::ProxyPolicySnapshot>,
        domain: &str,
        process_name: Option<String>,
        port: u16,
        protocol: super::protocol::Protocol,
        conn_type: &'static str,
        ai_provider: Option<crate::net::ai_traffic::provider::ProviderKind>,
        ai_protocol: Option<crate::net::ai_traffic::provider::ModelProtocol>,
        method: &str,
        path: &str,
        query: Option<String>,
        req_hdrs: &str,
        start_time: std::time::Instant,
        request_body_stats: Arc<Mutex<BodyStats>>,
        credential_ref: Option<String>,
        credential_observations: Vec<crate::credential_broker::CredentialObservation>,
        disarmed: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        let model_traffic = ai_protocol.is_some_and(|proto| super::util::is_llm_api_path(proto, path));
        let trace_id = capsem_foundation::telemetry::ambient_capsem_trace_id();
        Self {
            pipeline,
            conn: ConnMeta {
                domain: domain.to_string(),
                process_name: process_name.clone(),
                port,
                protocol,
                ai_provider,
                ai_protocol,
            },
            trace_id,
            disarmed,
            req_ctx: Some(super::telemetry_hook::TelemetryRequestContext {
                policy_snapshot,
                domain: domain.to_string(),
                process_name,
                ai_provider,
                ai_protocol,
                model_traffic,
                method: method.to_string(),
                path: path.to_string(),
                query,
                status_code: None,
                decision: capsem_logger::Decision::Error,
                matched_rule: Some("client_aborted".to_string()),
                request_headers: Some(req_hdrs.to_string()),
                response_headers: None,
                start_time,
                request_body_stats,
                max_response_body_capture: 0,
                port,
                conn_type,
                policy_mode: None,
                policy_action: None,
                policy_rule: None,
                policy_reason: None,
                credential_ref,
                credential_observations,
                credential_injections: Vec::new(),
            }),
        }
    }

    pub(super) fn set_ai_identity(
        &mut self,
        ai_provider: Option<crate::net::ai_traffic::provider::ProviderKind>,
        ai_protocol: Option<crate::net::ai_traffic::provider::ModelProtocol>,
        model_traffic: bool,
    ) {
        self.conn.ai_provider = ai_provider;
        self.conn.ai_protocol = ai_protocol;
        if let Some(ctx) = self.req_ctx.as_mut() {
            ctx.ai_provider = ai_provider;
            ctx.ai_protocol = ai_protocol;
            ctx.model_traffic = model_traffic;
        }
    }

    pub(super) fn update_security(
        &mut self,
        policy: &super::util::SecurityBoundaryDecisionFields,
        credential_ref: Option<String>,
        credential_observations: Vec<crate::credential_broker::CredentialObservation>,
        credential_injections: Vec<crate::credential_broker::CredentialInjection>,
    ) {
        if let Some(ctx) = self.req_ctx.as_mut() {
            ctx.policy_mode = policy.policy_mode.clone();
            ctx.policy_action = policy.policy_action.clone();
            ctx.policy_rule = policy.policy_rule.clone();
            ctx.policy_reason = policy.policy_reason.clone();
            ctx.credential_ref = credential_ref;
            ctx.credential_observations = credential_observations;
            ctx.credential_injections = credential_injections;
        }
    }

    pub(super) fn update_policy(&mut self, policy: &super::util::SecurityBoundaryDecisionFields) {
        if let Some(ctx) = self.req_ctx.as_mut() {
            ctx.policy_mode = policy.policy_mode.clone();
            ctx.policy_action = policy.policy_action.clone();
            ctx.policy_rule = policy.policy_rule.clone();
            ctx.policy_reason = policy.policy_reason.clone();
        }
    }

    pub(super) fn update_response_head(&mut self, status_code: u16, resp_hdrs: String, max_capture: usize) {
        if let Some(ctx) = self.req_ctx.as_mut() {
            ctx.status_code = Some(status_code);
            ctx.response_headers = Some(resp_hdrs);
            ctx.max_response_body_capture = max_capture;
        }
    }

    pub(super) fn disarm(&mut self) {
        self.disarmed.store(true, std::sync::atomic::Ordering::Release);
        self.req_ctx = None;
    }
}

impl Drop for InFlightRequestGuard {
    fn drop(&mut self) {
        if self.disarmed.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let Some(req_ctx) = self.req_ctx.take() else {
            return;
        };
        let mut state = HookState::default();
        state.set(super::telemetry_hook::ResponseAborted(true));
        state.set::<Option<super::telemetry_hook::TelemetryRequestContext>>(Some(req_ctx));
        let end = self
            .pipeline
            .dispatch_response_end(&mut state, &self.conn, self.trace_id.as_deref());
        poll_or_spawn_chunk_end_futures(end.pending);
    }
}

#[cfg(test)]
mod tests;
