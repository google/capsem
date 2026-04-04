use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use anyhow::{Context, Result, anyhow};
use clap::Parser;
use tracing::{info, error};
use axum::{
    routing::{get, post, delete},
    extract::{Path, State},
    response::IntoResponse,
    Json, Router,
};
use tokio::net::UnixListener;
use tokio_unix_ipc::{channel_from_std, Sender, Receiver};
use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
use tower_http::trace::TraceLayer;
use serde_json::json;

mod api;
use api::*;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)] foreground: bool,
    #[arg(long)] uds_path: Option<PathBuf>,
    #[arg(long)] process_binary: Option<PathBuf>,
    #[arg(long)] assets_dir: Option<PathBuf>,
}

struct ServiceState {
    /// Map of instance ID to Process Info
    instances: Mutex<HashMap<String, InstanceInfo>>,
    process_binary: PathBuf,
    assets_dir: PathBuf,
    run_dir: PathBuf,
    job_counter: AtomicU64,
    /// Multi-version asset manager (currently unused in simplistic endpoints, keeping for future-proofing but suppressing warning)
    #[allow(dead_code)]
    asset_manager: Arc<capsem_core::asset_manager::AssetManager>,
    /// Current workspace version
    current_version: String,
}

struct InstanceInfo {
    id: String,
    pid: u32,
    uds_path: PathBuf,
    session_dir: PathBuf,
    ram_mb: u64,
    cpus: u32,
    #[allow(dead_code)]
    start_time: std::time::Instant,
    /// Base version of assets used for this VM
    base_version: String,
    /// Whether to remove session files when the process exits
    auto_remove: bool,
}

impl ServiceState {
    fn next_job_id(&self) -> u64 {
        self.job_counter.fetch_add(1, Ordering::Relaxed)
    }

    fn cleanup_stale_instances(&self) {
        let mut instances = self.instances.lock().unwrap();
        let mut dead_ids = Vec::new();
        for (id, info) in instances.iter() {
            // Use kill(0) to check if process exists. It returns Ok(()) if it does.
            let res = unsafe { nix::libc::kill(info.pid as i32, 0) };
            if res != 0 {
                // If it returns -1 and errno is ESRCH (No such process), it's dead.
                dead_ids.push(id.clone());
            }
        }
        for id in dead_ids {
            info!(id, "removing stale instance record");
            if let Some(info) = instances.remove(&id) {
                if info.auto_remove {
                    info!(id, "auto-removing session files");
                    let _ = std::fs::remove_dir_all(&info.session_dir);
                }
                let _ = std::fs::remove_file(&info.uds_path);
            }
        }
    }

