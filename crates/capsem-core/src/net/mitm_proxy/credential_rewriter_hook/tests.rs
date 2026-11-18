use super::*;
use crate::net::mitm_proxy::hooks::{ConnMeta, HookState};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use std::sync::Arc;

fn rewrite() -> CredentialResponseRewrite {
    CredentialResponseRewrite {
        credential_ref: "credential:blake3:safe-reference".into(),
        raw_value: "github/pat/reflection_secret".into(),
    }
}

#[test]
fn rewrites_raw_and_encoded_secrets_across_chunk_boundaries() {
    let hook = CredentialRewriterHook;
    let mut state = HookState::default();
    state.set(CredentialResponseRewrites(vec![rewrite()]));
    let conn = ConnMeta::default();
    let mut ctx = ChunkCtx {
        state: &mut state,
        conn: &conn,
        trace_id: None,
    };
    let mut first = Bytes::from_static(b"before github/pat/reflec");
    let mut second = Bytes::from_static(b"tion_secret and github%2Fpat%2Freflection_secret after");

    hook.on_response_chunk(&mut first, &mut ctx);
    hook.on_response_chunk(&mut second, &mut ctx);
    let tail = hook.take_response_tail(&mut ctx);
    let output = [first, second, tail].concat();
    let output = String::from_utf8(output).unwrap();

    assert_eq!(output.matches("credential:blake3:safe-reference").count(), 2);
    assert!(!output.contains("reflection_secret"));
    assert!(output.starts_with("before "));
    assert!(output.ends_with(" after"));
}

#[test]
fn rewrites_reflected_response_headers() {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::LOCATION,
        http::HeaderValue::from_static("https://example.test/?token=github/pat/reflection_secret"),
    );
    headers.insert(http::header::CONTENT_LENGTH, http::HeaderValue::from_static("99"));
    headers.insert(http::header::ETAG, http::HeaderValue::from_static("\"stale\""));
    headers.insert("content-digest", http::HeaderValue::from_static("sha-256=:stale:"));
    sanitize_response_headers(&mut headers, &[rewrite()]);
    let location = headers[http::header::LOCATION].to_str().unwrap();
    assert!(location.contains("credential:blake3:safe-reference"));
    assert!(!location.contains("reflection_secret"));
    assert!(!headers.contains_key(http::header::CONTENT_LENGTH));
    assert!(!headers.contains_key(http::header::ETAG));
    assert!(!headers.contains_key("content-digest"));
}

#[test]
fn rewrites_common_encoded_reflection_forms() {
    let rewrite = CredentialResponseRewrite {
        credential_ref: "credential:blake3:safe-reference".into(),
        raw_value: "github pat/reflection_secret".into(),
    };
    for reflected in [
        "github%20pat%2Freflection_secret",
        "github%20pat%2freflection_secret",
        "github+pat%2Freflection_secret",
        r"github pat\/reflection_secret",
    ] {
        let output = rewrite_complete_body(reflected.as_bytes(), std::slice::from_ref(&rewrite));
        assert_eq!(output, b"credential:blake3:safe-reference", "{reflected}");
    }
}

#[test]
fn production_order_decompresses_then_restores_across_stream_chunks() {
    let plaintext = b"before github/pat/reflection_secret after";
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(plaintext).unwrap();
    let compressed = encoder.finish().unwrap();
    let pipeline = super::super::pipeline::Pipeline::builder()
        .register_chunk(Arc::new(super::super::decompression_hook::DecompressionHook::new()))
        .register_chunk(Arc::new(CredentialRewriterHook))
        .build();
    let mut state = HookState::default();
    state.set(super::super::decompression_hook::DecompressionConfig { gzip: true });
    state.set(CredentialResponseRewrites(vec![rewrite()]));
    let conn = ConnMeta::default();
    let mut output = Vec::new();

    for byte in compressed {
        let mut chunk = Bytes::from(vec![byte]);
        pipeline.dispatch_response_chunk(&mut chunk, &mut state, &conn, None);
        output.extend_from_slice(&chunk);
    }
    let end = pipeline.dispatch_response_end(&mut state, &conn, None);
    output.extend_from_slice(&end.tail);

    assert_eq!(
        output, b"before credential:blake3:safe-reference after",
        "the guest-facing decompressed stream must contain only the broker reference"
    );
}
