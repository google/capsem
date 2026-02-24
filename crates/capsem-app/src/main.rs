#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use std::io::{Read, Write};
use std::mem::ManuallyDrop;
use std::os::unix::io::{FromRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use capsem_core::{
    CoalesceBuffer, ControlMessage, VirtualMachine, VmConfig, VsockManager, VSOCK_PORT_CONTROL,
    VSOCK_PORT_TERMINAL, decode_control_message, encode_control_message,
};
use state::{AppState, VmInstance};
use tauri::{Emitter, Manager};
use tokio::sync::broadcast;
use tracing::{debug_span, error, info, info_span, warn};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::EnvFilter;

/// Default VM ID for the single-VM case.
const DEFAULT_VM_ID: &str = "default";

/// Borrow a raw fd as a File without taking ownership.
///
/// The returned `ManuallyDrop<File>` will NOT close the fd when dropped,
/// so it's safe to use for fds owned by other objects (VsockConnection, pipes).
///
/// # Safety
/// The caller must ensure `fd` is a valid, open file descriptor for the
/// lifetime of the returned value.
pub(crate) unsafe fn borrow_fd(fd: RawFd) -> ManuallyDrop<std::fs::File> {
    ManuallyDrop::new(std::fs::File::from_raw_fd(fd))
}

/// Find the assets directory containing kernel, initrd, and rootfs.
///
/// Checks (in order):
/// 1. `CAPSEM_ASSETS_DIR` env var (development override)
/// 2. macOS .app bundle: `Contents/Resources/` (sibling of `Contents/MacOS/`)
/// 3. `./assets` (workspace root, for `cargo run`)
/// 4. `../../assets` (when CWD is `crates/capsem-app/`)
fn resolve_assets_dir() -> Result<PathBuf> {
    let _span = debug_span!("resolve_assets").entered();
    // 1. Explicit env var (development override)
    if let Ok(dir) = std::env::var("CAPSEM_ASSETS_DIR") {
        let p = PathBuf::from(dir);
        if p.join("vmlinuz").exists() {
            return Ok(p);
        }
    }

    // 2. macOS .app bundle: Contents/Resources/ (sibling of Contents/MacOS/)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(macos_dir) = exe.parent() {
            let resources = macos_dir.parent().map(|p| p.join("Resources"));
            if let Some(ref res) = resources {
                if res.join("vmlinuz").exists() {
                    return Ok(res.clone());
                }
            }
        }
    }

    // 3. ./assets (workspace root, for `cargo run`)
    let cwd_assets = PathBuf::from("assets");
    if cwd_assets.join("vmlinuz").exists() {
        return Ok(cwd_assets);
    }

    // 4. ../../assets (when CWD is crates/capsem-app/)
    let parent_assets = PathBuf::from("../../assets");
    if parent_assets.join("vmlinuz").exists() {
        return Ok(parent_assets);
    }

    Err(anyhow::anyhow!(
        "VM assets not found. Set CAPSEM_ASSETS_DIR or run from workspace root."
    ))
}

/// Boot performance log entry.
struct PerfEntry {
    stage: &'static str,
    elapsed_ms: f64,
}

/// Write boot performance data to ~/.capsem/perf/<timestamp>.log
fn write_perf_log(entries: &[PerfEntry]) {
    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => return,
    };
    let dir = home.join(".capsem").join("perf");
    let _ = std::fs::create_dir_all(&dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let path = dir.join(format!("{ts}.log"));
    let mut lines = Vec::new();
    for e in entries {
        let line = format!("{:<30} {:>10.3} ms", e.stage, e.elapsed_ms);
        eprintln!("{line}");
        lines.push(line);
    }
    let _ = std::fs::write(&path, lines.join("\n") + "\n");
    eprintln!("perf log: {}", path.display());
}

