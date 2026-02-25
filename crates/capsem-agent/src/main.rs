// capsem-pty-agent: Guest-side PTY-over-vsock bridge.
//
// Runs inside the Linux VM as a child of capsem-init. Creates a PTY pair,
// forks bash on the slave side, and bridges the master PTY with the host
// over two vsock connections:
//   - Port 5001: raw PTY I/O (terminal data)
//   - Port 5000: control messages (resize, heartbeat, boot config)

#[path = "vsock_io.rs"]
mod vsock_io;

use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use capsem_proto::{
    GuestToHost, HostToGuest, MAX_FRAME_SIZE, decode_host_msg, encode_guest_msg,
};
use nix::libc;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::pty::openpty;
use nix::sys::signal::{SigHandler, Signal, signal};
use nix::unistd::{ForkResult, Pid, close, dup2, execvp, fork, setsid};

use vsock_io::{VSOCK_HOST_CID, read_exact_fd, vsock_connect_retry, write_all_fd};

/// vsock port for control messages.
const VSOCK_PORT_CONTROL: u32 = 5000;
/// vsock port for terminal data.
const VSOCK_PORT_TERMINAL: u32 = 5001;

// ---------------------------------------------------------------------------
// Control message framing (using capsem-proto types)
// ---------------------------------------------------------------------------

fn send_guest_msg(fd: RawFd, msg: &GuestToHost) -> io::Result<()> {
    let frame = encode_guest_msg(msg)
        .map_err(|e| io::Error::other(e))?;
    write_all_fd(fd, &frame)?;
    Ok(())
}

fn recv_host_msg(fd: RawFd) -> io::Result<HostToGuest> {
    let mut len_buf = [0u8; 4];
    read_exact_fd(fd, &mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_SIZE as usize {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "control frame too large"));
    }
    let mut payload = vec![0u8; len];
    read_exact_fd(fd, &mut payload)?;
    decode_host_msg(&payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

// ---------------------------------------------------------------------------
// Clock sync
// ---------------------------------------------------------------------------

fn set_system_clock(epoch_secs: u64) {
    let ts = libc::timespec {
        tv_sec: epoch_secs as _,
        tv_nsec: 0,
    };
    let ret = unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &ts) };
    if ret == 0 {
        eprintln!("[capsem-agent] clock set to epoch {epoch_secs}");
    } else {
        eprintln!(
            "[capsem-agent] WARNING: clock_settime failed ({}): \
             agent must run as root with CAP_SYS_TIME",
            std::io::Error::last_os_error()
        );
    }
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

    // Step 1: Connect to host vsock ports BEFORE PTY/fork.
    let terminal_fd = vsock_connect_retry(VSOCK_HOST_CID, VSOCK_PORT_TERMINAL, "terminal");
    let control_fd = vsock_connect_retry(VSOCK_HOST_CID, VSOCK_PORT_CONTROL, "control");

    // Step 2: Send Ready.
    if let Err(e) = send_guest_msg(control_fd, &GuestToHost::Ready {
        version: env!("CARGO_PKG_VERSION").to_string(),
    }) {
        eprintln!("[capsem-agent] failed to send Ready: {e}");
        process::exit(1);
    }

    // Step 3: Wait for BootConfig from host.
    let (boot_epoch, boot_env) = match recv_host_msg(control_fd) {
        Ok(HostToGuest::BootConfig { epoch_secs, env_vars }) => {
            eprintln!("[capsem-agent] received BootConfig (epoch={epoch_secs}, {} env vars)", env_vars.len());
            (epoch_secs, env_vars)
        }
        Ok(other) => {
            eprintln!("[capsem-agent] expected BootConfig, got {other:?}, continuing with defaults");
            (0, vec![])
        }
        Err(e) => {
            eprintln!("[capsem-agent] failed to receive BootConfig: {e}, continuing with defaults");
            (0, vec![])
        }
    };

    // Step 4: Set system clock.
    if boot_epoch > 0 {
        set_system_clock(boot_epoch);
    }

    // Step 5: Open PTY pair and set initial size.
    let pty = openpty(None, None).expect("openpty failed");
    let master_fd = pty.master.as_raw_fd();
    let slave_fd = pty.slave.as_raw_fd();
    set_winsize(master_fd, 80, 24);

    // Step 6: Fork -- child becomes bash on the slave PTY.
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

            // Set environment from BootConfig.
            // Hardcoded defaults first (in case BootConfig is empty / old host).
            std::env::set_var("TERM", "xterm-256color");
            std::env::set_var("HOME", "/root");
            std::env::set_var("LANG", "C");
            // BootConfig env vars override defaults (last wins).
            for (key, value) in &boot_env {
                std::env::set_var(key, value);
            }

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

            // Step 7: Send BootReady -- config applied, terminal ready.
            if let Err(e) = send_guest_msg(control_fd, &GuestToHost::BootReady) {
                eprintln!("[capsem-agent] failed to send BootReady: {e}");
            }

            // Enter bridge loop with already-connected fds.
            run_bridge(master_fd, child, terminal_fd, control_fd);
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

