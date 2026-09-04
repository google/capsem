// capsem-pty-agent: Guest-side PTY-over-vsock bridge.
//
// Runs inside the Linux VM as a child of capsem-init. Creates a PTY pair,
// forks bash on the slave side, and bridges the master PTY with the host
// over three vsock connections:
//   - Port 5001: raw PTY I/O (terminal data)
//   - Port 5000: control messages (resize, heartbeat, boot config)
//   - Port 5005: exec output (direct child process stdout, on demand)

mod audit;
mod control_writer;
mod shutdown;
mod terminal_bridge;
use control_writer::{control_writer_loop, heartbeat_loop, BridgeShared, CtrlSender, PendingResponses};
#[cfg(test)]
use control_writer::{frame_or_drop, SharedCtrlReceiver};
#[cfg(test)]
use shutdown::HostShutdown;
use terminal_bridge::bridge_loop;
#[path = "vsock_io.rs"]
mod vsock_io;

use std::io::{self, Read as _, Write as _};
use std::os::unix::io::{AsRawFd, RawFd};
use std::process;
use std::thread;

use capsem_proto::{
    decode_host_msg, encode_guest_msg, validate_env_key, validate_env_value, validate_file_path,
    validate_file_path_safe, BootStage, GuestToHost, HostToGuest, MAX_BOOT_ENV_VARS, MAX_BOOT_FILES,
    MAX_BOOT_FILE_BYTES, MAX_FRAME_SIZE, SHUTDOWN_GRACE_SECS, VSOCK_PORT_CONTROL, VSOCK_PORT_EXEC, VSOCK_PORT_TERMINAL,
};
use nix::libc;
use nix::pty::openpty;
use nix::sys::signal::{signal, SigHandler, Signal};
use nix::unistd::{close, dup2, execvp, fork, setsid, ForkResult, Pid};

use audit::audit_reader_loop;
use vsock_io::{read_exact_fd, vsock_connect, vsock_connect_retry, write_all_fd, VSOCK_HOST_CID};
/// Boot log persisted on the host-visible workspace mount for post-boot diagnosis.
const BOOT_LOG_PATH: &str = "/root/.capsem-agent-boot.log";
/// Fallback boot log inside the guest overlay when /root is not mounted yet.
const FALLBACK_BOOT_LOG_PATH: &str = "/var/log/capsem-boot.log";
/// Reconnect timeout before giving up (seconds).
const RECONNECT_TIMEOUT_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// Control message framing (using capsem-proto types)
// ---------------------------------------------------------------------------

fn send_guest_msg(fd: RawFd, msg: &GuestToHost) -> io::Result<()> {
    let frame = encode_guest_msg(msg).map_err(io::Error::other)?;
    write_all_fd(fd, &frame)?;
    Ok(())
}

/// Returns `Some(id)` for `GuestToHost` variants the agent retains in
/// its symmetric pending_responses map and replays on every fresh
/// control conn. Mirrors `vsock.rs::ackable_response_id` on the host
/// side; both ends must agree on the set or AckReply / replay drift.
pub(crate) fn ackable_response_id(msg: &GuestToHost) -> Option<u64> {
    match msg {
        GuestToHost::ExecDone { id, .. }
        | GuestToHost::FileOpDone { id }
        | GuestToHost::FileContent { id, .. }
        | GuestToHost::Error { id, .. } => Some(*id),
        _ => None,
    }
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
    decode_host_msg(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

const SYSTEM_FS_MOUNT: &str = "/dev/.capsem-system";

fn fsfreeze_command(mode: &'static str) -> std::process::Command {
    let mut command = std::process::Command::new("fsfreeze");
    command.args([mode, SYSTEM_FS_MOUNT]);
    command
}

fn set_system_filesystem_frozen(frozen: bool) -> io::Result<()> {
    let mode = if frozen { "-f" } else { "-u" };
    let status = fsfreeze_command(mode).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "fsfreeze {mode} {SYSTEM_FS_MOUNT} exited with {status}"
        )))
    }
}

fn freeze_system_filesystem() -> io::Result<()> {
    set_system_filesystem_frozen(true)
}

fn thaw_system_filesystem() -> io::Result<()> {
    set_system_filesystem_frozen(false)
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
    let _ = std::fs::create_dir_all("/root");
    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(BOOT_LOG_PATH)
        .unwrap_or_else(|_| {
            let _ = std::fs::create_dir_all("/var/log");
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(FALLBACK_BOOT_LOG_PATH)
                .unwrap_or_else(|_| {
                    // Fallback: /tmp is always writable.
                    std::fs::OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open("/tmp/capsem-boot.log")
                        .expect("cannot open boot log")
                })
        })
}

fn blog_line(log: &mut std::fs::File, msg: &str) {
    let _ = writeln!(log, "{msg}");
    eprintln!("[capsem-agent] {msg}");
}

/// W5: process-global boot traceparent received in BootConfig.
/// Stash so post-boot guest agent code can grep-correlate its log lines
/// with the host-side spans for the same operation.
static BOOT_TRACEPARENT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn set_boot_traceparent(tp: &str) {
    let _ = BOOT_TRACEPARENT.set(tp.to_string());
}

/// Lower 16 hex chars of the W3C trace_id (matches the `CAPSEM_TRACE_ID`
/// convention used elsewhere). Returns "" when no traceparent has been set.
/// The first 40 characters of an env value for the boot log.
///
/// Truncating by byte index panicked when a multibyte character straddled
/// byte 40. Values come from user settings and `--env`, and
/// `validate_env_value` only rejects NUL and oversize, so `"a" * 39 + "é"`
/// killed the agent before BootReady and the VM never became ready.
fn env_preview(value: &str) -> String {
    match value.char_indices().nth(40) {
        Some((cut, _)) => format!("{}...", &value[..cut]),
        None => value.to_string(),
    }
}