    fn provision_sandbox(&self, id: &str, ram_mb: u64, cpus: u32, version_override: Option<String>, auto_remove: bool) -> Result<()> {
        self.cleanup_stale_instances();

        let vm_settings = capsem_core::net::policy_config::load_merged_vm_settings();
        let max_concurrent_vms = vm_settings.max_concurrent_vms.unwrap_or(10) as usize;

        // Resource limit validation (fallback reasonable defaults if not statically enforced by UI/config)
        // Ensure values are within safe boundaries: 1-8 CPUs, 256MB-16GB RAM.
        if cpus < 1 || cpus > 8 {
            return Err(anyhow!("cpus must be between 1 and 8"));
        }
        if ram_mb < 256 || ram_mb > 16384 {
            return Err(anyhow!("ram_mb must be between 256 and 16384"));
        }

        {
            let instances = self.instances.lock().unwrap();
            if instances.contains_key(id) {
                return Err(anyhow!("sandbox already exists: {}", id));
            }
            if instances.len() >= max_concurrent_vms {
                return Err(anyhow!("maximum number of concurrent VMs reached ({})", max_concurrent_vms));
            }
        }
        
        let version = version_override.unwrap_or_else(|| self.current_version.clone());
        
        info!(id, version, "provision_sandbox called");
        
        let uds_path = self.run_dir.join("instances").join(format!("{}.sock", id));

        // sun_path max: 104 bytes on macOS, 108 on Linux (both include null terminator).
        const SUN_PATH_MAX: usize = if cfg!(target_os = "macos") { 104 } else { 108 };
        let path_len = uds_path.as_os_str().len();
        if path_len >= SUN_PATH_MAX {
            return Err(anyhow!(
                "VM name '{}' produces a socket path of {} bytes, exceeding the OS limit of {}. Use a shorter name.",
                id, path_len, SUN_PATH_MAX - 1
            ));
        }

        let session_dir = self.run_dir.join("sessions").join(id);
        
        info!(uds_path = %uds_path.display(), "using uds_path");
        info!(session_dir = %session_dir.display(), "using session_dir");

        let _ = std::fs::create_dir_all(uds_path.parent().unwrap());
        let _ = std::fs::create_dir_all(&session_dir);

        // Assets are resolved per-version from ~/.capsem/assets/v{version}/
        let v_assets_dir = self.assets_dir.join(format!("v{}", version));
        info!(v_assets_dir = %v_assets_dir.display(), exists = v_assets_dir.exists(), "checking v_assets_dir");

        let mut assets_to_use = if v_assets_dir.exists() {
            v_assets_dir
        } else {
            info!(assets_dir = %self.assets_dir.display(), "falling back to assets_dir");
            self.assets_dir.clone()
        };

        // If rootfs doesn't exist in assets_to_use, check if it's arch-prefixed (e.g. assets/arm64)
        info!(check_rootfs = %assets_to_use.join("rootfs.squashfs").display(), "checking rootfs existence");
        if !assets_to_use.join("rootfs.squashfs").exists() {
            let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
            let arch_dir = assets_to_use.join(arch);
            info!(arch_dir = %arch_dir.display(), "checking arch_dir for rootfs");
            if arch_dir.join("rootfs.squashfs").exists() {
                info!(arch_dir = %arch_dir.display(), "found arch-specific assets");
                assets_to_use = arch_dir;
            }
        }

        let rootfs = assets_to_use.join("rootfs.squashfs");
        info!(final_rootfs = %rootfs.display(), exists = rootfs.exists(), "final rootfs check");
        
        if !rootfs.exists() {
            let entries = std::fs::read_dir(&assets_to_use)
                .map(|d| d.map(|e| e.unwrap().file_name()).collect::<Vec<_>>())
                .unwrap_or_default();
            error!(assets_path = %assets_to_use.display(), ?entries, "rootfs.squashfs NOT FOUND");
            return Err(anyhow!("rootfs.squashfs not found in {}. Dir entries: {:?}", assets_to_use.display(), entries));
        }

        info!(process_binary = %self.process_binary.display(), exists = self.process_binary.exists(), "checking process_binary");
        
        info!(id, version, assets = %assets_to_use.display(), "spawning capsem-process");

        let mut child_cmd = tokio::process::Command::new(&self.process_binary);
        if !self.process_binary.exists() {
             info!("process_binary does not exist at absolute path, trying target/debug/capsem-process");
             child_cmd = tokio::process::Command::new("target/debug/capsem-process");
        }

        let process_log_path = session_dir.join("process.log");
        let process_log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&process_log_path)
            .context("failed to open process.log")?;

        let mut child = child_cmd
            .env("RUST_LOG", "debug")
            .arg("--id").arg(id)
            .arg("--assets-dir").arg(&assets_to_use)
            .arg("--rootfs").arg(&rootfs)
            .arg("--session-dir").arg(&session_dir)
            .arg("--cpus").arg(cpus.to_string())
            .arg("--ram-mb").arg(ram_mb.to_string())
            .arg("--uds-path").arg(&uds_path)
            .stdout(std::process::Stdio::from(process_log_file.try_clone()?))
            .stderr(std::process::Stdio::from(process_log_file))
            .spawn()
            .context("failed to spawn capsem-process")?;

        let pid = child.id().unwrap_or(0);
        info!(id, pid, version, "capsem-process spawned");
        
        let id_clone = id.to_string();
        tokio::spawn(async move {
            let _ = child.wait().await;
            info!(id_clone, "capsem-process exited, reaping complete");
        });

        let mut instances = self.instances.lock().unwrap();
        instances.insert(id.to_string(), InstanceInfo {
            id: id.to_string(),
            pid,
            uds_path,
            session_dir,
            ram_mb,
            cpus,
            start_time: std::time::Instant::now(),
            base_version: version,
            auto_remove,
        });

        Ok(())
    }
}

use axum::http::StatusCode;

struct AppError(StatusCode, String);

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (
            self.0,
            Json(ErrorResponse {
                error: self.1,
            }),
        )
            .into_response()
    }
}

