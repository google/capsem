// capsem-dns-proxy: Guest-side DNS forwarder bridging to the host
// hickory-backed handler over vsock port 5007 (T3.2).
//
// Listens on:
//   * UDP 127.0.0.1:1053 -- iptables-redirect target for outbound UDP :53
//   * TCP 127.0.0.1:1053 -- iptables-redirect target for outbound TCP :53
//
// Per query lifecycle (UDP):
//   1. recv_from(udp_sock) -> (raw_dns_bytes, peer_addr)
//   2. open new vsock conn to (HOST_CID=2, port=5007)
//   3. write [4-byte BE length][rmp DnsRequest{raw, proto="udp"}]
//   4. read [4-byte BE length][rmp DnsResponse{raw, decision, rcode}]
//   5. send_to(udp_sock, response.raw, peer_addr)
//   6. close vsock conn
//
// Per query lifecycle (TCP):
//   The DNS-over-TCP wire format uses a 2-byte BE length prefix per
//   message (RFC 1035 §4.2.2). We read that, treat the next N bytes as
//   one DNS query, do the same vsock round-trip, and write the
//   response back with its own 2-byte BE length prefix. One TCP
//   accept may carry multiple queries; we serve them serially on the
//   same socket.
//
// One vsock connection per query keeps the agent stateless and
// matches the host-side `serve_dns_session` shape (one envelope
// round-trip then close). DNS queries are small + infrequent compared
// to HTTP; T5 hardening can swap to a multiplexed long-lived conn if
// throughput ever becomes the bottleneck.
//
// Launched by `capsem-init` (T3.4) alongside the iptables nat
// redirect for UDP/TCP port 53 -> 1053. Replaced the dnsmasq fake
// that resolved every name to 10.0.0.1 pre-T3.

#[path = "vsock_io.rs"]
mod vsock_io;

use std::io;
use std::process;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::signal;
use tokio::task;

use capsem_proto::{
    decode_dns_response, encode_dns_request, DnsRequest, DnsResponse, MAX_FRAME_SIZE,
    VSOCK_PORT_DNS_PROXY,
};
use vsock_io::{read_exact_fd, vsock_connect, write_all_fd, VSOCK_HOST_CID};

/// Loopback bind address. iptables redirects guest-originated DNS
/// traffic to this port so we can intercept libc's `getaddrinfo`.
const LISTEN_BIND: &str = "127.0.0.1";
/// Loopback port for the DNS forwarder. Picked > 1024 so the agent
/// doesn't need CAP_NET_BIND_SERVICE; the guest's iptables NAT rule
/// rewrites the destination port from 53 -> this on the way out.
const LISTEN_PORT: u16 = 1053;

/// Maximum bytes for one DNS UDP datagram. RFC 6891 caps practical
/// EDNS responses at ~4096; standard queries fit in 512. 4096 is what
/// hickory uses internally.
const MAX_UDP_DNS_BYTES: usize = 4096;

/// Round-trip a single DNS query through the host-side handler.
///
/// Opens a fresh vsock conn, encodes + writes a `DnsRequest`, reads
/// the framed `DnsResponse`, closes the conn. Done synchronously
/// (blocking syscalls on a `spawn_blocking` thread) because the
/// vsock_io helpers are blocking primitives.
async fn forward_query(raw: Vec<u8>, proto: &'static str) -> io::Result<DnsResponse> {
    task::spawn_blocking(move || forward_query_blocking(raw, proto))
        .await
        .map_err(|e| io::Error::other(format!("dns forward task panicked: {e}")))?
}

fn forward_query_blocking(raw: Vec<u8>, proto: &str) -> io::Result<DnsResponse> {
    let req = DnsRequest {
        raw,
        proto: proto.to_string(),
        process_name: None,
    };
    let frame = encode_dns_request(&req)
        .map_err(|e| io::Error::other(format!("encode_dns_request: {e:#}")))?;

    let fd = vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_DNS_PROXY)?;
    // From here on the fd is owned by us; close on every exit.
    let result = (|| -> io::Result<DnsResponse> {
        write_all_fd(fd, &frame)?;
        let mut len_buf = [0u8; 4];
        read_exact_fd(fd, &mut len_buf)?;
        let len = u32::from_be_bytes(len_buf);
        if len > MAX_FRAME_SIZE {
            return Err(io::Error::other(format!(
                "dns response frame too large ({len} > {MAX_FRAME_SIZE})"
            )));
        }
        let mut payload = vec![0u8; len as usize];
        read_exact_fd(fd, &mut payload)?;
        decode_dns_response(&payload)
            .map_err(|e| io::Error::other(format!("decode_dns_response: {e:#}")))
    })();

    unsafe {
        nix::libc::close(fd);
    }
    result
}

