use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use capsem_core::net::ai_traffic::TraceState;
use capsem_proto::proxy_trace_hints::{encode_proxy_trace_hint, ProxyTraceHint, PROXY_TRACE_HINT_ACK};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

#[tokio::test]
async fn proxy_hint_updates_the_workspace_owners_trace_state() {
    let (proxy, owner) = UnixStream::pair().unwrap();
    let state = Arc::new(Mutex::new(TraceState::new()));
    let serving = tokio::spawn(super::serve(owner, Arc::clone(&state)));
    capsem_foundation::unix::fd::set_nonblocking(proxy.as_fd(), true).unwrap();
    let mut proxy = tokio::net::UnixStream::from_std(proxy).unwrap();
    let frame = encode_proxy_trace_hint(&ProxyTraceHint {
        trace_id: "2e934d32-1e3e-4d51-b816-e14cc662833f".into(),
        relative_path: "reports/result.txt".into(),
    })
    .unwrap();
    proxy.write_all(&frame).await.unwrap();
    let mut acknowledgement = [0; PROXY_TRACE_HINT_ACK.len()];
    proxy.read_exact(&mut acknowledgement).await.unwrap();
    assert_eq!(acknowledgement, PROXY_TRACE_HINT_ACK);
    drop(proxy);
    serving.await.unwrap().unwrap();

    assert_eq!(
        state.lock().unwrap().lookup_file_path("reports/result.txt").as_deref(),
        Some("2e934d32-1e3e-4d51-b816-e14cc662833f")
    );
}

#[tokio::test]
async fn escaping_proxy_hint_closes_the_capability() {
    let (proxy, owner) = UnixStream::pair().unwrap();
    let state = Arc::new(Mutex::new(TraceState::new()));
    let serving = tokio::spawn(super::serve(owner, state));
    capsem_foundation::unix::fd::set_nonblocking(proxy.as_fd(), true).unwrap();
    let mut proxy = tokio::net::UnixStream::from_std(proxy).unwrap();
    let frame = encode_proxy_trace_hint(&ProxyTraceHint {
        trace_id: "0707070707070707".into(),
        relative_path: "../escape.txt".into(),
    })
    .unwrap();
    proxy.write_all(&frame).await.unwrap();
    assert_eq!(
        serving.await.unwrap().unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
}
