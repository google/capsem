use super::*;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn push_then_poll_returns_bytes() {
    let q = TerminalOutputQueue::new();
    q.push(b"hello".to_vec());
    let out = q.poll().await.unwrap();
    assert_eq!(out, b"hello");
}

#[tokio::test]
async fn default_constructs_empty_queue() {
    let q = TerminalOutputQueue::default();
    q.push(b"x".to_vec());
    assert_eq!(q.poll().await.unwrap(), b"x");
}

#[tokio::test]
async fn poll_coalesces_multiple_chunks() {
    let q = TerminalOutputQueue::new();
    q.push(b"aa".to_vec());
    q.push(b"bb".to_vec());
    q.push(b"cc".to_vec());
    let out = q.poll().await.unwrap();
    // All three chunks fit well under 1MB — should coalesce into one.
    assert_eq!(out, b"aabbcc");
}

#[tokio::test]
async fn poll_stops_coalescing_at_batch_boundary() {
    let q = TerminalOutputQueue::new();
    // First chunk just under 1MB; second chunk would push us past it.
    let big = vec![0u8; DEFAULT_MAX_COALESCE_BYTES - 10];
    let tail = vec![1u8; 100];
    q.push(big.clone());
    q.push(tail.clone());
    let first = q.poll().await.unwrap();
    assert_eq!(first.len(), big.len(), "should not have coalesced the second chunk");
    let second = q.poll().await.unwrap();
    assert_eq!(second, tail);
}

#[tokio::test]
async fn close_returns_none_on_empty() {
    let q = TerminalOutputQueue::new();
    q.close();
    assert!(q.poll().await.is_none());
}

#[tokio::test]
async fn close_drains_before_returning_none() {
    let q = TerminalOutputQueue::new();
    q.push(b"tail".to_vec());
    q.close();
    assert_eq!(q.poll().await.unwrap(), b"tail");
    assert!(q.poll().await.is_none());
}

#[tokio::test]
async fn push_after_close_is_dropped() {
    let q = TerminalOutputQueue::new();
    q.close();
    q.push(b"ignored".to_vec());
    assert!(q.poll().await.is_none());
}

#[tokio::test]
async fn reset_reopens_closed_queue() {
    let q = TerminalOutputQueue::new();
    q.push(b"old".to_vec());
    q.close();
    q.reset();
    q.push(b"new".to_vec());
    assert_eq!(q.poll().await.unwrap(), b"new");
}

#[tokio::test]
async fn poll_waits_for_push_from_another_task() {
    let q = Arc::new(TerminalOutputQueue::new());
    let q2 = q.clone();
    let handle = tokio::spawn(async move { q2.poll().await });
    // Give the consumer a moment to reach the await.
    tokio::time::sleep(Duration::from_millis(10)).await;
    q.push(b"wake".to_vec());
    let out = handle.await.unwrap().unwrap();
    assert_eq!(out, b"wake");
}

#[tokio::test]
async fn queue_drops_oldest_at_capacity() {
    let q = TerminalOutputQueue::new();
    for i in 0..TERMINAL_QUEUE_CAPACITY {
        q.push(vec![(i & 0xff) as u8]);
    }
    // One more — forces the oldest to be dropped.
    q.push(b"latest".to_vec());
    // Drain everything.
    let mut seen_latest = false;
    let mut total_chunks = 0usize;
    while let Some(chunk) = tokio::time::timeout(Duration::from_millis(10), q.poll())
        .await
        .ok()
        .flatten()
    {
        total_chunks += 1;
        if chunk.ends_with(b"latest") || chunk == b"latest" {
            seen_latest = true;
        }
        if total_chunks > TERMINAL_QUEUE_CAPACITY + 2 {
            break;
        }
    }
    assert!(seen_latest, "new chunk should still be present after backpressure drop");
}
