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
use serde::{Deserialize, Serialize};
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

// ---------------------------------------------------------------------------
// Persistent VM registry (JSON-backed)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PersistentVmEntry {
    name: String,
    ram_mb: u64,
    cpus: u32,
    base_version: String,
    created_at: String,
    session_dir: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    source_image: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct PersistentRegistryData {
    vms: HashMap<String, PersistentVmEntry>,
}

struct PersistentRegistry {
    path: PathBuf,
    data: PersistentRegistryData,
}

impl PersistentRegistry {
    fn load(path: PathBuf) -> Self {
        let data = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, data }
    }

    fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.data)?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }

    fn register(&mut self, entry: PersistentVmEntry) -> Result<()> {
        if self.data.vms.contains_key(&entry.name) {
            return Err(anyhow!("persistent VM \"{}\" already exists. Use resume to reconnect.", entry.name));
        }
        self.data.vms.insert(entry.name.clone(), entry);
        self.save()
    }

    fn unregister(&mut self, name: &str) -> Result<()> {
        self.data.vms.remove(name);
        self.save()
    }

    fn get(&self, name: &str) -> Option<&PersistentVmEntry> {
        self.data.vms.get(name)
    }

    fn list(&self) -> impl Iterator<Item = &PersistentVmEntry> {
        self.data.vms.values()
    }

    fn contains(&self, name: &str) -> bool {
        self.data.vms.contains_key(name)
    }
}

