//! auditd log tailer: streams parsed execve records to the host audit port.

use std::os::unix::io::RawFd;
use std::thread;

use nix::libc;

use capsem_proto::{encode_audit_record, AuditRecord, VSOCK_PORT_AUDIT};

use crate::vsock_io::{vsock_connect, write_all_fd, VSOCK_HOST_CID};

/// Tail /var/log/audit/audit.log and stream parsed execve records to host via vsock:5006.
///
/// Waits for the audit log file to appear (auditd may start slightly after the agent),
/// then continuously reads new lines. Each complete audit record (correlated by audit ID)
/// is serialized as MessagePack and sent as a length-prefixed frame.
/// Poll interval when the audit log has no new lines.
const AUDIT_IDLE_POLL: std::time::Duration = std::time::Duration::from_millis(50);
/// Retry interval while the host audit port is unreachable (host restarting).
const AUDIT_RECONNECT_POLL: std::time::Duration = std::time::Duration::from_millis(500);
/// Incomplete record groups kept before dropping the ones without a SYSCALL.
const AUDIT_PENDING_CAP: usize = 1000;

/// Tail the guest audit log for the life of the process, streaming complete
/// execve records to the host audit port.
///
/// Runs once per process and owns its connection: when the host side goes
/// away (a suspend/resume replaces the host process) it reconnects and
/// continues from the current file offset. It used to be spawned per host
/// connection, so every reconnect started a second tailer at the top of the
/// log and re-streamed the whole history while the old one lingered until
/// its next write failed.
pub(crate) fn audit_reader_loop() {
    use std::collections::HashMap;
    use std::io::BufReader;

    const AUDIT_LOG: &str = "/var/log/audit/audit.log";

    // Wait for audit log to appear (up to 10 seconds)
    for _ in 0..100 {
        if std::path::Path::new(AUDIT_LOG).exists() {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(100));
    }
    if !std::path::Path::new(AUDIT_LOG).exists() {
        eprintln!("[capsem-agent] audit: {AUDIT_LOG} not found, skipping audit streaming");
        return;
    }
    let file = match std::fs::File::open(AUDIT_LOG) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[capsem-agent] audit: open failed: {e}");
            return;
        }
    };
    let mut reader = BufReader::new(file);
    let mut pending: HashMap<String, AuditRecordBuilder> = HashMap::new();
    let mut audit_fd = connect_audit_port();
    eprintln!("[capsem-agent] audit: connected to host, tailing {AUDIT_LOG}");
    let mut unsent: Option<Vec<u8>> = None;

    loop {
        if let Some(frame) = unsent.take() {
            if write_all_fd(audit_fd, &frame).is_err() {
                unsent = Some(frame);
                unsafe {
                    libc::close(audit_fd);
                }
                eprintln!("[capsem-agent] audit: write failed, reconnecting");
                audit_fd = connect_audit_port();
                continue;
            }
        }
        match tail_audit_log(&mut reader, &mut pending, &mut |frame| {
            write_all_fd(audit_fd, frame).is_ok()
        }) {
            Ok(()) => thread::sleep(AUDIT_IDLE_POLL),
            Err(frame) => unsent = Some(frame),
        }
    }
}

/// Connect to the host audit port, retrying until it answers.
fn connect_audit_port() -> RawFd {
    loop {
        match vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_AUDIT) {
            Ok(fd) => return fd,
            Err(e) => {
                eprintln!("[capsem-agent] audit: vsock connect failed: {e}; retrying");
                thread::sleep(AUDIT_RECONNECT_POLL);
            }
        }
    }
}

