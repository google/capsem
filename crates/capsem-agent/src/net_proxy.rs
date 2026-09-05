// capsem-net-proxy: Guest-side TCP-to-vsock relay for air-gapped networking.
//
// Listens on TWO localhost ports and bridges every connection to the host
// MITM proxy via vsock port 5002:
//   * 127.0.0.1:10443 -- intercepts iptables-redirected port 443 (HTTPS).
//   * 127.0.0.1:10080 -- intercepts iptables-redirected plain-HTTP ports
//                         (80 + the configured allowlist, including
//                         3128/3713/8080 and 11434 for Ollama). T2.2 added
//                         this listener.
//
// The host proxy runs a first-byte sniff (T2.1) and routes TLS handshakes
// to the rustls termination path and plain HTTP request lines to the
// hyper plain-HTTP path. Both listen ports forward to the SAME vsock port
// because the host classifier doesn't care which guest port the traffic
// originated on -- only the first byte.
//
// This binary runs inside the guest VM, launched by capsem-init.

#[path = "vsock_io.rs"]
mod vsock_io;

#[path = "procfs.rs"]
mod procfs;

#[path = "process_attribution.rs"]
mod process_attribution;

use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::process;
use std::sync::{Arc, Mutex};

use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::signal;

use capsem_proto::VSOCK_PORT_SNI_PROXY;
use process_attribution::encode_meta_line;
use vsock_io::{vsock_connect, AsyncVsock, VSOCK_HOST_CID};

/// TCP port to listen on for HTTPS traffic (iptables REDIRECT target
/// for outbound :443).
const LISTEN_PORT_HTTPS: u16 = 10443;
/// TCP port to listen on for plain-HTTP traffic (iptables REDIRECT
/// target for outbound :80 + the configurable allowlist, e.g.
/// :3128/:3713/:8080/:11434). Added in T2.2; the host proxy's first-byte
/// sniff distinguishes TLS from plain HTTP, so a dedicated guest
/// listener is just an iptables-target convenience.
const LISTEN_PORT_HTTP: u16 = 10080;
const RECENT_PID_CAPACITY: usize = 16;

#[derive(Default)]
struct ProcessAttributor {
    recent_pids: Mutex<VecDeque<u32>>,
}

impl ProcessAttributor {
    /// Retrieve the process name that initiated the TCP connection.
    async fn get_process_name(&self, client_port: u16) -> Option<String> {
        let recent_pids = self.recent_pids.lock().unwrap().iter().copied().collect::<Vec<_>>();
        let pid = tokio::task::spawn_blocking(move || find_process_pid(Path::new("/proc"), client_port, &recent_pids))
            .await
            .unwrap_or(None)?;

        self.remember(pid);
        Some(procfs::process_name_for_pid(pid))
    }

    fn remember(&self, pid: u32) {
        let mut recent_pids = self.recent_pids.lock().unwrap();
        recent_pids.retain(|candidate| *candidate != pid);
        recent_pids.push_front(pid);
        recent_pids.truncate(RECENT_PID_CAPACITY);
    }
}

fn find_process_pid(proc_root: &Path, client_port: u16, recent_pids: &[u32]) -> Option<u32> {
    let inode = find_socket_inode(proc_root, client_port)?;
    let target = format!("socket:[{inode}]");

    for pid in pid_candidates(proc_root, recent_pids) {
        let fd_dir = proc_root.join(pid.to_string()).join("fd");
        if let Ok(fds) = std::fs::read_dir(fd_dir) {
            for fd_entry in fds.flatten() {
                if let Ok(link) = std::fs::read_link(fd_entry.path()) {
                    if link.to_string_lossy() == target {
                        return Some(pid);
                    }
                }
            }
        }
    }
    None
}

fn find_socket_inode(proc_root: &Path, client_port: u16) -> Option<String> {
    let port_hex = format!("{:04X}", client_port);

    let mut inode = None;
    // Search /proc/net/tcp and tcp6 for a socket matching our client port.
    // Format: "local_address" is "IP:PORT" where PORT is uppercase hex.
    // Use rsplit(':') for exact port match (ends_with could false-match
    // if the hex port is a suffix of the IP hex).
    for proc_path in &[proc_root.join("net/tcp"), proc_root.join("net/tcp6")] {
        if inode.is_some() {
            break;
        }
        if let Ok(content) = std::fs::read_to_string(proc_path) {
            for line in content.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                // Index 1 is local_address (ip:port).
                // Index 9 is inode.
                if parts.len() >= 10 {
                    let local_addr = parts[1];
                    if let Some(port_part) = local_addr.rsplit(':').next() {
                        if port_part == port_hex {
                            inode = Some(parts[9].to_string());
                            break;
                        }
                    }
                }
            }
        }
    }

    inode
}

