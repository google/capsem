// capsem-mcp-server: Guest-side MCP stdio-to-vsock relay.
//
// Bridges NDJSON lines from an AI agent's MCP client (stdin/stdout) to the
// host MCP gateway over vsock:5003. The host handles all routing, policy,
// and tool execution. This binary just passes bytes through.
//
// Wire protocol:
//   1. Connect to vsock:5003
//   2. Send metadata line: \0CAPSEM_META:process_name\n
//   3. Relay: stdin -> vsock, vsock -> stdout (bidirectional, line-at-a-time)

#[path = "vsock_io.rs"]
mod vsock_io;

#[path = "procfs.rs"]
mod procfs;

use std::io::{self, BufRead, Write};
use std::os::unix::io::{FromRawFd, RawFd};
use std::process;
use std::thread;

use vsock_io::{VSOCK_HOST_CID, vsock_connect_retry, write_all_fd};

/// Vsock port for MCP gateway on the host.
const VSOCK_PORT_MCP: u32 = 5003;

/// Get the parent process name (the AI agent that spawned us).
fn get_parent_process_name() -> String {
    let ppid = nix::unistd::getppid();
    procfs::process_name_for_pid(ppid.as_raw() as u32)
}

fn main() {
    eprintln!("[capsem-mcp-server] starting (pid {})", process::id());

    let vsock_fd = vsock_connect_retry(VSOCK_HOST_CID, VSOCK_PORT_MCP, "mcp");

    // Send metadata line
    let process_name = get_parent_process_name();
    let meta = format!("\0CAPSEM_META:{}\n", process_name);
    if let Err(e) = write_all_fd(vsock_fd, meta.as_bytes()) {
        eprintln!("[capsem-mcp-server] failed to send metadata: {e}");
        process::exit(1);
    }

    // Spawn reader thread: vsock -> stdout
    let reader_fd = vsock_fd;
    let reader_handle = thread::spawn(move || {
        vsock_to_stdout(reader_fd);
    });

    // Main thread: stdin -> vsock
    stdin_to_vsock(vsock_fd);

    // stdin closed -- half-close the write end so the gateway sees EOF and
    // flushes its remaining responses. The reader thread keeps pulling
    // responses from the vsock until the gateway closes its end.
    unsafe { nix::libc::shutdown(vsock_fd, nix::libc::SHUT_WR); }
    let _ = reader_handle.join();
    unsafe { nix::libc::close(vsock_fd); }
}

/// Read lines from vsock and write to stdout.
fn vsock_to_stdout(fd: RawFd) {
    let dup_fd = unsafe { nix::libc::dup(fd) };
    let file = unsafe { std::fs::File::from_raw_fd(dup_fd) };
    let reader = io::BufReader::new(file);
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in reader.lines() {
        match line {
            Ok(l) => {
                if writeln!(out, "{}", l).is_err() || out.flush().is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// Read lines from stdin and write to vsock.
fn stdin_to_vsock(fd: RawFd) {
    let stdin = io::stdin();
    let reader = stdin.lock();

    for line in reader.lines() {
        match line {
            Ok(l) => {
                let mut data = l.into_bytes();
                data.push(b'\n');
                if write_all_fd(fd, &data).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vsock_port_matches_host() {
        assert_eq!(VSOCK_PORT_MCP, 5003);
    }

    #[test]
    fn meta_line_format() {
        let name = "claude";
        let meta = format!("\0CAPSEM_META:{}\n", name);
        assert!(meta.starts_with('\0'));
        assert!(meta.contains("CAPSEM_META:claude"));
        assert!(meta.ends_with('\n'));
    }
}
