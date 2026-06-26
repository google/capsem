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
        Self {
            inner,
            stats,
            max_size,
        }
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
                        return Poll::Ready(Some(Err(anyhow::anyhow!(
                            "body exceeded maximum size"
                        ))));
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
    preserve_size_hint: bool,
}

impl<B> ChunkDispatchBody<B> {
    pub fn new(
        inner: B,
        pipeline: Arc<Pipeline>,
        conn: ConnMeta,
        trace_id: Option<String>,
    ) -> Self {
        Self {
            inner,
            pipeline,
            state: HookState::default(),
            conn,
            trace_id,
            end_dispatched: false,
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
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if this.pipeline.has_chunk_hooks() {
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
                        return Poll::Ready(Some(Ok(hyper::body::Frame::data(chunk))));
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => {
                if !this.end_dispatched && this.pipeline.has_chunk_hooks() {
                    this.pipeline.dispatch_response_end(
                        &mut this.state,
                        &this.conn,
                        this.trace_id.as_deref(),
                    );
                    this.end_dispatched = true;
                }
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
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
            self.pipeline.dispatch_response_end(
                &mut self.state,
                &self.conn,
                self.trace_id.as_deref(),
            );
            self.end_dispatched = true;
        }
    }
}

#[cfg(test)]
mod tests;
