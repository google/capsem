// capsem-pty-agent: Guest-side PTY-over-vsock bridge.
//
// Runs inside the Linux VM as a child of capsem-init. Creates a PTY pair,
// forks bash on the slave side, and bridges the master PTY with the host
// over three vsock connections:
//   - Port 5001: raw PTY I/O (terminal data)
//   - Port 5000: control messages (resize, heartbeat, boot config)
//   - Port 5005: exec output (direct child process stdout, on demand)

#[path = "vsock_io.rs"]
mod vsock_io;

use std::io::{self, Read as _, Write as _};
use std::os::unix::io::{AsRawFd, RawFd};
use std::process;
use std::thread;

use capsem_proto::{
    AuditRecord, BootStage, GuestToHost, HostToGuest, MAX_FRAME_SIZE, SHUTDOWN_GRACE_SECS,
    VSOCK_PORT_AUDIT, VSOCK_PORT_CONTROL, VSOCK_PORT_EXEC, VSOCK_PORT_TERMINAL,
    decode_host_msg, encode_audit_record, encode_guest_msg,
    validate_env_key, validate_env_value, validate_file_path, validate_file_path_safe,
    MAX_BOOT_ENV_VARS, MAX_BOOT_FILES, MAX_BOOT_FILE_BYTES,
};
use nix::libc;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::pty::openpty;
use nix::sys::signal::{SigHandler, Signal, signal};
use nix::unistd::{ForkResult, Pid, close, dup2, execvp, fork, setsid};

use vsock_io::{VSOCK_HOST_CID, read_exact_fd, vsock_connect, vsock_connect_retry, write_all_fd};
/// Boot log persisted so it can be inspected after boot (`cat /var/log/capsem-boot.log`).
const BOOT_LOG_PATH: &str = "/var/log/capsem-boot.log";
/// Reconnect timeout before giving up (seconds).
const RECONNECT_TIMEOUT_SECS: u64 = 30;

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
            Ok(HostToGuest::FileWrite { id: _, path, data, mode }) => {
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
            Ok(HostToGuest::FileRead { .. }) => {
                eprintln!("[capsem-agent] ignoring FileRead during boot");
            }
            Ok(HostToGuest::FileDelete { .. }) => {
                eprintln!("[capsem-agent] ignoring FileDelete during boot");
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

    // Step 4b: Activate Python venv if capsem-init created one.
    // capsem-init creates the venv in the background and touches a ready flag when done.
    // Wait briefly for it to finish before checking.
    const VENV_DIR: &str = "/root/.venv";
    const VENV_READY: &str = "/run/capsem-venv-ready";
    let venv_activate = std::path::Path::new(VENV_DIR).join("bin/activate");
    if !venv_activate.exists() && !std::path::Path::new(VENV_READY).exists() {
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if std::path::Path::new(VENV_READY).exists() || venv_activate.exists() {
                break;
            }
        }
    }
    if venv_activate.exists() {
        boot_env.push(("VIRTUAL_ENV".into(), VENV_DIR.into()));
        // Prepend venv bin to PATH if PATH exists in boot_env.
        if let Some((_, path_val)) = boot_env.iter_mut().find(|(k, _)| k == "PATH") {
            *path_val = format!("{VENV_DIR}/bin:{path_val}");
        }
        blog_line(&mut blog, "venv activated in boot_env");
    } else {
        blog_line(&mut blog, "WARNING: venv not found after waiting, skipping activation");
    }

    // Step 4c: Set hostname from CAPSEM_VM_NAME if present.
    if let Some((_, name)) = boot_env.iter().find(|(k, _)| k == "CAPSEM_VM_NAME") {
        let c_name = std::ffi::CString::new(name.as_str()).unwrap_or_default();
        let ret = unsafe { libc::sethostname(c_name.as_ptr(), name.len() as _) };
        if ret == 0 {
            blog_line(&mut blog, &format!("hostname set to {name}"));
        } else {
            blog_line(&mut blog, &format!(
                "WARNING: sethostname failed: {}",
                std::io::Error::last_os_error()
            ));
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
            
            drop(blog); // flush and close boot log before loop

            let mut is_first = true;
            let mut t_fd = terminal_fd;
            let mut c_fd = control_fd;

            loop {
                if !is_first {
                    use capsem_proto::poll::{RetryOpts, retry_with_backoff};

                    let fds = retry_with_backoff(
                        &RetryOpts::new("reconnect", std::time::Duration::from_secs(RECONNECT_TIMEOUT_SECS)),
                        || {
                            let t = vsock_io::vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_TERMINAL).ok()?;
                            match vsock_io::vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_CONTROL) {
                                Ok(c) => Some((t, c)),
                                Err(_) => { unsafe { libc::close(t); } None }
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
                    rebind_workspace_after_resume();
                    let _ = send_guest_msg(c_fd, &GuestToHost::Ready { version: env!("CARGO_PKG_VERSION").to_string() });

                    // Drain abbreviated handshake, processing clock/timezone resync.
                    loop {
                        match recv_host_msg(c_fd) {
                            Ok(HostToGuest::BootConfigDone) => break,
                            Ok(HostToGuest::Shutdown) => {
                                let _ = nix::sys::signal::kill(child, Signal::SIGTERM);
                                process::exit(0);
                            }
                            Ok(HostToGuest::BootConfig { epoch_secs }) => {
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
                                        let _ = std::fs::set_permissions(
                                            &path,
                                            std::fs::Permissions::from_mode(mode),
                                        );
                                    }
                                    eprintln!("[capsem-agent] resume: wrote {path}");
                                }
                            }
                            Ok(_) => {}
                            Err(_) => break, // vsock broke again
                        }
                    }

                    // Unfreeze filesystem in case we were suspended
                    std::process::Command::new("fsfreeze").args(["-u", "/"]).status().ok();
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
                run_bridge(master_fd, child, t_fd, c_fd, &boot_env_for_parent);
                
                // Cleanup broken FDs
                unsafe { libc::close(t_fd); libc::close(c_fd); }
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

fn run_bridge(master_fd: RawFd, child_pid: Pid, terminal_fd: RawFd, control_fd: RawFd, boot_env: &[(String, String)]) {
    // Serialize all control channel writes through a single channel + writer
    // thread. The exec background thread and control_loop both need to write
    // to control_fd; concurrent writes would corrupt protocol framing.
    let (ctrl_write_tx, ctrl_write_rx) = std::sync::mpsc::channel::<GuestToHost>();

    // Single control channel writer thread. On write failure (host gone --
    // typically because the VM was suspended and resumed against a fresh
    // host process), shutdown both vsock fds so bridge_loop and control_loop
    // wake from their polls and the outer reconnect logic re-establishes
    // both connections against the new host.
    std::thread::spawn(move || {
        while let Ok(msg) = ctrl_write_rx.recv() {
            if send_guest_msg(control_fd, &msg).is_err() {
                unsafe {
                    libc::shutdown(control_fd, libc::SHUT_RDWR);
                    libc::shutdown(terminal_fd, libc::SHUT_RDWR);
                }
                break;
            }
        }
    });

    // Heartbeat. Without periodic probes the connection is invisible until
    // the next genuine traffic, which can be hours. After a suspend/resume
    // the host process is gone; the first failed write here trips the
    // shutdown path above and triggers reconnect within ~3s.
    let hb_tx = ctrl_write_tx.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3));
            if hb_tx.send(GuestToHost::Pong).is_err() {
                break;
            }
        }
    });

    // Spawn control channel handler in a background thread.
    let boot_env_owned = boot_env.to_vec();
    let ctrl_tx = ctrl_write_tx;
    thread::spawn(move || {
        control_loop(control_fd, master_fd, child_pid, &boot_env_owned, ctrl_tx);
    });

    // Spawn audit log reader thread (tails auditd output, streams to host).
    thread::spawn(move || {
        audit_reader_loop();
    });

    // Main I/O bridge: master PTY <-> vsock terminal port.
    bridge_loop(master_fd, terminal_fd);

    // If bridge exits, we just return. The reconnect loop will handle re-establishing vsock.
    // If it was a genuine Shutdown, control_loop already killed the child, and the process will eventually exit.
    eprintln!("[capsem-agent] bridge exited");
}

