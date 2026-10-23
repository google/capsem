use super::*;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use tokio::io::{DuplexStream, ReadBuf};
use tokio::sync::watch;

#[derive(Clone, Default)]
struct ReadState {
    bytes: usize,
    pending: usize,
    remaining: usize,
}

struct ObservedIo {
    inner: DuplexStream,
    reads: watch::Sender<ReadState>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl AsyncRead for ObservedIo {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        match &result {
            Poll::Ready(Ok(())) => self
                .reads
                .send_modify(|state| state.bytes += buf.filled().len() - before),
            Poll::Pending => {
                *self.waker.lock().unwrap() = Some(cx.waker().clone());
                self.reads.send_modify(|state| {
                    state.pending += 1;
                    state.remaining = buf.remaining();
                });
            }
            _ => {}
        }
        result
    }
}

impl AsyncWrite for ObservedIo {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[tokio::test]
async fn completing_previous_call_preserves_the_partially_read_next_frame() {
    let (endpoint, driver) = gated_endpoint(2, true);
    let db = Arc::new(DbWriter::open_in_memory(64).unwrap());
    let (mut guest, inner) = tokio::io::duplex(1 << 16);
    let (reads, mut observed) = watch::channel(ReadState::default());
    let waker = Arc::new(Mutex::new(None));
    let server = ObservedIo {
        inner,
        reads,
        waker: Arc::clone(&waker),
    };
    let owner = tokio::spawn(serve_io(Vec::new(), server, endpoint, db));
    let first = tool_call_request(1);
    let second = tool_call_request(2);
    guest.write_all(&first).await.unwrap();
    wait_for_started(&driver, 1).await;
    guest.write_all(&second[..7]).await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(2),
        observed.wait_for(|state| state.bytes == first.len() + 7),
    )
    .await
    .unwrap()
    .unwrap();
    driver.release.send(true).unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(2), read_reply(&mut guest))
        .await
        .unwrap()
        .error
        .is_none());
    let pending = observed.borrow().pending;
    waker.lock().unwrap().as_ref().unwrap().wake_by_ref();
    tokio::time::timeout(
        Duration::from_secs(2),
        observed.wait_for(|state| state.pending > pending),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        observed.borrow().remaining,
        second.len() - 7,
        "worker completion must retain the original partially consumed body, not restart its length prefix"
    );
    guest.write_all(&second[7..]).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(2), read_reply(&mut guest))
        .await
        .unwrap()
        .error
        .is_none());
    guest.write_all(&capsem_proto::MCP_SESSION_END).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), owner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(driver.started.load(Ordering::SeqCst), 2);
}
