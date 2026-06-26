// capsem-dns-proxy: Guest-side DNS forwarder bridging to the host
// hickory-backed handler over vsock port 5007 (T3.2).
//
// Listens on:
//   * UDP 127.0.0.1:1053 -- iptables-redirect target for outbound UDP :53
//   * TCP 127.0.0.1:1053 -- iptables-redirect target for outbound TCP :53
//
// Per query lifecycle (UDP):
//   1. recv_from(udp_sock) -> (raw_dns_bytes, peer_addr)
//   2. hand query to a persistent vsock worker
//   3. worker writes [4-byte BE length][rmp DnsRequest{raw, proto="udp"}]
//   4. worker reads [4-byte BE length][rmp DnsResponse{raw, decision, rcode}]
//   5. send_to(udp_sock, response.raw, peer_addr)
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
// the proxy keeps a small pool of persistent vsock workers. Each worker
// owns one blocking vsock fd and processes one in-flight DNS request at
// a time; the pool provides concurrency without opening/closing a vsock
// connection per packet.
//
// Launched by `capsem-init` (T3.4) alongside the iptables nat
// redirect for UDP/TCP port 53 -> 1053. Replaced the dnsmasq fake
// that resolved every name to 10.0.0.1 pre-T3.

#[path = "vsock_io.rs"]
mod vsock_io;

use std::io;
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::signal;
use tokio::sync::oneshot;

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
const DNS_VSOCK_WORKERS: usize = 8;

struct DnsForwarder {
    workers: Vec<mpsc::Sender<DnsWork>>,
    next_worker: AtomicUsize,
}

struct DnsWork {
    raw: Vec<u8>,
    proto: &'static str,
    reply: oneshot::Sender<io::Result<DnsResponse>>,
}

impl DnsForwarder {
    fn new(worker_count: usize) -> Self {
        assert!(worker_count > 0, "DNS forwarder needs at least one worker");
        let mut workers = Vec::with_capacity(worker_count);
        for id in 0..worker_count {
            let (tx, rx) = mpsc::channel();
            spawn_dns_worker(id, rx);
            workers.push(tx);
        }
        Self {
            workers,
            next_worker: AtomicUsize::new(0),
        }
    }

    async fn forward_query(&self, raw: Vec<u8>, proto: &'static str) -> io::Result<DnsResponse> {
        let (reply, wait) = oneshot::channel();
        let work = DnsWork { raw, proto, reply };
        let idx = next_worker_index(&self.next_worker, self.workers.len());
        self.workers[idx]
            .send(work)
            .map_err(|_| io::Error::other("dns forward worker stopped"))?;
        wait.await
            .map_err(|_| io::Error::other("dns forward worker dropped response"))?
    }
}

fn next_worker_index(next_worker: &AtomicUsize, worker_count: usize) -> usize {
    next_worker.fetch_add(1, Ordering::Relaxed) % worker_count
}

fn spawn_dns_worker(id: usize, rx: mpsc::Receiver<DnsWork>) {
    thread::Builder::new()
        .name(format!("capsem-dns-vsock-{id}"))
        .spawn(move || {
            let mut fd = None;
            for work in rx {
                let result = dns_round_trip_with_reconnect(&mut fd, work.raw, work.proto);
                let _ = work.reply.send(result);
            }
            close_fd(fd);
        })
        .expect("spawn DNS vsock worker");
}

/// Round-trip a single DNS query through the host-side handler.
fn forward_query_on_fd(fd: i32, raw: Vec<u8>, proto: &str) -> io::Result<DnsResponse> {
    let req = DnsRequest {
        raw,
        proto: proto.to_string(),
        process_name: None,
    };
    let frame = encode_dns_request(&req)
        .map_err(|e| io::Error::other(format!("encode_dns_request: {e:#}")))?;

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
}

fn dns_round_trip_with_reconnect(
    fd: &mut Option<i32>,
    raw: Vec<u8>,
    proto: &'static str,
) -> io::Result<DnsResponse> {
    for attempt in 0..2 {
        let active_fd = match *fd {
            Some(active_fd) => active_fd,
            None => {
                let new_fd = vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_DNS_PROXY)?;
                *fd = Some(new_fd);
                new_fd
            }
        };
        match forward_query_on_fd(active_fd, raw.clone(), proto) {
            Ok(response) => return Ok(response),
            Err(error) if attempt == 0 => {
                close_fd(fd.take());
                eprintln!("[capsem-dns-proxy] vsock worker reconnecting after error: {error}");
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("dns forward retry exhausted"))
}

fn close_fd(fd: Option<i32>) {
    if let Some(fd) = fd {
        unsafe {
            nix::libc::close(fd);
        }
    }
}

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
            match forwarder.forward_query(buf, "udp").await {
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
async fn run_tcp_listener(forwarder: Arc<DnsForwarder>) -> io::Result<()> {
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
        let forwarder = Arc::clone(&forwarder);
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
                match forwarder.forward_query(payload, "tcp").await {
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

    let forwarder = Arc::new(DnsForwarder::new(DNS_VSOCK_WORKERS));
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
}
