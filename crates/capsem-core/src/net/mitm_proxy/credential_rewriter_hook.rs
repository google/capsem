//! Restores broker references when an upstream reflects materialized secrets.
use bytes::{Bytes, BytesMut};

use crate::credential_broker::{percent_encode_query_value, CredentialResponseRewrite};

use super::hooks::{ChunkCtx, ChunkHook};

#[derive(Debug, Clone, Default)]
pub struct CredentialResponseRewrites(pub Vec<CredentialResponseRewrite>);

struct RewriteState {
    pending: Vec<u8>,
    patterns: Vec<(Vec<u8>, Vec<u8>)>,
}

impl RewriteState {
    fn new(rewrites: &[CredentialResponseRewrite]) -> Self {
        Self {
            pending: Vec::new(),
            patterns: rewrite_patterns(rewrites),
        }
    }
}

pub struct CredentialRewriterHook;

impl ChunkHook for CredentialRewriterHook {
    fn name(&self) -> &'static str {
        "credential_rewriter"
    }

    fn on_response_chunk(&self, chunk: &mut Bytes, ctx: &mut ChunkCtx<'_>) {
        let rewrites = ctx
            .state::<CredentialResponseRewrites>(CredentialResponseRewrites::default)
            .0
            .clone();
        if rewrites.is_empty() {
            return;
        }
        let state = ctx.state::<RewriteState>(|| RewriteState::new(&rewrites));
        let mut combined = std::mem::take(&mut state.pending);
        combined.extend_from_slice(chunk);
        let (rewritten, pending) = rewrite_bytes_with_patterns(&combined, &state.patterns, false);
        state.pending = pending;
        *chunk = Bytes::from(rewritten);
    }

    fn take_response_tail(&self, ctx: &mut ChunkCtx<'_>) -> Bytes {
        let rewrites = ctx
            .state::<CredentialResponseRewrites>(CredentialResponseRewrites::default)
            .0
            .clone();
        let state = ctx.state::<RewriteState>(|| RewriteState::new(&rewrites));
        if state.pending.is_empty() {
            return Bytes::new();
        }
        let pending = std::mem::take(&mut state.pending);
        Bytes::from(rewrite_bytes_with_patterns(&pending, &state.patterns, true).0)
    }
}

pub fn rewrite_headers(headers: &mut http::HeaderMap, rewrites: &[CredentialResponseRewrite]) {
    for header_value in headers.values_mut() {
        let rewritten = rewrite_bytes(header_value.as_bytes(), rewrites, true).0;
        if rewritten != header_value.as_bytes() {
            if let Ok(replacement) = http::HeaderValue::from_bytes(&rewritten) {
                *header_value = replacement;
            }
        }
    }
}

pub fn sanitize_response_headers(headers: &mut http::HeaderMap, rewrites: &[CredentialResponseRewrite]) {
    rewrite_headers(headers, rewrites);
    for name in [
        "content-length",
        "content-md5",
        "digest",
        "content-digest",
        "repr-digest",
        "etag",
    ] {
        headers.remove(name);
    }
}

pub fn rewrite_complete_body(input: &[u8], rewrites: &[CredentialResponseRewrite]) -> Vec<u8> {
    rewrite_bytes(input, rewrites, true).0
}

fn rewrite_bytes(input: &[u8], rewrites: &[CredentialResponseRewrite], final_chunk: bool) -> (Vec<u8>, Vec<u8>) {
    let patterns = rewrite_patterns(rewrites);
    rewrite_bytes_with_patterns(input, &patterns, final_chunk)
}

fn rewrite_patterns(rewrites: &[CredentialResponseRewrite]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut patterns = Vec::new();
    for rewrite in rewrites {
        if rewrite.raw_value.is_empty() {
            continue;
        }
        patterns.push((
            rewrite.raw_value.as_bytes().to_vec(),
            rewrite.credential_ref.as_bytes().to_vec(),
        ));
        let encoded = percent_encode_query_value(&rewrite.raw_value);
        if encoded != rewrite.raw_value {
            let lower_escapes = lowercase_percent_escapes(&encoded);
            let form_encoded = encoded.replace("%20", "+");
            patterns.push((encoded.into_bytes(), rewrite.credential_ref.as_bytes().to_vec()));
            patterns.push((lower_escapes.into_bytes(), rewrite.credential_ref.as_bytes().to_vec()));
            patterns.push((form_encoded.into_bytes(), rewrite.credential_ref.as_bytes().to_vec()));
        }
        let json_escaped = rewrite.raw_value.replace('/', "\\/");
        if json_escaped != rewrite.raw_value {
            patterns.push((json_escaped.into_bytes(), rewrite.credential_ref.as_bytes().to_vec()));
        }
    }
    patterns.sort_by(|left, right| right.0.len().cmp(&left.0.len()));
    patterns.dedup_by(|left, right| left.0 == right.0);
    patterns
}

fn rewrite_bytes_with_patterns(input: &[u8], patterns: &[(Vec<u8>, Vec<u8>)], final_chunk: bool) -> (Vec<u8>, Vec<u8>) {
    let max_pattern = patterns.iter().map(|(pattern, _)| pattern.len()).max().unwrap_or(0);
    if max_pattern == 0 {
        return (input.to_vec(), Vec::new());
    }
    let safe = if final_chunk {
        input.len()
    } else {
        input.len().saturating_sub(max_pattern.saturating_sub(1))
    };
    let mut output = BytesMut::with_capacity(input.len());
    let mut cursor = 0;
    while cursor < safe {
        if let Some((pattern, replacement)) = patterns
            .iter()
            .find(|(pattern, _)| input[cursor..].starts_with(pattern))
        {
            output.extend_from_slice(replacement);
            cursor += pattern.len();
        } else {
            output.extend_from_slice(&input[cursor..=cursor]);
            cursor += 1;
        }
    }
    (output.to_vec(), input[cursor..].to_vec())
}

fn lowercase_percent_escapes(value: &str) -> String {
    let mut bytes = value.as_bytes().to_vec();
    let mut index = 0;
    while index + 2 < bytes.len() {
        if bytes[index] == b'%' {
            bytes[index + 1] = bytes[index + 1].to_ascii_lowercase();
            bytes[index + 2] = bytes[index + 2].to_ascii_lowercase();
            index += 3;
        } else {
            index += 1;
        }
    }
    String::from_utf8(bytes).expect("percent-encoded credential is ASCII")
}

#[cfg(test)]
mod tests;
