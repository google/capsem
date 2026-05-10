//! Tests for ipc_handshake. Use a UnixStream pair (socketpair) to
//! exercise initiator+responder against the same wire.

use super::*;
use std::os::unix::net::UnixStream;

#[test]
fn negotiate_succeeds_when_both_sides_match() {
    let (mut a, mut b) = UnixStream::pair().unwrap();

    let initiator =
        std::thread::spawn(move || negotiate_initiator(&mut a, "capsem-service-test", ""));
    let responder =
        std::thread::spawn(move || negotiate_responder(&mut b, "capsem-process-test", ""));

    let init_peer = initiator.join().unwrap().unwrap();
    let resp_peer = responder.join().unwrap().unwrap();

    assert_eq!(init_peer.peer, "capsem-process-test");
    assert_eq!(resp_peer.peer, "capsem-service-test");
}

#[test]
#[ignore = "waits the full 5s HELLO_TIMEOUT; run with --include-ignored when verifying handshake"]
fn negotiate_times_out_when_peer_silent() {
    let (mut a, _b) = UnixStream::pair().unwrap();
    // _b kept alive but never writes a Hello -- our side waits for one.
    // Use a deliberately short timeout for the test by calling read_hello
    // directly (negotiate_responder reads first).
    let err = negotiate_responder(&mut a, "capsem-service-test", "").unwrap_err();
    // Default HELLO_TIMEOUT is 5s; this test waits the full 5s. Trade-off:
    // accept the latency to keep the public API minimal. To make tests
    // fast, we'd parameterize the timeout -- not worth doing today.
    assert!(matches!(err, HandshakeError::Timeout { .. }), "{err:?}");
}

#[test]
fn negotiate_fails_on_schema_mismatch() {
    let (mut a, mut b) = UnixStream::pair().unwrap();

    // Consume the initiator's Hello, then write a bad one. Reading first
    // makes the test deterministic against the initiator's write ordering.
    std::thread::spawn(move || {
        let mut len_buf = [0u8; 4];
        b.read_exact(&mut len_buf).unwrap();
        let n = u32::from_be_bytes(len_buf);
        let mut payload = vec![0u8; n as usize];
        b.read_exact(&mut payload).unwrap();

        let mut bad = Hello::ours("capsem-process-stale", "");
        bad.schema_hash = bad.schema_hash.wrapping_add(0xdead);
        let payload = rmp_serde::to_vec_named(&bad).unwrap();
        let len = (payload.len() as u32).to_be_bytes();
        b.write_all(&len).unwrap();
        b.write_all(&payload).unwrap();
        b.flush().unwrap();
    });

    let err = negotiate_initiator(&mut a, "capsem-service-test", "").unwrap_err();
    assert!(matches!(err, HandshakeError::Schema { .. }), "{err:?}");
    let msg = err.to_string();
    assert!(msg.contains("capsem-process-stale"), "msg: {msg}");
}
