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

/// SSE accounting follows rmcp 3.10's client_side_sse admission rules
/// (Apache-2.0 OR MIT): retain complete data fields, discard complete comments,
/// and treat CRLF as one line ending even across transport chunks.
#[derive(Default)]
struct EventSize {
    retained: usize,
    line: usize,
    comment: bool,
    after_cr: bool,
}

impl EventSize {
    fn observe(&mut self, bytes: &[u8], limit: usize) -> io::Result<()> {
        for &byte in bytes {
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
                    } else if !self.comment {
                        self.retained = self.retained.saturating_add(self.line).saturating_add(1);
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
                }
            }
            if self.retained.saturating_add(self.line) > limit {
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
    SseStream::from_bytes_stream(admitted).boxed()
}

#[cfg(test)]
mod tests;