fn run_bridge(master_fd: RawFd, child_pid: Pid, terminal_fd: RawFd, control_fd: RawFd) {
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
                            if !forward.is_empty()
                                && write_all_fd(vsock_fd, &forward).is_err() {
                                    break;
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
                        } else if write_all_fd(vsock_fd, &buf[..n]).is_err() {
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
        match recv_host_msg(control_fd) {
            Ok(HostToGuest::Resize { cols, rows }) => {
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
            Ok(HostToGuest::Ping) => {
                if let Err(e) = send_guest_msg(control_fd, &GuestToHost::Pong) {
                    eprintln!("[capsem-agent] failed to send Pong: {e}");
                    break;
                }
            }
            Ok(HostToGuest::Exec { id, command }) => {
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
                    let _ = send_guest_msg(control_fd, &GuestToHost::ExecDone {
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
                        if let Err(e) = send_guest_msg(control_fd, &GuestToHost::ExecDone {
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
                eprintln!("[capsem-agent] unhandled control message: {msg:?}");
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
    use super::vsock_io::{AF_VSOCK, SockaddrVm};
    use std::io::Write;
    use std::os::unix::io::FromRawFd;

    fn make_pipe() -> (RawFd, RawFd) {
        let mut fds = [0 as RawFd; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    }

    // -----------------------------------------------------------------------
    // Wire format compatibility: new disjoint types over pipes
    // -----------------------------------------------------------------------

    #[test]
    fn agent_ready_roundtrip() {
        let (read_fd, write_fd) = make_pipe();
        let msg = GuestToHost::Ready { version: "0.3.0".to_string() };
        send_guest_msg(write_fd, &msg).unwrap();
        // Simulate host-side receive.
        let mut len_buf = [0u8; 4];
        read_exact_fd(read_fd, &mut len_buf).unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        read_exact_fd(read_fd, &mut payload).unwrap();
        let decoded: GuestToHost = capsem_proto::decode_guest_msg(&payload).unwrap();
        match decoded {
            GuestToHost::Ready { version } => assert_eq!(version, "0.3.0"),
            other => panic!("expected Ready, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn host_resize_decodable_by_agent() {
        let (read_fd, write_fd) = make_pipe();
        let msg = HostToGuest::Resize { cols: 200, rows: 50 };
        let frame = capsem_proto::encode_host_msg(&msg).unwrap();
        write_all_fd(write_fd, &frame).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        match decoded {
            HostToGuest::Resize { cols, rows } => {
                assert_eq!(cols, 200);
                assert_eq!(rows, 50);
            }
            other => panic!("expected Resize, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn boot_config_roundtrip_over_pipe() {
        let (read_fd, write_fd) = make_pipe();
        let msg = HostToGuest::BootConfig {
            epoch_secs: 1708800000,
            env_vars: vec![("TERM".into(), "xterm-256color".into())],
        };
        let frame = capsem_proto::encode_host_msg(&msg).unwrap();
        write_all_fd(write_fd, &frame).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        match decoded {
            HostToGuest::BootConfig { epoch_secs, env_vars } => {
                assert_eq!(epoch_secs, 1708800000);
                assert_eq!(env_vars.len(), 1);
            }
            other => panic!("expected BootConfig, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn boot_ready_roundtrip_over_pipe() {
        let (read_fd, write_fd) = make_pipe();
        send_guest_msg(write_fd, &GuestToHost::BootReady).unwrap();
        let mut len_buf = [0u8; 4];
        read_exact_fd(read_fd, &mut len_buf).unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        read_exact_fd(read_fd, &mut payload).unwrap();
        let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
        assert!(matches!(decoded, GuestToHost::BootReady));
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn send_recv_exec_over_pipe() {
        let (read_fd, write_fd) = make_pipe();
        let msg = HostToGuest::Exec { id: 99, command: "echo hi".to_string() };
        let frame = capsem_proto::encode_host_msg(&msg).unwrap();
        write_all_fd(write_fd, &frame).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        match decoded {
            HostToGuest::Exec { id, command } => {
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
        send_guest_msg(write_fd, &GuestToHost::ExecDone { id: 99, exit_code: 1 }).unwrap();
        let mut len_buf = [0u8; 4];
        read_exact_fd(read_fd, &mut len_buf).unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        read_exact_fd(read_fd, &mut payload).unwrap();
        let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
        match decoded {
            GuestToHost::ExecDone { id, exit_code } => {
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

        // Send host messages.
        let ping_frame = capsem_proto::encode_host_msg(&HostToGuest::Ping).unwrap();
        write_all_fd(write_fd, &ping_frame).unwrap();
        let resize_frame = capsem_proto::encode_host_msg(&HostToGuest::Resize { cols: 80, rows: 24 }).unwrap();
        write_all_fd(write_fd, &resize_frame).unwrap();

        assert!(matches!(recv_host_msg(read_fd).unwrap(), HostToGuest::Ping));
        match recv_host_msg(read_fd).unwrap() {
            HostToGuest::Resize { cols, rows } => {
                assert_eq!(cols, 80);
                assert_eq!(rows, 24);
            }
            other => panic!("expected Resize, got {other:?}"),
        }

        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn recv_rejects_oversized_frame() {
        let (read_fd, write_fd) = make_pipe();
        // Write a length prefix claiming > MAX_FRAME_SIZE.
        let len_bytes = ((MAX_FRAME_SIZE + 1) as u32).to_be_bytes();
        let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
        writer.write_all(&len_bytes).unwrap();
        std::mem::forget(writer);

        let result = recv_host_msg(read_fd);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn recv_eof_returns_error() {
        let (read_fd, write_fd) = make_pipe();
        unsafe { libc::close(write_fd); }
        let result = recv_host_msg(read_fd);
        assert!(result.is_err());
        unsafe { libc::close(read_fd); }
    }

    // -----------------------------------------------------------------------
    // Clock sync
    // -----------------------------------------------------------------------

    #[test]
    fn set_system_clock_no_crash() {
        // On non-root systems this will fail with EPERM, but must not crash.
        set_system_clock(1708800000);
    }

    // -----------------------------------------------------------------------
    // SockaddrVm struct layout
    // -----------------------------------------------------------------------

    #[test]
    fn sockaddr_vm_size_matches_kernel() {
        assert_eq!(
            std::mem::size_of::<SockaddrVm>(),
            16,
            "SockaddrVm must be 16 bytes to match kernel struct"
        );
    }

    #[test]
    fn sockaddr_vm_field_offsets() {
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
        assert_eq!(VSOCK_PORT_CONTROL, 5000);
        assert_eq!(VSOCK_PORT_TERMINAL, 5001);
    }

    #[test]
    fn host_cid_is_two() {
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

        set_winsize(master_fd, 1, 1);
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        unsafe { libc::ioctl(master_fd, libc::TIOCGWINSZ, &mut ws); }
        assert_eq!(ws.ws_col, 1);
        assert_eq!(ws.ws_row, 1);

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
        let (forward1, result1) = scan_and_strip_sentinel(
            &mut tail,
            b"output\x1b_CAPSEM_EX",
        );
        assert!(result1.is_none());
        assert!(!forward1.is_empty());

        let (forward2, result2) = scan_and_strip_sentinel(
            &mut tail,
            b"IT:42:0\x1b\\trailing",
        );
        assert_eq!(result2, Some((42, 0)));
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
