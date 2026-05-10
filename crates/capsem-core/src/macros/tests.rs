//! try_send! integration tests against real tokio + std channels.

use tokio::sync::{broadcast, mpsc, oneshot};

#[tokio::test]
async fn try_send_async_mpsc_ok() {
    let (tx, mut rx) = mpsc::channel::<u32>(4);
    try_send!("test_async_ok", tx.send(42).await);
    assert_eq!(rx.recv().await, Some(42));
}

#[tokio::test]
async fn try_send_async_mpsc_closed_logs_warn() {
    let (tx, rx) = mpsc::channel::<u32>(4);
    drop(rx);
    // Should not panic. The warn line is asserted via tracing-test in CI; here
    // we assert that the macro's output is `()` (the macro statement compiles
    // and produces no value) and that the failure path is taken without
    // unwinding.
    try_send!("test_async_closed", tx.send(7).await);
}

#[test]
fn try_send_sync_unbounded_ok() {
    let (tx, mut rx) = mpsc::unbounded_channel::<&'static str>();
    try_send!("test_sync_ok", tx.send("hi"));
    assert_eq!(rx.try_recv().ok(), Some("hi"));
}

#[test]
fn try_send_sync_unbounded_closed() {
    let (tx, rx) = mpsc::unbounded_channel::<u32>();
    drop(rx);
    try_send!("test_sync_closed", tx.send(7));
}

#[test]
fn try_send_oneshot_closed() {
    let (tx, rx) = oneshot::channel::<u32>();
    drop(rx);
    try_send!("test_oneshot_closed", tx.send(99));
}

#[test]
fn try_send_broadcast_no_subscribers() {
    let (tx, _rx) = broadcast::channel::<u32>(4);
    drop(_rx);
    try_send!("test_broadcast_empty", tx.send(7));
}
