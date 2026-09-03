//! Every read after the first byte is bounded.
//!
//! The classification read had a deadline, but three reads after it did
//! not: the framed-MCP prefix loop, the guest TLS handshake, and hyper's
//! header read (whose default 30s timeout is silently dropped when the
//! server builder has no timer). A guest that sent one byte and stalled
//! pinned the handler task and its ACTIVE_CONNECTIONS slot forever.
//!
//! These tests run on tokio's paused clock: when every task is idle the
//! runtime jumps to the next timer, so a real deadline fires instantly and
//! a missing one leaves the proxy task pending until the outer bound.

use std::time::Duration;

use super::*;

/// Far beyond any proxy deadline. On the paused clock this only ever
/// elapses when the proxy has no timer of its own.
const OUTER_BOUND: Duration = Duration::from_secs(600);

/// Send `prefix`, then stall. The proxy task returning is the observable
/// release of the connection slot: `handle_connection` drops its gauge
/// guard on return, and nothing else does.
async fn stalled_guest_is_disconnected(prefix: &[u8]) {
    let (config, _db) = make_proxy_config(&[], &[], true);
    let (proxy_task, addr) = spawn_proxy(config).await;
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(prefix).await.unwrap();

    let finished = tokio::time::timeout(OUTER_BOUND, proxy_task).await;
    assert!(
        finished.is_ok(),
        "proxy kept the connection after {prefix:?} + stall: no deadline on the next read"
    );
    finished.unwrap().unwrap();

    // The guest side observes the teardown too: no bytes, or a close.
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(OUTER_BOUND, tcp.read_to_end(&mut buf))
        .await
        .expect("proxy closed the socket");
}

#[tokio::test(start_paused = true)]
async fn stalled_guest_after_tls_first_byte_is_disconnected() {
    // 0x16 = TLS handshake record: classification hands off to the TLS
    // acceptor, which then waits for the rest of the ClientHello.
    stalled_guest_is_disconnected(b"\x16").await;
}

#[tokio::test(start_paused = true)]
async fn stalled_guest_after_partial_client_hello_is_disconnected() {
    // A plausible record header announcing a 512-byte ClientHello that
    // never arrives.
    stalled_guest_is_disconnected(b"\x16\x03\x01\x02\x00\x01").await;
}

#[tokio::test(start_paused = true)]
async fn stalled_guest_after_http_first_byte_is_disconnected() {
    // 'G': plain HTTP, hyper waits for the rest of the request head.
    stalled_guest_is_disconnected(b"G").await;
}

#[tokio::test(start_paused = true)]
async fn stalled_guest_after_partial_http_head_is_disconnected() {
    stalled_guest_is_disconnected(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Slow: ").await;
}

#[tokio::test(start_paused = true)]
async fn stalled_guest_after_mcp_prefix_byte_is_disconnected() {
    // 0x00: the framed-MCP prefix loop pulls up to six bytes for the
    // classifier before deciding.
    stalled_guest_is_disconnected(b"\x00").await;
}

#[tokio::test(start_paused = true)]
async fn stalled_guest_after_partial_metadata_is_disconnected() {
    // Metadata prefix without its terminating newline.
    stalled_guest_is_disconnected(b"\x00CAPSEM_META:curl").await;
}
