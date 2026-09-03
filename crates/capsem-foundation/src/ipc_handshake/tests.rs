//! Tests for ipc_handshake. Use a UnixStream pair (socketpair) to
//! exercise initiator+responder against the same wire.

use super::*;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;

#[test]
fn negotiate_succeeds_when_both_sides_match() {
    let (mut a, mut b) = UnixStream::pair().unwrap();

    let initiator = std::thread::spawn(move || negotiate_initiator(&mut a, "capsem-service-test", ""));
    let responder = std::thread::spawn(move || negotiate_responder(&mut b, "capsem-process-test", ""));

    let init_peer = initiator.join().unwrap().unwrap();
    let resp_peer = responder.join().unwrap().unwrap();

    assert_eq!(init_peer.peer, "capsem-process-test");
    assert_eq!(resp_peer.peer, "capsem-service-test");
}

#[test]
fn negotiate_times_out_when_peer_silent() {
    let (mut a, _b) = UnixStream::pair().unwrap();
    let timeout = Duration::from_millis(10);

    let err = negotiate_responder_with_timeout(&mut a, "capsem-service-test", "", timeout).unwrap_err();

    match err {
        HandshakeError::Timeout { timeout_ms } => {
            assert_eq!(timeout_ms, timeout.as_millis() as u64);
        }
        other => panic!("expected timeout, got {other:?}"),
    }
}

#[test]
fn negotiate_fails_on_schema_mismatch() {
    let (mut a, mut b) = UnixStream::pair().unwrap();
    let (done_tx, done_rx) = mpsc::channel();

    // Consume the initiator's Hello, then write a bad one. Reading first
    // makes the test deterministic against the initiator's write ordering.
    std::thread::spawn(move || {
        let peer = read_hello(&mut b, HELLO_TIMEOUT).unwrap();
        verify(&peer).unwrap();

        let mut bad = Hello::ours("capsem-process-stale", "");
        bad.schema_hash = bad.schema_hash.wrapping_add(0xdead);
        write_hello(&mut b, &bad).unwrap();
        let _ = done_rx.recv();
    });

    let err = negotiate_initiator(&mut a, "capsem-service-test", "").unwrap_err();
    let _ = done_tx.send(());
    assert!(matches!(err, HandshakeError::Schema { .. }), "{err:?}");
    let msg = err.to_string();
    assert!(msg.contains("capsem-process-stale"), "msg: {msg}");
}

// The service ran the initiator inline on its runtime. On a single worker
// that is the whole service stalled for the length of the exchange; a
// ticker that must keep counting while a slow peer answers shows whether
// the handshake left the worker free.
#[tokio::test(flavor = "current_thread")]
async fn off_worker_initiator_leaves_the_runtime_responsive() {
    let (a, mut b) = UnixStream::pair().unwrap();
    let responder = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        negotiate_responder(&mut b, "capsem-process-test", "")
    });

    let ticks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let ticker = {
        let ticks = std::sync::Arc::clone(&ticks);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                ticks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        })
    };

    let (_stream, hello) = negotiate_initiator_off_worker(a, "capsem-service-test", "")
        .await
        .expect("handshake completes");
    ticker.abort();
    responder.join().unwrap().unwrap();

    assert_eq!(hello.peer, "capsem-process-test");
    let ticks = ticks.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        ticks >= 10,
        "the worker must keep running other tasks during the handshake; saw {ticks} ticks over ~300ms"
    );
}