fn current_boot_trace_id() -> String {
    let Some(tp) = BOOT_TRACEPARENT.get() else {
        return String::new();
    };
    let mut parts = tp.split('-');
    let _v = parts.next();
    let trace_id = parts.next().unwrap_or("");
    if trace_id.len() < 16 {
        return String::new();
    }
    trace_id[trace_id.len() - 16..].to_string()
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    eprintln!("[capsem-agent] starting (pid {})", process::id());

    // Open boot log (persists after boot for diagnosis).
    let mut blog = open_boot_log();
    blog_line(
        &mut blog,
        &format!(
            "capsem-agent {} starting (pid {})",
            env!("CARGO_PKG_VERSION"),
            process::id(),
        ),
    );

    // Step 1: Connect to host vsock ports BEFORE PTY/fork.
    let terminal_fd = vsock_connect_retry(VSOCK_HOST_CID, VSOCK_PORT_TERMINAL, "terminal");
    let control_fd = vsock_connect_retry(VSOCK_HOST_CID, VSOCK_PORT_CONTROL, "control");
    blog_line(&mut blog, "vsock connected (terminal + control)");

    // Step 2: Send Ready.
    if let Err(e) = send_guest_msg(
        control_fd,
        &GuestToHost::Ready {
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    ) {
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
        Ok(HostToGuest::BootConfig {
            epoch_secs,
            traceparent,
        }) => {
            eprintln!("[capsem-agent] received BootConfig (epoch={epoch_secs})");
            blog_line(
                &mut blog,
                &format!("BootConfig epoch={epoch_secs} traceparent_len={}", traceparent.len()),
            );
            if epoch_secs > 0 {
                set_system_clock(epoch_secs);
                blog_line(&mut blog, &format!("clock set to {epoch_secs}"));
            }
            // W5: stash the traceparent into a process-global so subsequent
            // blog_line calls can include the lower 16-hex trace_id, lining
            // up guest log lines with host-side spans for the same boot.
            if !traceparent.is_empty() {
                set_boot_traceparent(&traceparent);
                blog_line(&mut blog, &format!("trace_id={}", current_boot_trace_id()));
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

                blog_line(&mut blog, &format!("SetEnv {key}={}", env_preview(&value)));
                eprintln!("[capsem-agent] SetEnv {key}");
                boot_env.push((key, value));
            }
            Ok(HostToGuest::FileWrite {
                id: _,
                path,
                data,
                mode,
            }) => {
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
                blog_line(
                    &mut blog,
                    &format!("FileWrite {path} ({} bytes, mode={mode:#o})", data.len(),),
                );
                eprintln!("[capsem-agent] wrote {path} ({} bytes)", data.len());
            }
            Ok(HostToGuest::FileRead { .. }) => {
                eprintln!("[capsem-agent] ignoring FileRead during boot");
            }
            Ok(HostToGuest::FileDelete { .. }) => {
                eprintln!("[capsem-agent] ignoring FileDelete during boot");
            }
            Ok(HostToGuest::BootConfigDone) => {
                blog_line(
                    &mut blog,
                    &format!("BootConfigDone: {} env vars, {} files", boot_env.len(), file_count,),
                );
                eprintln!(
                    "[capsem-agent] boot config done ({} env vars, {} files)",
                    boot_env.len(),
                    file_count
                );
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

    // Step 4b: Publish the default Python venv path without blocking boot.
    // capsem-init creates the venv in the background and touches a ready flag
    // when done. The shell environment should point at the stable /root/.venv
    // contract immediately, but BootReady must not wait for Python packaging
    // setup when the first command is often a simple readiness probe.
    const VENV_DIR: &str = "/root/.venv";
    const VENV_TARGET: &str = "/run/capsem-venv";
    const VENV_READY: &str = "/run/capsem-venv-ready";
    boot_env.push(("VIRTUAL_ENV".into(), VENV_DIR.into()));
    if let Some((_, path_val)) = boot_env.iter_mut().find(|(k, _)| k == "PATH") {
        *path_val = format!("{VENV_DIR}/bin:{path_val}");
    }
    blog_line(&mut blog, "venv path activated in boot_env");
    std::thread::spawn(move || {
        let venv_activate = std::path::Path::new(VENV_DIR).join("bin/activate");
        for _ in 0..30 {
            if std::path::Path::new(VENV_READY).exists() || venv_activate.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if !venv_activate.exists() {
            eprintln!("[capsem-agent] venv missing after init wait; creating fallback");
            let _ = std::fs::remove_file(VENV_TARGET);
            let _ = std::fs::remove_dir_all(VENV_TARGET);
            let _ = std::fs::remove_file(VENV_DIR);
            let _ = std::os::unix::fs::symlink(VENV_TARGET, VENV_DIR);
            let created = std::process::Command::new("uv")
                .args(["venv", "--system-site-packages", VENV_TARGET])
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
                || std::process::Command::new("python3")
                    .args(["-m", "venv", "--system-site-packages", VENV_TARGET])
                    .status()
                    .map(|status| status.success())
                    .unwrap_or(false);
            if created {
                let _ = std::fs::write(VENV_READY, b"");
            }
        }
    });

    // Step 4c: Set hostname from CAPSEM_VM_NAME if present.
    if let Some((_, name)) = boot_env.iter().find(|(k, _)| k == "CAPSEM_VM_NAME") {
        let c_name = std::ffi::CString::new(name.as_str()).unwrap_or_default();
        let ret = unsafe { libc::sethostname(c_name.as_ptr(), name.len() as _) };
        if ret == 0 {
            blog_line(&mut blog, &format!("hostname set to {name}"));
        } else {
            blog_line(
                &mut blog,
                &format!("WARNING: sethostname failed: {}", std::io::Error::last_os_error()),
            );
        }
    }

    // Step 5: Open PTY pair and set initial size.
    let pty = openpty(None, None).expect("openpty failed");
    let master_fd = pty.master.as_raw_fd();
    let slave_fd = pty.slave.as_raw_fd();
    set_winsize(master_fd, 80, 24);

    // Clone boot env for the parent process (child consumes the original).
    let boot_env_for_parent = boot_env.clone();

    // Step 6: Fork -- child becomes bash on the slave PTY.
    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            // Close master in child.
            drop(pty.master);
            // The shell must take the hangup that ends it at shutdown even if
            // whatever started this agent left SIGHUP ignored: an ignored
            // disposition survives exec, and bash would then only die at the
            // end of the grace period, killed.
            unsafe { signal(Signal::SIGHUP, SigHandler::SigDfl) }.ok();

            // Create a new session so the slave PTY becomes the controlling terminal.
            setsid().expect("setsid failed");

            // Set the slave as the controlling terminal.
            #[cfg(target_os = "macos")]
            let tiocsctty = libc::c_ulong::from(libc::TIOCSCTTY);
            #[cfg(not(target_os = "macos"))]
            let tiocsctty = libc::TIOCSCTTY;
            unsafe {
                libc::ioctl(slave_fd, tiocsctty, 0);
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

            drop(blog); // flush and close boot log before loop

            let mut is_first = true;
            let mut t_fd = terminal_fd;
            let mut c_fd = control_fd;

            // Per-VM-session dedup state for Exec. Lifted to outer scope
            // so it survives reconnects: the host's ack/replay bridge may
            // re-send Exec on a fresh control conn after the previous one was
            // torn, and that replay must not double-execute. Two maps:
            //   exec_inflight: ids whose run_exec is still running -- a
            //                  duplicate must skip (the original will
            //                  send ExecDone when it finishes).
            //   exec_done:     ids whose run_exec has finished, mapped
            //                  to (exit_code) -- a duplicate must
            //                  re-send ExecDone with the cached code so
            //                  the host's j_rx can resolve when the
            //                  original ExecDone was lost on a torn
            //                  return path.
            // File ops are intentionally NOT deduped -- write/read/delete
            // are idempotent, and re-acking lets the host recover from a
            // FileOpDone lost on return.
            let exec_inflight = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::<u64>::new()));
            let exec_done: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u64, i32>>> =
                std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
            // Symmetric guest-side replay buffer: every ackable
            // GuestToHost (ExecDone / FileOpDone / FileContent / Error)
            // the writer thread sends gets inserted here keyed by `id`.
            // On every fresh control conn after reconnect, run_bridge
            // replays the pending entries before resuming normal writes
            // -- protocol-level cover for Apple VZ's silent-drop
            // pattern on the guest->host return path. The host sends
            // `HostToGuest::AckReply { id }` on receipt; control_loop
            // removes the entry. Lifted to outer scope so the map
            // survives reconnects (the writer thread is per-run_bridge).
            let pending_responses: PendingResponses =
                std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
            let (ctrl_sender, ctrl_rx) = CtrlSender::new(std::sync::Arc::clone(&pending_responses));
            // The audit tailer owns its host connection and reconnects on its
            // own, so it is spawned once for the process, not per bridge.
            thread::spawn(audit_reader_loop);
            let bridge_shared = BridgeShared {
                exec_inflight: std::sync::Arc::clone(&exec_inflight),
                exec_done: std::sync::Arc::clone(&exec_done),
                ctrl_sender,
                ctrl_rx,
            };

            loop {
                if !is_first {
                    use capsem_proto::poll::{retry_with_backoff, RetryOpts};

                    let fds = retry_with_backoff(
                        &RetryOpts::new("reconnect", std::time::Duration::from_secs(RECONNECT_TIMEOUT_SECS)),
                        || {
                            let t = vsock_io::vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_TERMINAL).ok()?;
                            match vsock_io::vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_CONTROL) {
                                Ok(c) => Some((t, c)),
                                Err(_) => {
                                    unsafe {
                                        libc::close(t);
                                    }
                                    None
                                }
                            }
                        },
                    );

                    match fds {
                        Ok((new_t, new_c)) => {
                            t_fd = new_t;
                            c_fd = new_c;
                        }
                        Err(e) => {
                            eprintln!("[capsem-agent] reconnect failed: {e}");
                            let _ = nix::sys::signal::kill(child, Signal::SIGHUP);
                            process::exit(1);
                        }
                    }

                    eprintln!("[capsem-agent] reconnected successfully");
                    // KVM restores the guest at the exact point after the
                    // persistent ext4 upper was frozen. Thaw it before any
                    // workspace rebinding or abbreviated BootConfig writes.
                    if let Err(e) = thaw_system_filesystem() {
                        eprintln!("[capsem-agent] resume: failed to thaw system filesystem: {e}");
                        let _ = nix::sys::signal::kill(child, Signal::SIGHUP);
                        process::exit(1);
                    }
                    rebind_workspace_after_resume();
                    let _ = send_guest_msg(
                        c_fd,
                        &GuestToHost::Ready {
                            version: env!("CARGO_PKG_VERSION").to_string(),
                        },
                    );

                    // Drain abbreviated handshake, processing clock/timezone resync.
                    loop {
                        match recv_host_msg(c_fd) {
                            Ok(HostToGuest::BootConfigDone) => break,
                            Ok(HostToGuest::Shutdown) => {
                                let _ = nix::sys::signal::kill(child, Signal::SIGTERM);
                                process::exit(0);
                            }
                            Ok(HostToGuest::BootConfig {
                                epoch_secs,
                                traceparent,
                            }) => {
                                if !traceparent.is_empty() {
                                    set_boot_traceparent(&traceparent);
                                }
                                if epoch_secs > 0 {
                                    set_system_clock(epoch_secs);
                                    eprintln!("[capsem-agent] resume: clock resynced to {epoch_secs}");
                                }
                            }
                            Ok(HostToGuest::SetEnv { key, value }) => {
                                std::env::set_var(&key, &value);
                                eprintln!("[capsem-agent] resume: set {key}");
                            }
                            Ok(HostToGuest::FileWrite { path, data, mode, .. }) => {
                                if let Some(parent) = std::path::Path::new(&path).parent() {
                                    let _ = std::fs::create_dir_all(parent);
                                }
                                if let Err(e) = std::fs::write(&path, &data) {
                                    eprintln!("[capsem-agent] resume: failed to write {path}: {e}");
                                } else {
                                    #[cfg(unix)]
                                    {
                                        use std::os::unix::fs::PermissionsExt;
                                        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode));
                                    }
                                    eprintln!("[capsem-agent] resume: wrote {path}");
                                }
                            }
                            Ok(_) => {}
                            Err(_) => break, // vsock broke again
                        }
                    }
                }

                // Send BootReady
                if let Err(e) = send_guest_msg(c_fd, &GuestToHost::BootReady) {
                    eprintln!("[capsem-agent] failed to send BootReady: {e}");
                }

                // Send boot timing only on first boot
                if is_first {
                    let stages = parse_boot_timing(BOOT_TIMING_PATH);
                    if !stages.is_empty() {
                        let _ = send_guest_msg(c_fd, &GuestToHost::BootTiming { stages });
                    }
                    is_first = false;
                }

                // Enter bridge loop
                run_bridge(
                    master_fd,
                    child,
                    t_fd,
                    c_fd,
                    &boot_env_for_parent,
                    bridge_shared.clone(),
                );

                // Cleanup broken FDs
                unsafe {
                    libc::close(t_fd);
                    libc::close(c_fd);
                }
                if bridge_shared.ctrl_sender.shutdown.is_requested() {
                    eprintln!("[capsem-agent] host shutdown reported; not reconnecting");
                    process::exit(0);
                }
            }
        }
        Err(e) => {
            eprintln!("[capsem-agent] fork failed: {e}");
            process::exit(1);
        }
    }
}

