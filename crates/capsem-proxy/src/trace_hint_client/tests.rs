use std::os::unix::net::UnixStream;

use capsem_core::net::ai_traffic::TraceHintSink as _;
use capsem_proto::proxy_trace_hints::PROXY_TRACE_HINT_ACK;
use capsem_proto::proxy_trace_hints::{decode_proxy_trace_hint, PROXY_TRACE_HINT_FRAME_SIZE};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

#[tokio::test]
async fn writes_fixed_bounded_trace_hints_to_the_owner() {
    let (proxy, owner) = UnixStream::pair().unwrap();
    let client = super::TraceHintClient::start(proxy).unwrap();
    let reader = tokio::spawn(async move {
        owner.set_nonblocking(true).unwrap();
        let mut owner = tokio::net::UnixStream::from_std(owner).unwrap();
        let mut frame = [0; PROXY_TRACE_HINT_FRAME_SIZE];
        owner.read_exact(&mut frame).await.unwrap();
        let hint = decode_proxy_trace_hint(&frame).unwrap();
        owner.write_all(&PROXY_TRACE_HINT_ACK).await.unwrap();
        hint
    });

    client
        .register("2e934d32-1e3e-4d51-b816-e14cc662833f", &["reports/result.txt".into()])
        .await
        .unwrap();
    let hint = reader.await.unwrap();
    assert_eq!(hint.relative_path, "reports/result.txt");
}
