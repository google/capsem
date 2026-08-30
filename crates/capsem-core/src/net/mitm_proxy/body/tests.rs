use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Bytes};

use super::*;
use crate::net::mitm_proxy::{
    hooks::{ChunkCtx, ChunkEndFuture, ChunkHook, ConnMeta},
    pipeline::Pipeline,
};

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