/// Path to the boot timing JSONL file written by capsem-init.
const BOOT_TIMING_PATH: &str = "/run/capsem-boot-timing";

/// After resume, the VirtioFS mount capsem-init set up in its pre-chroot
/// namespace (host path: /mnt/shared) is connected to a dead virtiofsd from
/// the previous capsem-process. /root was bind-mounted from that share, so
/// reads/writes against /root return ENOENT or hang.
///
/// The agent runs inside a chroot where /mnt/shared means /newroot/mnt/shared
/// -- NOT init's mount point. So we create a fresh virtiofs mount inside the
/// chroot (same "capsem" tag, new connection to the new host's virtiofsd)
/// and rebind /root onto it. Lazy-unmount the stale /root and any stale
/// chroot-local /mnt/shared first. mkdir -p ensures the mount point exists
/// in the overlay upper even on first resume.
///
/// Best-effort: log and continue on every failure. A wedged virtiofs is
/// better than crashing the agent.
fn rebind_workspace_after_resume() {
    use std::process::Command;
    let run = |args: &[&str]| -> bool {
        match Command::new(args[0]).args(&args[1..]).status() {
            Ok(s) if s.success() => true,
            Ok(s) => {
                eprintln!("[capsem-agent] rebind: {} exited {s}", args.join(" "));
                false
            }
            Err(e) => {
                eprintln!("[capsem-agent] rebind: failed to spawn {}: {e}", args[0]);
                false
            }
        }
    };
    eprintln!("[capsem-agent] rebinding workspace after resume");
    let _ = run(&["umount", "-l", "/root"]);
    let _ = run(&["umount", "-l", "/mnt/shared"]);
    let _ = run(&["mkdir", "-p", "/mnt/shared"]);
    if !run(&["mount", "-t", "virtiofs", "capsem", "/mnt/shared"]) {
        eprintln!("[capsem-agent] rebind: virtiofs remount failed; /root will be stale");
        return;
    }
    // Warm the virtiofs: on first mount after VM restore, FUSE lookups are
    // lazy. A plain stat (GETATTR) on the workspace dir can succeed before
    // virtiofsd has populated its child-inode map, and a bind against that
    // half-ready subtree leaves /root ENOENT-ing every child file.
    // std::fs::read_dir forces a real READDIR round-trip -- once that
    // succeeds, virtiofsd has enumerated the directory and subsequent
    // LOOKUPs on children will resolve. If warming never completes we abort
    // rather than binding against an empty view: the HTTP read_file will
    // fail loudly instead of silently returning ENOENT on a real file.
    let workspace_src = std::path::Path::new("/mnt/shared/workspace");
    let mut warmed_attempts = 0;
    let mut warmed = false;
    for attempt in 1..=50 {
        if std::fs::read_dir(workspace_src)
            .ok()
            .and_then(|mut it| it.next())
            .is_some()
        {
            warmed = true;
            warmed_attempts = attempt;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if !warmed {
        eprintln!("[capsem-agent] rebind: /mnt/shared/workspace not enumerable after 1s; aborting (no /root bind)");
        return;
    }
    eprintln!("[capsem-agent] rebind: virtiofs warmed after {warmed_attempts} attempts");
    let _ = run(&["mkdir", "-p", "/root"]);
    if !run(&["mount", "--bind", "/mnt/shared/workspace", "/root"]) {
        eprintln!("[capsem-agent] rebind: /root bind-mount failed");
    } else {
        eprintln!("[capsem-agent] rebind: /root reconnected to host workspace");
    }
}

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

fn run_bridge(
    master_fd: RawFd,
    child_pid: Pid,
    terminal_fd: RawFd,
    control_fd: RawFd,
    boot_env: &[(String, String)],
    shared: BridgeShared,
) {
    let BridgeShared {
        exec_inflight,
        exec_done,
        ctrl_sender,
        ctrl_rx,
    } = shared;
    // All control channel writes go through the shared channel and this
    // connection's writer thread. The exec background threads and
    // control_loop both write to control_fd; concurrent writes would corrupt
    // protocol framing. `alive` is this connection's lease on the shared
    // receiver: cleared by the writer on a failed write and below when the
    // bridge exits, so the next connection's writer can take over.
    let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let writer = {
        let rx = std::sync::Arc::clone(&ctrl_rx);
        let sender = ctrl_sender.clone();
        let alive = std::sync::Arc::clone(&alive);
        thread::spawn(move || control_writer_loop(&rx, &sender, control_fd, terminal_fd, &alive))
    };

    // Heartbeat. Without periodic probes the connection is invisible until
    // the next genuine traffic, which can be hours. After a suspend/resume
    // the host process is gone; the first failed write trips the writer's
    // shutdown path and triggers reconnect within ~3s.
    let heartbeat = {
        let hb = ctrl_sender.clone();
        let alive = std::sync::Arc::clone(&alive);
        thread::spawn(move || heartbeat_loop(&hb, &alive))
    };

    // Spawn control channel handler in a background thread.
    let boot_env_owned = boot_env.to_vec();
    let pending_for_ctrl = std::sync::Arc::clone(&ctrl_sender.pending);
    let host_shutdown = std::sync::Arc::clone(&ctrl_sender.shutdown);
    let ctrl_tx = ctrl_sender;
    let inflight_for_ctrl = exec_inflight;
    let done_for_ctrl = exec_done;
    let control = thread::spawn(move || {
        control_loop(
            control_fd,
            master_fd,
            child_pid,
            &boot_env_owned,
            ctrl_tx,
            inflight_for_ctrl,
            done_for_ctrl,
            pending_for_ctrl,
        );
    });

    // Main I/O bridge: master PTY <-> vsock terminal port.
    bridge_loop(master_fd, terminal_fd, &host_shutdown);

    // This connection is over. Release the shared receiver, wake the control
    // reader out of its blocking read, and wait for every thread that holds
    // these fd numbers before the caller closes them: the next connection
    // gets the same numbers back, and a thread still using the old ones
    // would read the new control stream alongside the new reader or write a
    // frame into it between the new writer's frames.
    alive.store(false, std::sync::atomic::Ordering::SeqCst);
    unsafe {
        libc::shutdown(control_fd, libc::SHUT_RDWR);
        libc::shutdown(terminal_fd, libc::SHUT_RDWR);
    }
    for (name, handle) in [("writer", writer), ("heartbeat", heartbeat), ("control", control)] {
        if handle.join().is_err() {
            eprintln!("[capsem-agent] {name} thread panicked");
        }
    }

    // If bridge exits, we just return. The reconnect loop will handle re-establishing vsock.
    // If it was a genuine Shutdown, control_loop already killed the child, and the process will eventually exit.
    eprintln!("[capsem-agent] bridge exited");
}

/// Maximum vsock_connect attempts when the host returns ECONNRESET, e.g.
/// briefly after `restoreMachineStateFromURL` while the kernel-side
/// accept queue is still settling. 5 attempts × ECONNRESET_BACKOFF_MS
/// keeps the transient retry short without hiding real connect failures.
const ECONNRESET_MAX_ATTEMPTS: usize = 5;
const ECONNRESET_BACKOFF_MS: u64 = 20;

/// Connect via the supplied closure, retrying on ECONNRESET only.
/// All other error kinds bail immediately so we don't paper over real
/// misconfiguration (refused, address-family-unsupported, etc.).
///
/// Bug C: post-`restoreState` the agent's `vsock_connect` to host port
/// 5005 (EXEC) can transiently see ECONNRESET while the kernel-side
/// accept queue is still attaching to the freshly-registered VZ
/// listener. A single-shot connect failed -> run_exec returned 126 ->
/// `exec_done` cached the bad code -> every host retry/replay was
/// poisoned. The retry isolates this transient transport state.
fn vsock_connect_with_econnreset_retry<F>(mut connect_fn: F) -> io::Result<RawFd>
where
    F: FnMut() -> io::Result<RawFd>,
{
    let mut last_err = None;
    for attempt in 1..=ECONNRESET_MAX_ATTEMPTS {
        match connect_fn() {
            Ok(fd) => return Ok(fd),
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {
                last_err = Some(e);
                if attempt < ECONNRESET_MAX_ATTEMPTS {
                    std::thread::sleep(std::time::Duration::from_millis(ECONNRESET_BACKOFF_MS));
                }
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::from(io::ErrorKind::ConnectionReset)))
}

/// Outcome of a `run_exec` call. Distinguishes a real child exit
/// (cache it for dedup-replay on host duplicate Exec delivery) from a
/// transport failure that never reached the child (do NOT cache --
/// the next host replay deserves a fresh attempt against a possibly
/// recovered transport).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecOutcome {
    /// Child process ran to completion with `i32` exit code.
    Done(i32),
    /// vsock_connect to the host EXEC port exhausted retries; ExecStarted
    /// or any subsequent step never landed. The host still gets an
    /// ExecDone {exit_code: 126} (so the caller sees a result), but the
    /// agent does not poison `exec_done` with this transient.
    TransportFailed,
}

impl ExecOutcome {
    fn should_cache(&self) -> bool {
        matches!(self, ExecOutcome::Done(_))
    }

    fn exit_code(&self) -> i32 {
        match self {
            ExecOutcome::Done(code) => *code,
            ExecOutcome::TransportFailed => 126,
        }
    }
}

/// Execute a command as a direct child process, streaming output over vsock:5005.
///
/// Runs in a background thread so control_loop remains responsive to heartbeats.
/// Output flows as raw bytes on a dedicated exec vsock connection. The exit code
/// is sent as ExecDone via the serialized control write channel.
fn run_exec(ctrl_tx: &CtrlSender, id: u64, command: &str, boot_env: &[(String, String)]) -> ExecOutcome {
    // Connect to host exec port. Retry on ECONNRESET only -- post-restore
    // VZ transient (Bug C). Other errors bail immediately.
    let exec_fd = match vsock_connect_with_econnreset_retry(|| vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_EXEC)) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("[capsem-agent] exec[{id}] vsock connect failed: {e}");
            let _ = ctrl_tx.send(GuestToHost::ExecDone { id, exit_code: 126 });
            return ExecOutcome::TransportFailed;
        }
    };

    ExecOutcome::Done(run_exec_on_fds(exec_fd, ctrl_tx, id, command, boot_env))
}