/// Build config, create VM, start it, and return the VM + serial receiver + input fd.
fn boot_vm(assets: &Path, cmdline: &str) -> Result<(VirtualMachine, broadcast::Receiver<Vec<u8>>, RawFd)> {
    let _span = info_span!("boot_vm").entered();
    let boot_start = Instant::now();
    let mut perf = Vec::new();

    let config = {
        let _span = debug_span!("config_build").entered();
        let t0 = Instant::now();
        let mut builder = VmConfig::builder()
            .cpu_count(2)
            .ram_bytes(512 * 1024 * 1024)
            .kernel_path(assets.join("vmlinuz"))
            .kernel_cmdline(cmdline);

        if let Some(hash) = option_env!("VMLINUZ_HASH") {
            builder = builder.expected_kernel_hash(hash);
        }

        if assets.join("initrd.img").exists() {
            builder = builder.initrd_path(assets.join("initrd.img"));
            if let Some(hash) = option_env!("INITRD_HASH") {
                builder = builder.expected_initrd_hash(hash);
            }
        }

        if assets.join("rootfs.img").exists() {
            builder = builder.disk_path(assets.join("rootfs.img"));
            if let Some(hash) = option_env!("ROOTFS_HASH") {
                builder = builder.expected_disk_hash(hash);
            }
        }

        let config = builder.build().context("failed to build VmConfig")?;
        perf.push(PerfEntry { stage: "config_build (incl hashing)", elapsed_ms: t0.elapsed().as_secs_f64() * 1000.0 });
        config
    };

    let (mut vm, rx, input_fd) = {
        let _span = debug_span!("vm_create").entered();
        let t0 = Instant::now();
        let result = VirtualMachine::create(&config).context("failed to create VM")?;
        perf.push(PerfEntry { stage: "vm_create", elapsed_ms: t0.elapsed().as_secs_f64() * 1000.0 });
        result
    };

    {
        let _span = debug_span!("vm_start").entered();
        let t0 = Instant::now();
        vm.start().context("failed to start VM")?;
        perf.push(PerfEntry { stage: "vm_start", elapsed_ms: t0.elapsed().as_secs_f64() * 1000.0 });
    }

    perf.push(PerfEntry { stage: "TOTAL boot_vm", elapsed_ms: boot_start.elapsed().as_secs_f64() * 1000.0 });
    write_perf_log(&perf);

    Ok((vm, rx, input_fd))
}

/// Forward serial console bytes to the Tauri frontend as events.
async fn serial_to_events(
    app_handle: tauri::AppHandle,
    mut rx: broadcast::Receiver<Vec<u8>>,
) {
    loop {
        match rx.recv().await {
            Ok(bytes) => {
                if let Err(e) = app_handle.emit("serial-output", &bytes) {
                    error!("failed to emit serial-output event: {e}");
                }
            }
            Err(broadcast::error::RecvError::Closed) => {
                info!("serial broadcast channel closed");
                break;
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                info!("serial receiver lagged by {n} messages");
            }
        }
    }
}

/// Forward vsock terminal data to the frontend with coalescing.
///
/// Reads raw bytes from the vsock fd in a blocking thread, then emits them
/// to the frontend. Coalesces output using `CoalesceBuffer` (8ms window,
/// 64KB cap) to prevent IPC saturation on high-throughput commands.
async fn vsock_terminal_to_events(app_handle: tauri::AppHandle, vsock_fd: RawFd) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);

    // Blocking reader thread: vsock fd -> channel
    std::thread::spawn(move || {
        // Safety: fd is valid for the lifetime of the VsockConnection.
        let mut file = unsafe { borrow_fd(vsock_fd) };
        let mut buf = [0u8; 8192];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut coalesce = CoalesceBuffer::new();
    loop {
        // Wait for the first chunk.
        match rx.recv().await {
            Some(chunk) => { coalesce.push(&chunk); }
            None => break,
        }

        // Coalesce additional chunks within the time window or until size cap.
        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(coalesce.window_ms());
        while !coalesce.is_full() {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(chunk)) => { coalesce.push(&chunk); }
                _ => break,
            }
        }

        coalesce.flush_to(|batch| {
            if let Err(e) = app_handle.emit("serial-output", batch) {
                error!("failed to emit vsock terminal data: {e}");
            }
        });
    }
}

/// Handle vsock control channel: read incoming messages, handle heartbeat.
async fn vsock_control_handler(app_handle: tauri::AppHandle, control_fd: RawFd) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ControlMessage>(32);

    // Blocking reader thread for control messages.
    std::thread::spawn(move || {
        // Safety: fd is valid for the lifetime of the VsockConnection.
        let mut file = unsafe { borrow_fd(control_fd) };
        loop {
            // Read length prefix.
            let mut len_buf = [0u8; 4];
            if file.read_exact(&mut len_buf).is_err() {
                break;
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            if len > 4096 {
                warn!("vsock control: frame too large ({len} bytes), dropping connection");
                break;
            }
            let mut payload = vec![0u8; len];
            if file.read_exact(&mut payload).is_err() {
                break;
            }
            match capsem_core::vsock::decode_control_message(&payload) {
                Ok(msg) => {
                    if tx.blocking_send(msg).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    warn!("vsock control: decode error: {e}");
                }
            }
        }
    });

    while let Some(msg) = rx.recv().await {
        match msg {
            ControlMessage::Ready { version } => {
                info!("vsock: guest agent ready (version {version})");
                let _ = app_handle.emit("terminal-source-changed", "vsock");
            }
            ControlMessage::Pong => {
                info!("vsock: heartbeat pong received");
            }
            other => {
                info!("vsock: unexpected control message: {other:?}");
            }
        }
    }
}

