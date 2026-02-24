// capsem-pty-agent: Guest-side PTY-over-vsock bridge.
//
// Runs inside the Linux VM as a child of capsem-init. Creates a PTY pair,
// forks bash on the slave side, and bridges the master PTY with the host
// over two vsock connections:
//   - Port 5001: raw PTY I/O (terminal data)
//   - Port 5000: control messages (resize, heartbeat)

use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use nix::libc;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::pty::openpty;
use nix::sys::signal::{SigHandler, Signal, signal};
use nix::unistd::{ForkResult, Pid, close, dup2, execvp, fork, setsid};

use serde::{Deserialize, Serialize};

/// vsock port for control messages.
const VSOCK_PORT_CONTROL: u32 = 5000;
/// vsock port for terminal data.
const VSOCK_PORT_TERMINAL: u32 = 5001;
/// Host CID (always 2 for the hypervisor).
const VSOCK_HOST_CID: u32 = 2;
/// AF_VSOCK address family.
const AF_VSOCK: i32 = 40;

/// Control messages shared with the host (must match capsem-core::vsock::ControlMessage).
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "t", content = "d", rename_all = "lowercase")]
enum ControlMessage {
    Ready { version: String },
    Resize { cols: u16, rows: u16 },
    Ping,
    Pong,
    Exec { id: u64, command: String },
    ExecDone { id: u64, exit_code: i32 },
}

// ---------------------------------------------------------------------------
// vsock helpers (using libc directly -- nix doesn't support AF_VSOCK)
// ---------------------------------------------------------------------------

#[repr(C)]
struct SockaddrVm {
    svm_family: libc::sa_family_t,
    svm_reserved1: u16,
    svm_port: u32,
    svm_cid: u32,
    svm_flags: u8,
    svm_zero: [u8; 3],
}

fn vsock_connect(cid: u32, port: u32) -> io::Result<RawFd> {
    let fd = unsafe { libc::socket(AF_VSOCK, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    let addr = SockaddrVm {
        svm_family: AF_VSOCK as libc::sa_family_t,
        svm_reserved1: 0,
        svm_port: port,
        svm_cid: cid,
        svm_flags: 0,
        svm_zero: [0; 3],
    };

    let ret = unsafe {
        libc::connect(
            fd,
            &addr as *const SockaddrVm as *const libc::sockaddr,
            std::mem::size_of::<SockaddrVm>() as libc::socklen_t,
        )
    };
    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd); }
        return Err(err);
    }

    Ok(fd)
}