/// Inner exec implementation that takes pre-connected fds (testable without vsock).
/// `ctrl_tx` serializes writes to the control channel (prevents frame corruption
/// from concurrent writers). `exec_fd` is consumed: closed on all exit paths.
fn run_exec_on_fds(exec_fd: RawFd, ctrl_tx: &CtrlSender, id: u64, command: &str, boot_env: &[(String, String)]) -> i32 {
    // RAII guard to ensure exec_fd is closed on all paths.
    struct FdGuard(RawFd);
    impl Drop for FdGuard {
        fn drop(&mut self) {
            unsafe {
                libc::close(self.0);
            }
        }
    }
    let _exec_guard = FdGuard(exec_fd);

    // Send ExecStarted handshake so host knows which exec ID this connection belongs to.
    if let Err(e) = send_guest_msg(exec_fd, &GuestToHost::ExecStarted { id }) {
        eprintln!("[capsem-agent] exec[{id}] handshake failed: {e}");
        let _ = ctrl_tx.send(GuestToHost::ExecDone { id, exit_code: 126 });
        return 126;
    }

    // Spawn child process with piped stdout and stderr.
    let cwd = default_exec_cwd();
    let mut child = match std::process::Command::new("bash")
        .arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(cwd)
        .envs(boot_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[capsem-agent] exec[{id}] spawn failed: {e}");
            let _ = ctrl_tx.send(GuestToHost::ExecDone { id, exit_code: 126 });
            return 126;
        }
    };

    // Forward child stdout and stderr to exec vsock fd as a merged stream.
    // The host reads all exec output as opaque bytes (no stdout/stderr separation),
    // so interleaving between the two is acceptable -- same as `docker exec` or `2>&1`.
    // Stderr is forwarded from a background thread; stdout is forwarded inline.
    let stderr_thread = child.stderr.take().map(|mut stderr| {
        let efd = exec_fd;
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match stderr.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let _ = write_all_fd(efd, &buf[..n]);
                    }
                }
            }
        })
    });

    if let Some(mut stdout) = child.stdout.take() {
        let mut buf = [0u8; 8192];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if write_all_fd(exec_fd, &buf[..n]).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }

    if let Some(t) = stderr_thread {
        let _ = t.join();
    }

    // Wait for child to exit and get exit code.
    let exit_code = match child.wait() {
        Ok(status) => status.code().unwrap_or(1),
        Err(_) => 126,
    };

    // exec_fd closed by _exec_guard drop (signals EOF to host).
    drop(_exec_guard);

    // Send ExecDone via serialized control write channel.
    eprintln!("[capsem-agent] exec[{id}] done: exit_code={exit_code}");
    let _ = ctrl_tx.send(GuestToHost::ExecDone { id, exit_code });
    exit_code
}

