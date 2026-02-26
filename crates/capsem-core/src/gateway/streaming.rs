/// SSE stream forwarding: tees upstream response chunks to the client while
/// accumulating the full body for audit logging.
///
/// Works for all providers -- does not parse SSE events semantically (that
/// comes later for tool call interception). Just forwards bytes and counts.
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures::Stream;

// Re-use the Bytes type from http (shared by axum, reqwest, hyper).
type Bytes = axum::body::Bytes;

/// Wraps a reqwest byte stream, forwarding chunks while accumulating body data.
pub struct StreamAccumulator<S> {
    inner: Pin<Box<S>>,
    accumulated: Arc<Mutex<Vec<u8>>>,
    bytes_count: Arc<AtomicU64>,
    max_capture: usize,
}

impl<S> StreamAccumulator<S> {
    pub fn new(stream: S, max_capture: usize) -> Self {
        Self {
            inner: Box::pin(stream),
            accumulated: Arc::new(Mutex::new(Vec::new())),
            bytes_count: Arc::new(AtomicU64::new(0)),
            max_capture,
        }
    }

    /// Get a handle to the accumulated buffer (for reading after stream ends).
    pub fn accumulated(&self) -> Arc<Mutex<Vec<u8>>> {
        Arc::clone(&self.accumulated)
    }

    /// Get a handle to the byte counter.
    pub fn bytes_count(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.bytes_count)
    }
}

impl<S, E> Stream for StreamAccumulator<S>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    type Item = Result<Bytes, E>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                let len = chunk.len() as u64;
                self.bytes_count.fetch_add(len, Ordering::Relaxed);

                if let Ok(mut buf) = self.accumulated.lock() {
                    let remaining = self.max_capture.saturating_sub(buf.len());
                    if remaining > 0 {
                        let to_copy = remaining.min(chunk.len());
                        buf.extend_from_slice(&chunk[..to_copy]);
                    }
                }

                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Extract the accumulated body as a string (lossy UTF-8 conversion).
pub fn drain_accumulated(accumulated: &Arc<Mutex<Vec<u8>>>) -> Option<String> {
    let buf = accumulated.lock().ok()?;
    if buf.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn mock_stream(
        chunks: Vec<&'static [u8]>,
    ) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Unpin {
        futures::stream::iter(
            chunks
                .into_iter()
                .map(|c| Ok(Bytes::from(c)))
                .collect::<Vec<_>>(),
        )
    }

    #[tokio::test]
    async fn accumulates_all_chunks() {
        let stream = mock_stream(vec![b"hello ", b"world"]);
        let mut acc = StreamAccumulator::new(stream, 1024);
        let accumulated = acc.accumulated();
        let count = acc.bytes_count();

        // Consume the stream.
        while let Some(Ok(_)) = acc.next().await {}

        assert_eq!(count.load(Ordering::Relaxed), 11);
        let body = drain_accumulated(&accumulated).unwrap();
        assert_eq!(body, "hello world");
    }

    #[tokio::test]
    async fn respects_max_capture() {
        let stream = mock_stream(vec![b"0123456789", b"abcdefghij"]);
        let mut acc = StreamAccumulator::new(stream, 5);
        let accumulated = acc.accumulated();
        let count = acc.bytes_count();

        while let Some(Ok(_)) = acc.next().await {}

        // Byte count tracks all bytes regardless of capture limit.
        assert_eq!(count.load(Ordering::Relaxed), 20);
        // But accumulated buffer is capped.
        let body = drain_accumulated(&accumulated).unwrap();
        assert_eq!(body, "01234");
    }

    #[tokio::test]
    async fn empty_stream() {
        let stream = mock_stream(vec![]);
        let mut acc = StreamAccumulator::new(stream, 1024);
        let accumulated = acc.accumulated();

        while let Some(Ok(_)) = acc.next().await {}

        assert!(drain_accumulated(&accumulated).is_none());
    }

    #[tokio::test]
    async fn forwards_chunks_faithfully() {
        let stream = mock_stream(vec![b"chunk1", b"chunk2", b"chunk3"]);
        let mut acc = StreamAccumulator::new(stream, 1024);

        let mut received = Vec::new();
        while let Some(Ok(chunk)) = acc.next().await {
            received.push(chunk);
        }

        assert_eq!(received.len(), 3);
        assert_eq!(&received[0][..], b"chunk1");
        assert_eq!(&received[1][..], b"chunk2");
        assert_eq!(&received[2][..], b"chunk3");
    }

    #[tokio::test]
    async fn max_capture_zero_accumulates_nothing() {
        let stream = mock_stream(vec![b"data"]);
        let mut acc = StreamAccumulator::new(stream, 0);
        let accumulated = acc.accumulated();

        while let Some(Ok(_)) = acc.next().await {}

        assert!(drain_accumulated(&accumulated).is_none());
    }
}
