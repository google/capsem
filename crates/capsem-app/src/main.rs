#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod boot;
mod commands;
mod gui;
mod logging;
mod session_mgmt;
mod state;
mod vsock_wiring;

use std::path::PathBuf;

use capsem_core::{VirtioFsShare, VmState, create_virtiofs_session};
use capsem_core::asset_manager;
use capsem_core::net::policy_config;
use capsem_core::session::{self, SessionIndex, SessionRecord};
use capsem_core::log_layer::TauriLogLayer;
use tauri::{Emitter, Manager};
use tracing::{error, info, warn};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

fn main() {
    // Global filter: always at least debug so file/UI layers capture boot events.
    let filter = EnvFilter::new("capsem=debug,capsem_core=debug");
    let stdout_filter = match std::env::var("RUST_LOG") {
        Ok(_) => EnvFilter::from_default_env(),
        Err(_) => EnvFilter::new("capsem=debug,capsem_core=debug"),
    };

    // Per-launch log file: ~/.capsem/logs/<timestamp>.jsonl
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let log_dir = PathBuf::from(&home).join(".capsem").join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    logging::cleanup_old_logs(&log_dir, 7);

    let launch_ts = {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs();
        let t = secs % 86400;
        let days = secs / 86400;
        // Simplified date from days since epoch
        let z = days as i64 + 719468;
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let doe = (z - era * 146097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        format!("{y:04}-{m:02}-{d:02}T{:02}-{:02}-{:02}", t / 3600, (t % 3600) / 60, t % 60)
    };

    let log_file = std::fs::File::create(log_dir.join(format!("{launch_ts}.jsonl")));
    let (_non_blocking_guard, file_layer) = match log_file {
        Ok(f) => {
            let (non_blocking, guard) = tracing_appender::non_blocking(f);
            let layer = Some(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(non_blocking)
                    .with_span_events(FmtSpan::CLOSE),
            );
            (Some(guard), layer)
        }
        Err(_) => (None, None),
    };

    // Layer 1: stdout (CLI uses warn to avoid noise, GUI uses debug)
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_span_events(FmtSpan::CLOSE)
        .with_filter(stdout_filter);

    // Layer 3: Tauri event emitter + per-VM file (deferred)
    let (tauri_layer, log_handle) = TauriLogLayer::new();

    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .with(tauri_layer)
        .init();

    // Open session index early (shared by CLI and GUI paths).
    let session_index = match session_mgmt::sessions_dir() {
        Some(d) => {
            let _ = std::fs::create_dir_all(&d);
            match SessionIndex::open(&d.join("main.db")) {
                Ok(idx) => idx,
                Err(e) => {
                    eprintln!("capsem: failed to open session index: {e}");
                    std::process::exit(1);
                }
            }
        }
        None => {
            eprintln!("capsem: HOME not set, cannot create session index");
            std::process::exit(1);
        }
    };

    info!("starting capsem");

    // Clean up stale sessions from previous runs.
    info!("[boot-audit] cleaning stale sessions");
    session_mgmt::cleanup_stale_sessions(&session_index);
    info!("[boot-audit] stale sessions cleaned");

    info!("[boot-audit] building tauri app");
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state::AppState::new(session_index, Some(log_handle)))
        .setup(|app| {
            info!("[boot-audit] tauri setup hook entered");

            // Inject Tauri event emitter into the log layer.
            {
                let app_state = app.state::<state::AppState>();
                if let Some(ref lh) = app_state.log_handle {
                    let handle = app.handle().clone();
                    lh.set_emitter(move |event| {
                        let _ = handle.emit("log-event", &event);
                    });
                }
            }
            info!("[boot-audit] log emitter injected");

            // Check for updates before booting the VM.
            let auto_update = {
                info!("[boot-audit] loading settings for auto-update check");
                let settings = policy_config::load_merged_settings();
                settings.iter()
                    .find(|s| s.id == "app.auto_update")
                    .and_then(|s| s.effective_value.as_bool())
                    .unwrap_or(true)
            };
            info!("[boot-audit] auto_update={auto_update}");
            if auto_update {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    gui::check_for_update(handle).await;
                });
            }

            info!("[boot-audit] resolving assets directory");
            let assets = match assets::resolve_assets_dir() {
                Ok(a) => a,
                Err(e) => {
                    error!("asset resolution failed: {e:#}");
                    info!("continuing without VM (frontend-only mode)");
                    {
                        let app_state = app.state::<state::AppState>();
                        *app_state.app_status.lock().unwrap() = VmState::Error.to_string();
                    }
                    let _ = app.handle().emit("vm-state-changed", serde_json::json!({
                        "state": "Error",
                        "trigger": "assets_not_found",
                        "message": format!("{e:#}"),
                    }));
                    return Ok(());
                }
            };

            info!("[boot-audit] assets directory: {}", assets.display());

            // Generate unique session ID for this boot.
            let gui_session_id = session::generate_session_id();
            info!("[boot-audit] session_id={gui_session_id}");

            // Create session directory with VirtioFS overlay for GUI mode.
            info!("[boot-audit] loading VM settings");
            let vm_settings = policy_config::load_merged_vm_settings();
            let cpu_count = vm_settings.cpu_count.unwrap_or(4);
            let ram_gb = vm_settings.ram_gb.unwrap_or(4);
            let ram_bytes: u64 = ram_gb as u64 * 1024 * 1024 * 1024;
            info!("[boot-audit] VM config: cpu={cpu_count} ram_gb={ram_gb}");

            info!("[boot-audit] creating session directory");
            let gui_session_dir = session_mgmt::session_dir_for(&gui_session_id);
            let gui_virtiofs_shares: Vec<VirtioFsShare> = gui_session_dir
                .as_ref()
                .and_then(|d| {
                    std::fs::create_dir_all(d).ok();
                    if let Err(e) = create_virtiofs_session(d, 2) {
                        warn!("failed to create VirtioFS session dir: {e}");
                        return None;
                    }
                    info!("[boot-audit] VirtioFS session dir created at {}", d.display());
                    Some(vec![VirtioFsShare {
                        tag: "capsem".to_string(),
                        host_path: d.clone(),
                        read_only: false,
                    }])
                })
                .unwrap_or_default();

            // Open per-VM log file for structured event capture.
            if let Some(ref dir) = gui_session_dir {
                let app_state = app.state::<state::AppState>();
                if let Some(ref lh) = app_state.log_handle {
                    if let Ok(f) = std::fs::File::create(dir.join("capsem.log")) {
                        lh.set_vm_writer(f);
                    }
                }
            }

            // Record session in main.db.
            info!("[boot-audit] recording session in index");
            {
                let app_state = app.state::<state::AppState>();
                let idx = app_state.session_index.lock().unwrap();
                let record = SessionRecord {
                    id: gui_session_id.clone(),
                    mode: "gui".to_string(),
                    command: None,
                    status: VmState::Running.to_string(),
                    created_at: session::now_iso(),
                    stopped_at: None,
                    scratch_disk_size_gb: 0,
                    ram_bytes,
                    total_requests: 0,
                    allowed_requests: 0,
                    denied_requests: 0,
                    total_input_tokens: 0,
                    total_output_tokens: 0,
                    total_estimated_cost: 0.0,
                    total_tool_calls: 0,
                    total_mcp_calls: 0,
                    total_file_events: 0,
                    compressed_size_bytes: None,
                    vacuumed_at: None,
                    storage_mode: if gui_virtiofs_shares.is_empty() { "block" } else { "virtiofs" }.to_string(),
                    rootfs_hash: None, // Will be populated when assets are fully resolved if possible
                    rootfs_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    source_image: None, // GUI currently doesn't support launching from images
                    persistent: false, // GUI currently only launches ephemeral VMs
                };
                if let Err(e) = idx.create_session(&record) {
                    warn!("failed to record session: {e}");
                }
                // Set active session ID.
                *app_state.active_session_id.lock().unwrap() = Some(gui_session_id.clone());
            }
            info!("[boot-audit] session recorded");

            // Resolve rootfs: check bundled assets dir, then ~/.capsem/assets/.
            info!("[boot-audit] resolving rootfs");
            let rootfs_path = assets::resolve_rootfs(&assets);
            info!("[boot-audit] rootfs resolved: {}", rootfs_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "none".to_string()));

            if rootfs_path.is_some() {
                // Rootfs available -- boot immediately on main thread.
                info!("[boot-audit] rootfs available, calling gui_boot_vm");
                {
                    let app_state = app.state::<state::AppState>();
                    *app_state.app_status.lock().unwrap() = VmState::Booting.to_string();
                }
                gui::gui_boot_vm(gui::GuiBootOptions {
                    handle: app.handle(),
                    assets: &assets,
                    rootfs: rootfs_path.as_deref(),
                    session_id: &gui_session_id,
                    scratch_path: None,
                    virtiofs_shares: gui_virtiofs_shares.clone(),
                    cpu_count,
                    ram_bytes,
                });
                info!("[boot-audit] gui_boot_vm returned");
            } else {
                // Rootfs not found -- download it first.
                info!("[boot-audit] rootfs not found, initiating download");
                {
                    let app_state = app.state::<state::AppState>();
                    *app_state.app_status.lock().unwrap() = VmState::Downloading.to_string();
                }
                let _ = app.handle().emit("vm-state-changed", serde_json::json!({
                    "state": "Downloading",
                    "trigger": "rootfs_missing",
                }));

                let handle = app.handle().clone();
                let assets_clone = assets.clone();
                let session_id = gui_session_id.clone();
                let vfs_shares = gui_virtiofs_shares;
                tauri::async_runtime::spawn(async move {
                    let mgr = match assets::create_asset_manager(&assets_clone) {
                        Ok(m) => m,
                        Err(e) => {
                            error!("asset manager init failed: {e:#}");
                            {
                                let state = handle.state::<state::AppState>();
                                *state.app_status.lock().unwrap() = VmState::Error.to_string();
                            }
                            let _ = handle.emit("vm-state-changed", serde_json::json!({
                                "state": "Error",
                                "trigger": "asset_init_failed",
                                "message": format!("{e:#}"),
                            }));
                            return;
                        }
                    };

                    let name = match assets::rootfs_manifest_name(&mgr) {
                        Ok(n) => n,
                        Err(e) => {
                            error!("rootfs not in manifest: {e:#}");
                            {
                                let state = handle.state::<state::AppState>();
                                *state.app_status.lock().unwrap() = VmState::Error.to_string();
                            }
                            let _ = handle.emit("vm-state-changed", serde_json::json!({
                                "state": "Error",
                                "trigger": "manifest_error",
                                "message": format!("{e:#}"),
                            }));
                            return;
                        }
                    };
                    info!(asset = %name, "starting rootfs download");

                    // Clean up stale assets from previous versions.
                    let _ = mgr.cleanup_unrecognized();

                    let h2 = handle.clone();
                    let client = reqwest::Client::new();
                    match mgr.download_asset(&name, &client, move |progress| {
                        let _ = h2.emit("download-progress", &progress);
                    }).await {
                        Ok(rootfs) => {
                            info!(path = %rootfs.display(), "rootfs download complete");
                            // Clean up old version directories.
                            if let Some(base) = asset_manager::default_assets_dir() {
                                let version = env!("CARGO_PKG_VERSION");
                                if let Err(e) = asset_manager::cleanup_old_versions(&base, version, None) {
                                    warn!("cleanup old versions failed: {e:#}");
                                }
                            }
                            info!("dispatching VM boot to main thread");
                            {
                                let state = handle.state::<state::AppState>();
                                *state.app_status.lock().unwrap() = VmState::Booting.to_string();
                            }
                            let h = handle.clone();
                            let a = assets_clone.clone();
                            let s = session_id.clone();
                            let r = rootfs.clone();
                            if let Err(e) = handle.run_on_main_thread(move || {
                                gui::gui_boot_vm(gui::GuiBootOptions {
                                    handle: &h,
                                    assets: &a,
                                    rootfs: Some(&r),
                                    session_id: &s,
                                    scratch_path: None,
                                    virtiofs_shares: vfs_shares,
                                    cpu_count,
                                    ram_bytes,
                                });
                            }) {
                                error!("failed to dispatch boot to main thread: {e}");
                            }
                        }
                        Err(e) => {
                            error!("rootfs download failed: {e:#}");
                            {
                                let state = handle.state::<state::AppState>();
                                *state.app_status.lock().unwrap() = VmState::Error.to_string();
                            }
                            let _ = handle.emit("vm-state-changed", serde_json::json!({
                                "state": "Error",
                                "trigger": "download_failed",
                                "message": format!("{e:#}"),
                            }));
                        }
                    }
                });
            }

            info!("[boot-audit] tauri setup hook complete");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::vm_status,
            commands::serial_input,
            commands::terminal_poll,
            commands::terminal_resize,
            commands::get_guest_config,
            commands::get_network_policy,
            commands::set_guest_env,
            commands::remove_guest_env,
            commands::get_vm_state,
            commands::get_settings,
            commands::get_settings_tree,
            commands::lint_config,
            commands::list_presets,
            commands::apply_preset,
            commands::update_setting,
            commands::load_settings,
            commands::save_settings,
            commands::get_session_info,
            commands::query_db,
            commands::get_mcp_servers,
            commands::get_mcp_tools,
            commands::get_mcp_policy,
            commands::set_mcp_server_enabled,
            commands::add_mcp_server,
            commands::remove_mcp_server,
            commands::set_mcp_global_policy,
            commands::set_mcp_default_permission,
            commands::set_mcp_tool_permission,
            commands::approve_mcp_tool,
            commands::refresh_mcp_tools,
            commands::open_url,
            commands::detect_host_config,
            commands::validate_api_key,
            commands::check_for_app_update,
            commands::load_session_log,
            commands::list_log_sessions,
            commands::call_mcp_tool,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
