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
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

/// Find the assets directory containing kernel, initrd, and rootfs.
///
/// Checks (in order):
/// 1. `CAPSEM_ASSETS_DIR` env var (development override)
/// 2. macOS .app bundle: `Contents/Resources/` (sibling of `Contents/MacOS/`)
/// 3. `./assets` (workspace root, for `cargo run`)
/// 4. `../../assets` (when CWD is `crates/capsem-app/`)
fn resolve_assets_dir() -> Result<PathBuf> {
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

/// Build config, create VM, start it, and return the VM + serial receiver + input fd.
fn boot_vm(assets: &Path, cmdline: &str) -> Result<(VirtualMachine, broadcast::Receiver<String>, RawFd)> {
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
    let (mut vm, rx, input_fd) = VirtualMachine::create(&config).context("failed to create VM")?;
    vm.start().context("failed to start VM")?;
    Ok((vm, rx, input_fd))
}

/// Forward serial console lines to the Tauri frontend as events.
async fn serial_to_events(
    app_handle: tauri::AppHandle,
    mut rx: broadcast::Receiver<String>,
) {
    loop {
        match rx.recv().await {
            Ok(line) => {
                if let Err(e) = app_handle.emit("serial-output", &line) {
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
            Ok(line) => {
                if !got_first_output {
                    got_first_output = true;
                    first_output_time = Instant::now();
                }
                // If we see the banner, we know the shell is ready.
                if line.contains("sandbox ready") || line.contains("capsem") {
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

        loop {
            match rx.blocking_recv() {
                Ok(line) => {
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
/// Uses a native dialog (not Svelte) since the webview gets replaced with
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

    let default_level = if cli_args.is_empty() { "capsem=debug" } else { "capsem=warn" };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive(default_level.parse().unwrap()),
        )
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
                    return Ok(());
                }
            };

            info!("assets directory: {}", assets.display());

            // GUI mode: tty0 is primary console (framebuffer shows the shell)
            match boot_vm(&assets, "console=hvc0 console=tty0 loglevel=1") {
                Ok((vm, rx, input_fd)) => {
                    info!("VM booted successfully");

                    // Embed VZVirtualMachineView into the Tauri window
                    if let Some(webview_window) = app.get_webview_window("main") {
                        let mtm = MainThreadMarker::new()
                            .expect("setup hook must run on the main thread");
                        unsafe {
                            let ns_window: *mut std::ffi::c_void = webview_window.ns_window()
                                .expect("failed to get NSWindow");
                            let ns_window: &NSWindow = &*(ns_window as *const NSWindow);
                            let content_view = ns_window.contentView()
                                .expect("NSWindow has no content view");
                            let frame: NSRect = content_view.bounds();

                            let vm_view = VZVirtualMachineView::initWithFrame(
                                mtm.alloc(),
                                frame,
                            );
                            vm_view.setVirtualMachine(Some(vm.inner_vz()));
                            vm_view.setAutomaticallyReconfiguresDisplay(true);

                            // Resize with the window
                            vm_view.setAutoresizingMask(
                                NSAutoresizingMaskOptions::ViewWidthSizable
                                    | NSAutoresizingMaskOptions::ViewHeightSizable,
                            );

                            // Replace the webview entirely with the VM view
                            ns_window.setContentView(Some(&vm_view));
                            ns_window.makeFirstResponder(Some(&vm_view));
                        }
                    }

                    {
                        let app_state = app.state::<AppState>();
                        let mut vm_guard = app_state.vm.lock().unwrap();
                        *vm_guard = Some(vm);
                        let mut fd_guard = app_state.serial_input_fd.lock().unwrap();
                        *fd_guard = Some(input_fd);
                    }
                    let handle = app.handle().clone();
                    tauri::async_runtime::spawn(serial_to_events(handle, rx));
                }
                Err(e) => {
                    error!("VM boot failed: {e:#}");
                    info!("continuing without VM (unsigned binary or missing entitlement)");
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::vm_status, commands::serial_input])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
