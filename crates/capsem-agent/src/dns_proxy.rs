// capsem-dns-proxy: Guest-side DNS forwarder bridging to the host
// hickory-backed handler over vsock port 5007 (T3.2).
//
// Listens on:
//   * UDP 127.0.0.1:1053 -- iptables-redirect target for outbound UDP :53
//   * TCP 127.0.0.1:1053 -- iptables-redirect target for outbound TCP :53
//
// Per query lifecycle (UDP):
//   1. recv_from(udp_sock) -> (raw_dns_bytes, peer_addr)
//   2. hand query to a persistent vsock session under a correlation id
//   3. session writes [4-byte BE length][rmp DnsRequest{id, raw, proto="udp"}]
//   4. session reads [4-byte BE length][rmp DnsResponse{id, raw, decision, rcode}]
//      and routes it to the query with that id, once the answer is checked
//      against the query's question
//   5. send_to(udp_sock, response.raw, peer_addr); SERVFAIL if unanswered
//
// Per query lifecycle (TCP):
//   The DNS-over-TCP wire format uses a 2-byte BE length prefix per
//   message (RFC 1035 §4.2.2). We read that, treat the next N bytes as
//   one DNS query, do the same vsock round-trip, and write the
//   response back with its own 2-byte BE length prefix. One TCP
//   accept may carry multiple queries; we serve them serially on the
//   same socket.
//
// DNS is latency-sensitive and high fan-out under agent workloads, so
// the proxy keeps two persistent vsock sessions and multiplexes queries
// over them by correlation id (see `session.rs`): many queries in flight
// per connection, answers in any order, each checked against its own
// question before it is handed back.
//
// Launched by `capsem-init` (T3.4) alongside the iptables nat
// redirect for UDP/TCP port 53 -> 1053. Replaced the dnsmasq fake
// that resolved every name to 10.0.0.1 pre-T3.

#[path = "dns_proxy/session.rs"]
mod session;
#[path = "vsock_io.rs"]
mod vsock_io;
#[path = "dns_proxy/wire.rs"]
mod wire;

use std::io;
use std::process;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::signal;

use capsem_proto::VSOCK_PORT_DNS_PROXY;
use session::{DnsForwarder, DNS_SESSIONS};
use vsock_io::{vsock_connect, VSOCK_HOST_CID};

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
/// UDP listener: read one datagram, forward, send the response back.
async fn run_udp_listener(forwarder: Arc<DnsForwarder>) -> io::Result<()> {
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
        let forwarder = Arc::clone(&forwarder);
        tokio::spawn(async move {
            let Some(answer) = answer_or_servfail(&forwarder, buf, "udp").await else {
                return;
            };
            if let Err(e) = sock_for_response.send_to(&answer, peer).await {
                eprintln!("[capsem-dns-proxy] udp send_to {peer}: {e}");
            }
        });
    }
}

/// TCP listener: each accepted conn carries one or more DNS messages,
/// each prefixed by a 2-byte BE length (RFC 1035 §4.2.2).
async fn run_tcp_listener(forwarder: Arc<DnsForwarder>) -> io::Result<()> {
    let listener = TcpListener::bind((LISTEN_BIND, LISTEN_PORT)).await?;
    eprintln!("[capsem-dns-proxy] tcp listening on {LISTEN_BIND}:{LISTEN_PORT}");
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("[capsem-dns-proxy] tcp accept error: {e}");
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        let forwarder = Arc::clone(&forwarder);
        tokio::spawn(serve_tcp_connection(stream, peer, forwarder));
    }
}

async fn serve_tcp_connection(
    mut stream: tokio::net::TcpStream,
    peer: std::net::SocketAddr,
    forwarder: Arc<DnsForwarder>,
) {
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
        let Some(answer) = answer_or_servfail(&forwarder, payload, "tcp").await else {
            return;
        };
        let resp_len = answer.len() as u16;
        let mut out = Vec::with_capacity(2 + answer.len());
        out.extend_from_slice(&resp_len.to_be_bytes());
        out.extend_from_slice(&answer);
        if let Err(e) = stream.write_all(&out).await {
            eprintln!("[capsem-dns-proxy] tcp write to {peer}: {e}");
            return;
        }
    }
}

/// The bytes to send back for `query`: the host's answer, or a SERVFAIL
/// when the host did not answer in time (deadline, shed, connection
/// lost). `None` when there is nothing sensible to send: the host could
/// not parse the query (it answers with empty bytes), or the query is so
/// malformed that not even a SERVFAIL header can be built from it.
async fn answer_or_servfail(forwarder: &DnsForwarder, query: Vec<u8>, proto: &'static str) -> Option<Vec<u8>> {
    let servfail = wire::servfail_for(&query);
    match forwarder.forward_query(query, proto).await {
        Ok(resp) if resp.raw.is_empty() => None,
        Ok(resp) => Some(resp.raw),
        Err(e) => {
            eprintln!("[capsem-dns-proxy] {proto} query unanswered ({e}); replying SERVFAIL");
            servfail
        }
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    eprintln!("[capsem-dns-proxy] starting (pid {})", process::id());

    let forwarder = Arc::new(DnsForwarder::new(
        DNS_SESSIONS,
        Arc::new(|| vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_DNS_PROXY)),
    ));
    let udp_task = tokio::spawn(run_udp_listener(Arc::clone(&forwarder)));
    let tcp_task = tokio::spawn(run_tcp_listener(forwarder));

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
#[path = "dns_proxy/tests.rs"]
mod tests;