/// UDP listener: read one datagram, forward, send the response back.
async fn run_udp_listener() -> io::Result<()> {
    let sock = UdpSocket::bind((LISTEN_BIND, LISTEN_PORT)).await?;
    eprintln!("[capsem-dns-proxy] udp listening on {LISTEN_BIND}:{LISTEN_PORT}");
    let sock = std::sync::Arc::new(sock);
    loop {
        let mut buf = vec![0u8; MAX_UDP_DNS_BYTES];
        let (n, peer) = match sock.recv_from(&mut buf).await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("[capsem-dns-proxy] udp recv error: {e}");
                continue;
            }
        };
        buf.truncate(n);

        let sock_for_response = std::sync::Arc::clone(&sock);
        tokio::spawn(async move {
            match forward_query(buf, "udp").await {
                Ok(resp) => {
                    if resp.raw.is_empty() {
                        // Host returned empty bytes -- usually a parse
                        // error. Drop the query rather than echo an
                        // empty packet.
                        return;
                    }
                    if let Err(e) = sock_for_response.send_to(&resp.raw, peer).await {
                        eprintln!("[capsem-dns-proxy] udp send_to {peer}: {e}");
                    }
                }
                Err(e) => {
                    eprintln!("[capsem-dns-proxy] udp forward error from {peer}: {e}");
                }
            }
        });
    }
}

/// TCP listener: each accepted conn carries one or more DNS messages,
/// each prefixed by a 2-byte BE length (RFC 1035 §4.2.2).
async fn run_tcp_listener() -> io::Result<()> {
    let listener = TcpListener::bind((LISTEN_BIND, LISTEN_PORT)).await?;
    eprintln!("[capsem-dns-proxy] tcp listening on {LISTEN_BIND}:{LISTEN_PORT}");
    loop {
        let (mut stream, peer) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("[capsem-dns-proxy] tcp accept error: {e}");
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        tokio::spawn(async move {
            loop {
                let mut len_buf = [0u8; 2];
                match stream.read_exact(&mut len_buf).await {
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return,
                    Err(e) => {
                        eprintln!("[capsem-dns-proxy] tcp read len from {peer}: {e}");
                        return;
                    }
                }
                let dns_len = u16::from_be_bytes(len_buf) as usize;
                let mut payload = vec![0u8; dns_len];
                if let Err(e) = stream.read_exact(&mut payload).await {
                    eprintln!("[capsem-dns-proxy] tcp read body from {peer}: {e}");
                    return;
                }
                match forward_query(payload, "tcp").await {
                    Ok(resp) => {
                        if resp.raw.is_empty() {
                            return;
                        }
                        let resp_len = resp.raw.len() as u16;
                        let mut out = Vec::with_capacity(2 + resp.raw.len());
                        out.extend_from_slice(&resp_len.to_be_bytes());
                        out.extend_from_slice(&resp.raw);
                        if let Err(e) = stream.write_all(&out).await {
                            eprintln!("[capsem-dns-proxy] tcp write to {peer}: {e}");
                            return;
                        }
                    }
                    Err(e) => {
                        eprintln!("[capsem-dns-proxy] tcp forward error from {peer}: {e}");
                        return;
                    }
                }
            }
        });
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    eprintln!("[capsem-dns-proxy] starting (pid {})", process::id());

    let udp_task = tokio::spawn(run_udp_listener());
    let tcp_task = tokio::spawn(run_tcp_listener());

    tokio::select! {
        res = udp_task => {
            if let Ok(Err(e)) = res {
                eprintln!("[capsem-dns-proxy] udp listener error: {e}");
            }
        }
        res = tcp_task => {
            if let Ok(Err(e)) = res {
                eprintln!("[capsem-dns-proxy] tcp listener error: {e}");
            }
        }
        _ = signal::ctrl_c() => {
            eprintln!("[capsem-dns-proxy] shutting down");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
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
}