/// Consume every line currently available from `reader`, emitting each
/// completed record through `sink`. Returns `Ok` at end of input; returns the
/// frame that `sink` refused so the caller can resend it after reconnecting
/// without rewinding the log.
pub(crate) fn tail_audit_log<R: std::io::BufRead>(
    reader: &mut R,
    pending: &mut std::collections::HashMap<String, AuditRecordBuilder>,
    sink: &mut dyn FnMut(&[u8]) -> bool,
) -> Result<(), Vec<u8>> {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return Ok(()),
            Ok(_) => {}
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse audit log line. Format: type=SYSCALL msg=audit(1713100000.001:42): ...
        let Some(audit_id) = extract_audit_id(line) else {
            continue;
        };
        let record_type = extract_field(line, "type=");
        let builder = pending.entry(audit_id.clone()).or_default();

        match record_type.as_deref() {
            Some("SYSCALL") => {
                builder.pid = extract_field(line, " pid=").and_then(|v| v.parse().ok());
                builder.ppid = extract_field(line, " ppid=").and_then(|v| v.parse().ok());
                builder.uid = extract_field(line, " uid=").and_then(|v| v.parse().ok());
                builder.exe = extract_field(line, " exe=").map(|s| s.trim_matches('"').to_string());
                builder.comm = extract_field(line, " comm=").map(|s| s.trim_matches('"').to_string());
                builder.tty = extract_field(line, " tty=").filter(|s| s != "(none)");
                builder.timestamp_us = extract_audit_timestamp_us(line);
                builder.has_syscall = true;
            }
            Some("EXECVE") => {
                builder.argv = extract_execve_argv(line);
            }
            Some("CWD") => {
                builder.cwd = extract_field(line, " cwd=").map(|s| s.trim_matches('"').to_string());
            }
            Some("PROCTITLE") => {
                // PROCTITLE is the last record in a group -- emit when we have SYSCALL + argv
                let frame = builder
                    .has_syscall
                    .then(|| builder.build(&audit_id))
                    .flatten()
                    .and_then(|record| encode_audit_record(&record).ok());
                pending.remove(&audit_id);
                if let Some(frame) = frame {
                    if !sink(&frame) {
                        return Err(frame);
                    }
                }
            }
            _ => {}
        }

        // Prevent memory leak for incomplete records
        if pending.len() > AUDIT_PENDING_CAP {
            pending.retain(|_, v| v.has_syscall);
        }
    }
}

/// Intermediate builder for multi-line audit records.
#[derive(Default)]
pub(crate) struct AuditRecordBuilder {
    has_syscall: bool,
    timestamp_us: Option<u64>,
    pid: Option<u32>,
    ppid: Option<u32>,
    uid: Option<u32>,
    exe: Option<String>,
    comm: Option<String>,
    argv: Option<String>,
    cwd: Option<String>,
    tty: Option<String>,
}

impl AuditRecordBuilder {
    fn build(&self, audit_id: &str) -> Option<AuditRecord> {
        Some(AuditRecord {
            timestamp_us: self.timestamp_us?,
            pid: self.pid?,
            ppid: self.ppid?,
            uid: self.uid.unwrap_or(0),
            exe: self.exe.clone()?,
            comm: self.comm.clone(),
            argv: self
                .argv
                .clone()
                .unwrap_or_else(|| self.exe.clone().unwrap_or_default()),
            cwd: self.cwd.clone(),
            tty: self.tty.clone(),
            session_id: None,
            parent_exe: None,
            audit_id: audit_id.to_string(),
        })
    }
}

/// Extract audit ID from a log line. Format: msg=audit(1713100000.001:42):
pub(crate) fn extract_audit_id(line: &str) -> Option<String> {
    let start = line.find("msg=audit(")? + "msg=audit(".len();
    let end = line[start..].find(')')? + start;
    Some(line[start..end].to_string())
}

/// Extract the audit timestamp as microseconds. Format: audit(1713100000.001:42)
pub(crate) fn extract_audit_timestamp_us(line: &str) -> Option<u64> {
    let id = extract_audit_id(line)?;
    let ts_part = id.split(':').next()?;
    let secs_f64: f64 = ts_part.parse().ok()?;
    Some((secs_f64 * 1_000_000.0) as u64)
}

/// Extract a field value from an audit log line. Fields are space-delimited key=value.
pub(crate) fn extract_field(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    // Value ends at next space (or end of line), unless quoted
    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')? + 2;
        Some(rest[..end].to_string())
    } else {
        let end = rest.find(' ').unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

/// Reconstruct argv from EXECVE audit record.
/// Format: type=EXECVE msg=audit(...): argc=3 a0="python3" a1="train.py" a2="--epochs"
pub(crate) fn extract_execve_argv(line: &str) -> Option<String> {
    let mut args = Vec::new();
    let mut i = 0;
    loop {
        let key = format!(" a{i}=");
        if let Some(val) = extract_field(line, &key) {
            args.push(val.trim_matches('"').to_string());
            i += 1;
        } else {
            break;
        }
    }
    if args.is_empty() {
        None
    } else {
        Some(args.join(" "))
    }
}

#[cfg(test)]
mod tests;