fn bridge_loop(master_fd: RawFd, vsock_fd: RawFd) {
    let mut buf = [0u8; 8192];

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
        // Poll vsock_fd too so a local shutdown (triggered by the heartbeat
        // detecting host death) wakes us up via POLLHUP. Otherwise we'd sit
        // in poll forever waiting for PTY activity that never comes.
        let mut poll_fds = [
            PollFd::new(unsafe { std::os::unix::io::BorrowedFd::borrow_raw(master_fd) }, PollFlags::POLLIN),
            PollFd::new(unsafe { std::os::unix::io::BorrowedFd::borrow_raw(vsock_fd) }, PollFlags::empty()),
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

        if let Some(revents) = poll_fds[1].revents() {
            if revents.intersects(PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL) {
                break;
            }
        }

        // Master PTY -> vsock (stdout direction)
        if let Some(revents) = poll_fds[0].revents() {
            if revents.contains(PollFlags::POLLIN) {
                match nix::unistd::read(master_fd, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if write_all_fd(vsock_fd, &buf[..n]).is_err() {
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

/// Tail /var/log/audit/audit.log and stream parsed execve records to host via vsock:5006.
///
/// Waits for the audit log file to appear (auditd may start slightly after the agent),
/// then continuously reads new lines. Each complete audit record (correlated by audit ID)
/// is serialized as MessagePack and sent as a length-prefixed frame.
fn audit_reader_loop() {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};

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

    // Connect to host audit port
    let audit_fd = match vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_AUDIT) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("[capsem-agent] audit: vsock connect failed: {e}");
            return;
        }
    };
    eprintln!("[capsem-agent] audit: connected to host, tailing {AUDIT_LOG}");

    let file = match std::fs::File::open(AUDIT_LOG) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[capsem-agent] audit: open failed: {e}");
            return;
        }
    };
    let mut reader = BufReader::new(file);

    // Accumulate multi-line audit records by audit ID.
    // Each execve event generates SYSCALL + EXECVE + CWD + PROCTITLE lines
    // sharing the same audit ID (e.g., "1713100000.001:42").
    let mut pending: HashMap<String, AuditRecordBuilder> = HashMap::new();
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // EOF -- audit log hasn't grown yet, poll
                thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            Ok(_) => {}
            Err(_) => break,
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse audit log line. Format: type=SYSCALL msg=audit(1713100000.001:42): ...
        let Some(audit_id) = extract_audit_id(line) else { continue };
        let record_type = extract_field(line, "type=");

        let builder = pending.entry(audit_id.clone()).or_default();

        match record_type.as_deref() {
            Some("SYSCALL") => {
                builder.pid = extract_field(line, " pid=").and_then(|v| v.parse().ok());
                builder.ppid = extract_field(line, " ppid=").and_then(|v| v.parse().ok());
                builder.uid = extract_field(line, " uid=").and_then(|v| v.parse().ok());
                builder.exe = extract_field(line, " exe=").map(|s| s.trim_matches('"').to_string());
                builder.comm = extract_field(line, " comm=").map(|s| s.trim_matches('"').to_string());
                builder.tty = extract_field(line, " tty=").and_then(|s| {
                    if s == "(none)" { None } else { Some(s) }
                });
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
                if builder.has_syscall {
                    if let Some(record) = builder.build(&audit_id) {
                        let frame = match encode_audit_record(&record) {
                            Ok(f) => f,
                            Err(_) => { pending.remove(&audit_id); continue; }
                        };
                        if write_all_fd(audit_fd, &frame).is_err() {
                            eprintln!("[capsem-agent] audit: write failed, disconnecting");
                            return;
                        }
                    }
                }
                pending.remove(&audit_id);
            }
            _ => {}
        }

        // Prevent memory leak for incomplete records
        if pending.len() > 1000 {
            pending.retain(|_, v| v.has_syscall);
        }
    }
}

