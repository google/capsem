use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{collections::VecDeque, convert::Infallible, pin::Pin, task::Context, task::Poll};

use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Bytes};

use super::*;
use crate::net::mitm_proxy::{
    hooks::{ChunkCtx, ChunkEndFuture, ChunkHook, ConnMeta},
    pipeline::Pipeline,
};

struct FrameBody {
    frames: VecDeque<hyper::body::Frame<Bytes>>,
}

impl Body for FrameBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(self.frames.pop_front().map(Ok))
    }
}

#[tokio::test]
async fn buffered_body_owns_admission_until_consumption_or_drop() {
    let budget = Arc::new(tokio::sync::Semaphore::new(1));
    let body = ChunkDispatchBody::new(
        Full::new(Bytes::from_static(b"buffered")),
        Arc::new(Pipeline::builder().build()),
        ConnMeta::default(),
        None,
    )
    .seed(Arc::clone(&budget).try_acquire_owned().unwrap());
    assert!(Arc::clone(&budget).try_acquire_owned().is_err());
    let bytes = body.collect().await.unwrap().to_bytes();
    assert_eq!(bytes, "buffered");
    assert_eq!(budget.available_permits(), 1);
    let body = ChunkDispatchBody::new(
        Full::new(Bytes::from_static(b"discarded")),
        Arc::new(Pipeline::builder().build()),
        ConnMeta::default(),
        None,
    )
    .seed(Arc::clone(&budget).try_acquire_owned().unwrap());
    assert_eq!(budget.available_permits(), 0);
    drop(body);
    assert_eq!(budget.available_permits(), 1);
}

struct AsyncEndHook {
    completed: Arc<AtomicBool>,
}

impl ChunkHook for AsyncEndHook {
    fn name(&self) -> &'static str {
        "async-end"
    }

    fn take_response_end_future(&self, _ctx: &mut ChunkCtx<'_>) -> Option<ChunkEndFuture> {
        let completed = Arc::clone(&self.completed);
        Some(ChunkEndFuture::new(async move {
            tokio::task::yield_now().await;
            completed.store(true, Ordering::Release);
        }))
    }
}

#[test]
fn chunk_dispatch_body_preserves_size_hint_by_default() {
    let body = ChunkDispatchBody::new(
        Full::new(Bytes::from_static(b"abc")),
        Arc::new(Pipeline::builder().build()),
        ConnMeta::default(),
        None,
    );

    assert_eq!(Body::size_hint(&body).exact(), Some(3));
}

#[test]
fn chunk_dispatch_body_can_drop_stale_size_hint() {
    let body = ChunkDispatchBody::new(
        Full::new(Bytes::from_static(b"abc")),
        Arc::new(Pipeline::builder().build()),
        ConnMeta::default(),
        None,
    )
    .without_size_hint();

    let hint = Body::size_hint(&body);
    assert_eq!(hint.exact(), None);
    assert_eq!(hint.upper(), None);
}

#[tokio::test]
async fn chunk_dispatch_body_awaits_async_end_work_before_final_frame() {
    let completed = Arc::new(AtomicBool::new(false));
    let pipeline = Pipeline::builder()
        .register_chunk(Arc::new(AsyncEndHook {
            completed: Arc::clone(&completed),
        }))
        .build();
    let mut body = ChunkDispatchBody::new(
        Full::new(Bytes::from_static(b"abc")),
        Arc::new(pipeline),
        ConnMeta::default(),
        None,
    );

    let frame = body.frame().await.unwrap().unwrap();
    let bytes = frame.into_data().unwrap();

    assert_eq!(bytes, Bytes::from_static(b"abc"));
    assert!(completed.load(Ordering::Acquire));
}

#[tokio::test]
async fn chunk_dispatch_body_restores_broker_references_in_trailers() {
    let raw = "github_pat_reflected_trailer_secret";
    let reference = "credential:blake3:safe-reference";
    let mut trailers = http::HeaderMap::new();
    trailers.insert("x-upstream-token", http::HeaderValue::from_str(raw).unwrap());
    let inner = FrameBody {
        frames: VecDeque::from([hyper::body::Frame::trailers(trailers)]),
    };
    let pipeline = Pipeline::builder()
        .register_chunk(Arc::new(
            crate::net::mitm_proxy::credential_rewriter_hook::CredentialRewriterHook,
        ))
        .build();
    let mut body = ChunkDispatchBody::new(inner, Arc::new(pipeline), ConnMeta::default(), None).seed(
        crate::net::mitm_proxy::credential_rewriter_hook::CredentialResponseRewrites(vec![
            crate::credential_broker::CredentialResponseRewrite {
                credential_ref: reference.into(),
                raw_value: raw.into(),
            },
        ]),
    );

    let frame = body.frame().await.unwrap().unwrap();
    let trailers = frame.into_trailers().unwrap();

    assert_eq!(trailers["x-upstream-token"], reference);
}
