/// SSE-aware response body wrapper for AI provider traffic.
///
/// Wraps a hyper Body, forwarding frames unchanged while feeding data to an
/// SSE parser -> provider stream parser pipeline. Parsing is synchronous and
/// runs inline in `poll_frame` -- zero added latency.
///
/// After the stream completes, the collected `LlmEvent`s can be read from
/// `AiStreamState` and summarized via `collect_summary()`.
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use hyper::body::{Body, Buf, Frame};

use super::events::{LlmEvent, ProviderStreamParser};
use super::sse::SseParser;

/// Shared state for SSE parsing, accessible after the stream completes.
pub struct AiStreamState {
    pub sse_parser: SseParser,
    pub provider_parser: Box<dyn ProviderStreamParser + Send>,
    pub events: Vec<LlmEvent>,
}

/// Body stats tracked alongside SSE parsing.
pub struct AiBodyStats {
    pub bytes: u64,
    pub preview: Vec<u8>,
    pub max_preview: usize,
}

pin_project_lite::pin_project! {
    /// A hyper Body wrapper that does SSE parsing inline during `poll_frame`.
    ///
    /// Forwards all frames unchanged to the downstream consumer (hyper server
    /// -> guest TLS stream) while simultaneously feeding the raw bytes to the
    /// SSE parser chain. This means the guest sees zero added latency.
    pub struct AiResponseBody<B> {
        #[pin]
        inner: B,
        stats: Arc<Mutex<AiBodyStats>>,
        ai_state: Arc<Mutex<AiStreamState>>,
        max_size: u64,
        on_drop: Option<tokio::sync::oneshot::Sender<()>>,
    }
}

impl<B> AiResponseBody<B> {
    pub fn new(
        inner: B,
        provider_parser: Box<dyn ProviderStreamParser + Send>,
        max_preview: usize,
        max_size: u64,
    ) -> Self {
        let stats = Arc::new(Mutex::new(AiBodyStats {
            bytes: 0,
            preview: Vec::new(),
            max_preview,
        }));
        let ai_state = Arc::new(Mutex::new(AiStreamState {
            sse_parser: SseParser::new(),
            provider_parser,
            events: Vec::new(),
        }));
        Self { inner, stats, ai_state, max_size, on_drop: None }
    }

    /// Set a oneshot sender that will be triggered (dropped) when this body is dropped.
    pub fn with_on_drop(mut self, tx: tokio::sync::oneshot::Sender<()>) -> Self {
        self.on_drop = Some(tx);
        self
    }

    /// Get a handle to the body stats (for reading after stream ends).
    pub fn stats(&self) -> Arc<Mutex<AiBodyStats>> {
        Arc::clone(&self.stats)
    }

    /// Get a handle to the AI stream state (for reading events after stream ends).
    pub fn ai_state(&self) -> Arc<Mutex<AiStreamState>> {
        Arc::clone(&self.ai_state)
    }
}

