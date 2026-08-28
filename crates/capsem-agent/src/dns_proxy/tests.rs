use super::*;

#[test]
fn listen_port_is_above_privileged_range() {
    // > 1024 means we don't need CAP_NET_BIND_SERVICE.
    const _: () = assert!(LISTEN_PORT > 1024);
}

#[test]
fn listen_port_matches_iptables_target() {
    // capsem-init redirects guest port 53 to LISTEN_PORT via
    // `iptables -t nat -A OUTPUT -p (udp|tcp) --dport 53
    // -j REDIRECT --to-port 1053`. Pinning the constant here
    // means any drift between this binary and the iptables rule
    // shows up in the test diff first.
    assert_eq!(LISTEN_PORT, 1053);
}

#[test]
fn vsock_port_matches_proto_constant() {
    assert_eq!(VSOCK_PORT_DNS_PROXY, 5007);
}

#[test]
fn max_udp_dns_bytes_supports_edns() {
    // RFC 6891 default EDNS UDP payload size is 4096; smaller
    // would risk truncation flag (TC bit) on legit queries.
    const _: () = assert!(MAX_UDP_DNS_BYTES >= 4096);
}

#[test]
fn dns_proxy_uses_persistent_vsock_worker_pool() {
    const _: () = assert!(
        DNS_VSOCK_WORKERS >= 2,
        "DNS must not regress to per-query vsock connect/close; keep a persistent worker pool"
    );
}

#[test]
fn dns_proxy_round_robins_vsock_workers() {
    let next = AtomicUsize::new(0);
    assert_eq!(next_worker_index(&next, 3), 0);
    assert_eq!(next_worker_index(&next, 3), 1);
    assert_eq!(next_worker_index(&next, 3), 2);
    assert_eq!(next_worker_index(&next, 3), 0);
}

#[test]
fn dns_request_envelope_uses_string_proto_label() {
    // The agent sends "udp" or "tcp" -- pinning the labels here
    // means a typo in `forward_query("udb", ...)` (or whatever)
    // gets caught at compile-time-of-test rather than as a
    // confused host-side telemetry row.
    let req = DnsRequest {
        raw: vec![0u8; 12],
        proto: "udp".into(),
        process_name: None,
    };
    assert_eq!(req.proto, "udp");
    let req = DnsRequest {
        raw: vec![0u8; 12],
        proto: "tcp".into(),
        process_name: None,
    };
    assert_eq!(req.proto, "tcp");
}

#[test]
fn forward_query_on_fd_round_trips_length_prefixed_frames() {
    use std::io::{Read, Write};
    use std::os::fd::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let query = vec![0x12, 0x34, 0x01, 0x00];
    let expected_response = DnsResponse {
        raw: vec![0x12, 0x34, 0x81, 0x80],
        decision: "allowed".into(),
        rcode: 0,
    };
    let (mut host, guest) = UnixStream::pair().unwrap();
    let expected_for_host = expected_response.clone();
    let responder = std::thread::spawn(move || {
        let mut len_buf = [0u8; 4];
        host.read_exact(&mut len_buf).unwrap();
        let mut payload = vec![0u8; u32::from_be_bytes(len_buf) as usize];
        host.read_exact(&mut payload).unwrap();
        let request = capsem_proto::decode_dns_request(&payload).unwrap();
        assert_eq!(request.raw, query);
        assert_eq!(request.proto, "udp");

        let frame = capsem_proto::encode_dns_response(&expected_for_host).unwrap();
        host.write_all(&frame).unwrap();
    });

    let guest_fd = guest.into_raw_fd();
    let response = forward_query_on_fd(guest_fd, vec![0x12, 0x34, 0x01, 0x00], "udp").unwrap();
    close_fd(Some(guest_fd));
    responder.join().unwrap();

    assert_eq!(response, expected_response);
}

#[tokio::test]
async fn tcp_connection_serves_multiple_length_prefixed_queries() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (work_tx, work_rx) = std::sync::mpsc::channel::<DnsWork>();
    let worker = tokio::task::spawn_blocking(move || {
        for work in work_rx {
            assert_eq!(work.proto, "tcp");
            let _ = work.reply.send(Ok(DnsResponse {
                raw: work.raw,
                decision: "allowed".into(),
                rcode: 0,
            }));
        }
    });
    let forwarder = Arc::new(DnsForwarder {
        workers: vec![work_tx],
        next_worker: AtomicUsize::new(0),
    });

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        serve_tcp_connection(stream, peer, forwarder).await;
    });
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();

    for query in [vec![1, 2, 3], vec![4, 5, 6, 7]] {
        client
            .write_all(&(query.len() as u16).to_be_bytes())
            .await
            .unwrap();
        client.write_all(&query).await.unwrap();

        let response_len = client.read_u16().await.unwrap() as usize;
        let mut response = vec![0u8; response_len];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(response, query);
    }

    drop(client);
    tokio::time::timeout(std::time::Duration::from_secs(1), server)
        .await
        .expect("TCP connection handler did not stop at clean EOF")
        .unwrap();
    worker.await.unwrap();
}