/// Validate that a persistent VM name is safe for use as a directory name.
fn validate_vm_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(anyhow!("VM name cannot be empty"));
    }
    if name.len() > 64 {
        return Err(anyhow!("VM name too long (max 64 characters)"));
    }
    if !name.chars().next().unwrap().is_ascii_alphanumeric() {
        return Err(anyhow!("VM name must start with a letter or digit"));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(anyhow!("VM name must contain only letters, digits, hyphens, and underscores"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Service state
// ---------------------------------------------------------------------------

struct ServiceState {
    /// Map of instance ID to Process Info
    instances: Mutex<HashMap<String, InstanceInfo>>,
    /// Registry of persistent (named) VMs
    persistent_registry: Mutex<PersistentRegistry>,
    image_registry: Arc<capsem_core::image::ImageRegistry>,
    process_binary: PathBuf,
    assets_dir: PathBuf,
    run_dir: PathBuf,
    job_counter: AtomicU64,
    #[allow(dead_code)]
    asset_manager: Arc<capsem_core::asset_manager::AssetManager>,
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
    base_version: String,
    /// Whether this is a persistent (named) VM
    persistent: bool,
    /// Environment variables injected at boot
    #[allow(dead_code)]
    env: Option<std::collections::HashMap<String, String>>,
    /// Image from which this VM was booted, if any
    source_image: Option<String>,
}

impl ServiceState {
    fn next_job_id(&self) -> u64 {
        self.job_counter.fetch_add(1, Ordering::Relaxed)
    }

    fn cleanup_stale_instances(&self) {
        let mut instances = self.instances.lock().unwrap();
        let mut dead_ids = Vec::new();
        for (id, info) in instances.iter() {
            let res = unsafe { nix::libc::kill(info.pid as i32, 0) };
            if res != 0 {
                dead_ids.push(id.clone());
            }
        }
        for id in dead_ids {
            info!(id, "removing stale instance record");
            if let Some(info) = instances.remove(&id) {
                if info.persistent {
                    // Persistent VMs: preserve session dir, just clean up socket
                    info!(id, "persistent VM process died, preserving session dir");
                } else {
                    // Ephemeral VMs: clean up everything
                    info!(id, "ephemeral VM process died, removing session files");
                    let _ = std::fs::remove_dir_all(&info.session_dir);
                }
                let _ = std::fs::remove_file(&info.uds_path);
            }
        }
    }

    fn provision_sandbox(
        self: &Arc<Self>,
        id: &str,
        ram_mb: u64,
        cpus: u32,
        version_override: Option<String>,
        persistent: bool,
        env: Option<std::collections::HashMap<String, String>>,
        image: Option<String>,
    ) -> Result<()> {
        self.cleanup_stale_instances();

        let vm_settings = capsem_core::net::policy_config::load_merged_vm_settings();
        let max_concurrent_vms = vm_settings.max_concurrent_vms.unwrap_or(10) as usize;

        if cpus < 1 || cpus > 8 {
            return Err(anyhow!("cpus must be between 1 and 8"));
        }
        if ram_mb < 256 || ram_mb > 16384 {
            return Err(anyhow!("ram_mb must be between 256 and 16384"));
        }

        // Persistent VMs: validate name and reject duplicates
        if persistent {
            validate_vm_name(id)?;
            let registry = self.persistent_registry.lock().unwrap();
            if registry.contains(id) {
                return Err(anyhow!("persistent VM \"{}\" already exists. Use `capsem resume {}` to reconnect.", id, id));
            }
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

        // Validate image if provided
        let image_entry = if let Some(img_name) = &image {
            let entry = self.image_registry.get(img_name)
                .context("failed to query image registry")?
                .ok_or_else(|| anyhow!("image '{}' not found", img_name))?;
            Some(entry)
        } else {
            None
        };

        // If booting from an image, override the base_version to match the image's base_version.
        let version = if let Some(ref entry) = image_entry {
            entry.base_version.clone()
        } else {
            version_override.unwrap_or_else(|| self.current_version.clone())
        };

        info!(id, version, persistent, image, "provision_sandbox called");

        let uds_path = self.run_dir.join("instances").join(format!("{}.sock", id));

        const SUN_PATH_MAX: usize = if cfg!(target_os = "macos") { 104 } else { 108 };
        let path_len = uds_path.as_os_str().len();
        if path_len >= SUN_PATH_MAX {
            return Err(anyhow!(
                "VM name '{}' produces a socket path of {} bytes, exceeding the OS limit of {}. Use a shorter name.",
                id, path_len, SUN_PATH_MAX - 1
            ));
        }

        // Persistent VMs go in persistent/, ephemeral in sessions/
        let session_dir = if persistent {
            self.run_dir.join("persistent").join(id)
        } else {
            self.run_dir.join("sessions").join(id)
        };

        info!(uds_path = %uds_path.display(), "using uds_path");
        info!(session_dir = %session_dir.display(), "using session_dir");

        let _ = std::fs::create_dir_all(uds_path.parent().unwrap());
        let _ = std::fs::create_dir_all(&session_dir);

        // If booting from an image, clone the image state into the new session directory
        if let Some(ref img_name) = image {
            info!(image = img_name, session_dir = %session_dir.display(), "cloning session from image");
            capsem_core::image::create_session_from_image(&self.image_registry, img_name, &session_dir)
                .context("failed to clone session from image")?;
        }

        let assets_to_use = self.resolve_assets_dir(&version)?;
        let rootfs = assets_to_use.join("rootfs.squashfs");
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

        // Inject VM identity so the guest knows its own name/ID.
        child_cmd.arg("--env").arg(format!("CAPSEM_VM_ID={}", id));
        child_cmd.arg("--env").arg(format!("CAPSEM_VM_NAME={}", id));

        // Add --env KEY=VALUE args for each user-specified env var
        if let Some(ref env_vars) = env {
            for (k, v) in env_vars {
                child_cmd.arg("--env").arg(format!("{}={}", k, v));
            }
        }

        let mut child = child_cmd
            .env("RUST_LOG", "capsem=info")
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
        let state_clone = Arc::clone(self);
        let uds_clone = uds_path.clone();
        tokio::spawn(async move {
            let _ = child.wait().await;
            info!(id_clone, "capsem-process exited, cleaning up");
            // Remove from active instances so the service knows this VM is gone.
            let removed = state_clone.instances.lock().unwrap().remove(&id_clone);
            let _ = std::fs::remove_file(&uds_clone);
            if let Some(info) = removed {
                if !info.persistent {
                    let _ = std::fs::remove_dir_all(&info.session_dir);
                    info!(id = id_clone, "ephemeral session cleaned up");
                }
            }
        });

        // Register persistent VM in the registry
        if persistent {
            let mut registry = self.persistent_registry.lock().unwrap();
            registry.register(PersistentVmEntry {
                name: id.to_string(),
                ram_mb,
                cpus,
                base_version: version.clone(),
                created_at: format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()),
                session_dir: session_dir.clone(),
                source_image: image.clone(),
            })?;
        }

        let mut instances = self.instances.lock().unwrap();
        instances.insert(id.to_string(), InstanceInfo {
            id: id.to_string(),
            pid,
            uds_path,
            session_dir: session_dir.clone(),
            ram_mb,
            cpus,
            start_time: std::time::Instant::now(),
            base_version: version.clone(),
            persistent,
            env,
            source_image: image.clone(),
        });

        Ok(())
    }

    /// Resume a stopped persistent VM by re-spawning capsem-process against its
    /// existing session directory.
    fn resume_sandbox(self: &Arc<Self>, name: &str, ram_mb_override: Option<u64>, cpus_override: Option<u32>) -> Result<String> {
        self.cleanup_stale_instances();

        // Check if already running
        {
            let instances = self.instances.lock().unwrap();
            if instances.contains_key(name) {
                return Ok(name.to_string()); // Already running, just return ID
            }
        }

        let entry = {
            let registry = self.persistent_registry.lock().unwrap();
            registry.get(name).cloned().ok_or_else(|| anyhow!("no persistent VM named \"{}\"", name))?
        };

        if !entry.session_dir.exists() {
            return Err(anyhow!("session directory for \"{}\" is missing", name));
        }

        let ram_mb = ram_mb_override.unwrap_or(entry.ram_mb);
        let cpus = cpus_override.unwrap_or(entry.cpus);
        let version = entry.base_version.clone();

        info!(name, version, "resume_sandbox: re-spawning process");

        let uds_path = self.run_dir.join("instances").join(format!("{}.sock", name));
        let _ = std::fs::create_dir_all(uds_path.parent().unwrap());

        let assets_to_use = self.resolve_assets_dir(&version)?;
        let rootfs = assets_to_use.join("rootfs.squashfs");
        if !rootfs.exists() {
            return Err(anyhow!("rootfs.squashfs not found in {}", assets_to_use.display()));
        }

        let process_log_path = entry.session_dir.join("process.log");
        let process_log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&process_log_path)
            .context("failed to open process.log")?;

        let mut child_cmd = tokio::process::Command::new(&self.process_binary);
        if !self.process_binary.exists() {
            child_cmd = tokio::process::Command::new("target/debug/capsem-process");
        }

        // Inject VM identity so the guest knows its own name/ID.
        child_cmd.arg("--env").arg(format!("CAPSEM_VM_ID={}", name));
        child_cmd.arg("--env").arg(format!("CAPSEM_VM_NAME={}", name));

        let mut child = child_cmd
            .env("RUST_LOG", "capsem=info")
            .arg("--id").arg(name)
            .arg("--assets-dir").arg(&assets_to_use)
            .arg("--rootfs").arg(&rootfs)
            .arg("--session-dir").arg(&entry.session_dir)
            .arg("--cpus").arg(cpus.to_string())
            .arg("--ram-mb").arg(ram_mb.to_string())
            .arg("--uds-path").arg(&uds_path)
            .stdout(std::process::Stdio::from(process_log_file.try_clone()?))
            .stderr(std::process::Stdio::from(process_log_file))
            .spawn()
            .context("failed to spawn capsem-process")?;

        let pid = child.id().unwrap_or(0);
        info!(name, pid, "capsem-process resumed");

        let name_clone = name.to_string();
        let state_clone = Arc::clone(self);
        let uds_clone = uds_path.clone();
        tokio::spawn(async move {
            let _ = child.wait().await;
            info!(name_clone, "capsem-process exited, cleaning up");
            // Persistent VMs: remove from instances but keep session dir.
            state_clone.instances.lock().unwrap().remove(&name_clone);
            let _ = std::fs::remove_file(&uds_clone);
        });

        let mut instances = self.instances.lock().unwrap();
        instances.insert(name.to_string(), InstanceInfo {
            id: name.to_string(),
            pid,
            uds_path,
            session_dir: entry.session_dir.clone(),
            ram_mb,
            cpus,
            start_time: std::time::Instant::now(),
            base_version: version,
            persistent: true,
            env: None,
            source_image: entry.source_image.clone(),
        });

        Ok(name.to_string())
    }

    /// Resolve versioned assets directory.
    fn resolve_assets_dir(&self, version: &str) -> Result<PathBuf> {
        let v_assets_dir = self.assets_dir.join(format!("v{}", version));
        let mut assets_to_use = if v_assets_dir.exists() {
            v_assets_dir
        } else {
            self.assets_dir.clone()
        };

        if !assets_to_use.join("rootfs.squashfs").exists() {
            let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
            let arch_dir = assets_to_use.join(arch);
            if arch_dir.join("rootfs.squashfs").exists() {
                assets_to_use = arch_dir;
            }
        }

        Ok(assets_to_use)
    }
}

use axum::http::StatusCode;

#[derive(Debug)]
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

// ---------------------------------------------------------------------------
// Image API Handlers
// ---------------------------------------------------------------------------

async fn handle_fork(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<ForkRequest>,
) -> Result<Json<ForkResponse>, AppError> {
    state.cleanup_stale_instances();

    let session_dir = {
        // Find in running instances or persistent registry
        if let Some(i) = state.instances.lock().unwrap().get(&id) {
            i.session_dir.clone()
        } else if let Some(p) = state.persistent_registry.lock().unwrap().get(&id) {
            p.session_dir.clone()
        } else {
            return Err(AppError(StatusCode::NOT_FOUND, format!("source sandbox not found: {}", id)));
        }
    };

    let base_version = {
        if let Some(i) = state.instances.lock().unwrap().get(&id) {
            i.base_version.clone()
        } else if let Some(p) = state.persistent_registry.lock().unwrap().get(&id) {
            p.base_version.clone()
        } else {
            state.current_version.clone()
        }
    };

    let parent_image = {
        // If the source VM was booted from an image, propagate it if possible
        // We don't currently track source_image in InstanceInfo/PersistentVmEntry, 
        // so we'll leave it None for now. It can be added later.
        None
    };

    let entry = match capsem_core::image::create_image_from_session(
        &state.image_registry,
        &session_dir,
        &payload.name,
        payload.description.clone(),
        &id,
        parent_image,
        &base_version,
    ) {
        Ok(e) => e,
        Err(e) => {
            let status = if e.to_string().contains("already exists") {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return Err(AppError(status, format!("failed to fork image: {e}")));
        }
    };

    Ok(Json(ForkResponse {
        name: entry.name,
        size_bytes: entry.size_bytes,
    }))
}

async fn handle_image_list(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<ImageListResponse>, AppError> {
    let entries = state.image_registry.list()
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to list images: {e}")))?;

    let images = entries.into_iter().map(|e| ImageInfo {
        name: e.name,
        description: e.description,
        source_vm: e.source_vm,
        parent_image: e.parent_image,
        base_version: e.base_version,
        created_at: capsem_core::session::epoch_to_iso(e.created_at.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()),
        size_bytes: e.size_bytes,
    }).collect();

    Ok(Json(ImageListResponse { images }))
}

async fn handle_image_inspect(
    State(state): State<Arc<ServiceState>>,
    Path(name): Path<String>,
) -> Result<Json<ImageInfo>, AppError> {
    let entry = state.image_registry.get(&name)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to query image: {e}")))?
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("image not found: {}", name)))?;

    Ok(Json(ImageInfo {
        name: entry.name,
        description: entry.description,
        source_vm: entry.source_vm,
        parent_image: entry.parent_image,
        base_version: entry.base_version,
        created_at: capsem_core::session::epoch_to_iso(entry.created_at.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()),
        size_bytes: entry.size_bytes,
    }))
}

async fn handle_image_delete(
    State(state): State<Arc<ServiceState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let removed = state.image_registry.remove(&name)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to delete image: {e}")))?;

    if !removed {
        return Err(AppError(StatusCode::NOT_FOUND, format!("image not found: {}", name)));
    }

    Ok(Json(serde_json::json!({ "success": true })))
}

async fn handle_provision(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, AppError> {
    let id = payload.name.clone().unwrap_or_else(|| {
        format!("vm-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs())
    });

    match state.provision_sandbox(&id, payload.ram_mb, payload.cpus, Some(state.current_version.clone()), payload.persistent, payload.env, payload.image) {
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

    let mut sandboxes: Vec<SandboxInfo> = Vec::new();

    // Running instances
    {
        let instances = state.instances.lock().unwrap();
        for i in instances.values() {
            sandboxes.push(SandboxInfo {
                id: i.id.clone(),
                pid: i.pid,
                status: "Running".to_string(),
                persistent: i.persistent,
                ram_mb: Some(i.ram_mb),
                cpus: Some(i.cpus),
                version: Some(i.base_version.clone()),
            });
        }
    }

    // Stopped persistent VMs (not in instances map)
    {
        let registry = state.persistent_registry.lock().unwrap();
        let instances = state.instances.lock().unwrap();
        for entry in registry.list() {
            if !instances.contains_key(&entry.name) {
                sandboxes.push(SandboxInfo {
                    id: entry.name.clone(),
                    pid: 0,
                    status: "Stopped".to_string(),
                    persistent: true,
                    ram_mb: Some(entry.ram_mb),
                    cpus: Some(entry.cpus),
                    version: Some(entry.base_version.clone()),
                });
            }
        }
    }

    Json(ListResponse { sandboxes })
}

async fn handle_info(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<SandboxInfo>, AppError> {
    state.cleanup_stale_instances();

    // Check running instances first
    {
        let instances = state.instances.lock().unwrap();
        if let Some(i) = instances.get(&id) {
            return Ok(Json(SandboxInfo {
                id: i.id.clone(),
                pid: i.pid,
                status: "Running".to_string(),
                persistent: i.persistent,
                ram_mb: Some(i.ram_mb),
                cpus: Some(i.cpus),
                version: Some(i.base_version.clone()),
            }));
        }
    }

    // Check stopped persistent VMs
    {
        let registry = state.persistent_registry.lock().unwrap();
        if let Some(entry) = registry.get(&id) {
            return Ok(Json(SandboxInfo {
                id: entry.name.clone(),
                pid: 0,
                status: "Stopped".to_string(),
                persistent: true,
                ram_mb: Some(entry.ram_mb),
                cpus: Some(entry.cpus),
                version: Some(entry.base_version.clone()),
            }));
        }
    }

    Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))
}

async fn handle_logs(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<LogsResponse>, AppError> {
    let session_dir = {
        let instances = state.instances.lock().unwrap();
        if let Some(i) = instances.get(&id) {
            i.session_dir.clone()
        } else {
            let registry = state.persistent_registry.lock().unwrap();
            registry.get(&id)
                .map(|e| e.session_dir.clone())
                .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?
        }
    };

    let serial_log_path = session_dir.join("serial.log");
    let process_log_path = session_dir.join("process.log");

    let (serial_logs, process_logs) = tokio::task::spawn_blocking(move || {
        let serial = std::fs::read_to_string(&serial_log_path).ok();
        let process = std::fs::read_to_string(&process_log_path).ok();
        (serial, process)
    }).await.map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("log read failed: {e}")))?;

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

/// Wait until a VM's IPC socket exists and responds to a ping.
/// Returns Ok(()) when the VM is ready, or Err after timeout.
async fn wait_for_vm_ready(uds_path: &std::path::Path, timeout_secs: u64) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        if uds_path.exists() {
            if let Ok(ProcessToService::Pong) = send_ipc_command(uds_path, ServiceToProcess::Ping, 5).await {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("VM did not become ready within {timeout_secs}s"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
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

    wait_for_vm_ready(&uds_path, 30).await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

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

    wait_for_vm_ready(&uds_path, 30).await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

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

    wait_for_vm_ready(&uds_path, 30).await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

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

/// Shutdown a running VM process by ID. Returns (session_dir, persistent) if found.
async fn shutdown_vm_process(state: &ServiceState, id: &str) -> Option<(PathBuf, bool)> {
    let (uds_path, session_dir, pid, persistent) = {
        let instances = state.instances.lock().unwrap();
        let i = instances.get(id)?;
        (i.uds_path.clone(), i.session_dir.clone(), i.pid, i.persistent)
    };

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

    // Give the agent time for sync + SIGTERM bash + 2s cleanup.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    if pid > 0 {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGKILL);
    }

    state.instances.lock().unwrap().remove(id);
    let _ = std::fs::remove_file(&uds_path);

    Some((session_dir, persistent))
}

async fn handle_stop(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    if let Some((session_dir, persistent)) = shutdown_vm_process(&state, &id).await {
        if !persistent {
            // Ephemeral VMs: destroy session dir on stop
            let _ = std::fs::remove_dir_all(&session_dir);
        }
        Ok(Json(json!({ "success": true, "persistent": persistent })))
    } else {
        Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))
    }
}

async fn handle_delete(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Try to shut down if running
    let session_dir = if let Some((session_dir, _)) = shutdown_vm_process(&state, &id).await {
        session_dir
    } else {
        // Not running -- check persistent registry for stopped VM
        let registry = state.persistent_registry.lock().unwrap();
        if let Some(entry) = registry.get(&id) {
            entry.session_dir.clone()
        } else {
            return Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")));
        }
    };

    // Unregister from persistent registry if applicable
    {
        let mut registry = state.persistent_registry.lock().unwrap();
        if registry.contains(&id) {
            let _ = registry.unregister(&id);
        }
    }

    // Always destroy session dir on delete
    let _ = std::fs::remove_dir_all(&session_dir);

    Ok(Json(json!({ "success": true })))
}

async fn handle_resume(
    State(state): State<Arc<ServiceState>>,
    Path(name): Path<String>,
) -> Result<Json<ProvisionResponse>, AppError> {
    match state.resume_sandbox(&name, None, None) {
        Ok(id) => Ok(Json(ProvisionResponse { id })),
        Err(e) => {
            error!(name, "resume failed: {e}");
            Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("resume failed: {e}")))
        }
    }
}

async fn handle_persist(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<PersistRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let name = &payload.name;
    validate_vm_name(name).map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;

    // Check name is not taken
    {
        let registry = state.persistent_registry.lock().unwrap();
        if registry.contains(name) {
            return Err(AppError(StatusCode::CONFLICT, format!("persistent VM \"{}\" already exists", name)));
        }
    }

    // Find the running ephemeral instance
    let (old_session_dir, ram_mb, cpus, base_version) = {
        let instances = state.instances.lock().unwrap();
        let i = instances.get(&id).ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        if i.persistent {
            return Err(AppError(StatusCode::BAD_REQUEST, format!("VM \"{}\" is already persistent", id)));
        }
        (i.session_dir.clone(), i.ram_mb, i.cpus, i.base_version.clone())
    };

    let source_image = {
        let instances = state.instances.lock().unwrap();
        instances.get(&id).and_then(|i| i.source_image.clone())
    };

    // Move session dir to persistent location
    let new_session_dir = state.run_dir.join("persistent").join(&name);
    let _ = std::fs::create_dir_all(state.run_dir.join("persistent"));
    std::fs::rename(&old_session_dir, &new_session_dir)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to move session dir: {e}")))?;

    // Register in persistent registry
    {
        let mut registry = state.persistent_registry.lock().unwrap();
        registry.register(PersistentVmEntry {
            name: name.clone(),
            ram_mb,
            cpus,
            base_version: base_version.clone(),
            created_at: format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()),
            session_dir: new_session_dir.clone(),
            source_image: source_image.clone(),
        }).map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    // Update instance info in-place
    {
        let mut instances = state.instances.lock().unwrap();
        if let Some(info) = instances.remove(&id) {
            instances.insert(name.clone(), InstanceInfo {
                id: name.clone(),
                pid: info.pid,
                uds_path: info.uds_path,
                session_dir: new_session_dir,
                ram_mb: info.ram_mb,
                cpus: info.cpus,
                start_time: info.start_time,
                base_version: info.base_version,
                persistent: true,
                env: info.env,
                source_image,
            });
        }
    }

    Ok(Json(json!({ "success": true, "name": name })))
}

async fn handle_purge(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<PurgeRequest>,
) -> Result<Json<PurgeResponse>, AppError> {
    state.cleanup_stale_instances();

    let mut ephemeral_purged: u32 = 0;
    let mut persistent_purged: u32 = 0;

    // Collect VMs to purge
    let to_purge: Vec<(String, bool)> = {
        let instances = state.instances.lock().unwrap();
        instances.values()
            .filter(|i| !i.persistent || payload.all)
            .map(|i| (i.id.clone(), i.persistent))
            .collect()
    };

    for (id, persistent) in &to_purge {
        if let Some((session_dir, _)) = shutdown_vm_process(&state, id).await {
            if *persistent {
                let mut registry = state.persistent_registry.lock().unwrap();
                let _ = registry.unregister(id);
            }
            let _ = std::fs::remove_dir_all(&session_dir);
            if *persistent { persistent_purged += 1; } else { ephemeral_purged += 1; }
        }
    }

    // If --all, also purge stopped persistent VMs
    if payload.all {
        let stopped_names: Vec<String> = {
            let registry = state.persistent_registry.lock().unwrap();
            let instances = state.instances.lock().unwrap();
            registry.list()
                .filter(|e| !instances.contains_key(&e.name))
                .map(|e| e.name.clone())
                .collect()
        };
        for name in &stopped_names {
            let session_dir = {
                let registry = state.persistent_registry.lock().unwrap();
                registry.get(name).map(|e| e.session_dir.clone())
            };
            if let Some(dir) = session_dir {
                let _ = std::fs::remove_dir_all(&dir);
            }
            let mut registry = state.persistent_registry.lock().unwrap();
            let _ = registry.unregister(name);
            persistent_purged += 1;
        }
    }

    let purged = ephemeral_purged + persistent_purged;
    Ok(Json(PurgeResponse { purged, persistent_purged, ephemeral_purged }))
}

/// One-shot exec: provision a temp VM, run a command, return output, destroy VM.
async fn handle_run(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<RunRequest>,
) -> Result<Json<ExecResponse>, AppError> {
    let id = format!("run-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());

    let ram_bytes = payload.ram_mb * 1024 * 1024;
    let session_dir = state.run_dir.join("sessions").join(&id);

    // 1. Provision ephemeral VM
    state.provision_sandbox(&id, payload.ram_mb, payload.cpus, Some(state.current_version.clone()), false, payload.env, None)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("provision failed: {e}")))?;

    // 2. Register session in main.db
    let sessions_db_dir = state.run_dir.parent()
        .unwrap_or(state.run_dir.as_path())
        .join("sessions");
    let _ = std::fs::create_dir_all(&sessions_db_dir);
    let index = capsem_core::session::SessionIndex::open(&sessions_db_dir.join("main.db")).ok();
    if let Some(ref idx) = index {
        let record = capsem_core::session::SessionRecord {
            id: id.clone(),
            mode: "run".to_string(),
            command: Some(payload.command.clone()),
            status: "running".to_string(),
            created_at: capsem_core::session::now_iso(),
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
            storage_mode: "virtiofs".to_string(),
            rootfs_hash: None,
            rootfs_version: Some(state.current_version.clone()),
            source_image: None,
            persistent: false,
        };
        if let Err(e) = idx.create_session(&record) {
            tracing::warn!("failed to register session in main.db: {e}");
        }
    }

    // 3. Wait for VM socket to appear
    let uds_path = state.run_dir.join("instances").join(format!("{}.sock", id));
    if let Err(e) = wait_for_vm_ready(&uds_path, 30).await {
        let _ = shutdown_vm_process(&state, &id).await;
        let _ = std::fs::remove_dir_all(&session_dir);
        return Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, e));
    }

    // 4. Execute command
    let job_id = state.next_job_id();
    let exec_result = send_ipc_command(
        &uds_path,
        ServiceToProcess::Exec { id: job_id, command: payload.command },
        payload.timeout_secs,
    ).await;

    // 5. Tear down VM process (session dir preserved for telemetry)
    let _ = shutdown_vm_process(&state, &id).await;

    // 6. Roll up session counters into main.db
    if let Some(ref idx) = index {
        let session_db_path = session_dir.join("session.db");
        if session_db_path.exists() {
            if let Ok(reader) = capsem_logger::DbReader::open(&session_db_path) {
                if let Ok(counts) = reader.net_event_counts() {
                    let _ = idx.update_request_counts(
                        &id, counts.total as u64, counts.allowed as u64, counts.denied as u64,
                    );
                }
                // Roll up file events and MCP calls.
                let file_events = reader.file_event_count().unwrap_or(0);
                let mcp_calls = reader.mcp_call_stats().map(|s| s.total).unwrap_or(0);
                let _ = idx.update_session_summary(
                    &id, 0, 0, 0.0, 0, mcp_calls, file_events,
                );
            }
        }
        let _ = idx.update_status(&id, "stopped", Some(&capsem_core::session::now_iso()));
    }

    // 7. Return result
    match exec_result {
        Ok(ProcessToService::ExecResult { stdout, stderr, exit_code, .. }) => {
            Ok(Json(ExecResponse {
                stdout: String::from_utf8_lossy(&stdout).to_string(),
                stderr: String::from_utf8_lossy(&stderr).to_string(),
                exit_code,
            }))
        }
        Ok(_) => Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, "unexpected IPC response".into())),
        Err(e) => Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("exec failed: {e}"))),
    }
}

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, fmt};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    let home = std::env::var("HOME").context("HOME not set")?;
    let run_dir = std::env::var("CAPSEM_RUN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(&home).join(".capsem/run"));
    
    let home_capsem = PathBuf::from(&home).join(".capsem");
    
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
    let persistent_dir = run_dir.join("persistent");
    let _ = std::fs::create_dir_all(&instances_dir);
    let _ = std::fs::create_dir_all(&sessions_dir);
    let _ = std::fs::create_dir_all(&persistent_dir);

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

    let registry_path = run_dir.join("persistent_registry.json");
    let persistent_registry = PersistentRegistry::load(registry_path);
    info!(persistent_vms = persistent_registry.data.vms.len(), "loaded persistent VM registry");

    let image_registry = Arc::new(capsem_core::image::ImageRegistry::new(&home_capsem));

    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(persistent_registry),
        image_registry,
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
        .route("/stop/{id}", post(handle_stop))
        .route("/delete/{id}", delete(handle_delete))
        .route("/resume/{name}", post(handle_resume))
        .route("/persist/{id}", post(handle_persist))
        .route("/purge", post(handle_purge))
        .route("/run", post(handle_run))
        .route("/reload-config", post(handle_reload_config))
        .route("/fork/{id}", post(handle_fork))
        .route("/images", get(handle_image_list))
        .route("/images/{name}", get(handle_image_inspect).delete(handle_image_delete))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    info!(socket = %service_sock.display(), "listening on UDS");

    let uds = UnixListener::bind(&service_sock).context("failed to bind UDS")?;

    // Spawn companion processes (gateway + tray) as children.
    // They are killed automatically when the service exits because we hold
    // the Child handles and drop them on shutdown.
    let mut children = spawn_companions(&service_sock, &run_dir).await;

    axum::serve(uds, app)
        .with_graceful_shutdown(async {
            shutdown_signal().await;
            info!("service shutting down, killing companions");
        })
        .await
        .context("server error")?;

    // Explicitly kill companion processes on shutdown
    for child in &mut children {
        let _ = child.kill().await;
    }

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }
}