/// Intermediate builder for multi-line audit records.
#[derive(Default)]
struct AuditRecordBuilder {
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
            argv: self.argv.clone().unwrap_or_else(|| self.exe.clone().unwrap_or_default()),
            cwd: self.cwd.clone(),
            tty: self.tty.clone(),
            session_id: None,
            parent_exe: None,
            audit_id: audit_id.to_string(),
        })
    }
}

/// Extract audit ID from a log line. Format: msg=audit(1713100000.001:42):
fn extract_audit_id(line: &str) -> Option<String> {
    let start = line.find("msg=audit(")? + "msg=audit(".len();
    let end = line[start..].find(')')? + start;
    Some(line[start..end].to_string())
}

/// Extract the audit timestamp as microseconds. Format: audit(1713100000.001:42)
fn extract_audit_timestamp_us(line: &str) -> Option<u64> {
    let id = extract_audit_id(line)?;
    let ts_part = id.split(':').next()?;
    let secs_f64: f64 = ts_part.parse().ok()?;
    Some((secs_f64 * 1_000_000.0) as u64)
}

/// Extract a field value from an audit log line. Fields are space-delimited key=value.
fn extract_field(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    // Value ends at next space (or end of line), unless quoted
    if rest.starts_with('"') {
        let end = rest[1..].find('"')? + 2;
        Some(rest[..end].to_string())
    } else {
        let end = rest.find(' ').unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

/// Reconstruct argv from EXECVE audit record.
/// Format: type=EXECVE msg=audit(...): argc=3 a0="python3" a1="train.py" a2="--epochs"
fn extract_execve_argv(line: &str) -> Option<String> {
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
    if args.is_empty() { None } else { Some(args.join(" ")) }
}

/// Execute a command as a direct child process, streaming output over vsock:5005.
///
/// Runs in a background thread so control_loop remains responsive to heartbeats.
/// Output flows as raw bytes on a dedicated exec vsock connection. The exit code
/// is sent as ExecDone via the serialized control write channel.
fn run_exec(ctrl_tx: &std::sync::mpsc::Sender<GuestToHost>, id: u64, command: &str, boot_env: &[(String, String)]) {
    // Connect to host exec port.
    let exec_fd = match vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_EXEC) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("[capsem-agent] exec[{id}] vsock connect failed: {e}");
            let _ = ctrl_tx.send(GuestToHost::ExecDone { id, exit_code: 126 });
            return;
        }
    };

    run_exec_on_fds(exec_fd, ctrl_tx, id, command, boot_env);
}

