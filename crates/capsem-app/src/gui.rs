use std::path::{Path, PathBuf};
use std::sync::Arc;

use capsem_core::{
    VirtioFsShare, VsockManager,
    VSOCK_PORT_CONTROL, VSOCK_PORT_MCP_GATEWAY, VSOCK_PORT_SNI_PROXY, VSOCK_PORT_TERMINAL,
};
use capsem_core::mcp::gateway::McpGatewayConfig;
use capsem_core::mcp::server_manager::McpServerManager;
use capsem_core::net::policy_config;
use tauri::{Emitter, Manager};
use tracing::{error, info, warn};

use crate::boot::boot_vm;
use crate::boot::create_net_state;
use crate::session_mgmt::{session_dir_for, open_session_db};
use crate::state::{AppState, VmInstance};
use crate::vsock_wiring::{serial_to_events, setup_vsock, wire_auto_snapshots, spawn_auto_snapshot_timer};

/// Check for app updates using Tauri's updater plugin.
pub(crate) async fn check_for_update(app: tauri::AppHandle) {
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

/// Boot the VM and set up all subsystems (vsock, serial, MITM proxy, MCP gateway).
/// Called either immediately from the setup hook (rootfs available in bundle) or
/// after async rootfs download completes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn gui_boot_vm(
    handle: &tauri::AppHandle,
    assets: &Path,
    rootfs: Option<&Path>,
    session_id: &str,
    scratch_path: Option<PathBuf>,
    virtiofs_shares: Vec<VirtioFsShare>,
    cpu_count: u32,
    ram_bytes: u64,
) {
    match boot_vm(assets, rootfs, "console=hvc0 ro loglevel=1 init_on_alloc=1 slab_nomerge page_alloc.shuffle=1", scratch_path.as_deref(), &virtiofs_shares, cpu_count, ram_bytes) {
        Ok((vm, rx, input_fd, sm)) => {
            info!("VM booted successfully");

            // Register vsock listeners on the socket device.
            let vsock_manager = {
                let socket_devices = vm.socket_devices();
                match VsockManager::new(
                    &socket_devices,
                    &[VSOCK_PORT_CONTROL, VSOCK_PORT_TERMINAL, VSOCK_PORT_SNI_PROXY, VSOCK_PORT_MCP_GATEWAY],
                ) {
                    Ok(mgr) => Some(mgr),
                    Err(e) => {
                        warn!("vsock setup failed: {e:#}, using serial-only mode");
                        None
                    }
                }
            };

            // Open session DB (independently of MITM proxy state).
            let gui_session_db = match open_session_db(session_id) {
                Ok(db) => db,
                Err(e) => {
                    error!("failed to open session db: {e:#}");
                    return;
                }
            };

            // Create per-VM network state (CA + policy for MITM proxy).
            let net_state = match create_net_state(session_id, Arc::clone(&gui_session_db)) {
                Ok(ns) => Some(ns),
                Err(e) => {
                    warn!("MITM proxy disabled: {e:#}");
                    None
                }
            };

            // Create MCP gateway config for vsock:5003 using MergedPolicies.
            let gui_policies = policy_config::MergedPolicies::from_disk();
            let (gui_user_sf, gui_corp_sf) = policy_config::load_settings_files();
            let gui_user_mcp = gui_user_sf.mcp.clone().unwrap_or_default();
            let gui_corp_mcp = gui_corp_sf.mcp.clone().unwrap_or_default();
            let mcp_servers = capsem_core::mcp::build_server_list(&gui_user_mcp, &gui_corp_mcp);
            let mcp_config: Option<Arc<McpGatewayConfig>> = {
                let http_client = reqwest::Client::builder()
                    .user_agent("capsem-mcp/0.8")
                    .timeout(std::time::Duration::from_secs(30))
                    .redirect(reqwest::redirect::Policy::limited(10))
                    .build()
                    .expect("reqwest client");
                Some(Arc::new(McpGatewayConfig {
                    server_manager: tokio::sync::Mutex::new(McpServerManager::new(mcp_servers.clone(), http_client.clone())),
                    db: Arc::clone(&gui_session_db),
                    policy: tokio::sync::RwLock::new(Arc::new(gui_policies.mcp)),
                    domain_policy: std::sync::RwLock::new(Arc::new(gui_policies.domain)),
                    http_client,
                    auto_snapshots: None,
                    workspace_dir: session_dir_for(session_id).map(|d| d.join("workspace")),
                }))
            };

            // Store MCP config on AppState for Tauri commands (call_mcp_tool).
            if let Some(ref config) = mcp_config {
                let app_state = handle.state::<AppState>();
                *app_state.mcp_config.lock().unwrap() = Some(Arc::clone(config));
            }

            // Initialize MCP servers in background (non-blocking in GUI mode).
            if let Some(ref config) = mcp_config {
                let config = Arc::clone(config);
                let h = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let mut mgr = config.server_manager.lock().await;
                    if let Err(e) = mgr.initialize_all().await {
                        tracing::error!("MCP server initialization failed: {e:#}");
                        let _ = h.emit("mcp-init-failed", format!("{e:#}"));
                        return;
                    }
                    // Tool cache pinning (detect rug pulls).
                    let cache = capsem_core::mcp::load_tool_cache();
                    let changes = capsem_core::mcp::detect_pin_changes(mgr.tool_catalog(), &cache);
                    for change in &changes {
                        match change {
                            capsem_core::mcp::PinChange::Changed { namespaced_name, .. } => {
                                tracing::warn!(tool = %namespaced_name, "MCP tool definition changed (possible rug pull)");
                            }
                            capsem_core::mcp::PinChange::New { namespaced_name } => {
                                tracing::info!(tool = %namespaced_name, "new MCP tool discovered");
                            }
                            capsem_core::mcp::PinChange::Removed { namespaced_name } => {
                                tracing::info!(tool = %namespaced_name, "MCP tool removed");
                            }
                        }
                    }
                    let new_cache = capsem_core::mcp::build_cache_entries(mgr.tool_catalog(), &cache);
                    if let Err(e) = capsem_core::mcp::save_tool_cache(&new_cache) {
                        tracing::warn!("failed to save MCP tool cache: {e}");
                    }
                });
            }

            // Start auto-snapshot scheduler and file monitor in VirtioFS mode.
            let mut fs_monitor: Option<capsem_core::fs_monitor::FsMonitor> = None;
            if !virtiofs_shares.is_empty() {
                if let Some(ref dir) = session_dir_for(session_id) {
                    // Wire auto-snapshot scheduler into MCP config.
                    if let Some(ref config) = mcp_config {
                        let config = Arc::clone(config);
                        let dir = dir.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Some((scheduler, interval)) = wire_auto_snapshots(&config, &dir).await {
                                spawn_auto_snapshot_timer(&tokio::runtime::Handle::current(), scheduler, interval);
                            }
                        });
                    }

                    // Start host file monitor.
                    let workspace = dir.join("workspace");
                    fs_monitor = match capsem_core::fs_monitor::FsMonitor::start(
                        workspace.clone(),
                        workspace.clone(),
                        Arc::clone(&gui_session_db),
                    ) {
                        Ok(monitor) => {
                            info!("host file monitor started");
                            Some(monitor)
                        }
                        Err(e) => {
                            warn!("failed to start host file monitor: {e}");
                            None
                        }
                    };
                }
            }

            // Store VM state.
            {
                let app_state = handle.state::<AppState>();
                let mut vms = app_state.vms.lock().unwrap();
                vms.insert(session_id.to_string(), VmInstance {
                    _vm: vm,
                    serial_input_fd: input_fd,
                    vsock_terminal_fd: None,
                    vsock_control_fd: None,
                    net_state,
                    mcp_state: mcp_config.clone(),
                    state_machine: sm,
                    _scratch_disk_path: scratch_path,
                    _fs_monitor: fs_monitor,
                });
            }

            // Reset the terminal output queue for the new session.
            {
                let app_state = handle.state::<AppState>();
                app_state.terminal_output.reset();
            }

            // Serial forwarding for boot logs (aborted once vsock connects).
            let serial_output = {
                let app_state = handle.state::<AppState>();
                Arc::clone(&app_state.terminal_output)
            };
            let serial_task = tauri::async_runtime::spawn(
                serial_to_events(serial_output, rx),
            );

            // Spawn vsock connection handler if available.
            let h = handle.clone();
            if let Some(mgr) = vsock_manager {
                tauri::async_runtime::spawn(
                    setup_vsock(h.clone(), mgr, serial_task),
                );
            }

            // Push initial state to frontend (Booting, not yet Running).
            let _ = h.emit("vm-state-changed", serde_json::json!({
                "state": "Booting",
                "trigger": "vm_started",
            }));
        }
        Err(e) => {
            error!("VM boot failed: {e:#}");
            info!("continuing without VM (unsigned binary or missing entitlement)");
            let _ = handle.emit("vm-state-changed", serde_json::json!({
                "state": "Error",
                "trigger": "boot_failed",
                "message": format!("{e:#}"),
            }));
        }
    }
}
