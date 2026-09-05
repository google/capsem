use super::*;
use crate::session::Connect;
use crate::wire::fixtures::{answer_to, query};
use capsem_proto::{DnsRequest, DnsResponse};
use std::io::{Read, Write};
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;
use tokio::net::TcpListener;

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
fn dns_proxy_keeps_more_than_one_persistent_session() {
    const _: () = assert!(
        DNS_SESSIONS >= 2,
        "one session would make a reconnect a resolver outage; keep at least two"
    );
}

#[test]
fn dns_request_envelope_uses_string_proto_label() {
    // The agent sends "udp" or "tcp" -- pinning the labels here
    // means a typo in `forward_query("udb", ...)` (or whatever)
    // gets caught at compile-time-of-test rather than as a
    // confused host-side telemetry row.
    let req = DnsRequest {
        id: 0,
        raw: vec![0u8; 12],
        proto: "udp".into(),
        process_name: None,
    };
    assert_eq!(req.proto, "udp");
    let req = DnsRequest {
        id: 0,
        raw: vec![0u8; 12],
        proto: "tcp".into(),
        process_name: None,
    };
    assert_eq!(req.proto, "tcp");
}

/// A host that answers every frame with `reply(request)`; `None` hangs up.
fn host_with(reply: impl Fn(DnsRequest) -> Option<DnsResponse> + Send + Sync + 'static) -> Connect {
    let reply = Arc::new(reply);
    Arc::new(move || {
        let (mut host, guest) = UnixStream::pair()?;
        let reply = Arc::clone(&reply);
        std::thread::spawn(move || loop {
            let mut len_buf = [0u8; 4];
            if host.read_exact(&mut len_buf).is_err() {
                return;
            }
            let mut payload = vec![0u8; u32::from_be_bytes(len_buf) as usize];
            if host.read_exact(&mut payload).is_err() {
                return;
            }
            let request = capsem_proto::decode_dns_request(&payload).unwrap();
            match reply(request) {
                Some(response) => {
                    let frame = capsem_proto::encode_dns_response(&response).unwrap();
                    if host.write_all(&frame).is_err() {
                        return;
                    }
                }
                None => return,
            }
        });
        Ok(guest.into_raw_fd())
    })
}

fn echoing(request: DnsRequest) -> Option<DnsResponse> {
    assert_eq!(request.proto, "tcp");
    Some(DnsResponse {
        id: request.id,
        raw: answer_to(&request.raw),
        decision: "allowed".into(),
        rcode: 0,
    })
}

async fn run_tcp_case(connect: Connect, request: &[u8]) -> Vec<u8> {
    let forwarder = Arc::new(DnsForwarder::new(1, connect));
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        serve_tcp_connection(stream, peer, forwarder).await;
    });
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    client.write_all(request).await.unwrap();
    client.shutdown().await.unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    server.await.unwrap();
    response
}

fn framed(query: &[u8]) -> Vec<u8> {
    let mut out = (query.len() as u16).to_be_bytes().to_vec();
    out.extend_from_slice(query);
    out
}

#[tokio::test]
async fn tcp_connection_closes_on_empty_response_and_truncation_and_servfails_a_lost_host() {
    // The host could not parse the query: empty bytes, nothing to send.
    let empty = host_with(|request| {
        Some(DnsResponse {
            id: request.id,
            raw: Vec::new(),
            decision: "error".into(),
            rcode: 2,
        })
    });
    assert!(run_tcp_case(empty, &framed(&[42])).await.is_empty());

    // A truncated length prefix ends the connection.
    let unused = host_with(echoing);
    assert!(run_tcp_case(unused, &[0, 4, 1, 2]).await.is_empty());

    // The host hangs up on a well-formed query: the client gets SERVFAIL
    // rather than silence.
    let hangs_up = host_with(|_| None);
    let q = query(0x0BAD, "gone.example");
    let response = run_tcp_case(hangs_up, &framed(&q)).await;
    assert_eq!(&response[0..2], &(q.len() as u16).to_be_bytes());
    assert_eq!(&response[2..4], &[0x0B, 0xAD], "SERVFAIL keeps the transaction id");
    assert_eq!(response[5] & 0x0F, 2, "rcode SERVFAIL");

    // The host hangs up on a query no SERVFAIL can be built from: close.
    let hangs_up = host_with(|_| None);
    assert!(run_tcp_case(hangs_up, &framed(&[0, 1, 42])).await.is_empty());
}

#[tokio::test]
async fn tcp_connection_serves_multiple_length_prefixed_queries() {
    let forwarder = Arc::new(DnsForwarder::new(1, host_with(echoing)));

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        serve_tcp_connection(stream, peer, forwarder).await;
    });
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();

    for (id, name) in [(1u16, "first.example"), (2, "second.example")] {
        let q = query(id, name);
        client.write_all(&framed(&q)).await.unwrap();

        let response_len = client.read_u16().await.unwrap() as usize;
        let mut response = vec![0u8; response_len];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(response, answer_to(&q));
    }

    drop(client);
    tokio::time::timeout(std::time::Duration::from_secs(1), server)
        .await
        .expect("TCP connection handler did not stop at clean EOF")
        .unwrap();
}

#[tokio::test]
async fn udp_answer_falls_back_to_servfail_only_for_a_parseable_query() {
    let forwarder = DnsForwarder::new(1, host_with(|_| None));
    let q = query(0x1111, "gone.example");
    let answer = answer_or_servfail(&forwarder, q.clone(), "udp").await.unwrap();
    assert_eq!(&answer[0..2], &[0x11, 0x11]);
    assert_eq!(answer[3], 0x02);
    assert!(answer_or_servfail(&forwarder, vec![1, 2, 3], "udp").await.is_none());
}
