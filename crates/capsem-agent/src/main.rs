// capsem-pty-agent: Guest-side PTY-over-vsock bridge.
//
// Runs inside the Linux VM as a child of capsem-init. Creates a PTY pair,
// forks bash on the slave side, and bridges the master PTY with the host
// over two vsock connections:
//   - Port 5001: raw PTY I/O (terminal data)
//   - Port 5000: control messages (resize, heartbeat, boot config)

#[path = "vsock_io.rs"]
mod vsock_io;

use std::io::{self, Write as _};
use std::os::unix::io::{AsRawFd, RawFd};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use capsem_proto::{
    BootStage, GuestToHost, HostToGuest, MAX_FRAME_SIZE, decode_host_msg, encode_guest_msg,
    validate_env_key, validate_env_value, validate_file_path,
    MAX_BOOT_ENV_VARS, MAX_BOOT_FILES, MAX_BOOT_FILE_BYTES,
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
/// Boot log persisted so it can be inspected after boot (`cat /var/log/capsem-boot.log`).
const BOOT_LOG_PATH: &str = "/var/log/capsem-boot.log";

// ---------------------------------------------------------------------------
// Control message framing (using capsem-proto types)
// ---------------------------------------------------------------------------

fn send_guest_msg(fd: RawFd, msg: &GuestToHost) -> io::Result<()> {
    let frame = encode_guest_msg(msg)
        .map_err(io::Error::other)?;
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
// Boot log -- persists at /var/log/capsem-boot.log for post-boot diagnosis
// ---------------------------------------------------------------------------

fn open_boot_log() -> std::fs::File {
    // Ensure /var/log exists (may be tmpfs).
    let _ = std::fs::create_dir_all("/var/log");
    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(BOOT_LOG_PATH)
        .unwrap_or_else(|_| {
            // Fallback: /tmp is always writable.
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open("/tmp/capsem-boot.log")
                .expect("cannot open boot log")
        })
}

fn blog_line(log: &mut std::fs::File, msg: &str) {
    let _ = writeln!(log, "{msg}");
    eprintln!("[capsem-agent] {msg}");
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    eprintln!("[capsem-agent] starting (pid {})", process::id());

    // Open boot log (persists after boot for diagnosis).
    let mut blog = open_boot_log();
    blog_line(&mut blog, &format!(
        "capsem-agent {} starting (pid {})",
        env!("CARGO_PKG_VERSION"),
        process::id(),
    ));

    // Step 1: Connect to host vsock ports BEFORE PTY/fork.
    let terminal_fd = vsock_connect_retry(VSOCK_HOST_CID, VSOCK_PORT_TERMINAL, "terminal");
    let control_fd = vsock_connect_retry(VSOCK_HOST_CID, VSOCK_PORT_CONTROL, "control");
    blog_line(&mut blog, "vsock connected (terminal + control)");

    // Step 2: Send Ready.
    if let Err(e) = send_guest_msg(control_fd, &GuestToHost::Ready {
        version: env!("CARGO_PKG_VERSION").to_string(),
    }) {
        blog_line(&mut blog, &format!("FATAL: failed to send Ready: {e}"));
        eprintln!("[capsem-agent] failed to send Ready: {e}");
        process::exit(1);
    }
    blog_line(&mut blog, "sent Ready");

    // Step 3: Boot handshake -- receive BootConfig, then SetEnv/FileWrite/BootConfigDone.
    let mut boot_env: Vec<(String, String)> = Vec::new();
    let mut file_count: usize = 0;

    // 3a: Receive BootConfig (clock sync).
    match recv_host_msg(control_fd) {
        Ok(HostToGuest::BootConfig { epoch_secs }) => {
            eprintln!("[capsem-agent] received BootConfig (epoch={epoch_secs})");
            blog_line(&mut blog, &format!("BootConfig epoch={epoch_secs}"));
            if epoch_secs > 0 {
                set_system_clock(epoch_secs);
                blog_line(&mut blog, &format!("clock set to {epoch_secs}"));
            }
        }
        Ok(other) => {
            blog_line(&mut blog, &format!("expected BootConfig, got {other:?}"));
            eprintln!("[capsem-agent] expected BootConfig, got {other:?}, continuing with defaults");
        }
        Err(e) => {
            blog_line(&mut blog, &format!("BootConfig error: {e}"));
            eprintln!("[capsem-agent] failed to receive BootConfig: {e}, continuing with defaults");
        }
    };

    // 3b: Receive individual SetEnv, FileWrite, and BootConfigDone messages.
    // Defense-in-depth: validate everything independently of the host.
    let mut total_file_bytes: usize = 0;

    loop {
        match recv_host_msg(control_fd) {
            Ok(HostToGuest::SetEnv { key, value }) => {
                // Validate env key (defense-in-depth).
                if let Err(e) = validate_env_key(&key) {
                    blog_line(&mut blog, &format!("SetEnv rejected: {e}"));
                    eprintln!("[capsem-agent] rejecting env var: {e}");
                    continue;
                }
                if let Err(e) = validate_env_value(&value) {
                    blog_line(&mut blog, &format!("SetEnv {key} rejected: {e}"));
                    eprintln!("[capsem-agent] rejecting env var {key}: {e}");
                    continue;
                }
                if boot_env.len() >= MAX_BOOT_ENV_VARS {
                    blog_line(&mut blog, &format!("SetEnv {key}: env var cap reached"));
                    eprintln!("[capsem-agent] env var cap reached ({MAX_BOOT_ENV_VARS}), skipping {key}");
                    continue;
                }

                let preview = if value.len() > 40 {
                    format!("{}...", &value[..40])
                } else {
                    value.clone()
                };
                blog_line(&mut blog, &format!("SetEnv {key}={preview}"));
                eprintln!("[capsem-agent] SetEnv {key}");
                boot_env.push((key, value));
            }
            Ok(HostToGuest::FileWrite { path, data, mode }) => {
                // Validate file path (defense-in-depth).
                if let Err(e) = validate_file_path(&path) {
                    blog_line(&mut blog, &format!("FileWrite rejected: {e}"));
                    eprintln!("[capsem-agent] rejecting file write: {e}");
                    continue;
                }
                if file_count >= MAX_BOOT_FILES {
                    blog_line(&mut blog, &format!("FileWrite {path}: file cap reached"));
                    eprintln!("[capsem-agent] file cap reached ({MAX_BOOT_FILES}), skipping {path}");
                    continue;
                }
                if total_file_bytes + data.len() > MAX_BOOT_FILE_BYTES {
                    blog_line(&mut blog, &format!("FileWrite {path}: total bytes cap reached"));
                    eprintln!("[capsem-agent] file bytes cap reached ({MAX_BOOT_FILE_BYTES}), skipping {path}");
                    continue;
                }

                if let Some(parent) = std::path::Path::new(&path).parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        blog_line(&mut blog, &format!("FileWrite {path}: mkdir failed: {e}"));
                        eprintln!("[capsem-agent] failed to create dir {}: {e}", parent.display());
                        continue;
                    }
                }
                if let Err(e) = std::fs::write(&path, &data) {
                    blog_line(&mut blog, &format!("FileWrite {path}: write failed: {e}"));
                    eprintln!("[capsem-agent] failed to write {path}: {e}");
                    continue;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode));
                }
                total_file_bytes += data.len();
                file_count += 1;
                blog_line(&mut blog, &format!(
                    "FileWrite {path} ({} bytes, mode={mode:#o})",
                    data.len(),
                ));
                eprintln!("[capsem-agent] wrote {path} ({} bytes)", data.len());
            }
            Ok(HostToGuest::BootConfigDone) => {
                blog_line(&mut blog, &format!(
                    "BootConfigDone: {} env vars, {} files",
                    boot_env.len(),
                    file_count,
                ));
                eprintln!("[capsem-agent] boot config done ({} env vars, {} files)", boot_env.len(), file_count);
                break;
            }
            Ok(other) => {
                blog_line(&mut blog, &format!("unexpected boot message: {other:?}"));
                eprintln!("[capsem-agent] unexpected message during boot: {other:?}");
            }
            Err(e) => {
                blog_line(&mut blog, &format!("boot handshake error: {e}"));
                eprintln!("[capsem-agent] boot handshake error: {e}, proceeding with what we have");
                break;
            }
        }
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

            // Set environment from boot handshake.
            // Hardcoded defaults first (in case BootConfig is empty / old host).
            std::env::set_var("TERM", "xterm-256color");
            std::env::set_var("HOME", "/root");
            std::env::set_var("LANG", "C");
            // Boot env vars override defaults (last wins).
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
            blog_line(&mut blog, "sending BootReady, entering bridge loop");
            if let Err(e) = send_guest_msg(control_fd, &GuestToHost::BootReady) {
                eprintln!("[capsem-agent] failed to send BootReady: {e}");
            }

            // Send boot timing from capsem-init (JSONL written to /run).
            let stages = parse_boot_timing(BOOT_TIMING_PATH);
            if !stages.is_empty() {
                let _ = send_guest_msg(control_fd, &GuestToHost::BootTiming { stages });
            }

            drop(blog); // flush and close boot log before bridge loop

            // Enter bridge loop with already-connected fds.
            run_bridge(master_fd, child, terminal_fd, control_fd);
        }
        Err(e) => {
            eprintln!("[capsem-agent] fork failed: {e}");
            process::exit(1);
        }
    }
}