fn default_exec_cwd() -> &'static str {
    if unsafe { libc::geteuid() } == 0 && std::path::Path::new("/root").is_dir() {
        "/root"
    } else {
        "/"
    }
}

/// Guest workspace root (VirtioFS mount point).
const GUEST_WORKSPACE_ROOT: &str = "/root";

// ---------------------------------------------------------------------------
// Symlink-safe file I/O (O_NOFOLLOW on final component)
// ---------------------------------------------------------------------------

/// Write a file, refusing to follow symlinks on the final path component.
/// Returns ELOOP if the target is a symlink.
fn write_nofollow(path: &str, data: &[u8], mode: u32) -> io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    // Create parent directories if they don't exist.
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.write_all(data)?;
    let _ = file.set_permissions(std::fs::Permissions::from_mode(mode));
    // On VirtioFS, close alone triggers FUSE_FLUSH which virtiofsd is free
    // to no-op -- the write stays in Apple VZ's in-process virtiofsd and
    // only reaches the host backing store opportunistically. A
    // capsem_suspend immediately after write_file then tears down VZ
    // before the data lands on host, and the resumed VM (with a fresh
    // virtiofsd) sees ENOENT. FUSE_FSYNC is a core FUSE opcode virtiofsd
    // must honor, so sync_all gives us a durability contract: when
    // write_file returns, the data is visible via the host filesystem.
    file.sync_all()?;
    Ok(())
}