async fn handle_provision(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, AppError> {
    let id = payload.name.clone().unwrap_or_else(|| {
        format!("vm-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs())
    });

    match state.provision_sandbox(&id, payload.ram_mb, payload.cpus, Some(state.current_version.clone()), payload.auto_remove) {
        Ok(_) => Ok(Json(ProvisionResponse { id })),
        Err(e) => {
            error!(id, "provision failed: {e}");
            let status = if e.to_string().contains("already exists") {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            Err(AppError(status, format!("provision failed: {e}")))
        }
    }
}

async fn handle_list(
    State(state): State<Arc<ServiceState>>,
) -> Json<ListResponse> {
    state.cleanup_stale_instances();
    let instances = state.instances.lock().unwrap();
    let sandboxes = instances.values().map(|i| SandboxInfo {
        id: i.id.clone(),
        pid: i.pid,
        status: "Running".to_string(), 
        ram_mb: Some(i.ram_mb),
        cpus: Some(i.cpus),
        version: Some(i.base_version.clone()),
    }).collect();

    Json(ListResponse { sandboxes })
}

async fn handle_info(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<SandboxInfo>, AppError> {
    state.cleanup_stale_instances();
    let instances = state.instances.lock().unwrap();
    let i = instances.get(&id).ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
    
    Ok(Json(SandboxInfo {
        id: i.id.clone(),
        pid: i.pid,
        status: "Running".to_string(),
        ram_mb: Some(i.ram_mb),
        cpus: Some(i.cpus),
        version: Some(i.base_version.clone()),
    }))
}

async fn handle_logs(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<LogsResponse>, AppError> {
    let session_dir = {
        let instances = state.instances.lock().unwrap();
        let i = instances.get(&id).ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        i.session_dir.clone()
    };
    
    let serial_log_path = session_dir.join("serial.log");
    let process_log_path = session_dir.join("process.log");
    
    let serial_logs = std::fs::read_to_string(&serial_log_path).ok();
    let process_logs = std::fs::read_to_string(&process_log_path).ok();
        
    Ok(Json(LogsResponse { 
        logs: serial_logs.clone().unwrap_or_default(),
        serial_logs,
        process_logs,
    }))
}

async fn send_ipc_command(uds_path: &std::path::Path, cmd: ServiceToProcess, timeout_secs: u64) -> Result<ProcessToService, String> {
    let stream = tokio::net::UnixStream::connect(uds_path).await
        .map_err(|e| format!("failed to connect to sandbox: {e}"))?;
    let std_stream = stream.into_std()
        .map_err(|e| format!("failed to convert stream: {e}"))?;
    let (tx, rx): (Sender<ServiceToProcess>, Receiver<ProcessToService>) = channel_from_std(std_stream)
        .map_err(|e| format!("failed to create IPC channel: {e}"))?;

    tx.send(cmd.clone()).await
        .map_err(|e| format!("failed to send IPC command: {e}"))?;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(ProcessToService::Pong)) => {
                if matches!(cmd, ServiceToProcess::Ping | ServiceToProcess::ReloadConfig) {
                    return Ok(ProcessToService::Pong);
                }
                continue;
            }
            Ok(Ok(ProcessToService::TerminalOutput { .. })) => continue,
            Ok(Ok(ProcessToService::StateChanged { .. })) => continue,
            Ok(Ok(res)) => return Ok(res),
            Ok(Err(e)) => {
                error!(?e, "IPC receive error");
                return Err(format!("IPC connection closed: {e}"));
            }
            Err(_) => return Err(format!("IPC command timed out after {timeout_secs}s")),
        }
    }
}

async fn handle_exec(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<ExecRequest>,
) -> Result<Json<ExecResponse>, AppError> {
    let uds_path = {
        let instances = state.instances.lock().unwrap();
        let i = instances.get(&id).ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        i.uds_path.clone()
    };

    let id_val = state.next_job_id();
    let res = send_ipc_command(&uds_path, ServiceToProcess::Exec { id: id_val, command: payload.command }, payload.timeout_secs).await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    
    match res {
        ProcessToService::ExecResult { stdout, stderr, exit_code, .. } => {
            Ok(Json(ExecResponse {
                stdout: String::from_utf8_lossy(&stdout).to_string(),
                stderr: String::from_utf8_lossy(&stderr).to_string(),
                exit_code,
            }))
        }
        _ => Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, "unexpected IPC response for exec".to_string())),
    }
}

