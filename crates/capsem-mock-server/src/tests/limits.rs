use super::*;
use crate::limits::MAX_WS_FRAME_BYTES;
use tokio::net::TcpStream;

/// A peer chooses the frame length and the generated body size. Both are
/// bounded before any allocation: a 10-byte frame header must not be able to
/// abort the process, and a URL must not be able to reserve a terabyte.

#[tokio::test]
async fn generated_bodies_above_the_cap_are_refused_with_a_client_error() {
    for path in [
        "/bytes/1099511627776",
        "/gzip/1099511627776",
        "/bytes/16777217",
        "/gzip/16777217",
    ] {
        let (status, _, body) = routed(Method::GET, path, None, HeaderMap::new(), json!({})).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{path}");
        assert!(body.is_empty(), "{path}");
    }
    // usize::MAX and beyond parse as "not a size" today and must stay refused.
    for path in ["/bytes/18446744073709551615", "/bytes/99999999999999999999999999"] {
        let (status, _, _) = routed(Method::GET, path, None, HeaderMap::new(), json!({})).await;
        assert!(status.is_client_error(), "{path}: {status}");
    }
    let (status, _, body) = routed(Method::GET, "/bytes/16777216", None, HeaderMap::new(), json!({})).await;
    assert_eq!(status, StatusCode::OK, "the cap itself is still served");
    assert_eq!(body.len(), 16 * 1024 * 1024);
}

async fn start_http_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(serve_http(listener, State { request_log: None }, false));
    addr
}

async fn open_ws(addr: SocketAddr) -> TcpStream {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(
            b"GET /ws/echo HTTP/1.1\r\nHost: mock\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
              Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
        )
        .await
        .expect("write upgrade");
    let mut response = Vec::new();
    let mut byte = [0_u8; 1];
    while !response.ends_with(b"\r\n\r\n") {
        assert_eq!(
            stream.read(&mut byte).await.expect("read upgrade"),
            1,
            "upgrade response truncated"
        );
        response.push(byte[0]);
    }
    assert!(
        response.starts_with(b"HTTP/1.1 101"),
        "{}",
        String::from_utf8_lossy(&response)
    );
    stream
}

fn masked_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mask = [0x11, 0x22, 0x33, 0x44];
    let mut frame = vec![
        0x80 | opcode,
        0x80 | u8::try_from(payload.len()).expect("short payload"),
    ];
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(idx, byte)| byte ^ mask[idx % 4]));
    frame
}

async fn read_until_eof(stream: &mut TcpStream) -> Vec<u8> {
    let mut received = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut received))
        .await
        .expect("server must close the connection")
        .expect("read");
    received
}

async fn assert_server_alive(addr: SocketAddr) {
    let mut ws = open_ws(addr).await;
    ws.write_all(&masked_frame(0x1, b"still here"))
        .await
        .expect("write echo");
    let mut echoed = [0_u8; 12];
    tokio::time::timeout(Duration::from_secs(10), ws.read_exact(&mut echoed))
        .await
        .expect("echo must arrive")
        .expect("read echo");
    assert_eq!(&echoed, b"\x81\x0astill here");

    let mut plain = TcpStream::connect(addr).await.expect("connect");
    plain
        .write_all(b"GET /tiny HTTP/1.1\r\nHost: mock\r\nConnection: close\r\n\r\n")
        .await
        .expect("write GET");
    let body = read_until_eof(&mut plain).await;
    assert!(String::from_utf8_lossy(&body).contains("capsem-mock-server:tiny"));
}

#[tokio::test]
async fn an_oversized_websocket_frame_closes_the_connection_and_leaves_the_server_running() {
    let addr = start_http_server().await;
    let mut ws = open_ws(addr).await;
    // A 10-byte header declaring a 1 TiB payload, plus the mask.
    let mut frame = vec![0x82, 0x80 | 127];
    frame.extend_from_slice(&(1_u64 << 40).to_be_bytes());
    frame.extend_from_slice(&[0, 0, 0, 0]);
    ws.write_all(&frame).await.expect("write frame header");

    let received = read_until_eof(&mut ws).await;
    assert_eq!(
        received.first(),
        Some(&0x88),
        "server must answer with a close frame: {received:?}"
    );
    assert_eq!(
        &received[2..4],
        &1009_u16.to_be_bytes(),
        "close status must be 1009 message too big"
    );

    assert_server_alive(addr).await;
}

#[tokio::test]
async fn a_frame_one_byte_over_the_cap_is_refused_and_one_at_the_cap_is_not_pre_rejected() {
    let addr = start_http_server().await;
    let mut ws = open_ws(addr).await;
    let mut frame = vec![0x82, 0x80 | 127];
    frame.extend_from_slice(&(MAX_WS_FRAME_BYTES + 1).to_be_bytes());
    frame.extend_from_slice(&[0, 0, 0, 0]);
    ws.write_all(&frame).await.expect("write frame header");
    let received = read_until_eof(&mut ws).await;
    assert_eq!(received.first(), Some(&0x88), "{received:?}");

    // At the cap the server waits for the payload rather than closing: the
    // client hanging up is what ends the frame.
    let mut ws = open_ws(addr).await;
    let mut frame = vec![0x82, 0x80 | 127];
    frame.extend_from_slice(&MAX_WS_FRAME_BYTES.to_be_bytes());
    frame.extend_from_slice(&[0, 0, 0, 0]);
    ws.write_all(&frame).await.expect("write frame header");
    let mut probe = [0_u8; 1];
    let waited = tokio::time::timeout(Duration::from_millis(300), ws.read(&mut probe)).await;
    assert!(
        waited.is_err(),
        "a frame at the cap must not be pre-rejected: {waited:?}"
    );
    drop(ws);

    assert_server_alive(addr).await;
}