/// Inner exec implementation that takes pre-connected fds (testable without vsock).
/// `ctrl_tx` serializes writes to the control channel (prevents frame corruption
/// from concurrent writers). `exec_fd` is consumed: closed on all exit paths.
fn run_exec_on_fds(exec_fd: RawFd, ctrl_tx: &std::sync::mpsc::Sender<GuestToHost>, id: u64, command: &str, boot_env: &[(String, String)]) {
    // RAII guard to ensure exec_fd is closed on all paths.
    struct FdGuard(RawFd);
    impl Drop for FdGuard {
        fn drop(&mut self) { unsafe { libc::close(self.0); } }
    }
    let _exec_guard = FdGuard(exec_fd);

    // Send ExecStarted handshake so host knows which exec ID this connection belongs to.
    if let Err(e) = send_guest_msg(exec_fd, &GuestToHost::ExecStarted { id }) {
        eprintln!("[capsem-agent] exec[{id}] handshake failed: {e}");
        let _ = ctrl_tx.send(GuestToHost::ExecDone { id, exit_code: 126 });
        return;
    }

    // Spawn child process with piped stdout and stderr.
    let cwd = if std::path::Path::new("/root").exists() { "/root" } else { "/" };
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
            return;
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
                    Ok(n) => { let _ = write_all_fd(efd, &buf[..n]); }
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

    if let Some(t) = stderr_thread { let _ = t.join(); }

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

/// Read a file, refusing to follow symlinks on the final path component.
/// Returns ELOOP if the target is a symlink.
fn read_nofollow(path: &str) -> io::Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
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

fn control_loop(
    control_fd: RawFd,
    master_fd: RawFd,
    child_pid: Pid,
    boot_env: &[(String, String)],
    ctrl_tx: std::sync::mpsc::Sender<GuestToHost>,
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
                // Flush dirty pages to disk.
                unsafe { libc::sync(); }
                // Ask bash to exit gracefully.
                let _ = nix::sys::signal::kill(child_pid, Signal::SIGTERM);
                // Give bash time to clean up (save history, run traps).
                thread::sleep(std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS));
                // Force-kill if bash ignored SIGTERM (interactive bash does this).
                let _ = nix::sys::signal::kill(child_pid, Signal::SIGKILL);
                break;
            }
            Ok(HostToGuest::Exec { id, command }) => {
                eprintln!("[capsem-agent] exec[{id}]: {command}");
                let boot_env = boot_env.to_vec();
                let tx = ctrl_tx.clone();
                thread::spawn(move || {
                    run_exec(&tx, id, &command, &boot_env);
                });
            }
            Ok(HostToGuest::FileWrite { id, path, data, mode }) => {
                eprintln!("[capsem-agent] FileWrite {path} ({} bytes)", data.len());
                let ws = std::path::Path::new(GUEST_WORKSPACE_ROOT);
                let msg = if let Err(e) = validate_file_path_safe(&path, ws) {
                    GuestToHost::Error { id, message: format!("FileWrite rejected: {e}") }
                } else if let Err(e) = write_nofollow(&path, &data, mode) {
                    GuestToHost::Error { id, message: format!("failed to write {path}: {e}") }
                } else {
                    GuestToHost::FileOpDone { id }
                };
                if ctrl_tx.send(msg).is_err() { break; }
            }
            Ok(HostToGuest::FileRead { id, path }) => {
                eprintln!("[capsem-agent] FileRead {path}");
                let ws = std::path::Path::new(GUEST_WORKSPACE_ROOT);
                let msg = if let Err(e) = validate_file_path_safe(&path, ws) {
                    GuestToHost::Error { id, message: format!("FileRead rejected: {e}") }
                } else {
                    match read_nofollow(&path) {
                        Ok(data) => GuestToHost::FileContent { id, path, data },
                        Err(e) => GuestToHost::Error { id, message: format!("failed to read {path}: {e}") },
                    }
                };
                if ctrl_tx.send(msg).is_err() { break; }
            }
            Ok(HostToGuest::FileDelete { id, path }) => {
                eprintln!("[capsem-agent] FileDelete {path}");
                let ws = std::path::Path::new(GUEST_WORKSPACE_ROOT);
                let msg = if let Err(e) = validate_file_path_safe(&path, ws) {
                    GuestToHost::Error { id, message: format!("FileDelete rejected: {e}") }
                } else if let Err(e) = delete_nofollow(&path) {
                    GuestToHost::Error { id, message: format!("failed to delete {path}: {e}") }
                } else {
                    GuestToHost::FileOpDone { id }
                };
                if ctrl_tx.send(msg).is_err() { break; }
            }
            Ok(HostToGuest::PrepareSnapshot) => {
                // sync() flushes dirty caches to the underlying FS (the host
                // via virtiofsd, or the block device). Then best-effort
                // fsfreeze: VirtioFS returns ENOTSUP because FUSE doesn't
                // implement the freeze_fs ioctl. That's fine -- Apple VZ
                // pauses the VM before save_state, which stops all guest
                // writes anyway. Proceed with SnapshotReady regardless so
                // the host never hangs waiting for a reply it won't get.
                eprintln!("[capsem-agent] PrepareSnapshot: syncing and freezing /");
                unsafe { libc::sync(); }
                match std::process::Command::new("fsfreeze").args(["-f", "/"]).status() {
                    Ok(st) if st.success() => {}
                    Ok(st) => {
                        eprintln!("[capsem-agent] fsfreeze -f not available ({st}); continuing after sync");
                    }
                    Err(e) => {
                        eprintln!("[capsem-agent] fsfreeze exec failed: {e}; continuing after sync");
                    }
                }
                if ctrl_tx.send(GuestToHost::SnapshotReady).is_err() { break; }
            }
            Ok(HostToGuest::Unfreeze) => {
                eprintln!("[capsem-agent] Unfreeze: thawing /");
                match std::process::Command::new("fsfreeze").args(["-u", "/"]).status() {
                    Ok(st) if !st.success() => eprintln!("[capsem-agent] fsfreeze -u failed: {}", st),
                    Err(e) => eprintln!("[capsem-agent] failed to execute fsfreeze: {e}"),
                    _ => {}
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
        // Set CLOEXEC so child processes (e.g., sleep in control_loop tests)
        // don't inherit these fds and prevent EOF detection.
        for &fd in &fds {
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
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
            id: 1,
            path: "/root/.gemini/settings.json".into(),
            data: b"{}".to_vec(),
            mode: 0o644,
        };
        let frame = capsem_proto::encode_host_msg(&msg).unwrap();
        write_all_fd(write_fd, &frame).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        match decoded {
            HostToGuest::FileWrite { id, path, data, mode } => {
                assert_eq!(id, 1);
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
    fn prepare_snapshot_roundtrip() {
        let (read_fd, write_fd) = make_pipe();
        let msg = HostToGuest::PrepareSnapshot;
        let frame = capsem_proto::encode_host_msg(&msg).unwrap();
        write_all_fd(write_fd, &frame).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        assert!(matches!(decoded, HostToGuest::PrepareSnapshot));
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn unfreeze_roundtrip() {
        let (read_fd, write_fd) = make_pipe();
        let msg = HostToGuest::Unfreeze;
        let frame = capsem_proto::encode_host_msg(&msg).unwrap();
        write_all_fd(write_fd, &frame).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        assert!(matches!(decoded, HostToGuest::Unfreeze));
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn snapshot_ready_roundtrip() {
        let (read_fd, write_fd) = make_pipe();
        send_guest_msg(write_fd, &GuestToHost::SnapshotReady).unwrap();
        let mut len_buf = [0u8; 4];
        read_exact_fd(read_fd, &mut len_buf).unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        read_exact_fd(read_fd, &mut payload).unwrap();
        let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
        assert!(matches!(decoded, GuestToHost::SnapshotReady));
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn send_recv_multiple_messages_over_pipe() {
        let (read_fd, write_fd) = make_pipe();

        // Send host messages.
        let ping_frame = capsem_proto::encode_host_msg(&HostToGuest::Ping { epoch_secs: 0 }).unwrap();
        write_all_fd(write_fd, &ping_frame).unwrap();
        let resize_frame = capsem_proto::encode_host_msg(&HostToGuest::Resize { cols: 80, rows: 24 }).unwrap();
        write_all_fd(write_fd, &resize_frame).unwrap();

        assert!(matches!(recv_host_msg(read_fd).unwrap(), HostToGuest::Ping { .. }));
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
    // Bridge loop concurrency
    // -----------------------------------------------------------------------

    #[test]
    fn bridge_loop_concurrency_no_deadlock() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::AsRawFd;

        let (mut master_host, master_guest) = UnixStream::pair().unwrap();
        let (mut vsock_host, vsock_guest) = UnixStream::pair().unwrap();

        let master_fd = master_guest.as_raw_fd();
        let vsock_fd = vsock_guest.as_raw_fd();

        let _bridge_thread = std::thread::spawn(move || {
            bridge_loop(master_fd, vsock_fd);
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
    // Exec over vsock
    // -----------------------------------------------------------------------

    /// Helper: read ExecStarted handshake from exec fd, return exec id.
    fn read_exec_started(exec_host: &mut std::os::unix::net::UnixStream) -> u64 {
        use std::io::Read;
        let mut len_buf = [0u8; 4];
        exec_host.read_exact(&mut len_buf).unwrap();
        let frame_len = u32::from_be_bytes(len_buf) as usize;
        let mut frame = vec![0u8; frame_len];
        exec_host.read_exact(&mut frame).unwrap();
        match capsem_proto::decode_guest_msg(&frame).unwrap() {
            GuestToHost::ExecStarted { id } => id,
            other => panic!("expected ExecStarted, got {other:?}"),
        }
    }

    /// Helper: receive ExecDone from mpsc channel, return (id, exit_code).
    fn recv_exec_done(rx: &std::sync::mpsc::Receiver<GuestToHost>) -> (u64, i32) {
        match rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap() {
            GuestToHost::ExecDone { id, exit_code } => (id, exit_code),
            other => panic!("expected ExecDone, got {other:?}"),
        }
    }

    #[test]
    fn exec_echo_captures_output_and_exit_code() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::IntoRawFd;
        use std::io::Read;

        let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
        let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel();

        let exec_fd = exec_guest.into_raw_fd();

        std::thread::spawn(move || {
            run_exec_on_fds(exec_fd, &ctrl_tx, 42, "echo hello", &[]);
        });

        let id = read_exec_started(&mut exec_host);
        assert_eq!(id, 42);

        let mut output = Vec::new();
        exec_host.read_to_end(&mut output).unwrap();
        assert_eq!(String::from_utf8_lossy(&output).trim(), "hello");

        let (done_id, exit_code) = recv_exec_done(&ctrl_rx);
        assert_eq!(done_id, 42);
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn exec_nonzero_exit_code() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::IntoRawFd;
        use std::io::Read;

        let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
        let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel();
        let exec_fd = exec_guest.into_raw_fd();

        std::thread::spawn(move || {
            run_exec_on_fds(exec_fd, &ctrl_tx, 7, "exit 42", &[]);
        });

        let _id = read_exec_started(&mut exec_host);
        let mut _output = Vec::new();
        exec_host.read_to_end(&mut _output).unwrap();

        let (done_id, exit_code) = recv_exec_done(&ctrl_rx);
        assert_eq!(done_id, 7);
        assert_eq!(exit_code, 42);
    }

    #[test]
    fn exec_boot_env_passed_to_child() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::IntoRawFd;
        use std::io::Read;

        let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
        let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel();
        let exec_fd = exec_guest.into_raw_fd();

        let env = vec![
            ("CAPSEM_TEST_VAR".to_string(), "test_value_42".to_string()),
        ];

        std::thread::spawn(move || {
            run_exec_on_fds(exec_fd, &ctrl_tx, 1, "echo $CAPSEM_TEST_VAR", &env);
        });

        let _id = read_exec_started(&mut exec_host);
        let mut output = Vec::new();
        exec_host.read_to_end(&mut output).unwrap();
        assert_eq!(String::from_utf8_lossy(&output).trim(), "test_value_42");

        let (_, exit_code) = recv_exec_done(&ctrl_rx);
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn exec_stderr_captured() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::IntoRawFd;
        use std::io::Read;

        let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
        let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel();
        let exec_fd = exec_guest.into_raw_fd();

        std::thread::spawn(move || {
            run_exec_on_fds(exec_fd, &ctrl_tx, 3, "echo out; echo err >&2", &[]);
        });

        let _id = read_exec_started(&mut exec_host);
        let mut output = Vec::new();
        exec_host.read_to_end(&mut output).unwrap();
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("out"), "stdout missing: {text}");
        assert!(text.contains("err"), "stderr missing: {text}");

        let (_, exit_code) = recv_exec_done(&ctrl_rx);
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn exec_sentinel_in_output_is_not_stripped() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::IntoRawFd;
        use std::io::Read;

        let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
        let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel();
        let exec_fd = exec_guest.into_raw_fd();

        std::thread::spawn(move || {
            run_exec_on_fds(
                exec_fd, &ctrl_tx, 99,
                r#"printf '\033_CAPSEM_EXIT:999:0\033\\'"#,
                &[],
            );
        });

        let _id = read_exec_started(&mut exec_host);
        let mut output = Vec::new();
        exec_host.read_to_end(&mut output).unwrap();
        assert!(output.windows(14).any(|w| w == b"\x1b_CAPSEM_EXIT:"),
            "sentinel sequence should pass through as plain output");

        let (done_id, exit_code) = recv_exec_done(&ctrl_rx);
        assert_eq!(done_id, 99);
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn exec_large_output_no_truncation() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::IntoRawFd;
        use std::io::Read;

        let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
        let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel();
        let exec_fd = exec_guest.into_raw_fd();

        std::thread::spawn(move || {
            run_exec_on_fds(
                exec_fd, &ctrl_tx, 5,
                "dd if=/dev/zero bs=1024 count=100 2>/dev/null | base64",
                &[],
            );
        });

        let _id = read_exec_started(&mut exec_host);
        let mut output = Vec::new();
        exec_host.read_to_end(&mut output).unwrap();
        assert!(output.len() > 100_000, "output too small: {} bytes", output.len());

        let (_, exit_code) = recv_exec_done(&ctrl_rx);
        assert_eq!(exit_code, 0);
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

    // -------------------------------------------------------------------
    // O_NOFOLLOW file I/O helpers
    // -------------------------------------------------------------------

    #[test]
    fn write_nofollow_works_for_regular_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-write-nofollow");
        write_nofollow(path.to_str().unwrap(), b"hello", 0o644).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_nofollow_works_for_regular_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-read-nofollow");
        std::fs::write(&path, b"world").unwrap();
        assert_eq!(read_nofollow(path.to_str().unwrap()).unwrap(), b"world");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_nofollow_rejects_symlink() {
        let dir = std::env::temp_dir();
        let target = dir.join("capsem-test-wn-target");
        let link = dir.join("capsem-test-wn-link");
        std::fs::write(&target, b"original").unwrap();
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = write_nofollow(link.to_str().unwrap(), b"evil", 0o644);
        assert!(err.is_err(), "write through symlink must fail");
        // Target must be unchanged.
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        std::fs::remove_file(&target).ok();
        std::fs::remove_file(&link).ok();
    }

    #[test]
    fn read_nofollow_rejects_symlink() {
        let dir = std::env::temp_dir();
        let target = dir.join("capsem-test-rn-target");
        let link = dir.join("capsem-test-rn-link");
        std::fs::write(&target, b"secret").unwrap();
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = read_nofollow(link.to_str().unwrap());
        assert!(err.is_err(), "read through symlink must fail");
        std::fs::remove_file(&target).ok();
        std::fs::remove_file(&link).ok();
    }

    #[test]
    fn delete_nofollow_rejects_symlink() {
        let dir = std::env::temp_dir();
        let target = dir.join("capsem-test-dn-target");
        let link = dir.join("capsem-test-dn-link");
        std::fs::write(&target, b"keep").unwrap();
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = delete_nofollow(link.to_str().unwrap());
        assert!(err.is_err(), "delete of symlink must fail");
        // Both should still exist.
        assert!(target.exists());
        assert!(link.symlink_metadata().is_ok());
        std::fs::remove_file(&target).ok();
        std::fs::remove_file(&link).ok();
    }

    #[test]
    fn delete_nofollow_deletes_regular_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-dn-regular");
        std::fs::write(&path, b"delete me").unwrap();
        delete_nofollow(path.to_str().unwrap()).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn delete_nofollow_nonexistent_returns_error() {
        let result = delete_nofollow("/tmp/capsem-test-dn-nonexistent-xyzzy");
        assert!(result.is_err());
    }

    #[test]
    fn write_nofollow_creates_parent_dirs() {
        let dir = std::env::temp_dir();
        let nested = dir.join("capsem-test-wn-nested/deep/path/file.txt");
        let _ = std::fs::remove_dir_all(dir.join("capsem-test-wn-nested"));
        write_nofollow(nested.to_str().unwrap(), b"nested", 0o644).unwrap();
        assert_eq!(std::fs::read(&nested).unwrap(), b"nested");
        std::fs::remove_dir_all(dir.join("capsem-test-wn-nested")).ok();
    }

    #[test]
    fn write_nofollow_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-wn-perms");
        write_nofollow(path.to_str().unwrap(), b"test", 0o755).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_nofollow_truncates_existing() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-wn-truncate");
        write_nofollow(path.to_str().unwrap(), b"long content here", 0o644).unwrap();
        write_nofollow(path.to_str().unwrap(), b"short", 0o644).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"short");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_nofollow_nonexistent_returns_error() {
        let result = read_nofollow("/tmp/capsem-test-rn-nonexistent-xyzzy");
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------
    // Exec: merged stdout + stderr stream
    // -------------------------------------------------------------------

    #[test]
    fn exec_stdout_and_stderr_both_appear_in_merged_stream() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::IntoRawFd;
        use std::io::Read;

        let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
        let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel();
        let exec_fd = exec_guest.into_raw_fd();

        // Generate distinct output on both stdout and stderr
        std::thread::spawn(move || {
            run_exec_on_fds(
                exec_fd, &ctrl_tx, 50,
                "echo STDOUT_MARKER; echo STDERR_MARKER >&2",
                &[],
            );
        });

        let _id = read_exec_started(&mut exec_host);
        let mut output = Vec::new();
        exec_host.read_to_end(&mut output).unwrap();
        let text = String::from_utf8_lossy(&output);

        assert!(text.contains("STDOUT_MARKER"), "stdout missing from merged stream: {text}");
        assert!(text.contains("STDERR_MARKER"), "stderr missing from merged stream: {text}");

        let (_, exit_code) = recv_exec_done(&ctrl_rx);
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn exec_invalid_command_returns_nonzero() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::IntoRawFd;
        use std::io::Read;

        let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
        let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel();
        let exec_fd = exec_guest.into_raw_fd();

        std::thread::spawn(move || {
            run_exec_on_fds(exec_fd, &ctrl_tx, 60, "nonexistent_command_xyz", &[]);
        });

        let _id = read_exec_started(&mut exec_host);
        let mut _output = Vec::new();
        exec_host.read_to_end(&mut _output).unwrap();

        let (_, exit_code) = recv_exec_done(&ctrl_rx);
        assert_ne!(exit_code, 0);
    }

    #[test]
    fn exec_empty_command_succeeds() {
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::IntoRawFd;
        use std::io::Read;

        let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
        let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel();
        let exec_fd = exec_guest.into_raw_fd();

        std::thread::spawn(move || {
            run_exec_on_fds(exec_fd, &ctrl_tx, 70, "true", &[]);
        });

        let _id = read_exec_started(&mut exec_host);
        let mut output = Vec::new();
        exec_host.read_to_end(&mut output).unwrap();
        assert!(output.is_empty(), "true should produce no output");

        let (_, exit_code) = recv_exec_done(&ctrl_rx);
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn exec_fd_closed_before_exec_done() {
        // Verify the agent closes exec_fd (EOF to host) before sending ExecDone.
        // The host relies on this ordering to accumulate all output before the
        // ExecDone arrives on the control channel.
        use std::os::unix::net::UnixStream;
        use std::os::unix::io::IntoRawFd;
        use std::io::Read;

        let (mut exec_host, exec_guest) = UnixStream::pair().unwrap();
        let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel();
        let exec_fd = exec_guest.into_raw_fd();

        std::thread::spawn(move || {
            run_exec_on_fds(exec_fd, &ctrl_tx, 80, "echo ordering_test", &[]);
        });

        let _id = read_exec_started(&mut exec_host);

        // Read until EOF -- this blocks until exec_fd is closed.
        let mut output = Vec::new();
        exec_host.read_to_end(&mut output).unwrap();

        // EOF received. ExecDone should now be available (or arrive shortly).
        let (done_id, exit_code) = recv_exec_done(&ctrl_rx);
        assert_eq!(done_id, 80);
        assert_eq!(exit_code, 0);
        assert!(String::from_utf8_lossy(&output).contains("ordering_test"));
    }

    // -------------------------------------------------------------------
    // Boot timing: additional edge cases
    // -------------------------------------------------------------------

    #[test]
    fn parse_boot_timing_empty_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-boot-timing-empty");
        std::fs::write(&path, "").unwrap();
        let result = parse_boot_timing(path.to_str().unwrap());
        assert!(result.is_empty());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_boot_timing_rejects_long_names() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-boot-timing-longname");
        let long_name = "a".repeat(65);
        std::fs::write(&path, format!(
            "{{\"name\":\"{long_name}\",\"duration_ms\":10}}\n\
             {{\"name\":\"ok\",\"duration_ms\":20}}\n"
        )).unwrap();
        let result = parse_boot_timing(path.to_str().unwrap());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "ok");
        std::fs::remove_file(&path).ok();
    }

    // -------------------------------------------------------------------
    // Control message: frame boundary
    // -------------------------------------------------------------------

    #[test]
    fn recv_truncated_payload_returns_error() {
        let (read_fd, write_fd) = make_pipe();
        // Write a valid length (10 bytes) but only 5 bytes of payload
        let len_bytes = 10u32.to_be_bytes();
        write_all_fd(write_fd, &len_bytes).unwrap();
        write_all_fd(write_fd, &[0u8; 5]).unwrap();
        unsafe { libc::close(write_fd); }

        let result = recv_host_msg(read_fd);
        assert!(result.is_err());
        unsafe { libc::close(read_fd); }
    }

    #[test]
    fn send_recv_pong_over_pipe() {
        let (read_fd, write_fd) = make_pipe();
        send_guest_msg(write_fd, &GuestToHost::Pong).unwrap();
        let mut len_buf = [0u8; 4];
        read_exact_fd(read_fd, &mut len_buf).unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        read_exact_fd(read_fd, &mut payload).unwrap();
        let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
        assert!(matches!(decoded, GuestToHost::Pong));
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn send_recv_file_content_over_pipe() {
        let (read_fd, write_fd) = make_pipe();
        let msg = GuestToHost::FileContent {
            id: 42,
            path: "/root/test.txt".to_string(),
            data: b"file contents here".to_vec(),
        };
        send_guest_msg(write_fd, &msg).unwrap();
        let mut len_buf = [0u8; 4];
        read_exact_fd(read_fd, &mut len_buf).unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        read_exact_fd(read_fd, &mut payload).unwrap();
        let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
        match decoded {
            GuestToHost::FileContent { id, path, data } => {
                assert_eq!(id, 42);
                assert_eq!(path, "/root/test.txt");
                assert_eq!(data, b"file contents here");
            }
            other => panic!("expected FileContent, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn send_recv_error_over_pipe() {
        let (read_fd, write_fd) = make_pipe();
        let msg = GuestToHost::Error {
            id: 7,
            message: "something went wrong".to_string(),
        };
        send_guest_msg(write_fd, &msg).unwrap();
        let mut len_buf = [0u8; 4];
        read_exact_fd(read_fd, &mut len_buf).unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        read_exact_fd(read_fd, &mut payload).unwrap();
        let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
        match decoded {
            GuestToHost::Error { id, message } => {
                assert_eq!(id, 7);
                assert_eq!(message, "something went wrong");
            }
            other => panic!("expected Error, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn send_recv_file_op_done_over_pipe() {
        let (read_fd, write_fd) = make_pipe();
        send_guest_msg(write_fd, &GuestToHost::FileOpDone { id: 99 }).unwrap();
        let mut len_buf = [0u8; 4];
        read_exact_fd(read_fd, &mut len_buf).unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        read_exact_fd(read_fd, &mut payload).unwrap();
        let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
        match decoded {
            GuestToHost::FileOpDone { id } => assert_eq!(id, 99),
            other => panic!("expected FileOpDone, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    // -------------------------------------------------------------------
    // Host message roundtrips: FileRead, FileDelete
    // -------------------------------------------------------------------

    #[test]
    fn file_read_roundtrip_over_pipe() {
        let (read_fd, write_fd) = make_pipe();
        let msg = HostToGuest::FileRead {
            id: 10,
            path: "/root/readme.md".into(),
        };
        let frame = capsem_proto::encode_host_msg(&msg).unwrap();
        write_all_fd(write_fd, &frame).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        match decoded {
            HostToGuest::FileRead { id, path } => {
                assert_eq!(id, 10);
                assert_eq!(path, "/root/readme.md");
            }
            other => panic!("expected FileRead, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    #[test]
    fn file_delete_roundtrip_over_pipe() {
        let (read_fd, write_fd) = make_pipe();
        let msg = HostToGuest::FileDelete {
            id: 11,
            path: "/root/temp.txt".into(),
        };
        let frame = capsem_proto::encode_host_msg(&msg).unwrap();
        write_all_fd(write_fd, &frame).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        match decoded {
            HostToGuest::FileDelete { id, path } => {
                assert_eq!(id, 11);
                assert_eq!(path, "/root/temp.txt");
            }
            other => panic!("expected FileDelete, got {other:?}"),
        }
        unsafe { libc::close(read_fd); libc::close(write_fd); }
    }

    // -------------------------------------------------------------------
    // control_loop integration tests
    // -------------------------------------------------------------------

    /// Feed host messages into control_loop and collect guest responses.
    fn run_control_loop_with_messages(
        messages: Vec<HostToGuest>,
    ) -> Vec<GuestToHost> {
        let (ctrl_read_fd, ctrl_write_fd) = make_pipe();
        let pty = openpty(None, None).expect("openpty");
        let master_fd = pty.master.as_raw_fd();
        // Spawn a child so we have a real PID for control_loop.
        let mut child = std::process::Command::new("sleep")
            .arg("300")
            .spawn()
            .expect("spawn sleep");
        let child_pid = Pid::from_raw(child.id() as i32);

        let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel();

        // Write all messages then close the write end so control_loop
        // sees EOF and exits.
        for msg in &messages {
            let frame = capsem_proto::encode_host_msg(msg).unwrap();
            write_all_fd(ctrl_write_fd, &frame).unwrap();
        }
        unsafe { libc::close(ctrl_write_fd); }

        let handle = thread::spawn(move || {
            control_loop(ctrl_read_fd, master_fd, child_pid, &[], ctrl_tx);
        });

        handle.join().unwrap();

        // Kill the sleep process immediately using std (handles waitpid internally).
        let _ = child.kill();
        let _ = child.wait();
        unsafe { libc::close(ctrl_read_fd); }

        // Drain the channel.
        let mut responses = Vec::new();
        while let Ok(msg) = ctrl_rx.try_recv() {
            responses.push(msg);
        }
        responses
    }

    #[test]
    fn control_loop_ping_responds_with_pong() {
        let responses = run_control_loop_with_messages(vec![HostToGuest::Ping { epoch_secs: 0 }]);
        assert_eq!(responses.len(), 1);
        assert!(matches!(responses[0], GuestToHost::Pong));
    }

    #[test]
    fn control_loop_multiple_pings() {
        let responses = run_control_loop_with_messages(vec![
            HostToGuest::Ping { epoch_secs: 0 },
            HostToGuest::Ping { epoch_secs: 0 },
            HostToGuest::Ping { epoch_secs: 0 },
        ]);
        assert_eq!(responses.len(), 3);
        for r in &responses {
            assert!(matches!(r, GuestToHost::Pong));
        }
    }

    #[test]
    fn control_loop_resize_changes_pty_winsize() {
        let (ctrl_read_fd, ctrl_write_fd) = make_pipe();
        let pty = openpty(None, None).expect("openpty");
        let master_fd = pty.master.as_raw_fd();
        let mut child = std::process::Command::new("sleep")
            .arg("300")
            .spawn()
            .expect("spawn sleep");
        let child_pid = Pid::from_raw(child.id() as i32);
        let (ctrl_tx, _ctrl_rx) = std::sync::mpsc::channel();

        // Send resize then close.
        let frame = capsem_proto::encode_host_msg(
            &HostToGuest::Resize { cols: 132, rows: 43 }
        ).unwrap();
        write_all_fd(ctrl_write_fd, &frame).unwrap();
        unsafe { libc::close(ctrl_write_fd); }

        let master_fd_check = master_fd;
        let handle = thread::spawn(move || {
            control_loop(ctrl_read_fd, master_fd, child_pid, &[], ctrl_tx);
        });
        handle.join().unwrap();

        // Verify the PTY was resized.
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::ioctl(master_fd_check, libc::TIOCGWINSZ, &mut ws) };
        assert_eq!(ret, 0);
        assert_eq!(ws.ws_col, 132);
        assert_eq!(ws.ws_row, 43);

        let _ = child.kill();
        let _ = child.wait();
        unsafe { libc::close(ctrl_read_fd); }
    }

    #[test]
    fn control_loop_file_write_path_traversal_rejected() {
        // Path traversal is rejected by validate_file_path (before workspace check),
        // so this works on macOS even though /root doesn't exist.
        let responses = run_control_loop_with_messages(vec![
            HostToGuest::FileWrite {
                id: 20,
                path: "/etc/../etc/passwd".into(),
                data: b"evil".to_vec(),
                mode: 0o644,
            },
        ]);
        assert_eq!(responses.len(), 1);
        match &responses[0] {
            GuestToHost::Error { id, message } => {
                assert_eq!(*id, 20);
                assert!(message.contains("rejected") || message.contains("traversal"),
                    "got: {message}");
            }
            other => panic!("expected Error for traversal, got {other:?}"),
        }
    }

    #[test]
    fn control_loop_file_read_rejected_outside_workspace() {
        // /etc/hostname is outside /root workspace, rejected by validate_file_path_safe
        // (or by workspace root canonicalization failure on macOS).
        let responses = run_control_loop_with_messages(vec![
            HostToGuest::FileRead {
                id: 10,
                path: "/etc/hostname".into(),
            },
        ]);
        assert_eq!(responses.len(), 1);
        match &responses[0] {
            GuestToHost::Error { id, .. } => assert_eq!(*id, 10),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn control_loop_file_delete_rejected_outside_workspace() {
        let responses = run_control_loop_with_messages(vec![
            HostToGuest::FileDelete {
                id: 30,
                path: "/tmp/some-file".into(),
            },
        ]);
        assert_eq!(responses.len(), 1);
        match &responses[0] {
            GuestToHost::Error { id, .. } => assert_eq!(*id, 30),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn control_loop_unhandled_message_does_not_crash() {
        // BootConfig is unexpected during control_loop (it's a boot-phase message).
        // control_loop should log it and continue.
        let responses = run_control_loop_with_messages(vec![
            HostToGuest::BootConfig { epoch_secs: 12345 },
            HostToGuest::Ping { epoch_secs: 0 },
        ]);
        // The BootConfig is just logged, only the Ping produces a response.
        assert_eq!(responses.len(), 1);
        assert!(matches!(responses[0], GuestToHost::Pong));
    }

    #[test]
    fn control_loop_eof_exits_cleanly() {
        // Empty message list = immediate EOF on the pipe = control_loop exits.
        let responses = run_control_loop_with_messages(vec![]);
        assert!(responses.is_empty());
    }

    // -------------------------------------------------------------------
    // Boot timing: exact boundary
    // -------------------------------------------------------------------

    #[test]
    fn parse_boot_timing_name_at_exact_boundary() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-boot-timing-boundary");
        let name_64 = "a".repeat(64); // exactly at limit, should pass
        std::fs::write(&path, format!(
            "{{\"name\":\"{name_64}\",\"duration_ms\":10}}\n"
        )).unwrap();
        let result = parse_boot_timing(path.to_str().unwrap());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, name_64);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_boot_timing_duration_at_exact_boundary() {
        let dir = std::env::temp_dir();
        let path = dir.join("capsem-test-boot-timing-dur-boundary");
        // 600_000 is exactly at limit, should pass
        std::fs::write(&path, "{\"name\":\"ok\",\"duration_ms\":600000}\n").unwrap();
        let result = parse_boot_timing(path.to_str().unwrap());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].duration_ms, 600_000);

        // 600_001 is over limit, should be rejected
        std::fs::write(&path, "{\"name\":\"bad\",\"duration_ms\":600001}\n").unwrap();
        let result = parse_boot_timing(path.to_str().unwrap());
        assert!(result.is_empty());
        std::fs::remove_file(&path).ok();
    }
}
