//! MCP response transport admission, before buffering or SSE parsing.
use std::io;

use bytes::Bytes;
use futures::{stream::BoxStream, Stream, StreamExt};
use sse_stream::{Error as SseError, Sse, SseStream};

fn too_large(limit: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("MCP response exceeds its {limit}-byte transport limit; the tool may already have run"),
    )
}

pub(super) async fn body(response: reqwest::Response, limit: usize) -> io::Result<Vec<u8>> {
    // The capability bridge wraps a stream, so reqwest may have no body size
    // hint even when the forwarded HTTP headers declare Content-Length.
    let declared = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if response.content_length().is_some_and(|length| length > limit as u64)
        || declared.is_some_and(|length| length > limit as u64)
    {
        return Err(too_large(limit));
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(io::Error::other)?;
        if chunk.len() > limit - body.len() {
            return Err(too_large(limit));
        }
        // One final buffer, admitted before allocation; the length hint never
        // selects capacity. MCP's external JSON-RPC is parsed from these bytes.
        body.try_reserve_exact(chunk.len()).map_err(io::Error::other)?;
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(super) async fn json_body(response: reqwest::Response) -> io::Result<Vec<u8>> {
    let bytes = body(response, capsem_proto::MCP_JSON_RPC_MAX_BYTES).await?;
    capsem_proto::mcp_contracts::json_rpc_wire::validate_size(&bytes)
        .map_err(|_| too_large(capsem_proto::MCP_PAYLOAD_MAX_BYTES))?;
    Ok(bytes)
}

/// Count normalized data bytes before handing chunks to sse-stream. Prefixes,
/// optional spaces and CRLF do not consume the usable JSON-RPC payload budget.
/// A separate derived wire allowance bounds parser storage for framing and
/// metadata; comments are bounded per line and do not accumulate per event.
#[derive(Default)]
struct EventSize {
    retained: usize,
    line: usize,
    comment: bool,
    after_cr: bool,
    prefix: [u8; 5],
    payload: usize,
    data_lines: usize,
    bom_prefix: usize,
    bom_checked: bool,
}

impl EventSize {
    fn observe(&mut self, bytes: &[u8], limit: usize) -> io::Result<()> {
        for &byte in bytes {
            if !self.bom_checked {
                if byte == b"\xef\xbb\xbf"[self.bom_prefix] {
                    self.bom_prefix += 1;
                    self.bom_checked = self.bom_prefix == 3;
                    continue;
                }
                self.bom_checked = true;
            }
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\r' | b'\n' => {
                    if self.line == 0 {
                        self.retained = 0;
                        self.payload = 0;
                        self.data_lines = 0;
                    } else if !self.comment {
                        self.retained = self.retained.saturating_add(self.line).saturating_add(1);
                        if self.line >= 5 && &self.prefix == b"data:" {
                            self.data_lines = self.data_lines.saturating_add(1);
                        }
                    }
                    self.line = 0;
                    self.comment = false;
                    self.after_cr = byte == b'\r';
                }
                _ => {
                    if self.line == 0 {
                        self.comment = byte == b':';
                    }
                    self.line = self.line.saturating_add(1);
                    if self.line <= 5 {
                        self.prefix[self.line - 1] = byte;
                        if self.line == 5 && &self.prefix == b"data:" && self.data_lines > 0 {
                            self.payload = self.payload.saturating_add(1);
                        }
                    } else if &self.prefix == b"data:" && !(self.line == 6 && byte == b' ') {
                        self.payload = self.payload.saturating_add(1);
                    }
                }
            }
            // Each normalized byte can occupy a separate "data: \n" line.
            // One extra line accounts for the final omitted data newline;
            // metadata shares this physical envelope rather than payload space.
            let wire_limit = limit.saturating_add(1).saturating_mul(b"data: \n".len());
            if self.payload > limit
                || self.line > limit.saturating_add(b"data: ".len())
                || self.retained.saturating_add(self.line) > wire_limit
            {
                return Err(too_large(limit));
            }
        }
        Ok(())
    }
}

pub(super) fn sse<S, E>(stream: S, limit: usize) -> BoxStream<'static, Result<Sse, SseError>>
where
    S: Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    let admitted = futures::stream::try_unfold(
        (stream.boxed(), EventSize::default()),
        move |(mut stream, mut size)| async move {
            let Some(chunk) = stream.next().await else {
                return Ok(None);
            };
            let chunk = chunk.map_err(io::Error::other)?;
            size.observe(&chunk, limit)?;
            Ok::<_, io::Error>(Some((chunk, (stream, size))))
        },
    );
    SseStream::from_bytes_stream(admitted)
        .map(|item| {
            item.and_then(|event| {
                if let Some(data) = &event.data {
                    capsem_proto::mcp_contracts::json_rpc_wire::validate_size(data.as_bytes())
                        .map_err(|_| SseError::Body(Box::new(too_large(capsem_proto::MCP_PAYLOAD_MAX_BYTES))))?;
                }
                Ok(event)
            })
        })
        .boxed()
}

#[cfg(test)]
mod tests;
