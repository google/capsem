/// Host-side SNI proxy: handles vsock:5002 connections from the guest,
/// reads TLS ClientHello to extract the domain, checks the domain policy,
/// and bridges to the real server if allowed.
///
/// Each connection is handled in a pair of blocking threads (vsock->upstream
/// and upstream->vsock), matching the existing PTY bridge pattern.
use std::io::{self, Read, Write};
use std::mem::ManuallyDrop;
use std::net::TcpStream;
use std::os::unix::io::{FromRawFd, RawFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use tracing::{info, warn};

use super::domain_policy::{Action, DomainPolicy};
use super::sni_parser;
use super::telemetry::{Decision, NetEvent, WebDb};

/// Maximum bytes to buffer while waiting for TLS ClientHello.
const MAX_HELLO_SIZE: usize = 16384;

/// Handle a single SNI proxy connection from the guest.
///
/// This function blocks the calling thread until the connection is complete.
/// It is designed to be spawned into a per-VM task group.
///
/// Deprecated: use `mitm_proxy::handle_connection` for full HTTP inspection.
#[deprecated(note = "use mitm_proxy::handle_connection for full HTTP inspection")]
pub fn handle_connection(
    vsock_fd: RawFd,
    policy: Arc<DomainPolicy>,
    web_db: Arc<Mutex<WebDb>>,
) {
    let start = Instant::now();
    let mut domain = String::new();
    let mut decision = Decision::Error;

    let result = handle_connection_inner(vsock_fd, &policy, &mut domain, &mut decision);

    let duration_ms = start.elapsed().as_millis() as u64;

    // Extract byte counts from the result if successful
    let (bytes_sent, bytes_received) = match &result {
        Ok((s, r)) => (*s, *r),
        Err(_) => (0, 0),
    };

    let reason = match &result {
        Ok(_) => None,
        Err(e) => Some(e.to_string()),
    };

    // Record telemetry
    let event = NetEvent {
        timestamp: SystemTime::now(),
        domain: if domain.is_empty() {
            "<unknown>".to_string()
        } else {
            domain.clone()
        },
        port: 443,
        decision,
        bytes_sent,
        bytes_received,
        duration_ms,
        reason,
        method: None,
        path: None,
        status_code: None,
        request_headers: None,
        response_headers: None,
        request_body_preview: None,
        response_body_preview: None,
        conn_type: Some("https-passthrough".to_string()),
    };

    if let Ok(db) = web_db.lock() {
        if let Err(e) = db.record(&event) {
            warn!(error = %e, "failed to record net event to web.db");
        }
    }

    match decision {
        Decision::Allowed => info!(
            domain,
            bytes_sent,
            bytes_received,
            duration_ms,
            "SNI proxy: connection completed"
        ),
        Decision::Denied => info!(domain, "SNI proxy: connection denied"),
        Decision::Error => {
            if let Err(ref e) = result {
                warn!(domain, error = %e, "SNI proxy: connection error");
            }
        }
    }
}

fn handle_connection_inner(
    vsock_fd: RawFd,
    policy: &DomainPolicy,
    out_domain: &mut String,
    out_decision: &mut Decision,
) -> io::Result<(u64, u64)> {
    // Safety: vsock_fd is valid for the lifetime of the VsockConnection
    // which is held alive by the caller.
    let mut vsock = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(vsock_fd) });

    // 1. Read initial bytes (TLS ClientHello)
    let mut initial_buf = vec![0u8; MAX_HELLO_SIZE];
    let n = vsock.read(&mut initial_buf)?;
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "empty connection"));
    }
    initial_buf.truncate(n);

    // 2. Extract SNI
    let domain = sni_parser::extract_sni(&initial_buf).ok_or_else(|| {
        *out_decision = Decision::Denied;
        io::Error::new(io::ErrorKind::InvalidData, "no SNI in ClientHello")
    })?;
    *out_domain = domain.clone();

    // 3. Check policy
    let (action, reason) = policy.evaluate(&domain);
    if action == Action::Deny {
        *out_decision = Decision::Denied;
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("domain denied: {domain} ({reason})"),
        ));
    }
    *out_decision = Decision::Allowed;

    // 4. Resolve DNS and connect to real server
    let upstream = TcpStream::connect(format!("{domain}:443")).map_err(|e| {
        *out_decision = Decision::Error;
        io::Error::new(e.kind(), format!("connect to {domain}:443 failed: {e}"))
    })?;

    // 5. Send the buffered ClientHello to upstream
    let mut upstream_write = upstream.try_clone()?;
    upstream_write.write_all(&initial_buf)?;

    // 6. Bidirectional bridge: vsock <-> real TCP
    let bytes_sent = Arc::new(AtomicU64::new(initial_buf.len() as u64));
    let bytes_received = Arc::new(AtomicU64::new(0));

    let bs = Arc::clone(&bytes_sent);
    let br = Arc::clone(&bytes_received);

    // upstream -> vsock (in a separate thread)
    let mut upstream_read = upstream.try_clone()?;
    let vsock_fd_copy = vsock_fd;
    let upstream_to_vsock = std::thread::spawn(move || {
        let mut vsock_out = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(vsock_fd_copy) });
        let mut buf = [0u8; 8192];
        loop {
            match upstream_read.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    br.fetch_add(n as u64, Ordering::Relaxed);
                    if vsock_out.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        // Signal vsock that upstream is done by shutting down write
        let _ = unsafe { libc::shutdown(vsock_fd_copy, libc::SHUT_WR) };
    });

    // vsock -> upstream (in current thread)
    let mut buf = [0u8; 8192];
    loop {
        match vsock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                bs.fetch_add(n as u64, Ordering::Relaxed);
                if upstream_write.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = upstream_write.shutdown(std::net::Shutdown::Write);

    let _ = upstream_to_vsock.join();

    Ok((
        bytes_sent.load(Ordering::Relaxed),
        bytes_received.load(Ordering::Relaxed),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;
    use crate::net::domain_policy::Action;

    fn make_client_hello(hostname: &str) -> Vec<u8> {
        let hostname_bytes = hostname.as_bytes();
        let sni_entry_len = 1 + 2 + hostname_bytes.len();
        let sni_list_len = sni_entry_len;
        let sni_ext_data_len = 2 + sni_list_len;

        let mut sni_ext = Vec::new();
        sni_ext.extend_from_slice(&0x0000u16.to_be_bytes());
        sni_ext.extend_from_slice(&(sni_ext_data_len as u16).to_be_bytes());
        sni_ext.extend_from_slice(&(sni_list_len as u16).to_be_bytes());
        sni_ext.push(0x00);
        sni_ext.extend_from_slice(&(hostname_bytes.len() as u16).to_be_bytes());
        sni_ext.extend_from_slice(hostname_bytes);

        let extensions_len = sni_ext.len();
        let mut hello_body = Vec::new();
        hello_body.extend_from_slice(&[0x03, 0x03]);
        hello_body.extend_from_slice(&[0u8; 32]);
        hello_body.push(0);
        hello_body.extend_from_slice(&2u16.to_be_bytes());
        hello_body.extend_from_slice(&[0x00, 0x2f]);
        hello_body.push(1);
        hello_body.push(0);
        hello_body.extend_from_slice(&(extensions_len as u16).to_be_bytes());
        hello_body.extend_from_slice(&sni_ext);

        let mut handshake = Vec::new();
        handshake.push(0x01);
        let hello_len = hello_body.len();
        handshake.push((hello_len >> 16) as u8);
        handshake.push((hello_len >> 8) as u8);
        handshake.push(hello_len as u8);
        handshake.extend_from_slice(&hello_body);

        let mut record = Vec::new();
        record.push(0x16);
        record.extend_from_slice(&[0x03, 0x01]);
        record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
        record.extend_from_slice(&handshake);

        record
    }

    #[test]
    fn proxy_empty_connection() {
        let (s1, s2) = UnixStream::pair().unwrap();
        let policy = DomainPolicy::new(&[], &[], Action::Deny);
        let mut domain = String::new();
        let mut decision = Decision::Error;

        drop(s1);

        let err = handle_connection_inner(s2.into_raw_fd(), &policy, &mut domain, &mut decision).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn proxy_invalid_data() {
        let (mut s1, s2) = UnixStream::pair().unwrap();
        let policy = DomainPolicy::new(&[], &[], Action::Deny);
        let mut domain = String::new();
        let mut decision = Decision::Error;

        s1.write_all(b"not a client hello").unwrap();
        drop(s1);

        let err = handle_connection_inner(s2.into_raw_fd(), &policy, &mut domain, &mut decision).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(decision, Decision::Denied);
    }

    #[test]
    fn proxy_domain_denied() {
        let (mut s1, s2) = UnixStream::pair().unwrap();
        let policy = DomainPolicy::new(&[], &["evil.com".to_string()], Action::Deny);
        let mut domain = String::new();
        let mut decision = Decision::Error;

        s1.write_all(&make_client_hello("evil.com")).unwrap();
        drop(s1);

        let err = handle_connection_inner(s2.into_raw_fd(), &policy, &mut domain, &mut decision).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(domain, "evil.com");
        assert_eq!(decision, Decision::Denied);
    }
}