async fn handle_write_file(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<WriteFileRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let uds_path = {
        let instances = state.instances.lock().unwrap();
        let i = instances.get(&id).ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        i.uds_path.clone()
    };

    let id_val = state.next_job_id();
    let data = payload.content.into_bytes();
    let res = send_ipc_command(&uds_path, ServiceToProcess::WriteFile { id: id_val, path: payload.path, data }, 30).await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    
    match res {
        ProcessToService::WriteFileResult { success, error, .. } => {
            if success { Ok(Json(json!({ "success": true }))) }
            else { Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, error.unwrap_or_else(|| "unknown write error".into()))) }
        }
        _ => Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, "unexpected IPC response for write_file".to_string())),
    }
}

async fn handle_read_file(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<ReadFileRequest>,
) -> Result<Json<ReadFileResponse>, AppError> {
    let path = &payload.path;
    let uds_path = {
        let instances = state.instances.lock().unwrap();
        let i = instances.get(&id).ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        i.uds_path.clone()
    };

    let id_val = state.next_job_id();
    let res = send_ipc_command(&uds_path, ServiceToProcess::ReadFile { id: id_val, path: path.clone() }, 30).await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    
    match res {
        ProcessToService::ReadFileResult { data, error, .. } => {
            if let Some(d) = data {
                Ok(Json(ReadFileResponse { content: String::from_utf8_lossy(&d).to_string() }))
            } else {
                Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, error.unwrap_or_else(|| "unknown read error".into())))
            }
        }
        _ => Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, "unexpected IPC response for read_file".to_string())),
    }
}

async fn handle_reload_config(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.cleanup_stale_instances();
    
    // Collect paths to broadcast to.
    let uds_paths = {
        let instances = state.instances.lock().unwrap();
        instances.iter().map(|(id, info)| (id.clone(), info.uds_path.clone())).collect::<Vec<_>>()
    };
    
    let mut failures = Vec::new();
    
    for (id, uds_path) in uds_paths.iter() {
        match send_ipc_command(uds_path, ServiceToProcess::ReloadConfig, 5).await {
            Ok(ProcessToService::Pong) => {} // Expected response
            Ok(_) => failures.push(format!("{id}: unexpected response")),
            Err(e) => failures.push(format!("{id}: {e}")),
        }
    }
    
    if failures.is_empty() {
        Ok(Json(serde_json::json!({ "success": true, "reloaded": uds_paths.len() })))
    } else {
        Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to reload config in some instances: {}", failures.join(", ")),
        ))
    }
}

async fn handle_inspect(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<InspectRequest>,
) -> Result<impl IntoResponse, AppError> {
    let db_path = {
        let instances = state.instances.lock().unwrap();
        let i = instances.get(&id).ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        i.session_dir.join("session.db")
    };
    
    let reader = capsem_logger::DbReader::open(&db_path)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to open DB: {e}")))?;
        
    let json_str = reader.query_raw(&payload.sql)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("query failed: {e}")))?;
        
    Ok((
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        json_str
    ))
}

