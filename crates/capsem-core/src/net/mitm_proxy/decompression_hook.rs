//! `DecompressionHook`: streaming gzip decompression as a sync
//! `ChunkHook`. Replaces `body::DecompressBody`'s
//! `async_compression::tokio::bufread::GzipDecoder` with the
//! lower-level `flate2::Decompress` raw-deflate state machine plus a
//! tiny hand-rolled gzip-header parser, driven inline from
//! `poll_frame`. gzip streaming-decode is fundamentally sync, so the
//! async wrapper was plumbing-only -- removing it removes one
//! `tokio::io::AsyncRead` adapter, one `StreamReader`, and one
//! `Body -> Stream` shim.
//!
//! T1 slice 7. The hook detects gzip from the first two bytes' magic
//! prefix (`0x1f 0x8b`) -- per RFC 1952 every gzip stream begins with
//! these. Once classified, the gzip header is buffered and parsed
//! (10-byte fixed prefix plus optional FEXTRA / FNAME / FCOMMENT /
//! FHCRC sections), then the deflate body is fed into
//! `flate2::Decompress`. If the magic is absent, the buffered prefix
//! is emitted as-is and the hook marks itself passthrough for the
//! rest of the body.
//!
//! `handle_request` seeds the per-body hook state with the response
//! `Content-Encoding` decision. That header must be authoritative:
//! package artifacts such as `.tgz` files are gzip containers as data,
//! not HTTP transfer encoding, and decompressing them corrupts client
//! integrity checks.

#![allow(dead_code)]

use bytes::Bytes;
use flate2::{Decompress, FlushDecompress, Status};

use super::hooks::{ChunkCtx, ChunkHook};

/// gzip magic per RFC 1952 §2.2: ID1=0x1f, ID2=0x8b.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Minimum gzip header size with FLG=0 (no optional fields).
const MIN_HEADER_LEN: usize = 10;

/// FLG bits we care about (per RFC 1952 §2.3.1).
const FHCRC: u8 = 0b0000_0010;
const FEXTRA: u8 = 0b0000_0100;
const FNAME: u8 = 0b0000_1000;
const FCOMMENT: u8 = 0b0001_0000;

/// Result of attempting to parse a gzip header out of a buffer.
enum HeaderParse {
    /// Header parsed successfully. The deflate body starts at
    /// `header_len` bytes into the buffer.
    Parsed { header_len: usize },
    /// Need more bytes before we can decide.
    NeedMore,
    /// First two bytes don't match gzip magic.
    NotGzip,
    /// Header looked like gzip but is malformed.
    Malformed,
}

/// Try to parse a gzip header out of `buf`. Pure (no allocation).
fn parse_gzip_header(buf: &[u8]) -> HeaderParse {
    if buf.len() < 2 {
        return HeaderParse::NeedMore;
    }
    if buf[..2] != GZIP_MAGIC {
        return HeaderParse::NotGzip;
    }
    if buf.len() < MIN_HEADER_LEN {
        return HeaderParse::NeedMore;
    }
    // Per RFC 1952 §2.3, CM must be 8 (deflate). Anything else is
    // either reserved or unsupported -- treat as malformed.
    if buf[2] != 8 {
        return HeaderParse::Malformed;
    }
    let flg = buf[3];
    // RFC 1952 reserves FLG bits 5-7. If any are set, this is not a
    // valid gzip member; pass it through rather than silently eating bytes.
    if flg & 0b1110_0000 != 0 {
        return HeaderParse::Malformed;
    }
    let mut pos = MIN_HEADER_LEN;

    if flg & FEXTRA != 0 {
        if buf.len() < pos + 2 {
            return HeaderParse::NeedMore;
        }
        let xlen = u16::from_le_bytes([buf[pos], buf[pos + 1]]) as usize;
        pos += 2;
        if buf.len() < pos + xlen {
            return HeaderParse::NeedMore;
        }
        pos += xlen;
    }
    if flg & FNAME != 0 {
        match buf[pos..].iter().position(|&b| b == 0) {
            Some(off) => pos += off + 1,
            None => return HeaderParse::NeedMore,
        }
    }
    if flg & FCOMMENT != 0 {
        match buf[pos..].iter().position(|&b| b == 0) {
            Some(off) => pos += off + 1,
            None => return HeaderParse::NeedMore,
        }
    }
    if flg & FHCRC != 0 {
        if buf.len() < pos + 2 {
            return HeaderParse::NeedMore;
        }
        pos += 2;
    }
    HeaderParse::Parsed { header_len: pos }
}