/// Path to the boot timing JSONL file written by capsem-init.
const BOOT_TIMING_PATH: &str = "/run/capsem-boot-timing";

/// Parse boot timing JSONL file. Each line: {"name":"...","duration_ms":...}
/// Rejects entries with non-alphanumeric names (defense against injection).
fn parse_boot_timing(path: &str) -> Vec<BootStage> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| serde_json::from_str::<BootStage>(line).ok())
        .filter(|s| {
            s.name.len() <= 64
                && !s.name.is_empty()
                && s.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && s.duration_ms <= 600_000
        })
        .take(32)
        .collect()
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

    // No sentinel prefix found. Determine how many bytes to keep at the end
    // to avoid splitting a sentinel across chunks.
    // OPTIMIZATION: Only keep bytes if the tail ends with a prefix of the sentinel.
    // This eliminates the 18-byte lag for normal interactive output.
    let mut keep = 0;
    for i in (1..SENTINEL_PREFIX.len()).rev() {
        if tail.ends_with(&SENTINEL_PREFIX[..i]) {
            keep = i;
            break;
        }
    }

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

    // Spawn a dedicated thread for vsock -> Master PTY (stdin direction)
    // This prevents deadlocks when both master_fd and vsock_fd buffers are full.
    let master_fd_clone = master_fd;
    let vsock_fd_clone = vsock_fd;
    std::thread::spawn(move || {
        let mut local_buf = [0u8; 8192];
        loop {
            let mut poll_fds = [
                PollFd::new(unsafe { std::os::unix::io::BorrowedFd::borrow_raw(vsock_fd_clone) }, PollFlags::POLLIN),
            ];

            match poll(&mut poll_fds, PollTimeout::from(1000u16)) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(_) => break,
            }

            if let Some(revents) = poll_fds[0].revents() {
                if revents.contains(PollFlags::POLLIN) {
                    match nix::unistd::read(vsock_fd_clone, &mut local_buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if write_all_fd(master_fd_clone, &local_buf[..n]).is_err() {
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
    });

    loop {
        let mut poll_fds = [
            PollFd::new(unsafe { std::os::unix::io::BorrowedFd::borrow_raw(master_fd) }, PollFlags::POLLIN),
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
        };
        let frame = capsem_proto::encode_host_msg(&msg).unwrap();
        write_all_fd(write_fd, &frame).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        match decoded {
            HostToGuest::BootConfig { epoch_secs } => {
                assert_eq!(epoch_secs, 1708800000);
            }
            other => panic!("expected BootConfig, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn boot_handshake_set_env_roundtrip() {
        let (read_fd, write_fd) = make_pipe();
        let msg = HostToGuest::SetEnv {
            key: "TERM".into(),
            value: "xterm-256color".into(),
        };
        let frame = capsem_proto::encode_host_msg(&msg).unwrap();
        write_all_fd(write_fd, &frame).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        match decoded {
            HostToGuest::SetEnv { key, value } => {
                assert_eq!(key, "TERM");
                assert_eq!(value, "xterm-256color");
            }
            other => panic!("expected SetEnv, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn boot_handshake_file_write_roundtrip() {
        let (read_fd, write_fd) = make_pipe();
        let msg = HostToGuest::FileWrite {
            path: "/root/.gemini/settings.json".into(),
            data: b"{}".to_vec(),
            mode: 0o644,
        };
        let frame = capsem_proto::encode_host_msg(&msg).unwrap();
        write_all_fd(write_fd, &frame).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        match decoded {
            HostToGuest::FileWrite { path, data, mode } => {
                assert_eq!(path, "/root/.gemini/settings.json");
                assert_eq!(data, b"{}");
                assert_eq!(mode, 0o644);
            }
            other => panic!("expected FileWrite, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn boot_config_done_roundtrip() {
        let (read_fd, write_fd) = make_pipe();
        let msg = HostToGuest::BootConfigDone;
        let frame = capsem_proto::encode_host_msg(&msg).unwrap();
        write_all_fd(write_fd, &frame).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        assert!(matches!(decoded, HostToGuest::BootConfigDone));
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
        let len_bytes = (MAX_FRAME_SIZE + 1).to_be_bytes();
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

    #[test]
    fn bridge_loop_concurrency_no_deadlock() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::AsRawFd;
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        use std::sync::mpsc;
        use std::sync::Mutex;

        let (mut master_host, master_guest) = UnixStream::pair().unwrap();
        let (mut vsock_host, vsock_guest) = UnixStream::pair().unwrap();

        let exec_state = Arc::new(ExecState {
            active: AtomicBool::new(false),
            current_id: Mutex::new(None),
        });
        let (exec_done_tx, _exec_done_rx) = mpsc::channel();

        let master_fd = master_guest.as_raw_fd();
        let vsock_fd = vsock_guest.as_raw_fd();

        let exec_state_clone = Arc::clone(&exec_state);
        let _bridge_thread = std::thread::spawn(move || {
            bridge_loop(master_fd, vsock_fd, &exec_state_clone, exec_done_tx);
        });

        // 1MB ensures internal buffers (usually 8KB) fill up, triggering backpressure
        // and testing deadlock immunity.
        let data_size = 1024 * 1024;
        let test_data = vec![0x42u8; data_size];

        let mut master_host_read = master_host.try_clone().unwrap();
        let mut vsock_host_read = vsock_host.try_clone().unwrap();

        let t_master_write = std::thread::spawn({
            let test_data = test_data.clone();
            move || {
                std::io::Write::write_all(&mut master_host, &test_data).unwrap();
            }
        });

        let t_vsock_write = std::thread::spawn({
            let test_data = test_data.clone();
            move || {
                std::io::Write::write_all(&mut vsock_host, &test_data).unwrap();
            }
        });

        let t_master_read = std::thread::spawn(move || {
            let mut buf = vec![0u8; data_size];
            std::io::Read::read_exact(&mut master_host_read, &mut buf).unwrap();
            buf
        });

        let t_vsock_read = std::thread::spawn(move || {
            let mut buf = vec![0u8; data_size];
            std::io::Read::read_exact(&mut vsock_host_read, &mut buf).unwrap();
            buf
        });

        t_master_write.join().unwrap();
        t_vsock_write.join().unwrap();

        let master_out = t_master_read.join().unwrap();
        let vsock_out = t_vsock_read.join().unwrap();

        assert_eq!(master_out, test_data);
        assert_eq!(vsock_out, test_data);
    }

    // -----------------------------------------------------------------------
    // Boot timing parser
    // -----------------------------------------------------------------------

    #[test]
    fn parse_boot_timing_valid_jsonl() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-boot-timing");
        std::fs::write(
            &path,
            "{\"name\":\"squashfs\",\"duration_ms\":50}\n{\"name\":\"network\",\"duration_ms\":120}\n",
        ).unwrap();
        let result = parse_boot_timing(path.to_str().unwrap());
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "squashfs");
        assert_eq!(result[0].duration_ms, 50);
        assert_eq!(result[1].name, "network");
        assert_eq!(result[1].duration_ms, 120);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_boot_timing_missing_file() {
        let result = parse_boot_timing("/nonexistent/capsem-boot-timing");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_boot_timing_skips_malformed_lines() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-boot-timing-bad");
        std::fs::write(
            &path,
            "{\"name\":\"good\",\"duration_ms\":100}\nnot json\n{\"name\":\"also_good\",\"duration_ms\":200}\n",
        ).unwrap();
        let result = parse_boot_timing(path.to_str().unwrap());
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "good");
        assert_eq!(result[1].name, "also_good");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_boot_timing_rejects_xss_names() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-boot-timing-xss");
        std::fs::write(&path, concat!(
            "{\"name\":\"<script>alert(1)</script>\",\"duration_ms\":10}\n",
            "{\"name\":\"normal\",\"duration_ms\":20}\n",
            "{\"name\":\"a]};fetch('http://evil')\",\"duration_ms\":30}\n",
            "{\"name\":\"\",\"duration_ms\":40}\n",
            "{\"name\":\"has spaces\",\"duration_ms\":50}\n",
            "{\"name\":\"path/../traversal\",\"duration_ms\":60}\n",
        )).unwrap();
        let result = parse_boot_timing(path.to_str().unwrap());
        assert_eq!(result.len(), 1, "only 'normal' should survive: {result:?}");
        assert_eq!(result[0].name, "normal");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_boot_timing_rejects_huge_duration() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-boot-timing-huge");
        std::fs::write(&path, concat!(
            "{\"name\":\"ok\",\"duration_ms\":1000}\n",
            "{\"name\":\"huge\",\"duration_ms\":999999999}\n",
        )).unwrap();
        let result = parse_boot_timing(path.to_str().unwrap());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "ok");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_boot_timing_caps_at_32_entries() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-boot-timing-cap");
        let lines: String = (0..50)
            .map(|i| format!("{{\"name\":\"stage{i}\",\"duration_ms\":{i}}}\n"))
            .collect();
        std::fs::write(&path, &lines).unwrap();
        let result = parse_boot_timing(path.to_str().unwrap());
        assert_eq!(result.len(), 32);
        std::fs::remove_file(&path).ok();
    }
}
