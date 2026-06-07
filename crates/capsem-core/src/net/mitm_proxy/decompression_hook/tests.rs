use super::super::hooks::{ChunkCtx, ChunkHook, ConnMeta, HookState};
use super::*;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;

fn ctx_for<'a>(state: &'a mut HookState, conn: &'a ConnMeta) -> ChunkCtx<'a> {
    ChunkCtx {
        state,
        conn,
        trace_id: None,
    }
}

fn any_conn() -> ConnMeta {
    ConnMeta {
        domain: "any.example".into(),
        port: 443,
        process_name: None,
        ..Default::default()
    }
}

fn gzip(input: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(input).unwrap();
    enc.finish().unwrap()
}

fn mark_gzip(state: &mut HookState) {
    state.set(DecompressionConfig { gzip: true });
}

/// Single-chunk gzip body decompresses in place.
#[test]
fn single_chunk_gzip_is_decompressed() {
    let plaintext = b"hello world hello world hello world";
    let compressed = gzip(plaintext);

    let hook = DecompressionHook::new();
    let mut state = HookState::default();
    mark_gzip(&mut state);
    let conn = any_conn();
    let mut chunk = Bytes::from(compressed);

    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_end(&mut ctx);
    }

    assert_eq!(chunk.as_ref(), plaintext);
}

#[test]
fn gzip_payload_without_content_encoding_is_pass_through() {
    let compressed = gzip(b"this is a gzip artifact payload, not HTTP content-encoding");

    let hook = DecompressionHook::new();
    let mut state = HookState::default();
    let conn = any_conn();
    let mut chunk = Bytes::from(compressed.clone());

    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_end(&mut ctx);
    }

    assert_eq!(chunk.as_ref(), compressed.as_slice());
}

/// Compressed bytes split across two chunks decompress correctly --
/// the decoder retains state across calls.
#[test]
fn multi_chunk_gzip_streaming_decompress() {
    let plaintext = b"this is a test of streaming gzip across chunks of bytes oh yes";
    let compressed = gzip(plaintext);
    let mid = compressed.len() / 2;
    let mut a = Bytes::from(compressed[..mid].to_vec());
    let mut b = Bytes::from(compressed[mid..].to_vec());

    let hook = DecompressionHook::new();
    let mut state = HookState::default();
    mark_gzip(&mut state);
    let conn = any_conn();

    let mut decompressed = Vec::new();
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut a, &mut ctx);
    }
    decompressed.extend_from_slice(&a);
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut b, &mut ctx);
    }
    decompressed.extend_from_slice(&b);
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_end(&mut ctx);
    }

    assert_eq!(decompressed, plaintext);
}

/// Non-gzip body passes through untouched.
#[test]
fn non_gzip_body_is_pass_through() {
    let plaintext = b"this is plain JSON, definitely not gzip";

    let hook = DecompressionHook::new();
    let mut state = HookState::default();
    mark_gzip(&mut state);
    let conn = any_conn();
    let mut chunk = Bytes::copy_from_slice(plaintext);

    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
    }

    // First chunk arrives with a non-magic prefix, so it's
    // immediately re-emitted untouched.
    assert_eq!(chunk.as_ref(), plaintext);
}

/// After classifying as passthrough on the first chunk, subsequent
/// chunks are not touched even if they happen to start with bytes
/// that look like gzip magic.
#[test]
fn passthrough_classification_sticks() {
    let hook = DecompressionHook::new();
    let mut state = HookState::default();
    mark_gzip(&mut state);
    let conn = any_conn();

    // First chunk: plain text. Classifies as passthrough.
    let mut a = Bytes::from(b"hello".to_vec());
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut a, &mut ctx);
    }
    assert_eq!(a.as_ref(), b"hello");

    // Second chunk: starts with gzip magic. Should still be passthrough.
    let mut b = Bytes::from(vec![0x1f, 0x8b, 0x08, 0xff, 0xff]);
    let original = b.clone();
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut b, &mut ctx);
    }
    assert_eq!(b, original);
}

/// Many small chunks (one byte each) still decompress correctly.
#[test]
fn byte_by_byte_chunks_decompress() {
    let plaintext = b"sometimes upstream sends bytes one at a time, just to see what we do";
    let compressed = gzip(plaintext);

    let hook = DecompressionHook::new();
    let mut state = HookState::default();
    mark_gzip(&mut state);
    let conn = any_conn();
    let mut decompressed = Vec::new();

    for byte in &compressed {
        let mut chunk = Bytes::from(vec![*byte]);
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut chunk, &mut ctx);
        decompressed.extend_from_slice(&chunk);
    }
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_end(&mut ctx);
    }

    assert_eq!(decompressed, plaintext);
}

/// A first chunk shorter than 2 bytes defers classification: the
/// hook buffers it and emits an empty chunk. The next chunk
/// completes the magic-byte probe.
#[test]
fn one_byte_first_chunk_defers_classification() {
    let hook = DecompressionHook::new();
    let mut state = HookState::default();
    mark_gzip(&mut state);
    let conn = any_conn();

    // Plain text body whose first byte (0x68 = 'h') is not gzip
    // magic. With a single-byte first chunk we can't yet decide,
    // so the hook holds the byte back and emits empty.
    let mut a = Bytes::from(vec![b'h']);
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut a, &mut ctx);
    }
    assert!(a.is_empty(), "deferred byte must not pass through yet");
    let s = state
        .peek::<DecompressionState>()
        .expect("slot allocated even on deferred init");
    assert!(!s.initialized);

    // Second byte: now we have 2 bytes total; magic doesn't match;
    // hook flushes the deferred prefix + this chunk untouched.
    let mut b = Bytes::from(vec![b'i']);
    {
        let mut ctx = ctx_for(&mut state, &conn);
        hook.on_response_chunk(&mut b, &mut ctx);
    }
    assert_eq!(b.as_ref(), b"hi");
}