/// Set up vsock listeners and handle connections after VM boot.
///
/// Once vsock connects, the serial forwarding task is aborted since all
/// terminal I/O now flows through the vsock PTY bridge.
async fn setup_vsock(
    app_handle: tauri::AppHandle,
    mut vsock_manager: VsockManager,
    serial_task: tauri::async_runtime::JoinHandle<()>,
) {
    // Wait for both terminal and control connections from the guest agent.
    let mut terminal_conn = None;
    let mut control_conn = None;

    while terminal_conn.is_none() || control_conn.is_none() {
        match vsock_manager.accept().await {
            Some(conn) => {
                info!(port = conn.port, fd = conn.fd, "vsock: accepted connection");
                match conn.port {
                    VSOCK_PORT_TERMINAL => terminal_conn = Some(conn),
                    VSOCK_PORT_CONTROL => control_conn = Some(conn),
                    other => warn!("vsock: unexpected port {other}, ignoring"),
                }
            }
            None => {
                warn!("vsock: manager channel closed before all connections established");
                return;
            }
        }
    }

    let terminal = terminal_conn.unwrap();
    let control = control_conn.unwrap();

    info!("vsock: both channels connected, stopping serial forwarding");
    serial_task.abort();

    // Store vsock fds in app state so commands can use them.
    {
        let state = app_handle.state::<AppState>();
        let mut vms = state.vms.lock().unwrap();
        if let Some(instance) = vms.get_mut(DEFAULT_VM_ID) {
            instance.vsock_terminal_fd = Some(terminal.fd);
            instance.vsock_control_fd = Some(control.fd);
        }
    }

    // Spawn forwarding tasks.
    let handle1 = app_handle.clone();
    tokio::spawn(vsock_terminal_to_events(handle1, terminal.fd));
    tokio::spawn(vsock_control_handler(app_handle, control.fd));

    // Keep the connections alive by holding them here.
    // They'll be dropped when the task is cancelled (app shutdown).
    let _keep_terminal = terminal;
    let _keep_control = control;
    // Wait forever (connections are long-lived).
    std::future::pending::<()>().await;
}

const CLI_TIMEOUT: Duration = Duration::from_secs(120);

/// Read exactly `n` bytes from a raw fd, retrying on partial reads.
fn read_exact_fd(fd: RawFd, buf: &mut [u8]) -> std::io::Result<()> {
    // Safety: fd is valid for the duration of this call.
    let mut file = unsafe { borrow_fd(fd) };
    let mut pos = 0;
    while pos < buf.len() {
        let n = file.read(&mut buf[pos..])?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "unexpected EOF"));
        }
        pos += n;
    }
    Ok(())
}

/// Write all bytes to a raw fd.
fn write_all_fd(fd: RawFd, data: &[u8]) -> std::io::Result<()> {
    // Safety: fd is valid for the duration of this call.
    let mut file = unsafe { borrow_fd(fd) };
    file.write_all(data)?;
    Ok(())
}