/// Read a file, refusing to follow symlinks on the final path component and
/// refusing more than `max_bytes`. Returns ELOOP if the target is a symlink
/// and `FileTooLarge` past the cap.
///
/// The cap bounds memory; whether the reply fits one control frame is decided
/// by encoding it. A `FileContent` over `MAX_FRAME_SIZE` was never decoded by
/// the host, so never acked, so replayed on every reconnect: one read of a
/// 3 MiB file wedged the control channel for the life of the VM.
fn read_nofollow(path: &str, max_bytes: usize) -> io::Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let mut data = Vec::new();
    file.take(max_bytes.saturating_add(1) as u64).read_to_end(&mut data)?;
    if data.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            format!("file exceeds the {max_bytes}-byte control frame budget"),
        ));
    }
    Ok(data)
}

/// Delete a file only if it is not itself a symlink.
fn delete_nofollow(path: &str) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to delete symlink",
        ));
    }
    std::fs::remove_file(path)
}

#[allow(clippy::too_many_arguments)]
fn control_loop(
    control_fd: RawFd,
    master_fd: RawFd,
    child_pid: Pid,
    boot_env: &[(String, String)],
    ctrl_tx: CtrlSender,
    exec_inflight: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<u64>>>,
    exec_done: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u64, i32>>>,
    pending_responses: PendingResponses,
) {
    loop {
        match recv_host_msg(control_fd) {
            Ok(HostToGuest::AckReply { id }) => {
                // Host received the corresponding ackable response;
                // drop it from the replay buffer so the next rekey
                // does not re-send it. No-op if already removed (e.g.
                // duplicate AckReply from a replayed response that
                // actually did land twice).
                pending_responses.lock().unwrap().remove(&id);
            }
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
            Ok(HostToGuest::Ping { epoch_secs }) => {
                if epoch_secs > 0 {
                    set_system_clock(epoch_secs);
                }
                if ctrl_tx.send(GuestToHost::Pong).is_err() {
                    eprintln!("[capsem-agent] control write channel closed");
                    break;
                }
            }
            Ok(HostToGuest::Shutdown) => {
                eprintln!("[capsem-agent] received Shutdown from host");
                // Flag first: the bridge must not end the connection when
                // the shell exits, or the report below is lost with it.
                ctrl_tx.shutdown.request();
                let end = shutdown::end_terminal_shell(child_pid, std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS));
                eprintln!("[capsem-agent] shutdown: {end}");
                // Tell the host it can stop the VM now rather than at the
                // end of its own timer, and wait for the writer to confirm
                // the bytes went out. A miss means the host is on its timer.
                if ctrl_tx.send(GuestToHost::ShutdownComplete).is_err()
                    || !ctrl_tx.shutdown.wait_reported(shutdown::REPORT_WRITE_WAIT)
                {
                    eprintln!("[capsem-agent] shutdown report not delivered; host will time out");
                }
                break;
            }
            Ok(HostToGuest::Exec { id, command }) => {
                // Ack immediately on receipt -- before any processing or
                // dedup -- so the host bridge can clear this id from its
                // pending-ack map and stop re-replaying it on rekey.
                // The host writes Exec into the pending map *before*
                // sending; if our Ack is itself lost, the bridge
                // re-sends Exec on the next conn, we re-ack: idempotent.
                if ctrl_tx.send(GuestToHost::Ack { id }).is_err() {
                    break;
                }
                // Three states for `id`:
                //   - Done (in exec_done):    cached exit_code; replay
                //                             ExecDone so the host's
                //                             j_rx resolves even when
                //                             the original was lost on
                //                             return.
                //   - In-flight (in_inflight): original is still
                //                              running; ignore the
                //                              retry, the original
                //                              will send ExecDone.
                //   - Fresh:                   record as inflight, run.
                let cached = {
                    let done = exec_done.lock().unwrap();
                    done.get(&id).copied()
                };
                if let Some(exit_code) = cached {
                    eprintln!(
                        "[capsem-agent] exec[{id}] duplicate (already done, exit={exit_code}); replaying ExecDone"
                    );
                    if ctrl_tx.send(GuestToHost::ExecDone { id, exit_code }).is_err() {
                        break;
                    }
                    continue;
                }
                let inserted = exec_inflight.lock().unwrap().insert(id);
                if !inserted {
                    eprintln!("[capsem-agent] exec[{id}] duplicate (still inflight); ignoring");
                    continue;
                }
                eprintln!("[capsem-agent] exec[{id}]: {command}");
                let boot_env = boot_env.to_vec();
                let tx = ctrl_tx.clone();
                let inflight = std::sync::Arc::clone(&exec_inflight);
                let done = std::sync::Arc::clone(&exec_done);
                thread::spawn(move || {
                    let outcome = run_exec(&tx, id, &command, &boot_env);
                    // Record completion *before* dropping inflight so a
                    // host replay that arrives between the two
                    // unlocks cannot miss both maps. Only cache real
                    // child exits -- transport failures (vsock connect
                    // exhausted retries) are transient; caching 126
                    // would poison every subsequent replay
                    // even after the transport recovered (Bug C).
                    if outcome.should_cache() {
                        let mut d = done.lock().unwrap();
                        if d.len() >= 4096 {
                            d.clear();
                        }
                        d.insert(id, outcome.exit_code());
                    }
                    inflight.lock().unwrap().remove(&id);
                });
            }
            Ok(HostToGuest::FileWrite { id, path, data, mode }) => {
                // Ack on receipt so the host bridge clears the
                // pending-ack entry. No dedup: write_nofollow over the
                // same path with the same bytes is idempotent, and
                // re-acking lets the host recover from a lost FileOpDone.
                if ctrl_tx.send(GuestToHost::Ack { id }).is_err() {
                    break;
                }
                eprintln!("[capsem-agent] FileWrite {path} ({} bytes)", data.len());
                let ws = std::path::Path::new(GUEST_WORKSPACE_ROOT);
                let msg = if let Err(e) = validate_file_path_safe(&path, ws) {
                    GuestToHost::Error {
                        id,
                        message: format!("FileWrite rejected: {e}"),
                    }
                } else if let Err(e) = write_nofollow(&path, &data, mode) {
                    GuestToHost::Error {
                        id,
                        message: format!("failed to write {path}: {e}"),
                    }
                } else {
                    GuestToHost::FileOpDone { id }
                };
                if ctrl_tx.send(msg).is_err() {
                    break;
                }
            }
            Ok(HostToGuest::FileRead { id, path }) => {
                // Ack on receipt so the host bridge clears the
                // pending-ack entry. No dedup: re-reading is idempotent
                // and lets the host recover when FileContent was lost on
                // the return path.
                if ctrl_tx.send(GuestToHost::Ack { id }).is_err() {
                    break;
                }
                eprintln!("[capsem-agent] FileRead {path}");
                let ws = std::path::Path::new(GUEST_WORKSPACE_ROOT);
                let msg = if let Err(e) = validate_file_path_safe(&path, ws) {
                    GuestToHost::Error {
                        id,
                        message: format!("FileRead rejected: {e}"),
                    }
                } else {
                    match read_nofollow(&path, MAX_FRAME_SIZE as usize) {
                        Ok(data) => {
                            let reply = GuestToHost::FileContent {
                                id,
                                path: path.clone(),
                                data,
                            };
                            if capsem_proto::guest_msg_fits_frame(&reply) {
                                reply
                            } else {
                                GuestToHost::Error {
                                    id,
                                    message: format!(
                                        "failed to read {path}: file too large for one control frame ({MAX_FRAME_SIZE} bytes)"
                                    ),
                                }
                            }
                        }
                        Err(e) => GuestToHost::Error {
                            id,
                            message: format!("failed to read {path}: {e}"),
                        },
                    }
                };
                if ctrl_tx.send(msg).is_err() {
                    break;
                }
            }
            Ok(HostToGuest::FileDelete { id, path }) => {
                // Ack on receipt so the host bridge clears the
                // pending-ack entry. No dedup: a second delete of an
                // already-removed file returns ENOENT, which we coerce
                // to FileOpDone so a retried delete (whose original
                // FileOpDone was lost on return) ends up as a success
                // rather than an Error.
                if ctrl_tx.send(GuestToHost::Ack { id }).is_err() {
                    break;
                }
                eprintln!("[capsem-agent] FileDelete {path}");
                let ws = std::path::Path::new(GUEST_WORKSPACE_ROOT);
                let msg = if let Err(e) = validate_file_path_safe(&path, ws) {
                    GuestToHost::Error {
                        id,
                        message: format!("FileDelete rejected: {e}"),
                    }
                } else {
                    match delete_nofollow(&path) {
                        Ok(()) => GuestToHost::FileOpDone { id },
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => GuestToHost::FileOpDone { id },
                        Err(e) => GuestToHost::Error {
                            id,
                            message: format!("failed to delete {path}: {e}"),
                        },
                    }
                };
                if ctrl_tx.send(msg).is_err() {
                    break;
                }
            }
            Ok(HostToGuest::PrepareSnapshot) => {
                // Flush guest dirty pages out to the system-overlay
                // virtio-blk device (/dev/vdb) before host save_state /
                // host-side APFS clonefile runs. sync() drains the page
                // cache; BLKFLSBUF + fsync flush the block device's own
                // buffers. Finally freeze the ext4 upper so no new write can
                // race between this acknowledgement and the host pausing all
                // vCPUs. The host then captures a coherent file.
                //
                // /mnt/shared/system/rootfs.img is no longer the guest's
                // data path -- the guest only writes through /dev/vdb
                // -- so no FUSE_FSYNC over VirtioFS is needed here.
                eprintln!("[capsem-agent] PrepareSnapshot: syncing and flushing /dev/vdb");
                unsafe {
                    libc::sync();
                }

                let fd = unsafe { libc::open(c"/dev/vdb".as_ptr(), libc::O_RDWR) };
                if fd >= 0 {
                    const BLKFLSBUF: i32 = 0x1261;
                    unsafe {
                        if libc::ioctl(fd, BLKFLSBUF.try_into().unwrap()) != 0 {
                            eprintln!(
                                "[capsem-agent] ioctl(BLKFLSBUF) failed: {}",
                                std::io::Error::last_os_error()
                            );
                        }
                        if libc::fsync(fd) != 0 {
                            eprintln!("[capsem-agent] fsync failed: {}", std::io::Error::last_os_error());
                        }
                        libc::close(fd);
                    }
                } else {
                    eprintln!(
                        "[capsem-agent] failed to open /dev/vdb for flush: {}",
                        std::io::Error::last_os_error()
                    );
                }

                if let Err(e) = freeze_system_filesystem() {
                    eprintln!("[capsem-agent] PrepareSnapshot: failed to freeze system filesystem: {e}");
                    // Do not acknowledge an unsafe snapshot. The host timeout
                    // path sends Unfreeze and fails the suspend closed.
                    continue;
                }

                if ctrl_tx.send(GuestToHost::SnapshotReady).is_err() {
                    let _ = thaw_system_filesystem();
                    break;
                }
            }
            Ok(HostToGuest::Unfreeze) => {
                eprintln!("[capsem-agent] Unfreeze: thawing {SYSTEM_FS_MOUNT}");
                if let Err(e) = thaw_system_filesystem() {
                    eprintln!("[capsem-agent] failed to thaw system filesystem: {e}");
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
mod tests;