/// Find a sibling binary next to the current executable, falling back to
/// target/debug/ for development builds.
fn find_sibling_binary(name: &str) -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.parent().unwrap().join(name);
        if sibling.exists() {
            return sibling;
        }
    }
    PathBuf::from(format!("target/debug/{name}"))
}

/// Spawn the gateway and tray as child processes of the service.
async fn spawn_companions(
    service_sock: &std::path::Path,
    run_dir: &std::path::Path,
) -> Vec<tokio::process::Child> {
    let mut children = Vec::new();

    // 1. Spawn capsem-gateway (TCP reverse proxy -> UDS)
    let gateway_bin = find_sibling_binary("capsem-gateway");
    info!(binary = %gateway_bin.display(), "spawning capsem-gateway");
    match tokio::process::Command::new(&gateway_bin)
        .arg("--uds-path")
        .arg(service_sock)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => {
            info!(pid = child.id(), "capsem-gateway spawned");
            children.push(child);

            // Wait for gateway to write token + port files (up to 5s)
            let token_path = run_dir.join("gateway.token");
            let port_path = run_dir.join("gateway.port");
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline {
                if token_path.exists() && port_path.exists() {
                    info!("gateway ready (token + port files present)");
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }

            // 2. Spawn capsem-tray (menu bar) -- only on macOS, only after gateway ready
            #[cfg(target_os = "macos")]
            if token_path.exists() {
                let tray_bin = find_sibling_binary("capsem-tray");
                info!(binary = %tray_bin.display(), "spawning capsem-tray");
                match tokio::process::Command::new(&tray_bin)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                {
                    Ok(child) => {
                        info!(pid = child.id(), "capsem-tray spawned");
                        children.push(child);
                    }
                    Err(e) => {
                        tracing::warn!("failed to spawn capsem-tray: {e} (non-fatal)");
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("failed to spawn capsem-gateway: {e} (non-fatal)");
        }
    }

    children
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    fn make_test_state() -> Arc<ServiceState> {
        let dummy_hash = "a".repeat(64);
        let manifest_json = format!(
            r#"{{"latest":"0.0.0","releases":{{"0.0.0":{{"assets":[{{"filename":"dummy.img","hash":"{}","size":0}}]}}}}}}"#,
            dummy_hash
        );
        let manifest = capsem_core::asset_manager::Manifest::from_json(&manifest_json).unwrap();
        let am = capsem_core::asset_manager::AssetManager::from_manifest(
            &manifest, "0.0.0", PathBuf::from("/tmp/capsem-test-assets"), None
        ).unwrap();
        let registry_path = PathBuf::from("/tmp/capsem-test-svc/persistent_registry.json");
        Arc::new(ServiceState {
            instances: Mutex::new(HashMap::new()),
            persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
            image_registry: Arc::new(capsem_core::image::ImageRegistry::new(std::path::Path::new("/tmp/capsem-test-svc"))),
            process_binary: PathBuf::from("/nonexistent/capsem-process"),
            assets_dir: PathBuf::from("/nonexistent/assets"),
            run_dir: PathBuf::from("/tmp/capsem-test-svc"),
            job_counter: AtomicU64::new(1),
            asset_manager: Arc::new(am),
            current_version: "0.0.0".into(),
        })
    }

    fn insert_fake_instance(state: &ServiceState, id: &str, pid: u32) {
        state.instances.lock().unwrap().insert(
            id.to_string(),
            InstanceInfo {
                id: id.to_string(),
                pid,
                uds_path: PathBuf::from(format!("/tmp/{}.sock", id)),
                session_dir: PathBuf::from(format!("/tmp/sessions/{}", id)),
                ram_mb: 2048,
                cpus: 2,
                start_time: std::time::Instant::now(),
                base_version: "0.0.0".into(),
                persistent: false,
                env: None,
                source_image: None,
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
        let result = state.provision_sandbox(&long_name, 2048, 2, None, false, None, None);
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
        let result = state.provision_sandbox(&boundary_name, 2048, 2, None, false, None, None);
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
        let result = state.provision_sandbox(&ok_name, 2048, 2, None, false, None, None);
        // Will fail later (missing rootfs), but NOT for path length
        if let Err(e) = &result {
            let msg = e.to_string();
            assert!(!msg.contains("socket path"), "short name should not hit path limit: {msg}");
        }
    }

    #[test]
    fn provision_short_name_passes_path_check() {
        let state = make_test_state();
        let result = state.provision_sandbox("my-vm", 2048, 2, None, false, None, None);
        // Fails for missing assets, not path length
        if let Err(e) = &result {
            let msg = e.to_string();
            assert!(!msg.contains("socket path"), "normal name should not hit path limit: {msg}");
        }
    }

    // -----------------------------------------------------------------------
    // VM name validation
    // -----------------------------------------------------------------------

    #[test]
    fn validate_vm_name_valid() {
        assert!(validate_vm_name("mydev").is_ok());
        assert!(validate_vm_name("my-dev").is_ok());
        assert!(validate_vm_name("my_dev").is_ok());
        assert!(validate_vm_name("dev123").is_ok());
        assert!(validate_vm_name("a").is_ok());
    }

    #[test]
    fn validate_vm_name_empty() {
        assert!(validate_vm_name("").is_err());
    }

    #[test]
    fn validate_vm_name_path_separator() {
        assert!(validate_vm_name("../escape").is_err());
        assert!(validate_vm_name("foo/bar").is_err());
    }

    #[test]
    fn validate_vm_name_starts_with_hyphen() {
        assert!(validate_vm_name("-bad").is_err());
    }

    #[test]
    fn validate_vm_name_spaces() {
        assert!(validate_vm_name("has space").is_err());
    }

    #[test]
    fn validate_vm_name_too_long() {
        assert!(validate_vm_name(&"a".repeat(65)).is_err());
        assert!(validate_vm_name(&"a".repeat(64)).is_ok());
    }

    // -----------------------------------------------------------------------
    // PersistentRegistry
    // -----------------------------------------------------------------------

    #[test]
    fn persistent_registry_roundtrip() {
        let dir = std::env::temp_dir().join("capsem-test-registry");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_registry.json");
        let _ = std::fs::remove_file(&path);

        let mut registry = PersistentRegistry::load(path.clone());
        assert_eq!(registry.data.vms.len(), 0);

        registry.register(PersistentVmEntry {
            name: "mydev".into(),
            ram_mb: 4096,
            cpus: 4,
            base_version: "0.1.0".into(),
            created_at: "12345".into(),
            session_dir: dir.join("mydev"),
            source_image: None,
        }).unwrap();

        assert!(registry.contains("mydev"));
        assert_eq!(registry.get("mydev").unwrap().ram_mb, 4096);

        // Reload from disk
        let registry2 = PersistentRegistry::load(path.clone());
        assert!(registry2.contains("mydev"));
        assert_eq!(registry2.get("mydev").unwrap().cpus, 4);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persistent_registry_rejects_duplicate() {
        let dir = std::env::temp_dir().join("capsem-test-registry-dup");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_registry.json");
        let _ = std::fs::remove_file(&path);

        let mut registry = PersistentRegistry::load(path);
        let entry = PersistentVmEntry {
            name: "dup".into(),
            ram_mb: 2048,
            cpus: 2,
            base_version: "0.1.0".into(),
            created_at: "12345".into(),
            session_dir: dir.join("dup"),
            source_image: None,
        };
        registry.register(entry.clone()).unwrap();
        let err = registry.register(entry).unwrap_err();
        assert!(err.to_string().contains("already exists"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persistent_registry_unregister() {
        let dir = std::env::temp_dir().join("capsem-test-registry-unreg");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_registry.json");
        let _ = std::fs::remove_file(&path);

        let mut registry = PersistentRegistry::load(path);
        registry.register(PersistentVmEntry {
            name: "tmp".into(),
            ram_mb: 2048,
            cpus: 2,
            base_version: "0.1.0".into(),
            created_at: "12345".into(),
            session_dir: dir.join("tmp"),
            source_image: None,
        }).unwrap();
        assert!(registry.contains("tmp"));
        registry.unregister("tmp").unwrap();
        assert!(!registry.contains("tmp"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Provision rejects duplicate persistent VM
    // -----------------------------------------------------------------------

    #[test]
    fn provision_persistent_rejects_duplicate_name() {
        let state = make_test_state();
        // Pre-register a persistent VM directly in the registry data
        {
            let mut reg = state.persistent_registry.lock().unwrap();
            reg.data.vms.insert("taken".into(), PersistentVmEntry {
                name: "taken".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: PathBuf::from("/tmp/taken"),
                source_image: None,
            });
        }
        let result = state.provision_sandbox("taken", 2048, 2, None, true, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("already exists"), "expected duplicate error, got: {err}");
        assert!(err.contains("resume"), "should suggest resume, got: {err}");
    }

    #[test]
    fn provision_persistent_validates_name() {
        let state = make_test_state();
        let result = state.provision_sandbox("../evil", 2048, 2, None, true, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("must start with") || err.contains("must contain only"),
            "expected name validation error, got: {err}");
    }

    // -----------------------------------------------------------------------
    // Image handler tests (service-level unit tests)
    // -----------------------------------------------------------------------

    fn make_test_state_with_tempdir() -> (Arc<ServiceState>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let dummy_hash = "a".repeat(64);
        let manifest_json = format!(
            r#"{{"latest":"0.0.0","releases":{{"0.0.0":{{"assets":[{{"filename":"dummy.img","hash":"{}","size":0}}]}}}}}}"#,
            dummy_hash
        );
        let manifest = capsem_core::asset_manager::Manifest::from_json(&manifest_json).unwrap();
        let am = capsem_core::asset_manager::AssetManager::from_manifest(
            &manifest, "0.0.0", dir.path().join("assets"), None
        ).unwrap();
        let registry_path = dir.path().join("persistent_registry.json");
        let state = Arc::new(ServiceState {
            instances: Mutex::new(HashMap::new()),
            persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
            image_registry: Arc::new(capsem_core::image::ImageRegistry::new(dir.path())),
            process_binary: PathBuf::from("/nonexistent/capsem-process"),
            assets_dir: dir.path().join("assets"),
            run_dir: dir.path().to_path_buf(),
            job_counter: AtomicU64::new(1),
            asset_manager: Arc::new(am),
            current_version: "0.0.0".into(),
        });
        (state, dir)
    }

    fn seed_image(state: &ServiceState, name: &str) -> capsem_core::image::ImageEntry {
        // Create a fake session dir with content, then fork it into an image
        let session_dir = state.run_dir.join("seed-session");
        let _ = std::fs::create_dir_all(session_dir.join("system"));
        let _ = std::fs::create_dir_all(session_dir.join("workspace"));
        std::fs::write(session_dir.join("system/rootfs.img"), b"test").unwrap();
        capsem_core::image::create_image_from_session(
            &state.image_registry, &session_dir, name,
            Some("test image".into()), "seed-vm", None, "0.0.0",
        ).unwrap()
    }

    #[tokio::test]
    async fn handle_image_list_empty() {
        let (state, _dir) = make_test_state_with_tempdir();
        // state is already Arc<ServiceState> from make_test_state*
        let result = handle_image_list(State(state)).await.unwrap();
        assert!(result.0.images.is_empty());
    }

    #[tokio::test]
    async fn handle_image_list_returns_entries() {
        let (state, _dir) = make_test_state_with_tempdir();
        seed_image(&state, "img-a");
        seed_image(&state, "img-b");
        // state is already Arc<ServiceState> from make_test_state*
        let result = handle_image_list(State(state)).await.unwrap();
        assert_eq!(result.0.images.len(), 2);
        let names: Vec<&str> = result.0.images.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"img-a"));
        assert!(names.contains(&"img-b"));
    }

    #[tokio::test]
    async fn handle_image_inspect_found() {
        let (state, _dir) = make_test_state_with_tempdir();
        seed_image(&state, "my-img");
        // state is already Arc<ServiceState> from make_test_state*
        let result = handle_image_inspect(State(state), Path("my-img".into())).await.unwrap();
        assert_eq!(result.0.name, "my-img");
        assert_eq!(result.0.description.as_deref(), Some("test image"));
        assert_eq!(result.0.source_vm, "seed-vm");
        assert_eq!(result.0.base_version, "0.0.0");
    }

    #[tokio::test]
    async fn handle_image_inspect_not_found() {
        let (state, _dir) = make_test_state_with_tempdir();
        // state is already Arc<ServiceState> from make_test_state*
        let err = handle_image_inspect(State(state), Path("ghost".into())).await.unwrap_err();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handle_image_delete_success() {
        let (state, _dir) = make_test_state_with_tempdir();
        seed_image(&state, "del-me");
        // state is already Arc<ServiceState> from make_test_state*
        let result = handle_image_delete(State(state.clone()), Path("del-me".into())).await.unwrap();
        assert_eq!(result.0["success"], true);
        // Verify gone
        let list = handle_image_list(State(state)).await.unwrap();
        assert!(list.0.images.is_empty());
    }

    #[tokio::test]
    async fn handle_image_delete_not_found() {
        let (state, _dir) = make_test_state_with_tempdir();
        // state is already Arc<ServiceState> from make_test_state*
        let err = handle_image_delete(State(state), Path("ghost".into())).await.unwrap_err();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handle_fork_from_running_instance() {
        let (state, _dir) = make_test_state_with_tempdir();
        // Create a real session dir for the fake instance
        let session_dir = state.run_dir.join("sessions/fork-src");
        std::fs::create_dir_all(session_dir.join("system")).unwrap();
        std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
        std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
        state.instances.lock().unwrap().insert(
            "fork-src".into(),
            InstanceInfo {
                id: "fork-src".into(),
                pid: std::process::id(),
                uds_path: PathBuf::from("/tmp/fork-src.sock"),
                session_dir: session_dir.clone(),
                ram_mb: 2048,
                cpus: 2,
                start_time: std::time::Instant::now(),
                base_version: "0.0.0".into(),
                persistent: false,
                env: None,
                source_image: None,
            },
        );
        // state is already Arc<ServiceState> from make_test_state*
        let result = handle_fork(
            State(state.clone()),
            Path("fork-src".into()),
            Json(ForkRequest { name: "forked-img".into(), description: Some("test".into()) }),
        ).await.unwrap();
        assert_eq!(result.0.name, "forked-img");
        assert!(result.0.size_bytes > 0);
        // Verify it shows up in list
        let list = handle_image_list(State(state)).await.unwrap();
        assert_eq!(list.0.images.len(), 1);
        assert_eq!(list.0.images[0].source_vm, "fork-src");
    }

    #[tokio::test]
    async fn handle_fork_not_found() {
        let (state, _dir) = make_test_state_with_tempdir();
        // state is already Arc<ServiceState> from make_test_state*
        let err = handle_fork(
            State(state),
            Path("ghost".into()),
            Json(ForkRequest { name: "img".into(), description: None }),
        ).await.unwrap_err();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handle_fork_duplicate_returns_conflict() {
        let (state, _dir) = make_test_state_with_tempdir();
        let session_dir = state.run_dir.join("sessions/dup-src");
        std::fs::create_dir_all(session_dir.join("system")).unwrap();
        std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
        std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
        state.instances.lock().unwrap().insert(
            "dup-src".into(),
            InstanceInfo {
                id: "dup-src".into(),
                pid: std::process::id(),
                uds_path: PathBuf::from("/tmp/dup-src.sock"),
                session_dir,
                ram_mb: 2048,
                cpus: 2,
                start_time: std::time::Instant::now(),
                base_version: "0.0.0".into(),
                persistent: false,
                env: None,
                source_image: None,
            },
        );
        // state is already Arc<ServiceState> from make_test_state*
        // First fork succeeds
        let _ = handle_fork(
            State(state.clone()),
            Path("dup-src".into()),
            Json(ForkRequest { name: "same-name".into(), description: None }),
        ).await.unwrap();
        // Second fork with same name returns CONFLICT
        let err = handle_fork(
            State(state),
            Path("dup-src".into()),
            Json(ForkRequest { name: "same-name".into(), description: None }),
        ).await.unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn handle_fork_from_persistent_registry() {
        let (state, _dir) = make_test_state_with_tempdir();
        let session_dir = state.run_dir.join("persistent/pers-vm");
        std::fs::create_dir_all(session_dir.join("system")).unwrap();
        std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
        std::fs::write(session_dir.join("system/rootfs.img"), b"data").unwrap();
        {
            let mut reg = state.persistent_registry.lock().unwrap();
            reg.data.vms.insert("pers-vm".into(), PersistentVmEntry {
                name: "pers-vm".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                session_dir: session_dir.clone(),
                source_image: None,
            });
        }
        // state is already Arc<ServiceState> from make_test_state*
        let result = handle_fork(
            State(state),
            Path("pers-vm".into()),
            Json(ForkRequest { name: "from-pers".into(), description: None }),
        ).await.unwrap();
        assert_eq!(result.0.name, "from-pers");
    }

    #[test]
    fn provision_rejects_nonexistent_image() {
        let (state, _dir) = make_test_state_with_tempdir();
        let result = state.provision_sandbox("vm1", 2048, 2, None, false, None, Some("ghost-img".into()));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "expected image not found, got: {err}");
    }
}
