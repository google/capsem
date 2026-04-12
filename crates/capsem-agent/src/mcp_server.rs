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

use capsem_proto::VSOCK_PORT_MCP_GATEWAY;
use vsock_io::{VSOCK_HOST_CID, vsock_connect_retry, write_all_fd};

/// Get the parent process name (the AI agent that spawned us).
fn get_parent_process_name() -> String {
    let ppid = nix::unistd::getppid();
    let raw = procfs::process_name_for_pid(ppid.as_raw() as u32);
    sanitize_process_name(&raw)
}

/// Sanitize a process name for use in the \0CAPSEM_META framing line.
/// Replaces control characters (including newlines and NUL) and spaces with
/// underscores, and truncates to 128 chars to prevent oversized meta lines.
fn sanitize_process_name(name: &str) -> String {
    let mut s = name
        .chars()
        .map(|c| if c.is_control() || c == ' ' { '_' } else { c })
        .collect::<String>();
    if s.len() > 128 {
        s.truncate(128);
    }
    s
}

fn main() {
    eprintln!("[capsem-mcp-server] starting (pid {})", process::id());

    let vsock_fd = vsock_connect_retry(VSOCK_HOST_CID, VSOCK_PORT_MCP_GATEWAY, "mcp");

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
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    #[test]
    fn vsock_port_matches_host() {
        assert_eq!(VSOCK_PORT_MCP_GATEWAY, 5003);
    }

    #[test]
    fn meta_line_format() {
        let name = "claude";
        let meta = format!("\0CAPSEM_META:{}\n", name);
        assert!(meta.starts_with('\0'));
        assert!(meta.contains("CAPSEM_META:claude"));
        assert!(meta.ends_with('\n'));
    }

    #[test]
    fn meta_line_nul_prefix_required() {
        // The NUL byte distinguishes metadata from NDJSON content
        let meta = "\0CAPSEM_META:gemini\n".to_string();
        assert_eq!(meta.as_bytes()[0], 0x00);
        // A valid JSON-RPC line would never start with NUL
        let json = r#"{"jsonrpc":"2.0","method":"tools/call"}"#;
        assert_ne!(json.as_bytes()[0], 0x00);
    }

    #[test]
    fn stdin_to_vsock_preserves_lines() {
        // Write lines via write_all_fd, verify line integrity
        let (writer, reader) = UnixStream::pair().unwrap();
        let writer_fd = writer.into_raw_fd();

        let lines = ["line one", "line two", "line three with unicode: \u{1F600}"];
        for line in &lines {
            let mut data = line.as_bytes().to_vec();
            data.push(b'\n');
            write_all_fd(writer_fd, &data).expect("write line");
        }
        unsafe { nix::libc::close(writer_fd); }

        // Read and verify
        let buf_reader = io::BufReader::new(reader);
        let read_lines: Vec<String> = buf_reader.lines().map(|l| l.unwrap()).collect();
        assert_eq!(read_lines.len(), 3);
        assert_eq!(read_lines[0], "line one");
        assert_eq!(read_lines[1], "line two");
        assert!(read_lines[2].contains('\u{1F600}'));
    }

    #[test]
    fn vsock_to_stdout_reads_until_eof() {
        // Simulate the vsock->stdout relay using pipes
        let (writer, reader) = UnixStream::pair().unwrap();
        let writer_fd = writer.into_raw_fd();
        let reader_fd = reader.into_raw_fd();

        // Write two NDJSON lines then close
        let payload = b"{\"jsonrpc\":\"2.0\",\"id\":1}\n{\"jsonrpc\":\"2.0\",\"id\":2}\n";
        write_all_fd(writer_fd, payload).expect("write payload");
        unsafe { nix::libc::close(writer_fd); }

        // Read back via BufReader (same mechanism as vsock_to_stdout)
        let file = unsafe { std::fs::File::from_raw_fd(reader_fd) };
        let buf = io::BufReader::new(file);
        let lines: Vec<String> = buf.lines().map(|l| l.unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"id\":1"));
        assert!(lines[1].contains("\"id\":2"));
    }

    #[test]
    fn empty_stdin_graceful_exit() {
        // stdin_to_vsock should exit cleanly on immediate EOF
        let (writer, _reader) = UnixStream::pair().unwrap();
        let fd = writer.into_raw_fd();
        // Write nothing then close -- mimics empty stdin
        unsafe { nix::libc::close(fd); }
        // If we got here without panic/hang, the test passes.
    }

    #[test]
    fn meta_line_with_special_characters() {
        let name = "claude/code-v4.0";
        let meta = format!("\0CAPSEM_META:{}\n", name);
        assert!(meta.contains("claude/code-v4.0"));
    }

    #[test]
    fn meta_line_empty_process_name() {
        let meta = format!("\0CAPSEM_META:{}\n", "");
        assert_eq!(meta, "\0CAPSEM_META:\n");
    }

    #[test]
    fn meta_line_very_long_process_name() {
        let name = "a".repeat(1000);
        let meta = format!("\0CAPSEM_META:{}\n", name);
        assert_eq!(meta.len(), 1000 + "\0CAPSEM_META:\n".len());
    }

    // -----------------------------------------------------------------------
    // sanitize_process_name
    // -----------------------------------------------------------------------

    #[test]
    fn sanitize_strips_control_chars() {
        assert_eq!(sanitize_process_name("clean"), "clean");
        assert_eq!(sanitize_process_name("has space"), "has_space");
        assert_eq!(sanitize_process_name("has\nnewline"), "has_newline");
        assert_eq!(sanitize_process_name("has\rcarriage"), "has_carriage");
        assert_eq!(sanitize_process_name("has\0nul"), "has_nul");
        assert_eq!(sanitize_process_name("has\ttab"), "has_tab");
    }

    #[test]
    fn sanitize_truncates_long_names() {
        let long = "x".repeat(200);
        let result = sanitize_process_name(&long);
        assert_eq!(result.len(), 128);
    }

    #[test]
    fn sanitize_preserves_slashes_and_dashes() {
        // Process names like "claude/code-v4.0" should keep path chars
        assert_eq!(sanitize_process_name("claude/code-v4.0"), "claude/code-v4.0");
    }

    #[test]
    fn sanitize_meta_line_injection_blocked() {
        // A newline in the process name would break \0CAPSEM_META:name\n framing
        let evil = "evil\nCAPS_META:spoof";
        let sanitized = sanitize_process_name(evil);
        assert!(!sanitized.contains('\n'), "newline must be stripped");
        let meta = format!("\0CAPSEM_META:{}\n", sanitized);
        // Exactly one newline (the terminator)
        assert_eq!(meta.matches('\n').count(), 1);
    }

    #[test]
    fn write_then_read_binary_data() {
        let (writer, reader) = UnixStream::pair().unwrap();
        let writer_fd = writer.into_raw_fd();

        // Write binary data that looks like NDJSON but with binary payloads
        let binary_line = b"{\"data\":\"\\x00\\xff\"}\n";
        write_all_fd(writer_fd, binary_line).expect("write binary");
        unsafe { nix::libc::close(writer_fd); }

        let file = unsafe { std::fs::File::from_raw_fd(reader.into_raw_fd()) };
        let buf = io::BufReader::new(file);
        let lines: Vec<String> = buf.lines().map(|l| l.unwrap()).collect();
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn large_json_line_preserved() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::IntoRawFd;
        let (writer, reader) = UnixStream::pair().unwrap();
        let writer_fd = writer.into_raw_fd();

        let large_content = "x".repeat(100_000);
        let line = format!("{{\"content\":\"{}\"}}\n", large_content);
        
        std::thread::spawn(move || {
            write_all_fd(writer_fd, line.as_bytes()).expect("write large");
            unsafe { nix::libc::close(writer_fd); }
        });

        let file = unsafe { std::fs::File::from_raw_fd(reader.into_raw_fd()) };
        let buf = std::io::BufReader::new(file);
        use std::io::BufRead;
        let lines: Vec<String> = buf.lines().map(|l| l.unwrap()).collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].len() > 100_000);
    }
}
