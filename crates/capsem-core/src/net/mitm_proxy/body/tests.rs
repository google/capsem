use std::sync::Arc;

use http_body_util::Full;
use hyper::body::{Body, Bytes};

use super::*;
use crate::net::mitm_proxy::{hooks::ConnMeta, pipeline::Pipeline};

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