/// Per-request decoder state.
#[derive(Default)]
struct DecompressionState {
    /// Set on first chunk; final once `initialized == true`.
    initialized: bool,
    /// `is_gzip` and `decoder` only meaningful after `initialized`.
    is_gzip: bool,
    decoder: Option<Decompress>,
    /// Pending bytes while we wait to either parse a complete gzip
    /// header or rule out gzip via magic-byte mismatch.
    header_buf: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DecompressionConfig {
    pub gzip: bool,
}

/// Decompress one input slice through `decoder`, growing the output
/// buffer as needed. Returns the produced bytes. The decoder retains
/// state across calls, so partial gzip blocks split across chunks
/// decode correctly on the next call.
fn decompress_chunk(decoder: &mut Decompress, input: &[u8]) -> Vec<u8> {
    let mut output: Vec<u8> = Vec::with_capacity(input.len() * 4 + 64);
    let mut input_pos = 0usize;

    loop {
        // Reserve some headroom each iteration so the decoder always
        // has room to write at least one frame of output.
        let cur = output.len();
        let extra = (input.len() - input_pos).max(1024).max(64) * 2;
        output.resize(cur + extra, 0);

        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = match decoder.decompress(
            &input[input_pos..],
            &mut output[cur..],
            FlushDecompress::None,
        ) {
            Ok(s) => s,
            Err(_) => {
                output.truncate(cur);
                break;
            }
        };
        let in_used = (decoder.total_in() - before_in) as usize;
        let out_used = (decoder.total_out() - before_out) as usize;

        input_pos += in_used;
        output.truncate(cur + out_used);

        if let Status::StreamEnd = status {
            break;
        }
        if in_used == 0 && out_used == 0 {
            // No progress -- decoder needs more input than we have,
            // or this is end-of-input for a streaming chunk. Stop;
            // the next chunk will resume.
            break;
        }
    }

    output
}

/// Streaming gzip decompression `ChunkHook`. Mutates response chunks
/// in place: gzip-encoded input is replaced with the decompressed
/// bytes. Non-gzip bodies are passed through untouched after a
/// single magic-byte check on the first chunk.
pub struct DecompressionHook;

impl DecompressionHook {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DecompressionHook {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkHook for DecompressionHook {
    fn name(&self) -> &'static str {
        "decompression"
    }

    fn on_response_chunk(&self, chunk: &mut Bytes, ctx: &mut ChunkCtx<'_>) {
        let enabled = ctx
            .state::<DecompressionConfig>(DecompressionConfig::default)
            .gzip;
        if !enabled {
            return;
        }

        let state = ctx.state::<DecompressionState>(DecompressionState::default);

        if !state.initialized {
            // Append this chunk to the header buffer and try to parse.
            state.header_buf.extend_from_slice(chunk);
            match parse_gzip_header(&state.header_buf) {
                HeaderParse::NeedMore => {
                    // Not enough bytes to decide. Hold them back; the
                    // next chunk will retry. Emit empty so downstream
                    // doesn't see partial / unclassified bytes.
                    *chunk = Bytes::new();
                }
                HeaderParse::NotGzip | HeaderParse::Malformed => {
                    // Flush the buffered prefix as-is and mark the
                    // hook passthrough for the rest of the body.
                    let buffered = std::mem::take(&mut state.header_buf);
                    state.is_gzip = false;
                    state.initialized = true;
                    *chunk = Bytes::from(buffered);
                }
                HeaderParse::Parsed { header_len } => {
                    // Build the decoder, feed everything past the
                    // header into it, surface decoded bytes.
                    let mut decoder = Decompress::new(false);
                    let buffered = std::mem::take(&mut state.header_buf);
                    let body = &buffered[header_len..];
                    let decoded = decompress_chunk(&mut decoder, body);
                    state.decoder = Some(decoder);
                    state.is_gzip = true;
                    state.initialized = true;
                    *chunk = Bytes::from(decoded);
                }
            }
            return;
        }

        if !state.is_gzip {
            return;
        }
        let decoder = state
            .decoder
            .as_mut()
            .expect("is_gzip implies decoder initialized");
        let decoded = decompress_chunk(decoder, chunk);
        *chunk = Bytes::from(decoded);
    }

    fn on_response_end(&self, ctx: &mut ChunkCtx<'_>) {
        let state = ctx.state::<DecompressionState>(DecompressionState::default);
        // Drop the decoder; mid-stream EOF is structurally fine.
        // CRC validation against the gzip trailer would be
        // diagnostically interesting but isn't actionable here --
        // downstream telemetry already captures the body bytes either
        // way.
        state.decoder = None;
    }
}

#[cfg(test)]
mod tests;