fn vsock_connect_retry(cid: u32, port: u32, label: &str) -> RawFd {
    let mut delay_ms = 100;
    loop {
        match vsock_connect(cid, port) {
            Ok(fd) => {
                eprintln!("[capsem-agent] {label} connected (port {port})");
                return fd;
            }
            Err(e) => {
                eprintln!("[capsem-agent] {label} connect failed: {e}, retrying in {delay_ms}ms");
                thread::sleep(Duration::from_millis(delay_ms));
                delay_ms = (delay_ms * 2).min(2000);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Control message framing
// ---------------------------------------------------------------------------

fn send_control_msg(fd: RawFd, msg: &ControlMessage) -> io::Result<()> {
    let payload = rmp_serde::to_vec_named(msg)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let len = (payload.len() as u32).to_be_bytes();
    write_all_fd(fd, &len)?;
    write_all_fd(fd, &payload)?;
    Ok(())
}

fn recv_control_msg(fd: RawFd) -> io::Result<ControlMessage> {
    let mut len_buf = [0u8; 4];
    read_exact_fd(fd, &mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 4096 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "control frame too large"));
    }
    let mut payload = vec![0u8; len];
    read_exact_fd(fd, &mut payload)?;
    rmp_serde::from_slice(&payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

// ---------------------------------------------------------------------------
// PTY resize
// ---------------------------------------------------------------------------

fn set_winsize(master_fd: RawFd, cols: u16, rows: u16) {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        libc::ioctl(master_fd, libc::TIOCSWINSZ, &ws);
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    eprintln!("[capsem-agent] starting (pid {})", process::id());

    // Open PTY pair.
    let pty = openpty(None, None).expect("openpty failed");
    let master_fd = pty.master.as_raw_fd();
    let slave_fd = pty.slave.as_raw_fd();

    // Set initial terminal size (80x24 default).
    set_winsize(master_fd, 80, 24);

    // Fork: child becomes bash on the slave PTY.
    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            // Close master in child.
            drop(pty.master);

            // Create a new session so the slave PTY becomes the controlling terminal.
            setsid().expect("setsid failed");

            // Set the slave as the controlling terminal.
            unsafe {
                libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0);
            }

            // Redirect stdio to the slave PTY.
            dup2(slave_fd, 0).expect("dup2 stdin failed");
            dup2(slave_fd, 1).expect("dup2 stdout failed");
            dup2(slave_fd, 2).expect("dup2 stderr failed");

            if slave_fd > 2 {
                let _ = close(slave_fd);
            }

            // Set environment.
            std::env::set_var("TERM", "xterm-256color");
            std::env::set_var("HOME", "/root");
            std::env::set_var("LANG", "C");

            // Exec bash (never returns on success).
            let bash = std::ffi::CString::new("/bin/bash").unwrap();
            let rcfile = std::ffi::CString::new("--rcfile").unwrap();
            let rcpath = std::ffi::CString::new("/etc/capsem-bashrc").unwrap();
            let interactive = std::ffi::CString::new("-i").unwrap();
            match execvp(&bash, &[&bash, &rcfile, &rcpath, &interactive]) {
                Ok(infallible) => match infallible {},
                Err(e) => {
                    eprintln!("[capsem-agent] execvp failed: {e}");
                    process::exit(1);
                }
            }
        }
        Ok(ForkResult::Parent { child }) => {
            // Close slave in parent.
            drop(pty.slave);

            // Ignore SIGHUP so we don't die when the child exits.
            unsafe { signal(Signal::SIGHUP, SigHandler::SigIgn) }.ok();

            run_bridge(master_fd, child);
        }
        Err(e) => {
            eprintln!("[capsem-agent] fork failed: {e}");
            process::exit(1);
        }
    }
}

/// Sentinel prefix for exec completion detection.
/// Format: ESC _ CAPSEM_EXIT:{id}:{exit_code} ESC \
const SENTINEL_PREFIX: &[u8] = b"\x1b_CAPSEM_EXIT:";
const SENTINEL_TERMINATOR: &[u8] = b"\x1b\\";

/// Shared state between control_loop and bridge_loop for exec tracking.
struct ExecState {
    active: AtomicBool,
    current_id: Mutex<Option<u64>>,
}

fn run_bridge(master_fd: RawFd, child_pid: Pid) {
    // Connect to host vsock ports with retry.
    let terminal_fd = vsock_connect_retry(VSOCK_HOST_CID, VSOCK_PORT_TERMINAL, "terminal");
    let control_fd = vsock_connect_retry(VSOCK_HOST_CID, VSOCK_PORT_CONTROL, "control");

    // Send Ready message.
    if let Err(e) = send_control_msg(control_fd, &ControlMessage::Ready {
        version: env!("CARGO_PKG_VERSION").to_string(),
    }) {
        eprintln!("[capsem-agent] failed to send Ready: {e}");
    }

    // Shared exec state between control and bridge loops.
    let exec_state = Arc::new(ExecState {
        active: AtomicBool::new(false),
        current_id: Mutex::new(None),
    });
    // Channel for bridge_loop to report exec completion to control_loop.
    let (exec_done_tx, exec_done_rx) = mpsc::channel::<(u64, i32)>();

    // Spawn control channel handler in a background thread.
    let exec_state_ctrl = Arc::clone(&exec_state);
    let master_fd_for_ctrl = master_fd;
    thread::spawn(move || {
        control_loop(control_fd, master_fd_for_ctrl, exec_state_ctrl, exec_done_rx);
    });

    // Main I/O bridge: master PTY <-> vsock terminal port.
    bridge_loop(master_fd, terminal_fd, &exec_state, exec_done_tx);

    // If bridge exits, kill the child shell to prevent orphans.
    eprintln!("[capsem-agent] bridge exited, killing child shell");
    let _ = nix::sys::signal::kill(child_pid, Signal::SIGHUP);
    let _ = nix::sys::wait::waitpid(child_pid, None);
}

/// Write all bytes to an fd, retrying on partial writes.
fn write_all_fd(fd: RawFd, data: &[u8]) -> io::Result<()> {
    let mut written = 0;
    while written < data.len() {
        match nix::unistd::write(
            unsafe { std::os::unix::io::BorrowedFd::borrow_raw(fd) },
            &data[written..],
        ) {
            Ok(n) => written += n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Read exactly `buf.len()` bytes from an fd, retrying on partial reads.
fn read_exact_fd(fd: RawFd, buf: &mut [u8]) -> io::Result<()> {
    let mut pos = 0;
    while pos < buf.len() {
        match nix::unistd::read(fd, &mut buf[pos..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "unexpected EOF",
                ))
            }
            Ok(n) => pos += n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Scan for sentinel in data, stripping it from the forwarded output.
/// Returns (data_to_forward, optional (id, exit_code) if sentinel found).
///
/// The sentinel format is: ESC _ CAPSEM_EXIT:{id}:{exit_code} ESC \
/// We use a tail buffer to handle sentinels that span read boundaries.
fn scan_and_strip_sentinel(
    tail: &mut Vec<u8>,
    new_data: &[u8],
) -> (Vec<u8>, Option<(u64, i32)>) {
    // Combine tail with new data for scanning.
    tail.extend_from_slice(new_data);

    // Search for the sentinel start marker in the combined buffer.
    if let Some(start) = find_subsequence(tail, SENTINEL_PREFIX) {
        // Look for the terminator after the prefix.
        let after_prefix = start + SENTINEL_PREFIX.len();
        if let Some(term_offset) = find_subsequence(&tail[after_prefix..], SENTINEL_TERMINATOR) {
            let term_pos = after_prefix + term_offset;
            // Extract the payload between prefix and terminator: "{id}:{exit_code}"
            let payload = &tail[after_prefix..term_pos];
            if let Some(result) = parse_sentinel_payload(payload) {
                // Data before sentinel goes to host; sentinel + terminator stripped.
                let before = tail[..start].to_vec();
                let after_sentinel = term_pos + SENTINEL_TERMINATOR.len();
                // Keep any data after the sentinel in tail for next iteration.
                let remainder = tail[after_sentinel..].to_vec();
                tail.clear();
                tail.extend_from_slice(&remainder);
                return (before, Some(result));
            }
        }
        // Sentinel started but not yet complete -- keep everything from the
        // start marker in tail, forward everything before it.
        let before = tail[..start].to_vec();
        let kept = tail[start..].to_vec();
        tail.clear();
        tail.extend_from_slice(&kept);
        return (before, None);
    }

    // No sentinel prefix found. Forward all data except the last few bytes
    // that could be the start of a partial sentinel prefix.
    let keep = SENTINEL_PREFIX.len() - 1;
    if tail.len() > keep {
        let forward_end = tail.len() - keep;
        let forward = tail[..forward_end].to_vec();
        let kept = tail[forward_end..].to_vec();
        tail.clear();
        tail.extend_from_slice(&kept);
        (forward, None)
    } else {
        // Not enough data to forward anything yet.
        (Vec::new(), None)
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn parse_sentinel_payload(payload: &[u8]) -> Option<(u64, i32)> {
    let s = std::str::from_utf8(payload).ok()?;
    let mut parts = s.splitn(2, ':');
    let id: u64 = parts.next()?.parse().ok()?;
    let exit_code: i32 = parts.next()?.parse().ok()?;
    Some((id, exit_code))
}

fn bridge_loop(
    master_fd: RawFd,
    vsock_fd: RawFd,
    exec_state: &ExecState,
    exec_done_tx: mpsc::Sender<(u64, i32)>,
) {
    let mut buf = [0u8; 8192];
    // Rolling tail buffer for sentinel detection across read boundaries.
    let mut tail: Vec<u8> = Vec::with_capacity(128);

    loop {
        let mut poll_fds = [
            PollFd::new(unsafe { std::os::unix::io::BorrowedFd::borrow_raw(master_fd) }, PollFlags::POLLIN),
            PollFd::new(unsafe { std::os::unix::io::BorrowedFd::borrow_raw(vsock_fd) }, PollFlags::POLLIN),
        ];

        match poll(&mut poll_fds, PollTimeout::from(1000u16)) {
            Ok(0) => continue,
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => {
                eprintln!("[capsem-agent] poll error: {e}");
                break;
            }
        }

        // Master PTY -> vsock (stdout direction)
        if let Some(revents) = poll_fds[0].revents() {
            if revents.contains(PollFlags::POLLIN) {
                match nix::unistd::read(master_fd, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if exec_state.active.load(Ordering::Acquire) {
                            // Exec active: scan for sentinel before forwarding.
                            let (forward, result) = scan_and_strip_sentinel(
                                &mut tail,
                                &buf[..n],
                            );
                            if !forward.is_empty() {
                                if write_all_fd(vsock_fd, &forward).is_err() {
                                    break;
                                }
                            }
                            if let Some((id, exit_code)) = result {
                                exec_state.active.store(false, Ordering::Release);
                                // Flush remaining tail data BEFORE signaling
                                // ExecDone so terminal output reaches the host
                                // before the control message triggers shutdown.
                                if !tail.is_empty() {
                                    let remaining = std::mem::take(&mut tail);
                                    if write_all_fd(vsock_fd, &remaining).is_err() {
                                        break;
                                    }
                                }
                                let _ = exec_done_tx.send((id, exit_code));
                            }
                        } else {
                            if write_all_fd(vsock_fd, &buf[..n]).is_err() {
                                break;
                            }
                        }
                    }
                    Err(nix::errno::Errno::EAGAIN) => {}
                    Err(_) => break,
                }
            }
            if revents.intersects(PollFlags::POLLHUP | PollFlags::POLLERR) {
                break;
            }
        }

        // vsock -> Master PTY (stdin direction)
        if let Some(revents) = poll_fds[1].revents() {
            if revents.contains(PollFlags::POLLIN) {
                match nix::unistd::read(vsock_fd, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if write_all_fd(master_fd, &buf[..n]).is_err() {
                            break;
                        }
                    }
                    Err(nix::errno::Errno::EAGAIN) => {}
                    Err(_) => break,
                }
            }
            if revents.intersects(PollFlags::POLLHUP | PollFlags::POLLERR) {
                break;
            }
        }
    }
}

fn control_loop(
    control_fd: RawFd,
    master_fd: RawFd,
    exec_state: Arc<ExecState>,
    exec_done_rx: mpsc::Receiver<(u64, i32)>,
) {
    loop {
        match recv_control_msg(control_fd) {
            Ok(ControlMessage::Resize { cols, rows }) => {
                eprintln!("[capsem-agent] resize: {cols}x{rows}");
                set_winsize(master_fd, cols, rows);
                // Send SIGWINCH to the foreground process group.
                unsafe {
                    let mut pgrp: libc::pid_t = 0;
                    if libc::ioctl(master_fd, libc::TIOCGPGRP, &mut pgrp) == 0 && pgrp > 0 {
                        libc::kill(-pgrp, libc::SIGWINCH);
                    }
                }
            }
            Ok(ControlMessage::Ping) => {
                if let Err(e) = send_control_msg(control_fd, &ControlMessage::Pong) {
                    eprintln!("[capsem-agent] failed to send Pong: {e}");
                    break;
                }
            }
            Ok(ControlMessage::Exec { id, command }) => {
                eprintln!("[capsem-agent] exec[{id}]: {command}");
                // Store exec id and activate sentinel scanning.
                {
                    let mut current = exec_state.current_id.lock().unwrap();
                    *current = Some(id);
                }
                exec_state.active.store(true, Ordering::Release);

                // Disable PTY echo so the injected command text isn't shown.
                // Command output (stdout/stderr) still appears -- ECHO only
                // controls input echoing, not program output.
                unsafe {
                    let mut termios: libc::termios = std::mem::zeroed();
                    libc::tcgetattr(master_fd, &mut termios);
                    termios.c_lflag &= !libc::ECHO;
                    libc::tcsetattr(master_fd, libc::TCSANOW, &termios);
                }

                // Inject command into PTY with sentinel.
                // Use printf so the sentinel only appears in evaluated output,
                // not in the PTY echo of the command text.
                let injection = format!(
                    "bash -c '{}' ; printf '\\033_CAPSEM_EXIT:{}:%d\\033\\\\' $?\n",
                    command.replace('\'', "'\\''"),
                    id,
                );
                if let Err(e) = write_all_fd(master_fd, injection.as_bytes()) {
                    eprintln!("[capsem-agent] failed to inject exec command: {e}");
                    exec_state.active.store(false, Ordering::Release);
                    // Send ExecDone with error exit code.
                    let _ = send_control_msg(control_fd, &ControlMessage::ExecDone {
                        id,
                        exit_code: 126,
                    });
                    continue;
                }

                // Wait for bridge_loop to detect the sentinel and report.
                match exec_done_rx.recv() {
                    Ok((done_id, exit_code)) => {
                        eprintln!("[capsem-agent] exec[{done_id}] done: exit_code={exit_code}");
                        // Re-enable PTY echo for interactive use.
                        unsafe {
                            let mut termios: libc::termios = std::mem::zeroed();
                            libc::tcgetattr(master_fd, &mut termios);
                            termios.c_lflag |= libc::ECHO;
                            libc::tcsetattr(master_fd, libc::TCSANOW, &termios);
                        }
                        if let Err(e) = send_control_msg(control_fd, &ControlMessage::ExecDone {
                            id: done_id,
                            exit_code,
                        }) {
                            eprintln!("[capsem-agent] failed to send ExecDone: {e}");
                            break;
                        }
                    }
                    Err(_) => {
                        eprintln!("[capsem-agent] exec_done channel closed");
                        break;
                    }
                }
            }
            Ok(msg) => {
                eprintln!("[capsem-agent] unexpected control message: {msg:?}");
            }
            Err(e) => {
                eprintln!("[capsem-agent] control channel error: {e}");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::io::FromRawFd;

    fn make_pipe() -> (RawFd, RawFd) {
        let mut fds = [0 as RawFd; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    }

    // -----------------------------------------------------------------------
    // Wire format compatibility with host
    // -----------------------------------------------------------------------

    #[test]
    fn agent_ready_decodable_by_host() {
        // Agent encodes Ready; host must be able to decode it.
        let msg = ControlMessage::Ready { version: "0.3.0".to_string() };
        let payload = rmp_serde::to_vec_named(&msg).unwrap();
        // Simulate host-side decode using rmp_serde directly.
        let decoded: ControlMessage = rmp_serde::from_slice(&payload).unwrap();
        match decoded {
            ControlMessage::Ready { version } => assert_eq!(version, "0.3.0"),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn host_resize_decodable_by_agent() {
        // Host encodes Resize; agent must be able to decode it.
        let msg = ControlMessage::Resize { cols: 200, rows: 50 };
        let payload = rmp_serde::to_vec_named(&msg).unwrap();
        let decoded: ControlMessage = rmp_serde::from_slice(&payload).unwrap();
        match decoded {
            ControlMessage::Resize { cols, rows } => {
                assert_eq!(cols, 200);
                assert_eq!(rows, 50);
            }
            other => panic!("expected Resize, got {other:?}"),
        }
    }

    #[test]
    fn host_ping_decodable_by_agent() {
        let msg = ControlMessage::Ping;
        let payload = rmp_serde::to_vec_named(&msg).unwrap();
        let decoded: ControlMessage = rmp_serde::from_slice(&payload).unwrap();
        assert!(matches!(decoded, ControlMessage::Ping));
    }

    #[test]
    fn agent_pong_decodable_by_host() {
        let msg = ControlMessage::Pong;
        let payload = rmp_serde::to_vec_named(&msg).unwrap();
        let decoded: ControlMessage = rmp_serde::from_slice(&payload).unwrap();
        assert!(matches!(decoded, ControlMessage::Pong));
    }

    #[test]
    fn exec_roundtrip_host_to_agent() {
        let msg = ControlMessage::Exec { id: 42, command: "ls -la".to_string() };
        let payload = rmp_serde::to_vec_named(&msg).unwrap();
        let decoded: ControlMessage = rmp_serde::from_slice(&payload).unwrap();
        match decoded {
            ControlMessage::Exec { id, command } => {
                assert_eq!(id, 42);
                assert_eq!(command, "ls -la");
            }
            other => panic!("expected Exec, got {other:?}"),
        }
    }

    #[test]
    fn exec_done_roundtrip_agent_to_host() {
        let msg = ControlMessage::ExecDone { id: 42, exit_code: 0 };
        let payload = rmp_serde::to_vec_named(&msg).unwrap();
        let decoded: ControlMessage = rmp_serde::from_slice(&payload).unwrap();
        match decoded {
            ControlMessage::ExecDone { id, exit_code } => {
                assert_eq!(id, 42);
                assert_eq!(exit_code, 0);
            }
            other => panic!("expected ExecDone, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Framing over pipes (simulates vsock fd)
    // -----------------------------------------------------------------------

    #[test]
    fn send_recv_roundtrip_over_pipe() {
        let (read_fd, write_fd) = make_pipe();
        let msg = ControlMessage::Resize { cols: 132, rows: 43 };
        send_control_msg(write_fd, &msg).unwrap();
        let decoded = recv_control_msg(read_fd).unwrap();
        match decoded {
            ControlMessage::Resize { cols, rows } => {
                assert_eq!(cols, 132);
                assert_eq!(rows, 43);
            }
            other => panic!("expected Resize, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn send_recv_exec_over_pipe() {
        let (read_fd, write_fd) = make_pipe();
        let msg = ControlMessage::Exec { id: 99, command: "echo hi".to_string() };
        send_control_msg(write_fd, &msg).unwrap();
        let decoded = recv_control_msg(read_fd).unwrap();
        match decoded {
            ControlMessage::Exec { id, command } => {
                assert_eq!(id, 99);
                assert_eq!(command, "echo hi");
            }
            other => panic!("expected Exec, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn send_recv_exec_done_over_pipe() {
        let (read_fd, write_fd) = make_pipe();
        let msg = ControlMessage::ExecDone { id: 99, exit_code: 1 };
        send_control_msg(write_fd, &msg).unwrap();
        let decoded = recv_control_msg(read_fd).unwrap();
        match decoded {
            ControlMessage::ExecDone { id, exit_code } => {
                assert_eq!(id, 99);
                assert_eq!(exit_code, 1);
            }
            other => panic!("expected ExecDone, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn send_recv_multiple_messages_over_pipe() {
        let (read_fd, write_fd) = make_pipe();

        send_control_msg(write_fd, &ControlMessage::Ping).unwrap();
        send_control_msg(write_fd, &ControlMessage::Resize { cols: 80, rows: 24 }).unwrap();
        send_control_msg(write_fd, &ControlMessage::Pong).unwrap();

        assert!(matches!(recv_control_msg(read_fd).unwrap(), ControlMessage::Ping));
        match recv_control_msg(read_fd).unwrap() {
            ControlMessage::Resize { cols, rows } => {
                assert_eq!(cols, 80);
                assert_eq!(rows, 24);
            }
            other => panic!("expected Resize, got {other:?}"),
        }
        assert!(matches!(recv_control_msg(read_fd).unwrap(), ControlMessage::Pong));

        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn recv_rejects_oversized_frame() {
        let (read_fd, write_fd) = make_pipe();
        // Write a length prefix claiming 8KB (> 4KB limit).
        let len_bytes = (8192u32).to_be_bytes();
        let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
        writer.write_all(&len_bytes).unwrap();
        std::mem::forget(writer);

        let result = recv_control_msg(read_fd);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn recv_eof_returns_error() {
        let (read_fd, write_fd) = make_pipe();
        unsafe { libc::close(write_fd); }
        let result = recv_control_msg(read_fd);
        assert!(result.is_err());
        unsafe { libc::close(read_fd); }
    }

    // -----------------------------------------------------------------------
    // SockaddrVm struct layout
    // -----------------------------------------------------------------------

    #[test]
    fn sockaddr_vm_size_matches_kernel() {
        // Linux sockaddr_vm is 16 bytes.
        assert_eq!(
            std::mem::size_of::<SockaddrVm>(),
            16,
            "SockaddrVm must be 16 bytes to match kernel struct"
        );
    }

    #[test]
    fn sockaddr_vm_field_offsets() {
        // Verify critical fields are at the right byte offsets.
        let addr = SockaddrVm {
            svm_family: 0,
            svm_reserved1: 0,
            svm_port: 0,
            svm_cid: 0,
            svm_flags: 0,
            svm_zero: [0; 3],
        };
        let base = &addr as *const _ as usize;
        let family_offset = &addr.svm_family as *const _ as usize - base;
        let port_offset = &addr.svm_port as *const _ as usize - base;
        let cid_offset = &addr.svm_cid as *const _ as usize - base;
        assert_eq!(family_offset, 0, "svm_family must be at offset 0");
        assert_eq!(port_offset, 4, "svm_port must be at offset 4");
        assert_eq!(cid_offset, 8, "svm_cid must be at offset 8");
    }

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    #[test]
    fn port_constants_match_host() {
        // These must match capsem-core::vsock constants.
        assert_eq!(VSOCK_PORT_CONTROL, 5000);
        assert_eq!(VSOCK_PORT_TERMINAL, 5001);
    }

    #[test]
    fn host_cid_is_two() {
        // The host/hypervisor CID is always 2 in the vsock spec.
        assert_eq!(VSOCK_HOST_CID, 2);
    }

    #[test]
    fn af_vsock_is_40() {
        assert_eq!(AF_VSOCK, 40);
    }

    // -----------------------------------------------------------------------
    // PTY winsize
    // -----------------------------------------------------------------------

    #[test]
    fn set_winsize_on_real_pty() {
        let pty = openpty(None, None).expect("openpty failed");
        let master_fd = pty.master.as_raw_fd();
        set_winsize(master_fd, 200, 50);

        // Read it back.
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::ioctl(master_fd, libc::TIOCGWINSZ, &mut ws) };
        assert_eq!(ret, 0);
        assert_eq!(ws.ws_col, 200);
        assert_eq!(ws.ws_row, 50);
    }

    #[test]
    fn set_winsize_boundary_values() {
        let pty = openpty(None, None).expect("openpty failed");
        let master_fd = pty.master.as_raw_fd();

        // Minimum.
        set_winsize(master_fd, 1, 1);
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        unsafe { libc::ioctl(master_fd, libc::TIOCGWINSZ, &mut ws); }
        assert_eq!(ws.ws_col, 1);
        assert_eq!(ws.ws_row, 1);

        // Large.
        set_winsize(master_fd, 500, 200);
        unsafe { libc::ioctl(master_fd, libc::TIOCGWINSZ, &mut ws); }
        assert_eq!(ws.ws_col, 500);
        assert_eq!(ws.ws_row, 200);
    }

    // -----------------------------------------------------------------------
    // Sentinel scanning
    // -----------------------------------------------------------------------

    #[test]
    fn sentinel_detected_in_single_chunk() {
        let mut tail = Vec::new();
        let data = b"some output\x1b_CAPSEM_EXIT:42:0\x1b\\more data";
        let (forward, result) = scan_and_strip_sentinel(&mut tail, data);
        assert_eq!(&forward, b"some output");
        assert_eq!(result, Some((42, 0)));
        assert_eq!(&tail, b"more data");
    }

    #[test]
    fn sentinel_with_nonzero_exit_code() {
        let mut tail = Vec::new();
        let data = b"error output\x1b_CAPSEM_EXIT:7:127\x1b\\";
        let (forward, result) = scan_and_strip_sentinel(&mut tail, data);
        assert_eq!(&forward, b"error output");
        assert_eq!(result, Some((7, 127)));
    }

    #[test]
    fn sentinel_split_across_two_reads() {
        let mut tail = Vec::new();
        // First chunk: partial sentinel (tail keeps last SENTINEL_PREFIX.len()-1 bytes).
        let (forward1, result1) = scan_and_strip_sentinel(
            &mut tail,
            b"output\x1b_CAPSEM_EX",
        );
        assert!(result1.is_none());
        // Conservative tail: some leading bytes may be held back.
        assert!(!forward1.is_empty());

        // Second chunk: rest of sentinel.
        let (forward2, result2) = scan_and_strip_sentinel(
            &mut tail,
            b"IT:42:0\x1b\\trailing",
        );
        assert_eq!(result2, Some((42, 0)));
        // All data before sentinel should have been forwarded across both calls.
        let mut all_forwarded = forward1.clone();
        all_forwarded.extend_from_slice(&forward2);
        assert_eq!(&all_forwarded, b"output");
        assert_eq!(&tail, b"trailing");
    }

    #[test]
    fn no_sentinel_forwards_data() {
        let mut tail = Vec::new();
        let data = b"just normal terminal output here\n";
        let (forward, result) = scan_and_strip_sentinel(&mut tail, data);
        assert!(result.is_none());
        // Should forward most data, keeping a small tail for potential partial sentinel.
        assert!(!forward.is_empty());
        assert!(forward.len() + tail.len() == data.len());
    }

    #[test]
    fn sentinel_negative_exit_code() {
        let mut tail = Vec::new();
        let data = b"\x1b_CAPSEM_EXIT:1:-1\x1b\\";
        let (forward, result) = scan_and_strip_sentinel(&mut tail, data);
        assert!(forward.is_empty());
        assert_eq!(result, Some((1, -1)));
    }

    #[test]
    fn parse_sentinel_payload_valid() {
        assert_eq!(parse_sentinel_payload(b"42:0"), Some((42, 0)));
        assert_eq!(parse_sentinel_payload(b"1:127"), Some((1, 127)));
        assert_eq!(parse_sentinel_payload(b"18446744073709551615:0"), Some((u64::MAX, 0)));
    }

    #[test]
    fn parse_sentinel_payload_invalid() {
        assert_eq!(parse_sentinel_payload(b""), None);
        assert_eq!(parse_sentinel_payload(b"42"), None);
        assert_eq!(parse_sentinel_payload(b"abc:0"), None);
        assert_eq!(parse_sentinel_payload(b"42:abc"), None);
    }

    #[test]
    fn find_subsequence_basic() {
        assert_eq!(find_subsequence(b"hello world", b"world"), Some(6));
        assert_eq!(find_subsequence(b"hello world", b"xyz"), None);
        assert_eq!(find_subsequence(b"abc", b"abc"), Some(0));
    }
}