fn pid_candidates(proc_root: &Path, recent_pids: &[u32]) -> Vec<u32> {
    let mut pids = std::fs::read_dir(proc_root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().to_string_lossy().parse().ok())
        .collect::<Vec<_>>();
    pids.sort_unstable();

    let mut candidates = Vec::with_capacity(pids.len());
    for pid in recent_pids {
        if pids.binary_search(pid).is_ok() && !candidates.contains(pid) {
            candidates.push(*pid);
        }
    }
    for pid in pids {
        if !candidates.contains(&pid) {
            candidates.push(pid);
        }
    }
    candidates
}

async fn handle_connection(mut tcp_stream: TcpStream, attributor: Arc<ProcessAttributor>) {
    let peer_addr = match tcp_stream.peer_addr() {
        Ok(addr) => addr,
        Err(_) => return,
    };

    // Attribution walks /proc and the vsock connect crosses to the host;
    // neither needs the other, and both sit in front of the first byte of
    // every outbound connection, so they run at the same time.
    let (process_name, vsock_raw) = tokio::join!(
        attributor.get_process_name(peer_addr.port()),
        tokio::task::spawn_blocking(|| vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_SNI_PROXY)),
    );
    let process_name = process_name.unwrap_or_else(|| "unknown".to_string());
    let vsock_raw = match vsock_raw {
        Ok(Ok(fd)) => fd,
        _ => {
            eprintln!("[capsem-net-proxy] vsock connect failed");
            return;
        }
    };

    // AsyncVsock::new owns the fd on every path; a failure has already closed it.
    let mut vsock_stream = match AsyncVsock::new(vsock_raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[capsem-net-proxy] failed to create AsyncVsock: {e}");
            return;
        }
    };

    let meta = encode_meta_line(&process_name);
    if let Err(e) = vsock_stream.write_all(&meta).await {
        eprintln!("[capsem-net-proxy] failed to inject process meta: {e}");
        return;
    }

    if let Err(e) = tokio::io::copy_bidirectional(&mut tcp_stream, &mut vsock_stream).await {
        let is_normal = e.kind() == io::ErrorKind::ConnectionReset
            || e.kind() == io::ErrorKind::UnexpectedEof
            || e.kind() == io::ErrorKind::BrokenPipe;
        if !is_normal {
            eprintln!("[capsem-net-proxy] bridge error: {e}");
        }
    }
}

/// Spawn the per-port accept loop. Every accepted TCP connection is
/// forwarded to vsock 5002 via `handle_connection`; the listen port
/// itself is not preserved across the bridge -- the host's first-byte
/// sniff classifies on wire bytes.
async fn run_listener(port: u16, attributor: Arc<ProcessAttributor>) -> io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    eprintln!("[capsem-net-proxy] listening on 127.0.0.1:{port}");
    loop {
        let (stream, _) = listener.accept().await?;
        let _ = stream.set_nodelay(true);
        let attributor = Arc::clone(&attributor);
        tokio::spawn(async move {
            handle_connection(stream, attributor).await;
        });
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    eprintln!("[capsem-net-proxy] starting (pid {})", process::id());

    let attributor = Arc::new(ProcessAttributor::default());
    let https_task = tokio::spawn(run_listener(LISTEN_PORT_HTTPS, Arc::clone(&attributor)));
    let http_task = tokio::spawn(run_listener(LISTEN_PORT_HTTP, attributor));

    tokio::select! {
        res = https_task => {
            if let Ok(Err(e)) = res {
                eprintln!("[capsem-net-proxy] HTTPS listener error: {e}");
            }
        }
        res = http_task => {
            if let Ok(Err(e)) = res {
                eprintln!("[capsem-net-proxy] HTTP listener error: {e}");
            }
        }
        _ = signal::ctrl_c() => {
            eprintln!("[capsem-net-proxy] shutting down");
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "net_proxy/tests.rs"]
mod tests;