async fn handle_delete(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (uds_path, session_dir, pid) = {
        let instances = state.instances.lock().unwrap();
        let i = instances.get(&id).ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        (i.uds_path.clone(), i.session_dir.clone(), i.pid)
    };

    // Shutdown the process before removing from the map.
    let stream_res = tokio::net::UnixStream::connect(&uds_path).await;
    if let Ok(stream) = stream_res {
        if let Ok(std_stream) = stream.into_std() {
            if let Ok((tx, _)) = channel_from_std::<ServiceToProcess, ProcessToService>(std_stream) {
                let _ = tx.send(ServiceToProcess::Shutdown).await;
            }
        }
    } else if pid > 0 {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGTERM);
    }

    // Give it a brief moment to write its DBs before deleting its files
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    if pid > 0 {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGKILL);
    }

    // Now remove from map and clean up.
    state.instances.lock().unwrap().remove(&id);
    let _ = std::fs::remove_dir_all(&session_dir);
    let _ = std::fs::remove_file(&uds_path);

    Ok(Json(json!({ "success": true })))
}

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, fmt};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    let home = std::env::var("HOME").context("HOME not set")?;
    let run_dir = std::env::var("CAPSEM_RUN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(&home).join(".capsem/run"));
    
    let _ = std::fs::create_dir_all(&run_dir);

    let log_path = run_dir.join("service.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().json().with_writer(Arc::new(log_file)))
        .init();

    info!("capsem-service starting up");
    info!(args = ?args, run_dir = %run_dir.display(), "environment initialized");

    let instances_dir = run_dir.join("instances");
    let sessions_dir = run_dir.join("sessions");
    let _ = std::fs::create_dir_all(&instances_dir);
    let _ = std::fs::create_dir_all(&sessions_dir);

    let service_sock = args.uds_path.unwrap_or_else(|| run_dir.join("service.sock"));
    if service_sock.exists() {
        let _ = std::fs::remove_file(&service_sock);
    }

    let process_binary = args.process_binary.unwrap_or_else(|| PathBuf::from("target/debug/capsem-process"));
    let assets_base_dir = args.assets_dir.unwrap_or_else(|| run_dir.parent().unwrap().join("assets"));

    // Determine arch for manifest lookup
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };

    // Initialize AssetManager from manifest.json. 
    let manifest_path = if assets_base_dir.join("manifest.json").exists() {
        assets_base_dir.join("manifest.json")
    } else {
        assets_base_dir.parent().unwrap().join("manifest.json")
    };
    
    let manifest_content = std::fs::read_to_string(&manifest_path)
        .context(format!("failed to read manifest from {}", manifest_path.display()))?;
    
    let manifest = capsem_core::asset_manager::Manifest::from_json_for_arch(&manifest_content, arch)
        .or_else(|_| capsem_core::asset_manager::Manifest::from_json(&manifest_content))?;
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    
    let asset_manager = capsem_core::asset_manager::AssetManager::from_manifest(
        &manifest, &current_version, assets_base_dir.clone(), Some(arch)
    )?;

    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        process_binary,
        assets_dir: assets_base_dir,
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        asset_manager: Arc::new(asset_manager),
        current_version,
    });

    info!("scanning for existing sandboxes in {}", instances_dir.display());
    if let Ok(entries) = std::fs::read_dir(&instances_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "sock" {
                    // Stale socket from previous run, remove it
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    let app = Router::new()
        .route("/provision", post(handle_provision))
        .route("/list", get(handle_list))
        .route("/info/{id}", get(handle_info))
        .route("/logs/{id}", get(handle_logs))
        .route("/inspect/{id}", post(handle_inspect))
        .route("/exec/{id}", post(handle_exec))
        .route("/write_file/{id}", post(handle_write_file))
        .route("/read_file/{id}", post(handle_read_file))
        .route("/delete/{id}", delete(handle_delete))
        .route("/reload-config", post(handle_reload_config))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    info!(socket = %service_sock.display(), "listening on UDS");

    let uds = UnixListener::bind(&service_sock).context("failed to bind UDS")?;
    axum::serve(uds, app).await.context("server error")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    fn make_test_state() -> ServiceState {
        let dummy_hash = "a".repeat(64);
        let manifest_json = format!(
            r#"{{"latest":"0.0.0","releases":{{"0.0.0":{{"assets":[{{"filename":"dummy.img","hash":"{}","size":0}}]}}}}}}"#,
            dummy_hash
        );
        let manifest = capsem_core::asset_manager::Manifest::from_json(&manifest_json).unwrap();
        let am = capsem_core::asset_manager::AssetManager::from_manifest(
            &manifest, "0.0.0", PathBuf::from("/tmp/capsem-test-assets"), None
        ).unwrap();
        ServiceState {
            instances: Mutex::new(HashMap::new()),
            process_binary: PathBuf::from("/nonexistent/capsem-process"),
            assets_dir: PathBuf::from("/nonexistent/assets"),
            run_dir: PathBuf::from("/tmp/capsem-test-svc"),
            job_counter: AtomicU64::new(1),
            asset_manager: Arc::new(am),
            current_version: "0.0.0".into(),
        }
    }

    fn insert_fake_instance(state: &ServiceState, id: &str, pid: u32) {
        state.instances.lock().unwrap().insert(
            id.to_string(),
            InstanceInfo {
                id: id.to_string(),
                pid,
                uds_path: PathBuf::from(format!("/tmp/{}.sock", id)),
                session_dir: PathBuf::from(format!("/tmp/{}", id)),
                ram_mb: 2048,
                cpus: 2,
                start_time: std::time::Instant::now(),
                base_version: "0.0.0".into(),
                auto_remove: false,
            },
        );
    }

    // -----------------------------------------------------------------------
    // next_job_id
    // -----------------------------------------------------------------------

    #[test]
    fn next_job_id_starts_at_1() {
        let state = make_test_state();
        assert_eq!(state.next_job_id(), 1);
    }

    #[test]
    fn next_job_id_increments() {
        let state = make_test_state();
        let a = state.next_job_id();
        let b = state.next_job_id();
        let c = state.next_job_id();
        assert_eq!(b, a + 1);
        assert_eq!(c, a + 2);
    }

    #[test]
    fn next_job_id_unique_across_many() {
        let state = make_test_state();
        let ids: Vec<u64> = (0..1000).map(|_| state.next_job_id()).collect();
        let unique: std::collections::HashSet<u64> = ids.iter().copied().collect();
        assert_eq!(unique.len(), 1000);
    }

    // -----------------------------------------------------------------------
    // Instance map CRUD
    // -----------------------------------------------------------------------

    #[test]
    fn instance_insert_and_lookup() {
        let state = make_test_state();
        insert_fake_instance(&state, "test-vm", std::process::id());
        let instances = state.instances.lock().unwrap();
        assert!(instances.contains_key("test-vm"));
        assert_eq!(instances["test-vm"].ram_mb, 2048);
    }

    #[test]
    fn instance_remove() {
        let state = make_test_state();
        insert_fake_instance(&state, "test-vm", std::process::id());
        state.instances.lock().unwrap().remove("test-vm");
        assert!(!state.instances.lock().unwrap().contains_key("test-vm"));
    }

    #[test]
    fn instance_lookup_missing() {
        let state = make_test_state();
        assert!(!state.instances.lock().unwrap().contains_key("no-such-vm"));
    }

    #[test]
    fn instance_count() {
        let state = make_test_state();
        insert_fake_instance(&state, "vm-1", std::process::id());
        insert_fake_instance(&state, "vm-2", std::process::id());
        insert_fake_instance(&state, "vm-3", std::process::id());
        assert_eq!(state.instances.lock().unwrap().len(), 3);
    }

    // -----------------------------------------------------------------------
    // cleanup_stale_instances
    // -----------------------------------------------------------------------

    #[test]
    fn cleanup_removes_dead_pid() {
        let state = make_test_state();
        // PID 99999999 should not exist
        insert_fake_instance(&state, "dead-vm", 99999999);
        assert_eq!(state.instances.lock().unwrap().len(), 1);
        state.cleanup_stale_instances();
        assert_eq!(state.instances.lock().unwrap().len(), 0);
    }

    #[test]
    fn cleanup_keeps_live_pid() {
        let state = make_test_state();
        // Current process PID should be alive
        insert_fake_instance(&state, "live-vm", std::process::id());
        state.cleanup_stale_instances();
        assert_eq!(state.instances.lock().unwrap().len(), 1);
    }

    #[test]
    fn cleanup_mixed_live_and_dead() {
        let state = make_test_state();
        insert_fake_instance(&state, "live", std::process::id());
        insert_fake_instance(&state, "dead", 99999999);
        state.cleanup_stale_instances();
        let instances = state.instances.lock().unwrap();
        assert_eq!(instances.len(), 1);
        assert!(instances.contains_key("live"));
    }

    // -----------------------------------------------------------------------
    // Auto-ID generation format
    // -----------------------------------------------------------------------

    #[test]
    fn auto_id_format() {
        // Verify the auto-ID pattern used in handle_provision
        let id = format!(
            "vm-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        );
        assert!(id.starts_with("vm-"));
        // Should be "vm-" followed by digits
        let suffix = &id[3..];
        assert!(suffix.chars().all(|c| c.is_ascii_digit()));
    }

    // -----------------------------------------------------------------------
    // AppError
    // -----------------------------------------------------------------------

    #[test]
    fn app_error_formats_json() {
        let err = AppError(StatusCode::NOT_FOUND, "sandbox not found".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn app_error_internal_server() {
        let err = AppError(StatusCode::INTERNAL_SERVER_ERROR, "boom".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn app_error_conflict() {
        let err = AppError(StatusCode::CONFLICT, "already exists".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    // -----------------------------------------------------------------------
    // Input validation edge cases (DTO level)
    // -----------------------------------------------------------------------

    #[test]
    fn provision_request_no_name() {
        let json = serde_json::json!({"ram_mb": 2048, "cpus": 2});
        let req: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert!(req.name.is_none());
    }

    #[test]
    fn provision_request_empty_name() {
        let json = serde_json::json!({"name": "", "ram_mb": 2048, "cpus": 2});
        let req: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.name.unwrap(), "");
    }

    #[test]
    fn provision_request_name_with_path_separator() {
        // This is a security edge case -- names with / could create path traversal
        let json = serde_json::json!({"name": "../escape", "ram_mb": 2048, "cpus": 2});
        let req: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.name.unwrap(), "../escape");
        // Note: the service SHOULD reject this, but currently doesn't validate
    }

    #[test]
    fn exec_request_empty_command() {
        let json = serde_json::json!({"command": ""});
        let req: ExecRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.command, "");
    }

    #[test]
    fn exec_request_shell_metacharacters() {
        let json = serde_json::json!({"command": "echo $(whoami) && rm -rf /"});
        let req: ExecRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.command, "echo $(whoami) && rm -rf /");
    }

    #[test]
    fn write_file_request_path_traversal() {
        let json = serde_json::json!({"path": "../../etc/passwd", "content": "evil"});
        let req: WriteFileRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.path, "../../etc/passwd");
        // Note: no validation at DTO level -- relies on guest-side enforcement
    }

    #[test]
    fn inspect_request_sql_injection() {
        let json = serde_json::json!({"sql": "SELECT * FROM net_events; DROP TABLE net_events; --"});
        let req: InspectRequest = serde_json::from_value(json).unwrap();
        assert!(req.sql.contains("DROP TABLE"));
        // Note: backend should use read-only DB connection to prevent writes
    }

    // -----------------------------------------------------------------------
    // Asset path resolution
    // -----------------------------------------------------------------------

    #[test]
    fn asset_version_path_construction() {
        let base = PathBuf::from("/home/user/.capsem/assets");
        let version = "0.16.1";
        let v_path = base.join(format!("v{}", version));
        assert_eq!(v_path, PathBuf::from("/home/user/.capsem/assets/v0.16.1"));
    }

    #[test]
    fn arch_detection_aarch64() {
        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
        assert!(arch == "arm64" || arch == "x86_64");
    }

    // -----------------------------------------------------------------------
    // UDS path length validation (macOS 104, Linux 108 including null)
    // -----------------------------------------------------------------------

    #[test]
    fn provision_rejects_vm_name_exceeding_uds_path_limit() {
        let state = make_test_state();
        // run_dir = /tmp/capsem-test-svc
        // path   = /tmp/capsem-test-svc/instances/{name}.sock
        // A 100-char name will blow past either OS limit.
        let long_name = "a".repeat(100);
        let result = state.provision_sandbox(&long_name, 2048, 2, None, false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("socket path"), "expected socket path error, got: {err}");
    }

    #[test]
    fn provision_rejects_name_at_exact_boundary() {
        let state = make_test_state();
        // Compute the prefix: /tmp/capsem-test-svc/instances/ + .sock
        let prefix = state.run_dir.join("instances").join("").as_os_str().len();
        let suffix_len = ".sock".len();
        let sun_path_max: usize = if cfg!(target_os = "macos") { 104 } else { 108 };
        // Name length that makes total path == sun_path_max (one byte over usable limit)
        let name_len = sun_path_max - prefix - suffix_len;
        let boundary_name = "x".repeat(name_len);
        let result = state.provision_sandbox(&boundary_name, 2048, 2, None, false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("socket path"), "expected socket path error at boundary, got: {err}");
    }

    #[test]
    fn provision_accepts_name_just_under_uds_limit() {
        let state = make_test_state();
        let prefix = state.run_dir.join("instances").join("").as_os_str().len();
        let suffix_len = ".sock".len();
        let sun_path_max: usize = if cfg!(target_os = "macos") { 104 } else { 108 };
        // One byte shorter than the limit -- should pass path validation
        let name_len = sun_path_max - prefix - suffix_len - 1;
        let ok_name = "x".repeat(name_len);
        let result = state.provision_sandbox(&ok_name, 2048, 2, None, false);
        // Will fail later (missing rootfs), but NOT for path length
        if let Err(e) = &result {
            let msg = e.to_string();
            assert!(!msg.contains("socket path"), "short name should not hit path limit: {msg}");
        }
    }

    #[test]
    fn provision_short_name_passes_path_check() {
        let state = make_test_state();
        let result = state.provision_sandbox("my-vm", 2048, 2, None, false);
        // Fails for missing assets, not path length
        if let Err(e) = &result {
            let msg = e.to_string();
            assert!(!msg.contains("socket path"), "normal name should not hit path limit: {msg}");
        }
    }
}