impl<B> Body for AiResponseBody<B>
where
    B: Body,
    B::Error: Into<anyhow::Error>,
{
    type Data = B::Data;
    type Error = anyhow::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.project();
        match this.inner.poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let chunk = data.chunk();
                    let len = chunk.len() as u64;

                    // Update stats
                    if let Ok(mut st) = this.stats.lock() {
                        st.bytes += len;
                        if st.bytes > *this.max_size {
                            return Poll::Ready(Some(Err(
                                anyhow::anyhow!("AI response body exceeded maximum size"),
                            )));
                        }
                        let remaining = st.max_preview.saturating_sub(st.preview.len());
                        if remaining > 0 {
                            let to_copy = remaining.min(chunk.len());
                            st.preview.extend_from_slice(&chunk[..to_copy]);
                        }
                    }

                    // Feed to SSE parser -> provider parser (synchronous, fast)
                    if let Ok(mut ai) = this.ai_state.lock() {
                        let sse_events = ai.sse_parser.feed(chunk);
                        for sse_event in &sse_events {
                            let llm_events = ai.provider_parser.parse_event(sse_event);
                            ai.events.extend(llm_events);
                        }
                    }
                }

                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e.into()))),
            Poll::Ready(None) => {
                // Stream ended -- flush the SSE parser
                if let Ok(mut ai) = this.ai_state.lock() {
                    if let Some(sse_event) = ai.sse_parser.flush() {
                        let llm_events = ai.provider_parser.parse_event(&sse_event);
                        ai.events.extend(llm_events);
                    }
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
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::anthropic::AnthropicStreamParserWithState;
    use crate::gateway::events::collect_summary;
    use http_body_util::BodyExt;
    use hyper::body::Bytes;

    /// Create a simple body from byte chunks for testing.
    fn chunks_body(chunks: Vec<&'static [u8]>) -> http_body_util::StreamBody<futures::stream::Iter<std::vec::IntoIter<Result<Frame<Bytes>, std::io::Error>>>> {
        let frames: Vec<Result<Frame<Bytes>, std::io::Error>> = chunks
            .into_iter()
            .map(|c| Ok(Frame::data(Bytes::from(c))))
            .collect();
        http_body_util::StreamBody::new(futures::stream::iter(frames))
    }

    #[tokio::test]
    async fn parses_anthropic_stream_inline() {
        let sse_data: &[u8] = b"\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi!\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

        let body = chunks_body(vec![sse_data]);
        let parser = Box::new(AnthropicStreamParserWithState::new());
        let ai_body = AiResponseBody::new(body, parser, 64 * 1024, 100 * 1024 * 1024);
        let ai_state = ai_body.ai_state();
        let stats = ai_body.stats();

        // Consume the body (simulates hyper streaming to the client)
        let collected = ai_body.collect().await.unwrap();
        let total_bytes = collected.to_bytes().len();
        assert!(total_bytes > 0);

        // Check stats
        let st = stats.lock().unwrap();
        assert_eq!(st.bytes, total_bytes as u64);

        // Check parsed events
        let ai = ai_state.lock().unwrap();
        assert!(!ai.events.is_empty());
        let summary = collect_summary(&ai.events);
        assert_eq!(summary.message_id.as_deref(), Some("msg_01"));
        assert_eq!(summary.text, "Hi!");
        assert_eq!(summary.input_tokens, Some(10));
        assert_eq!(summary.output_tokens, Some(5));
    }

    #[tokio::test]
    async fn handles_chunked_sse_data() {
        // Split the SSE data across multiple chunks
        let chunk1: &[u8] = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"model\":\"test\"";
        let chunk2: &[u8] = b"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n";
        let chunk3: &[u8] = b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

        let body = chunks_body(vec![chunk1, chunk2, chunk3]);
        let parser = Box::new(AnthropicStreamParserWithState::new());
        let ai_body = AiResponseBody::new(body, parser, 64 * 1024, 100 * 1024 * 1024);
        let ai_state = ai_body.ai_state();

        // Consume
        let _ = ai_body.collect().await.unwrap();

        let ai = ai_state.lock().unwrap();
        let summary = collect_summary(&ai.events);
        assert_eq!(summary.text, "Hello");
        assert_eq!(summary.message_id.as_deref(), Some("m1"));
    }

    #[tokio::test]
    async fn respects_max_preview() {
        let data = "data: ".to_string() + &"x".repeat(1000) + "\n\n";
        let body = chunks_body(vec![data.leak().as_bytes()]);
        let parser = Box::new(AnthropicStreamParserWithState::new());
        let ai_body = AiResponseBody::new(body, parser, 100, 100 * 1024 * 1024);
        let stats = ai_body.stats();

        let _ = ai_body.collect().await.unwrap();

        let st = stats.lock().unwrap();
        assert!(st.bytes > 100);
        assert_eq!(st.preview.len(), 100);
    }

    #[tokio::test]
    async fn empty_body_ok() {
        let body = chunks_body(vec![]);
        let parser = Box::new(AnthropicStreamParserWithState::new());
        let ai_body = AiResponseBody::new(body, parser, 1024, 100 * 1024 * 1024);
        let ai_state = ai_body.ai_state();

        let _ = ai_body.collect().await.unwrap();

        let ai = ai_state.lock().unwrap();
        assert!(ai.events.is_empty());
    }
}