/// Read one control message from an fd (blocking).
fn read_control_msg(fd: RawFd) -> Result<ControlMessage> {
    let mut len_buf = [0u8; 4];
    read_exact_fd(fd, &mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 4096 {
        anyhow::bail!("control frame too large ({len} bytes)");
    }
    let mut payload = vec![0u8; len];
    read_exact_fd(fd, &mut payload)?;
    decode_control_message(&payload).map_err(Into::into)
}

/// Write one control message to an fd.
fn write_control_msg(fd: RawFd, msg: &ControlMessage) -> Result<()> {
    let frame = encode_control_message(msg)?;
    write_all_fd(fd, &frame)?;
    Ok(())
}

fn run_cli(command: &str) -> Result<()> {
    let assets = resolve_assets_dir()?;
    let (vm, mut rx, _serial_input_fd) = boot_vm(&assets, "console=hvc0 loglevel=1")?;

    // Set up vsock listeners.
    let socket_devices = vm.socket_devices();
    let mut mgr = VsockManager::new(
        &socket_devices,
        &[VSOCK_PORT_CONTROL, VSOCK_PORT_TERMINAL],
    ).context("failed to set up vsock")?;

    // Print serial boot logs to stderr in a background thread.
    std::thread::spawn(move || {
        loop {
            match rx.blocking_recv() {
                Ok(bytes) => {
                    let _ = std::io::stderr().write_all(&bytes);
                    let _ = std::io::stderr().flush();
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    // Accept vsock connections with CFRunLoop pumping.
    // The VZ framework delivers connections via ObjC callbacks that require
    // CFRunLoop to be running on the main thread.
    let deadline = Instant::now() + CLI_TIMEOUT;
    let mut terminal_fd: Option<RawFd> = None;
    let mut control_fd: Option<RawFd> = None;
    let mut _conns = Vec::new(); // Keep connections alive.

    while terminal_fd.is_none() || control_fd.is_none() {
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for vsock connections from guest agent");
        }
        // Pump CFRunLoop to deliver ObjC callbacks.
        unsafe {
            core_foundation_sys::runloop::CFRunLoopRunInMode(
                core_foundation_sys::runloop::kCFRunLoopDefaultMode,
                0.05,
                0,
            );
        }
        // Check for accepted connections (non-blocking via try_recv on the channel).
        while let Ok(conn) = mgr.try_accept() {
            match conn.port {
                VSOCK_PORT_TERMINAL => terminal_fd = Some(conn.fd),
                VSOCK_PORT_CONTROL => control_fd = Some(conn.fd),
                _ => {}
            }
            _conns.push(conn);
        }
    }

    let terminal_fd = terminal_fd.unwrap();
    let control_fd = control_fd.unwrap();

    // Wait for Ready message from guest agent.
    let (ctrl_msg_tx, ctrl_msg_rx) = std::sync::mpsc::channel::<ControlMessage>();
    let ctrl_fd_reader = control_fd;
    std::thread::spawn(move || {
        loop {
            match read_control_msg(ctrl_fd_reader) {
                Ok(msg) => {
                    if ctrl_msg_tx.send(msg).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Wait for Ready, pumping CFRunLoop.
    loop {
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for guest agent Ready");
        }
        unsafe {
            core_foundation_sys::runloop::CFRunLoopRunInMode(
                core_foundation_sys::runloop::kCFRunLoopDefaultMode,
                0.05,
                0,
            );
        }
        match ctrl_msg_rx.try_recv() {
            Ok(ControlMessage::Ready { version }) => {
                eprintln!("[capsem] guest agent ready (v{version})");
                break;
            }
            Ok(other) => {
                eprintln!("[capsem] unexpected control message before Ready: {other:?}");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                anyhow::bail!("control channel closed before Ready");
            }
        }
    }

    // Send Exec command.
    let exec_id: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    write_control_msg(control_fd, &ControlMessage::Exec {
        id: exec_id,
        command: command.to_string(),
    })?;

    // Stream terminal output from vsock to stdout in a background thread.
    // Track whether the last byte written was a newline so we can add one
    // before exiting if needed.
    let last_was_newline = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let lwn = last_was_newline.clone();
    let terminal_reader = std::thread::spawn(move || {
        // Safety: fd is valid for the lifetime of the VsockConnection.
        let mut file = unsafe { borrow_fd(terminal_fd) };
        let mut buf = [0u8; 8192];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = std::io::stdout().write_all(&buf[..n]);
                    let _ = std::io::stdout().flush();
                    lwn.store(buf[n - 1] == b'\n', std::sync::atomic::Ordering::Relaxed);
                }
                Err(_) => break,
            }
        }
    });

    // Wait for ExecDone, pumping CFRunLoop.
    let exit_code;
    loop {
        if Instant::now() >= deadline {
            eprintln!("[capsem] timed out waiting for command completion");
            exit_code = 124; // Same as `timeout` command.
            break;
        }
        unsafe {
            core_foundation_sys::runloop::CFRunLoopRunInMode(
                core_foundation_sys::runloop::kCFRunLoopDefaultMode,
                0.05,
                0,
            );
        }
        match ctrl_msg_rx.try_recv() {
            Ok(ControlMessage::ExecDone { id, exit_code: code }) if id == exec_id => {
                exit_code = code;
                break;
            }
            Ok(other) => {
                eprintln!("[capsem] control message during exec: {other:?}");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                eprintln!("[capsem] control channel closed during exec");
                exit_code = 1;
                break;
            }
        }
    }

    // Stop VM and drop connections (closes vsock fds, unblocks the reader).
    let _ = vm.stop();
    drop(_conns);
    // Wait for terminal reader to drain remaining output.
    let _ = terminal_reader.join();
    // Ensure the host shell prompt starts on a fresh line.
    if !last_was_newline.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = std::io::stdout().write_all(b"\n");
        let _ = std::io::stdout().flush();
    }
    std::process::exit(exit_code);
}

/// Check for app updates using Tauri's updater plugin.
/// Uses a native dialog (not the WebView) since the webview gets replaced with
/// VZVirtualMachineView after VM boot.
async fn check_for_update(app: tauri::AppHandle) {
    use tauri_plugin_updater::UpdaterExt;
    use tauri_plugin_dialog::DialogExt;

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            info!("updater not available: {e:#}");
            return;
        }
    };

    let update = match updater.check().await {
        Ok(Some(update)) => update,
        Ok(None) => {
            info!("no update available");
            return;
        }
        Err(e) => {
            info!("update check failed: {e:#}");
            return;
        }
    };

    let current_version = app.package_info().version.to_string();
    let accepted = app
        .dialog()
        .message(format!(
            "Capsem {} is available (you have {}). Download and install?",
            update.version, current_version
        ))
        .title("Update Available")
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancel)
        .blocking_show();

    if accepted {
        if let Err(e) = update.download_and_install(|_, _| {}, || {}).await {
            error!("update failed: {e:#}");
        } else {
            app.restart();
        }
    }
}

fn main() {
    let cli_args: Vec<String> = std::env::args().skip(1).collect();

    let filter = match std::env::var("RUST_LOG") {
        Ok(_) => EnvFilter::from_default_env(),
        Err(_) => {
            let level = if cli_args.is_empty() { "debug" } else { "warn" };
            EnvFilter::new(format!("capsem={level},capsem_core={level}"))
        }
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_span_events(FmtSpan::CLOSE)
        .init();

    if !cli_args.is_empty() {
        let command = cli_args.join(" ");
        if let Err(e) = run_cli(&command) {
            eprintln!("capsem: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    info!("starting capsem");

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .setup(|app| {
            info!("tauri setup hook running");

            // Check for updates before booting the VM (the webview gets
            // replaced with VZVirtualMachineView after boot, so we use a
            // native dialog for the update prompt).
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                check_for_update(handle).await;
            });

            let assets = match resolve_assets_dir() {
                Ok(a) => a,
                Err(e) => {
                    error!("asset resolution failed: {e:#}");
                    info!("continuing without VM (frontend-only mode)");
                    let _ = app.handle().emit("vm-state-changed", "not created");
                    return Ok(());
                }
            };

            info!("assets directory: {}", assets.display());

            // Headless mode: hvc0 is primary console (routed to the frontend)
            match boot_vm(&assets, "console=hvc0 loglevel=1") {
                Ok((vm, rx, input_fd)) => {
                    info!("VM booted successfully");

                    // Register vsock listeners on the socket device.
                    let vsock_manager = {
                        let socket_devices = vm.socket_devices();
                        match VsockManager::new(
                            &socket_devices,
                            &[VSOCK_PORT_CONTROL, VSOCK_PORT_TERMINAL],
                        ) {
                            Ok(mgr) => Some(mgr),
                            Err(e) => {
                                warn!("vsock setup failed: {e:#}, using serial-only mode");
                                None
                            }
                        }
                    };

                    // Store VM state.
                    {
                        let app_state = app.state::<AppState>();
                        let mut vms = app_state.vms.lock().unwrap();
                        vms.insert(DEFAULT_VM_ID.to_string(), VmInstance {
                            vm,
                            serial_input_fd: input_fd,
                            vsock_terminal_fd: None,
                            vsock_control_fd: None,
                        });
                    }

                    let handle = app.handle().clone();
                    // Serial forwarding for boot logs (aborted once vsock connects).
                    let serial_task = tauri::async_runtime::spawn(
                        serial_to_events(handle.clone(), rx),
                    );

                    // Spawn vsock connection handler if available.
                    if let Some(mgr) = vsock_manager {
                        tauri::async_runtime::spawn(
                            setup_vsock(handle.clone(), mgr, serial_task),
                        );
                    }

                    // Push initial "running" state to frontend.
                    let _ = handle.emit("vm-state-changed", "running");
                }
                Err(e) => {
                    error!("VM boot failed: {e:#}");
                    info!("continuing without VM (unsigned binary or missing entitlement)");
                    let _ = app.handle().emit("vm-state-changed", "not created");
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::vm_status,
            commands::serial_input,
            commands::terminal_resize,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
