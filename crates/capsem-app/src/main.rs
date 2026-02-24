#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use std::io::Write;
use std::os::unix::io::{FromRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use capsem_core::{VirtualMachine, VmConfig};
use state::AppState;
use tauri::{Emitter, Manager};
use tokio::sync::broadcast;
use tracing::{debug_span, error, info, info_span};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::EnvFilter;

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

/// Strip ANSI escape sequences from a string.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip escape sequence: ESC [ ... final_byte
            if let Some(next) = chars.next() {
                if next == '[' {
                    for ch in chars.by_ref() {
                        if ch.is_ascii_alphabetic() || ch == 'm' {
                            break;
                        }
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

const CLI_START_MARKER: &str = "<<<CAPSEM_START>>>";
const CLI_DONE_MARKER: &str = "<<<CAPSEM_DONE>>>";
const CLI_TIMEOUT: Duration = Duration::from_secs(30);

fn run_cli(command: &str) -> Result<()> {
    let assets = resolve_assets_dir()?;
    // CLI mode: hvc0 is primary console (serial output routes to terminal)
    let (vm, mut rx, input_fd) = boot_vm(&assets, "console=tty0 console=hvc0")?;

    // The initramfs loads virtio_console.ko as a module, then drops to a
    // shell via break=modules.  We must wait for the shell to be ready
    // before writing commands.  We subscribe a second receiver to watch
    // for the prompt on the raw line stream, and pass the primary receiver
    // to the output-parsing thread.
    let (done_tx, done_rx) = std::sync::mpsc::channel();

    // Wait for the shell to be ready. Look for the welcome banner or
    // fall back to waiting for serial output to settle.
    let wait_deadline = Instant::now() + CLI_TIMEOUT;
    let mut got_first_output = false;
    let mut first_output_time = Instant::now();
    loop {
        if Instant::now() >= wait_deadline {
            anyhow::bail!("timed out waiting for shell");
        }
        // Pump the CFRunLoop so the VM can produce serial data.
        unsafe {
            core_foundation_sys::runloop::CFRunLoopRunInMode(
                core_foundation_sys::runloop::kCFRunLoopDefaultMode,
                0.05,
                0,
            );
        }
        match rx.try_recv() {
            Ok(bytes) => {
                let line = String::from_utf8_lossy(&bytes);
                print!("{}", line); std::io::Write::flush(&mut std::io::stdout()).unwrap();
                if !got_first_output {
                    got_first_output = true;
                    first_output_time = Instant::now();
                }
                // If we see the banner, we know the shell is ready.
                if line.contains("sandbox ready") {
                    std::thread::sleep(Duration::from_millis(200));
                    while rx.try_recv().is_ok() {}
                    break;
                }
            }
            Err(broadcast::error::TryRecvError::Empty) => {
                // If we already got output and it's been quiet for 1s, shell is ready.
                if got_first_output
                    && first_output_time.elapsed() > Duration::from_secs(1)
                {
                    while rx.try_recv().is_ok() {}
                    break;
                }
            }
            Err(broadcast::error::TryRecvError::Closed) => {
                anyhow::bail!("serial channel closed before shell started");
            }
            Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
        }
    }

    // Now write the command wrapped in sentinel markers.
    {
        let mut pipe = unsafe { std::fs::File::from_raw_fd(input_fd) };
        write!(
            pipe,
            "echo '{}'\n{}\necho '{}'\n",
            CLI_START_MARKER, command, CLI_DONE_MARKER
        )?;
        std::mem::forget(pipe);
    }

    // Parse output between markers in a background thread.
    std::thread::spawn(move || {
        let deadline = Instant::now() + CLI_TIMEOUT;
        let mut inside_output = false;
        let mut partial = String::new();

        loop {
            match rx.blocking_recv() {
                Ok(bytes) => {
                    let chunk = String::from_utf8_lossy(&bytes);
                    partial.push_str(&chunk);

                    while let Some(pos) = partial.find('\n') {
                        let line = partial[..pos].to_string();
                        let rest = partial[pos + 1..].to_string();
                        partial = rest;

                        let clean = strip_ansi(line.trim_end_matches('\r'));
                        if !inside_output {
                            if clean.contains(CLI_START_MARKER) && !clean.contains("echo") {
                                inside_output = true;
                            }
                            continue;
                        }
                        if clean.contains(CLI_DONE_MARKER) && !clean.contains("echo") {
                            let _ = done_tx.send(Ok(()));
                            return;
                        }
                        // Skip lines that contain the shell prompt (echoed commands).
                        let trimmed = clean.trim();
                        if trimmed.contains("capsem:/") {
                            continue;
                        }
                        if trimmed.is_empty() {
                            continue;
                        }
                        println!("{}", trimmed);
                    }

                    if Instant::now() >= deadline {
                        let _ = done_tx.send(Err(anyhow::anyhow!(
                            "timed out waiting for command output"
                        )));
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    let _ = done_tx.send(Ok(()));
                    return;
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    info!("serial receiver lagged by {n} messages");
                }
            }
        }
    });

    // Pump the CFRunLoop on the main thread until the reader thread signals done.
    loop {
        match done_rx.try_recv() {
            Ok(result) => {
                result?;
                break;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                unsafe {
                    core_foundation_sys::runloop::CFRunLoopRunInMode(
                        core_foundation_sys::runloop::kCFRunLoopDefaultMode,
                        0.05,
                        0,
                    );
                }
            }
        }
    }

    let _ = vm.stop();
    Ok(())
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

                    {
                        let app_state = app.state::<AppState>();
                        let mut vm_guard = app_state.vm.lock().unwrap();
                        *vm_guard = Some(vm);
                        let mut fd_guard = app_state.serial_input_fd.lock().unwrap();
                        *fd_guard = Some(input_fd);
                    }
                    let handle = app.handle().clone();
                    tauri::async_runtime::spawn(serial_to_events(handle.clone(), rx));

                    // Push initial "running" state to frontend
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
        .invoke_handler(tauri::generate_handler![commands::vm_status, commands::serial_input])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
