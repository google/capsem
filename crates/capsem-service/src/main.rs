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
use rand::Rng;

mod api;

/// Generate a fun temporary VM name like `tmp-brave-falcon`.
fn generate_tmp_name() -> String {
    const ADJECTIVES: &[&str] = &[
        "brave", "calm", "clever", "daring", "eager", "fancy", "gentle",
        "happy", "jolly", "keen", "lively", "lucky", "merry", "noble",
        "plucky", "quick", "quiet", "sharp", "smart", "swift", "witty",
        "zany", "bright", "bold", "proud", "fierce", "steady", "agile",
        "cosmic", "epic", "grand", "mighty", "nimble", "stellar", "vivid",
    ];
    const NOUNS: &[&str] = &[
        "phoenix", "falcon", "otter", "panda", "wolf", "tiger", "raven",
        "cobra", "dolphin", "hawk", "lynx", "puma", "fox", "owl", "bear",
        "jaguar", "eagle", "heron", "bison", "coral", "amber", "jade",
        "onyx", "ruby", "opal", "ivory", "crimson", "indigo", "violet",
        "bronze", "silver", "cedar", "maple", "willow", "aurora", "comet",
        "nova", "nebula", "summit", "ridge", "canyon", "glacier", "thunder",
        "blaze", "ember", "frost", "breeze",
    ];
    let mut rng = rand::thread_rng();
    let adj = ADJECTIVES[rng.gen_range(0..ADJECTIVES.len())];
    let noun = NOUNS[rng.gen_range(0..NOUNS.len())];
    format!("tmp-{adj}-{noun}")
}
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
    #[serde(skip_serializing_if = "Option::is_none", default, alias = "source_image")]
    forked_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    description: Option<String>,
    #[serde(default)]
    suspended: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    checkpoint_path: Option<String>,
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
        // Atomic write: write to temp file, fsync, then rename.
        // Prevents torn writes on crash from losing all persistent VM state.
        let tmp_path = self.path.with_extension("json.tmp");
        let mut f = std::fs::File::create(&tmp_path)?;
        std::io::Write::write_all(&mut f, json.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp_path, &self.path)?;
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

    fn get_mut(&mut self, name: &str) -> Option<&mut PersistentVmEntry> {
        self.data.vms.get_mut(name)
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
    /// Sandbox this VM was cloned from, if any
    forked_from: Option<String>,
}

pub struct ProvisionOptions<'a> {
    pub id: &'a str,
    pub ram_mb: u64,
    pub cpus: u32,
    pub version_override: Option<String>,
    pub persistent: bool,
    pub env: Option<std::collections::HashMap<String, String>>,
    pub from: Option<String>,
    pub description: Option<String>,
}

impl ServiceState {
    /// Build the Unix socket path for a VM instance.
    ///
    /// Prefers `{run_dir}/instances/{id}.sock` but falls back to
    /// `/tmp/capsem/{hash}.sock` when the path would exceed the macOS
    /// 104-byte `SUN_LEN` limit (common with `/var/folders/...` temp dirs).
    fn instance_socket_path(&self, id: &str) -> PathBuf {
        const SUN_PATH_MAX: usize = 90;
        let preferred = self.run_dir.join("instances").join(format!("{id}.sock"));
        if preferred.as_os_str().len() < SUN_PATH_MAX {
            return preferred;
        }
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        id.hash(&mut h);
        self.run_dir.hash(&mut h);
        let dir = PathBuf::from("/tmp/capsem");
        let _ = std::fs::create_dir_all(&dir);
        let short = dir.join(format!("{:x}.sock", h.finish()));
        tracing::info!(%id, original = %preferred.display(), short = %short.display(),
                       "socket path too long, using /tmp/capsem/");
        short
    }

    /// Path to main.db (global session index).
    /// Layout: run_dir = ~/.capsem/run, main.db lives at ~/.capsem/sessions/main.db.
    fn main_db_path(&self) -> PathBuf {
        self.run_dir
            .parent()
            .unwrap_or(self.run_dir.as_path())
            .join("sessions")
            .join("main.db")
    }

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
                let _ = std::fs::remove_file(info.uds_path.with_extension("ready"));
            }
        }
    }

    fn provision_sandbox(
        self: &Arc<Self>,
        options: ProvisionOptions,
    ) -> Result<()> {
        let ProvisionOptions { id, ram_mb, cpus, version_override, persistent, env, from, description } = options;
        self.cleanup_stale_instances();

        let vm_settings = capsem_core::net::policy_config::load_merged_vm_settings();
        let max_concurrent_vms = vm_settings.max_concurrent_vms.unwrap_or(10) as usize;

        if !(1..=8).contains(&cpus) {
            return Err(anyhow!("cpus must be between 1 and 8"));
        }
        if !(256..=16384).contains(&ram_mb) {
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

        // Validate source sandbox if --from provided
        let source_entry = if let Some(ref from_name) = from {
            let registry = self.persistent_registry.lock().unwrap();
            let entry = registry.get(from_name)
                .ok_or_else(|| anyhow!("source sandbox '{}' not found", from_name))?
                .clone();
            Some(entry)
        } else {
            None
        };

        // If cloning from a source sandbox, inherit its base_version.
        let version = if let Some(ref entry) = source_entry {
            entry.base_version.clone()
        } else {
            version_override.unwrap_or_else(|| self.current_version.clone())
        };

        info!(id, version, persistent, from, "provision_sandbox called");

        let uds_path = self.instance_socket_path(id);

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

        // If cloning from a source sandbox, clone its state into the new session directory
        if let Some(ref entry) = source_entry {
            info!(from = entry.name, session_dir = %session_dir.display(), "cloning session from source sandbox");
            capsem_core::auto_snapshot::clone_sandbox_state(&entry.session_dir, &session_dir)
                .context("failed to clone sandbox state")?;
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

        // Clear inherited env to prevent API key/token leakage, then
        // re-add only the minimal set needed for the process to function.
        child_cmd.env_clear();
        for key in &["HOME", "PATH", "USER", "TMPDIR"] {
            if let Ok(val) = std::env::var(key) {
                child_cmd.env(key, val);
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
        let session_dir_clone = session_dir.clone();
        tokio::spawn(async move {
            let _ = child.wait().await;
            info!(id_clone, "capsem-process exited, cleaning up");
            
            // If this was a persistent VM and checkpoint.vzsave exists, mark it suspended
            {
                let mut registry = state_clone.persistent_registry.lock().unwrap();
                if let Some(entry) = registry.data.vms.get_mut(&id_clone) {
                    let checkpoint_path = session_dir_clone.join("checkpoint.vzsave");
                    if checkpoint_path.exists() {
                        info!(id_clone, "Checkpoint file found, marking VM as suspended");
                        entry.suspended = true;
                        entry.checkpoint_path = Some("checkpoint.vzsave".to_string());
                        if let Err(e) = registry.save() {
                            error!(id_clone, "failed to save persistent registry: {e}");
                        }
                    } else {
                        // Ensure it's not stuck in a suspended state if it crashed or was stopped manually
                        entry.suspended = false;
                        entry.checkpoint_path = None;
                        if let Err(e) = registry.save() {
                            error!(id_clone, "failed to save persistent registry: {e}");
                        }
                    }
                }
            }

            // Remove from active instances so the service knows this VM is gone.
            // Session directory cleanup is handled by the caller (handle_stop,
            // handle_run, handle_purge) to avoid racing with telemetry reads.
            state_clone.instances.lock().unwrap().remove(&id_clone);
            let _ = std::fs::remove_file(&uds_clone);
            let _ = std::fs::remove_file(uds_clone.with_extension("ready"));
        });

        if persistent {
            let mut registry = self.persistent_registry.lock().unwrap();
            registry.register(PersistentVmEntry {
                name: id.to_string(),
                ram_mb,
                cpus,
                base_version: version.clone(),
                created_at: format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()),
                session_dir: session_dir.clone(),
                forked_from: from.clone(),
                description: description.clone(),
                suspended: false,
                checkpoint_path: None,
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
            forked_from: from.clone(),
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

        let uds_path = self.instance_socket_path(name);
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

        // Pass checkpoint path for warm restore from suspended state
        if entry.suspended {
            if let Some(ref cp) = entry.checkpoint_path {
                let full_checkpoint = entry.session_dir.join(cp);
                if full_checkpoint.exists() {
                    child_cmd.arg("--checkpoint-path").arg(&full_checkpoint);
                    info!(name, checkpoint = %full_checkpoint.display(), "warm restore from checkpoint");
                } else {
                    tracing::warn!(name, checkpoint = %full_checkpoint.display(), "checkpoint file missing, cold booting");
                }
            }
        }

        // Clear inherited env to prevent API key/token leakage, then
        // re-add only the minimal set needed for the process to function.
        child_cmd.env_clear();
        for key in &["HOME", "PATH", "USER", "TMPDIR"] {
            if let Ok(val) = std::env::var(key) {
                child_cmd.env(key, val);
            }
        }

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
            let _ = std::fs::remove_file(uds_clone.with_extension("ready"));
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
            forked_from: entry.forked_from.clone(),
        });

        // Clear suspended state now that the VM is running again
        {
            let mut registry = self.persistent_registry.lock().unwrap();
            if let Some(reg_entry) = registry.get_mut(name) {
                reg_entry.suspended = false;
                reg_entry.checkpoint_path = None;
                if let Err(e) = registry.save() {
                    error!(name, "failed to save persistent registry: {e}");
                }
            }
        }

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
    let name = &payload.name;
    validate_vm_name(name).map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;

    // Check name is not taken
    {
        let registry = state.persistent_registry.lock().unwrap();
        if registry.contains(name) {
            return Err(AppError(StatusCode::CONFLICT, format!("sandbox '{}' already exists", name)));
        }
    }

    // Find source: running instance or stopped persistent VM
    let (session_dir, ram_mb, cpus, base_version, uds_path) = {
        let instances = state.instances.lock().unwrap();
        if let Some(i) = instances.get(&id) {
            (i.session_dir.clone(), i.ram_mb, i.cpus, i.base_version.clone(), Some(i.uds_path.clone()))
        } else {
            drop(instances);
            let registry = state.persistent_registry.lock().unwrap();
            if let Some(p) = registry.get(&id) {
                (p.session_dir.clone(), p.ram_mb, p.cpus, p.base_version.clone(), None)
            } else {
                return Err(AppError(StatusCode::NOT_FOUND, format!("source sandbox not found: {}", id)));
            }
        }
    };

    // Freeze + thaw the guest root filesystem so the ext4 loopback overlay
    // (rootfs.img) is fully flushed through VirtioFS to the host file.
    if let Some(ref uds) = uds_path {
        let freeze_id = state.next_job_id();
        if let Err(e) = send_ipc_command(uds, ServiceToProcess::Exec {
            id: freeze_id, command: "fsfreeze -f / 2>/dev/null; sync; fsfreeze -u / 2>/dev/null; true".to_string(),
        }, 10).await {
            tracing::warn!("pre-fork fsfreeze failed (non-fatal): {e}");
        }
    }

    // Clone state into new persistent sandbox
    let new_session_dir = state.run_dir.join("persistent").join(name);
    let _ = std::fs::create_dir_all(state.run_dir.join("persistent"));
    let _ = std::fs::create_dir_all(&new_session_dir);

    let size_bytes = capsem_core::auto_snapshot::clone_sandbox_state(&session_dir, &new_session_dir)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to fork sandbox: {e}")))?;

    // Register as persistent VM
    {
        let mut registry = state.persistent_registry.lock().unwrap();
        registry.register(PersistentVmEntry {
            name: name.clone(),
            ram_mb,
            cpus,
            base_version,
            created_at: format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()),
            session_dir: new_session_dir,
            forked_from: Some(id.clone()),
            description: payload.description.clone(),
            suspended: false,
            checkpoint_path: None,
        }).map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(Json(ForkResponse {
        name: name.clone(),
        size_bytes,
    }))
}

async fn handle_provision(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, AppError> {
    let id = payload.name.clone().unwrap_or_else(generate_tmp_name);

    match state.provision_sandbox(ProvisionOptions {
        id: &id,
        ram_mb: payload.ram_mb,
        cpus: payload.cpus,
        version_override: Some(state.current_version.clone()),
        persistent: payload.persistent,
        env: payload.env,
        from: payload.from,
        description: None,
    }) {
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

/// Attach live telemetry from session.db to a SandboxInfo.
/// Shared by handle_list (all VMs) and handle_info (single VM).
fn enrich_telemetry(info: &mut SandboxInfo, session_dir: &std::path::Path) {
    let db_path = session_dir.join("session.db");
    if let Ok(reader) = capsem_logger::DbReader::open(&db_path) {
        if let Ok(stats) = reader.session_stats() {
            info.total_input_tokens = Some(stats.total_input_tokens);
            info.total_output_tokens = Some(stats.total_output_tokens);
            info.total_estimated_cost = Some(stats.total_estimated_cost_usd);
            info.total_tool_calls = Some(stats.total_tool_calls);
            info.total_requests = Some(stats.net_total);
            info.allowed_requests = Some(stats.net_allowed);
            info.denied_requests = Some(stats.net_denied);
            info.model_call_count = Some(stats.model_call_count);
        }
        if let Ok(fc) = reader.file_event_count() {
            info.total_file_events = Some(fc as u64);
        }
        if let Ok(mcp) = reader.mcp_call_stats() {
            info.total_mcp_calls = Some(mcp.total as u64);
        }
    }
}

async fn handle_list(
    State(state): State<Arc<ServiceState>>,
) -> Json<ListResponse> {
    let mut sandboxes: Vec<SandboxInfo> = Vec::new();

    // Running instances (with live telemetry)
    {
        let instances = state.instances.lock().unwrap();
        for i in instances.values() {
            let mut info = SandboxInfo::new(i.id.clone(), i.pid, "Running".into(), i.persistent);
            info.name = if i.persistent { Some(i.id.clone()) } else { None };
            info.ram_mb = Some(i.ram_mb);
            info.cpus = Some(i.cpus);
            info.version = Some(i.base_version.clone());
            info.forked_from = i.forked_from.clone();
            info.uptime_secs = Some(i.start_time.elapsed().as_secs());
            enrich_telemetry(&mut info, &i.session_dir);
            sandboxes.push(info);
        }
    }

    // Stopped/Suspended persistent VMs (not in instances map)
    {
        let registry = state.persistent_registry.lock().unwrap();
        let instances = state.instances.lock().unwrap();
        for entry in registry.list() {
            if !instances.contains_key(&entry.name) {
                let status = if entry.suspended { "Suspended" } else { "Stopped" };
                let mut info = SandboxInfo::new(entry.name.clone(), 0, status.into(), true);
                info.name = Some(entry.name.clone());
                info.ram_mb = Some(entry.ram_mb);
                info.cpus = Some(entry.cpus);
                info.version = Some(entry.base_version.clone());
                info.forked_from = entry.forked_from.clone();
                info.description = entry.description.clone();
                sandboxes.push(info);
            }
        }
    }

    Json(ListResponse { sandboxes })
}

async fn handle_info(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<SandboxInfo>, AppError> {
    // Check running instances first
    {
        let (instance_data, session_dir) = {
            let instances = state.instances.lock().unwrap();
            match instances.get(&id) {
                Some(i) => {
                    let mut info = SandboxInfo::new(i.id.clone(), i.pid, "Running".into(), i.persistent);
                    info.name = if i.persistent { Some(i.id.clone()) } else { None };
                    info.ram_mb = Some(i.ram_mb);
                    info.cpus = Some(i.cpus);
                    info.version = Some(i.base_version.clone());
                    info.forked_from = i.forked_from.clone();
                    info.uptime_secs = Some(i.start_time.elapsed().as_secs());
                    (Some(info), Some(i.session_dir.clone()))
                }
                None => (None, None),
            }
        };
        if let (Some(mut info), Some(dir)) = (instance_data, session_dir) {
            enrich_telemetry(&mut info, &dir);
            return Ok(Json(info));
        }
    }

    // Check stopped/suspended persistent VMs
    {
        let registry = state.persistent_registry.lock().unwrap();
        if let Some(entry) = registry.get(&id) {
            let status = if entry.suspended { "Suspended" } else { "Stopped" };
            let mut info = SandboxInfo::new(entry.name.clone(), 0, status.into(), true);
            info.name = Some(entry.name.clone());
            info.ram_mb = Some(entry.ram_mb);
            info.cpus = Some(entry.cpus);
            info.version = Some(entry.base_version.clone());
            info.forked_from = entry.forked_from.clone();
            info.description = entry.description.clone();
            return Ok(Json(info));
        }
    }

    Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))
}

/// GET /stats -- return full main.db aggregation in one response.
async fn handle_stats(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<StatsResponse>, AppError> {
    let db_path = state.main_db_path();
    let index = capsem_core::session::SessionIndex::open(&db_path)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to open main.db: {e}")))?;

    let global = index.global_stats()
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("global_stats: {e}")))?;
    let sessions = index.recent(100)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("recent: {e}")))?;
    let top_providers = index.top_providers(20)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("top_providers: {e}")))?;
    let top_tools = index.top_tools(20)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("top_tools: {e}")))?;
    let top_mcp_tools = index.top_mcp_tools(20)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("top_mcp_tools: {e}")))?;

    Ok(Json(StatsResponse {
        global,
        sessions,
        top_providers,
        top_tools,
        top_mcp_tools,
    }))
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
        logs: serial_logs.as_deref().unwrap_or("").to_string(),
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

/// Wait until a VM signals readiness via a `.ready` sentinel file.
/// The capsem-process creates this file once the guest handshake completes.
/// Falls back to IPC Ping if the sentinel never appears (defensive).
async fn wait_for_vm_ready(uds_path: &std::path::Path, timeout_secs: u64) -> Result<(), String> {
    let ready_path = uds_path.with_extension("ready");
    capsem_core::poll::poll_until(
        capsem_core::poll::PollOpts {
            label: "vm-ready",
            timeout: std::time::Duration::from_secs(timeout_secs),
            initial_delay: std::time::Duration::from_millis(5),
            max_delay: std::time::Duration::from_millis(50),
        },
        || {
            let ready = ready_path.clone();
            async move {
                if ready.exists() { Some(()) } else { None }
            }
        },
    ).await.map_err(|e| format!("{e}"))
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
                stdout: String::from_utf8(stdout).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
                stderr: String::from_utf8(stderr).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
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
                Ok(Json(ReadFileResponse { content: String::from_utf8(d).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()) }))
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
    // Collect paths to broadcast to.
    let uds_paths = {
        let instances = state.instances.lock().unwrap();
        instances.iter().map(|(id, info)| (id.clone(), info.uds_path.clone())).collect::<Vec<_>>()
    };
    
    let results = futures::future::join_all(uds_paths.iter().map(|(id, uds_path)| {
        let id = id.clone();
        async move {
            match send_ipc_command(uds_path, ServiceToProcess::ReloadConfig, 5).await {
                Ok(ProcessToService::Pong) => None,
                Ok(_) => Some(format!("{id}: unexpected response")),
                Err(e) => Some(format!("{id}: {e}")),
            }
        }
    })).await;
    let failures: Vec<String> = results.into_iter().flatten().collect();

    if failures.is_empty() {
        Ok(Json(serde_json::json!({ "success": true, "reloaded": uds_paths.len() })))
    } else {
        Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to reload config in some instances: {}", failures.join(", ")),
        ))
    }
}

// ---------------------------------------------------------------------------
// Settings endpoints
// ---------------------------------------------------------------------------

/// GET /settings -- unified settings tree + issues + presets.
async fn handle_get_settings() -> Json<serde_json::Value> {
    let resp = capsem_core::net::policy_config::load_settings_response();
    Json(serde_json::to_value(resp).unwrap_or_default())
}

/// POST /settings -- batch-update settings and return the refreshed tree.
async fn handle_save_settings(
    Json(raw): Json<HashMap<String, serde_json::Value>>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Convert JSON values to SettingValue via serde round-trip.
    let mut changes = HashMap::new();
    for (key, val) in raw {
        let sv: capsem_core::net::policy_config::SettingValue =
            serde_json::from_value(val.clone()).map_err(|e| {
                AppError(StatusCode::BAD_REQUEST, format!("invalid value for {key}: {e}"))
            })?;
        changes.insert(key, sv);
    }
    capsem_core::net::policy_config::batch_update_settings(&changes)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
    let resp = capsem_core::net::policy_config::load_settings_response();
    Ok(Json(serde_json::to_value(resp).unwrap_or_default()))
}

/// GET /settings/presets -- list security presets.
async fn handle_get_presets() -> Json<serde_json::Value> {
    let presets = capsem_core::net::policy_config::security_presets();
    Json(serde_json::to_value(presets).unwrap_or_default())
}

/// POST /settings/presets/{id} -- apply a security preset, return refreshed tree.
async fn handle_apply_preset(
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    capsem_core::net::policy_config::apply_preset(&id)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
    let resp = capsem_core::net::policy_config::load_settings_response();
    Ok(Json(serde_json::to_value(resp).unwrap_or_default()))
}

/// POST /settings/lint -- validate config and return issues.
async fn handle_lint_config() -> Json<serde_json::Value> {
    let issues = capsem_core::net::policy_config::load_merged_lint();
    Json(serde_json::to_value(issues).unwrap_or_default())
}

async fn handle_inspect(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<InspectRequest>,
) -> Result<impl IntoResponse, AppError> {
    // _main sentinel routes to the global session index (main.db).
    if id == "_main" {
        let db_path = state.main_db_path();
        let index = capsem_core::session::SessionIndex::open(&db_path)
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to open main.db: {e}")))?;
        let json_str = index.query_raw(&payload.sql, &[])
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("query failed: {e}")))?;
        return Ok((
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            json_str,
        ));
    }

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
        json_str,
    ))
}

/// Wait for a process to exit, force-killing after timeout.
async fn wait_for_process_exit(pid: u32, timeout: std::time::Duration) {
    if pid == 0 {
        return;
    }
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if unsafe { nix::libc::kill(pid as i32, 0) } != 0 {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(pid, "VM process did not exit within timeout, sending SIGKILL");
            let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGKILL);
            // Wait up to 2s for SIGKILL to take effect
            for _ in 0..20 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                if unsafe { nix::libc::kill(pid as i32, 0) } != 0 {
                    return;
                }
            }
            tracing::error!(pid, "VM process survived SIGKILL");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Shutdown a running VM process by ID. Returns (session_dir, persistent, pid).
///
/// Sends the shutdown signal and removes the instance from the registry
/// immediately, then spawns a background task to wait for process exit and
/// force-kill if needed. Callers do not block on process teardown.
///
/// Callers that need the session DB (e.g. handle_run telemetry rollup) can
/// use the returned pid with `wait_for_process_exit` before reading.
async fn shutdown_vm_process(state: &ServiceState, id: &str) -> Option<(PathBuf, bool, u32)> {
    let (uds_path, session_dir, pid, persistent) = {
        let instances = state.instances.lock().unwrap();
        let i = instances.get(id)?;
        (i.uds_path.clone(), i.session_dir.clone(), i.pid, i.persistent)
    };

    // Send shutdown command via IPC (or SIGTERM as fallback).
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

    // Remove from active instances immediately so the service considers this
    // VM gone. The spawned child-exit handler may also call remove (idempotent).
    state.instances.lock().unwrap().remove(id);

    // Background: wait for process exit, force-kill if stuck, clean up socket.
    tokio::spawn(async move {
        wait_for_process_exit(pid, std::time::Duration::from_secs(5)).await;
        let _ = std::fs::remove_file(&uds_path);
        let _ = std::fs::remove_file(uds_path.with_extension("ready"));
    });

    Some((session_dir, persistent, pid))
}

async fn handle_suspend(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (uds_path, pid) = {
        let mut instances = state.instances.lock().unwrap();
        let i = instances.get_mut(&id).ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        if !i.persistent {
            return Err(AppError(StatusCode::BAD_REQUEST, "ephemeral VMs cannot be suspended (persist first)".into()));
        }
        (i.uds_path.clone(), i.pid)
    };

    let stream = tokio::net::UnixStream::connect(&uds_path)
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to connect to VM IPC: {e}")))?;
    let std_stream = stream
        .into_std()
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to convert stream: {e}")))?;
    let (tx, rx) = channel_from_std::<ServiceToProcess, ProcessToService>(std_stream)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to create IPC channel: {e}")))?;

    let checkpoint_path = "checkpoint.vzsave".to_string();
    tx.send(ServiceToProcess::Suspend { checkpoint_path })
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to send suspend command: {e}")))?;

    // Wait for StateChanged { state: "Suspended" } or process exit
    let mut suspended = false;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while let Ok(msg) = rx.recv().await {
            if let ProcessToService::StateChanged { state, .. } = msg {
                if state == "Suspended" {
                    suspended = true;
                    break;
                }
            }
        }
    }).await;

    if !suspended {
        return Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "suspend timed out: VM did not confirm suspended state".into(),
        ));
    }

    // Poll for process exit (up to 500ms) instead of unconditional sleep.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(500);
    loop {
        if pid == 0 || unsafe { nix::libc::kill(pid as i32, 0) } != 0 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGKILL);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    state.instances.lock().unwrap().remove(&id);
    let _ = std::fs::remove_file(&uds_path);
    let _ = std::fs::remove_file(uds_path.with_extension("ready"));

    // Update persistent registry
    {
        let mut registry = state.persistent_registry.lock().unwrap();
        if let Some(entry) = registry.get_mut(&id) {
            entry.suspended = true;
            entry.checkpoint_path = Some("checkpoint.vzsave".to_string());
            if let Err(e) = registry.save() {
                error!(id, "failed to save persistent registry: {e}");
            }
        }
    }

    Ok(Json(serde_json::json!({ "success": true })))
}

async fn handle_stop(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    if let Some((session_dir, persistent, pid)) = shutdown_vm_process(&state, &id).await {
        // Wait for process to actually exit before returning, so resume
        // doesn't race with the old process on the same socket.
        if pid > 0 {
            for _ in 0..10 {
                if unsafe { nix::libc::kill(pid as i32, 0) } != 0 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            if unsafe { nix::libc::kill(pid as i32, 0) } == 0 {
                let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGKILL);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
        if !persistent {
            let dir = session_dir;
            tokio::task::spawn_blocking(move || { let _ = std::fs::remove_dir_all(&dir); });
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
    // Shut down if running, then ensure the process is dead before returning.
    // shutdown_vm_process sends graceful Shutdown + spawns a background reaper.
    // Give the process 500ms to flush session DB, then SIGKILL if still alive.
    let session_dir = if let Some((session_dir, _, pid)) = shutdown_vm_process(&state, &id).await {
        if pid > 0 {
            // Wait up to 500ms for graceful exit (DB flush, cleanup)
            for _ in 0..10 {
                if unsafe { nix::libc::kill(pid as i32, 0) } != 0 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            // Force-kill if still alive
            if unsafe { nix::libc::kill(pid as i32, 0) } == 0 {
                let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGKILL);
                for _ in 0..10 {
                    if unsafe { nix::libc::kill(pid as i32, 0) } != 0 {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
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
    tokio::task::spawn_blocking(move || { let _ = std::fs::remove_dir_all(&session_dir); });

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
    let (old_session_dir, ram_mb, cpus, base_version, forked_from) = {
        let instances = state.instances.lock().unwrap();
        let i = instances.get(&id).ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        if i.persistent {
            return Err(AppError(StatusCode::BAD_REQUEST, format!("VM \"{}\" is already persistent", id)));
        }
        (i.session_dir.clone(), i.ram_mb, i.cpus, i.base_version.clone(), i.forked_from.clone())
    };

    // Move session dir to persistent location
    let new_session_dir = state.run_dir.join("persistent").join(name);
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
            forked_from: forked_from.clone(),
            description: None,
            suspended: false,
            checkpoint_path: None,
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
                forked_from,
            });
        }
    }

    Ok(Json(json!({ "success": true, "name": name })))
}

async fn handle_purge(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<PurgeRequest>,
) -> Result<Json<PurgeResponse>, AppError> {
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

    let results = futures::future::join_all(to_purge.iter().map(|(id, persistent)| {
        let state_ref = &state;
        let id = id.clone();
        let persistent = *persistent;
        async move {
            if let Some((session_dir, _, _pid)) = shutdown_vm_process(state_ref, &id).await {
                Some((id, session_dir, persistent))
            } else {
                None
            }
        }
    })).await;

    for item in results.into_iter().flatten() {
        let (id, session_dir, persistent) = item;
        if persistent {
            let mut registry = state.persistent_registry.lock().unwrap();
            let _ = registry.unregister(&id);
        }
        let dir = session_dir;
        tokio::task::spawn_blocking(move || { let _ = std::fs::remove_dir_all(&dir); });
        if persistent { persistent_purged += 1; } else { ephemeral_purged += 1; }
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
                tokio::task::spawn_blocking(move || { let _ = std::fs::remove_dir_all(&dir); });
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
    let id = generate_tmp_name();

    let ram_bytes = payload.ram_mb * 1024 * 1024;
    let session_dir = state.run_dir.join("sessions").join(&id);

    // 1. Provision ephemeral VM
    state.provision_sandbox(ProvisionOptions {
        id: &id,
        ram_mb: payload.ram_mb,
        cpus: payload.cpus,
        version_override: Some(state.current_version.clone()),
        persistent: false,
        env: payload.env,
        from: None,
        description: None,
    })
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
            forked_from: None,
            persistent: false,
        };
        if let Err(e) = idx.create_session(&record) {
            tracing::warn!("failed to register session in main.db: {e}");
        }
    }

    // 3. Wait for VM socket to appear
    let uds_path = state.instance_socket_path(&id);
    if let Err(e) = wait_for_vm_ready(&uds_path, 30).await {
        let _ = shutdown_vm_process(&state, &id).await;
        let dir = session_dir;
        tokio::task::spawn_blocking(move || { let _ = std::fs::remove_dir_all(&dir); });
        return Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, e));
    }

    // 4. Execute command
    let job_id = state.next_job_id();
    let exec_result = send_ipc_command(
        &uds_path,
        ServiceToProcess::Exec { id: job_id, command: payload.command },
        payload.timeout_secs,
    ).await;

    // 5. Tear down VM process and build response immediately.
    let pid = shutdown_vm_process(&state, &id).await.map(|(_, _, p)| p).unwrap_or(0);

    let response = match exec_result {
        Ok(ProcessToService::ExecResult { stdout, stderr, exit_code, .. }) => {
            Ok(Json(ExecResponse {
                stdout: String::from_utf8(stdout).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
                stderr: String::from_utf8(stderr).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
                exit_code,
            }))
        }
        Ok(_) => Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, "unexpected IPC response".into())),
        Err(e) => Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("exec failed: {e}"))),
    };

    // 6. Roll up session counters before returning, so callers see consistent
    //    data in main.db. Wait for the process to exit so DbWriter has flushed.
    if let Some(idx) = index {
        wait_for_process_exit(pid, std::time::Duration::from_secs(5)).await;
        let session_db_path = session_dir.join("session.db");
        if session_db_path.exists() {
            if let Ok(reader) = capsem_logger::DbReader::open(&session_db_path) {
                if let Ok(counts) = reader.net_event_counts() {
                    let _ = idx.update_request_counts(
                        &id, counts.total as u64, counts.allowed as u64, counts.denied as u64,
                    );
                }
                let file_events = reader.file_event_count().unwrap_or(0);
                let mcp_calls = reader.mcp_call_stats().map(|s| s.total).unwrap_or(0);
                let _ = idx.update_session_summary(
                    &id, 0, 0, 0.0, 0, mcp_calls, file_events,
                );
            }
        }
        let _ = idx.update_status(&id, "stopped", Some(&capsem_core::session::now_iso()));
    }

    response
}

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, fmt};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    let home = std::env::var("HOME").context("HOME not set")?;
    let run_dir = std::env::var("CAPSEM_RUN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(&home).join(".capsem/run"));
    
    let _home_capsem = PathBuf::from(&home).join(".capsem");
    
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

    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(persistent_registry),
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

    // Periodic cleanup of stale instances (replaces per-handler calls).
    {
        let state_for_cleanup = Arc::clone(&state);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                state_for_cleanup.cleanup_stale_instances();
            }
        });
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
        .route("/suspend/{id}", post(handle_suspend))
        .route("/delete/{id}", delete(handle_delete))
        .route("/resume/{name}", post(handle_resume))
        .route("/persist/{id}", post(handle_persist))
        .route("/purge", post(handle_purge))
        .route("/run", post(handle_run))
        .route("/stats", get(handle_stats))
        .route("/reload-config", post(handle_reload_config))
        .route("/fork/{id}", post(handle_fork))
        .route("/settings", get(handle_get_settings).post(handle_save_settings))
        .route("/settings/presets", get(handle_get_presets))
        .route("/settings/presets/{id}", post(handle_apply_preset))
        .route("/settings/lint", post(handle_lint_config))
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

/// Open a log file for a companion process, returning Stdio handles for stdout and stderr.
/// Falls back to null if the file cannot be opened.
fn companion_stdio(log_path: &std::path::Path) -> (std::process::Stdio, std::process::Stdio) {
    match std::fs::OpenOptions::new().create(true).append(true).open(log_path) {
        Ok(f) => {
            let stdout = f
                .try_clone()
                .map(std::process::Stdio::from)
                .unwrap_or_else(|_| std::process::Stdio::null());
            let stderr = std::process::Stdio::from(f);
            (stdout, stderr)
        }
        Err(_) => (std::process::Stdio::null(), std::process::Stdio::null()),
    }
}

/// Spawn the gateway and tray as child processes of the service.
async fn spawn_companions(
    service_sock: &std::path::Path,
    run_dir: &std::path::Path,
) -> Vec<tokio::process::Child> {
    let mut children = Vec::new();

    // Log files for companion processes (~/Library/Logs/capsem/ on macOS)
    let log_dir = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Library/Logs/capsem"))
        .unwrap_or_else(|_| run_dir.join("logs"));
    let _ = std::fs::create_dir_all(&log_dir);

    // 1. Spawn capsem-gateway (TCP reverse proxy -> UDS)
    let gateway_bin = find_sibling_binary("capsem-gateway");
    let (gw_out, gw_err) = companion_stdio(&log_dir.join("gateway.log"));
    info!(binary = %gateway_bin.display(), "spawning capsem-gateway");
    match tokio::process::Command::new(&gateway_bin)
        .arg("--uds-path")
        .arg(service_sock)
        .stdout(gw_out)
        .stderr(gw_err)
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => {
            info!(pid = child.id(), "capsem-gateway spawned");
            children.push(child);

            // Wait for gateway to write token + port files (up to 5s)
            let token_path = run_dir.join("gateway.token");
            let port_path = run_dir.join("gateway.port");
            {
                let tp = token_path.clone();
                let pp = port_path.clone();
                let _ = capsem_core::poll::poll_until(
                    capsem_core::poll::PollOpts::new("gateway-ready", std::time::Duration::from_secs(5)),
                    || {
                        let tp = tp.clone();
                        let pp = pp.clone();
                        async move {
                            if tp.exists() && pp.exists() { Some(()) } else { None }
                        }
                    },
                ).await;
            }

            // 2. Spawn capsem-tray (menu bar) -- only on macOS, only after gateway ready
            #[cfg(target_os = "macos")]
            if token_path.exists() {
                let tray_bin = find_sibling_binary("capsem-tray");
                let (tray_out, tray_err) = companion_stdio(&log_dir.join("tray.log"));
                info!(binary = %tray_bin.display(), "spawning capsem-tray");
                match tokio::process::Command::new(&tray_bin)
                    .stdout(tray_out)
                    .stderr(tray_err)
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
                forked_from: None,
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
    fn long_vm_name_falls_back_to_tmp_socket() {
        let state = make_test_state();
        // A 100-char name exceeds SUN_PATH_MAX via run_dir/instances/ path,
        // but instance_socket_path should fall back to /tmp/capsem/.
        let long_name = "a".repeat(100);
        let path = state.instance_socket_path(&long_name);
        assert!(path.starts_with("/tmp/capsem/"), "expected /tmp/capsem/ fallback, got: {}", path.display());
        assert!(path.as_os_str().len() < 104, "fallback path still too long: {}", path.as_os_str().len());
    }

    #[test]
    fn short_vm_name_uses_run_dir() {
        let state = make_test_state();
        let path = state.instance_socket_path("test-vm");
        assert_eq!(path, state.run_dir.join("instances/test-vm.sock"));
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
        let result = state.provision_sandbox(ProvisionOptions {
            id: &ok_name,
            ram_mb: 2048,
            cpus: 2,
            version_override: None,
            persistent: false,
            env: None,
            from: None,
            description: None,
        });
        // Will fail later (missing rootfs), but NOT for path length
        if let Err(e) = &result {
            let msg = e.to_string();
            assert!(!msg.contains("socket path"), "short name should not hit path limit: {msg}");
        }
    }

    #[test]
    fn provision_short_name_passes_path_check() {
        let state = make_test_state();
        let result = state.provision_sandbox(ProvisionOptions {
            id: "my-vm",
            ram_mb: 2048,
            cpus: 2,
            version_override: None,
            persistent: false,
            env: None,
            from: None,
            description: None,
        });
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
    // Image name validation (path traversal defense)
    // -----------------------------------------------------------------------

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
            forked_from: None,
            description: None,
            suspended: false,
            checkpoint_path: None,
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
            forked_from: None,
            description: None,
            suspended: false,
            checkpoint_path: None,
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
            forked_from: None,
            description: None,
            suspended: false,
            checkpoint_path: None,
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
                forked_from: None,
                description: None,
                suspended: false,
                checkpoint_path: None,
            });
        }
        let result = state.provision_sandbox(ProvisionOptions {
            id: "taken",
            ram_mb: 2048,
            cpus: 2,
            version_override: None,
            persistent: true,
            env: None,
            from: None,
            description: None,
        });
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("already exists"), "expected duplicate error, got: {err}");
        assert!(err.contains("resume"), "should suggest resume, got: {err}");
    }

    #[test]
    fn provision_persistent_validates_name() {
        let state = make_test_state();
        let result = state.provision_sandbox(ProvisionOptions {
            id: "../evil",
            ram_mb: 2048,
            cpus: 2,
            version_override: None,
            persistent: true,
            env: None,
            from: None,
            description: None,
        });
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
            process_binary: PathBuf::from("/nonexistent/capsem-process"),
            assets_dir: dir.path().join("assets"),
            run_dir: dir.path().to_path_buf(),
            job_counter: AtomicU64::new(1),
            asset_manager: Arc::new(am),
            current_version: "0.0.0".into(),
        });
        (state, dir)
    }

    #[tokio::test]
    async fn handle_fork_creates_persistent_sandbox() {
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
                forked_from: None,
            },
        );
        let result = handle_fork(
            State(state.clone()),
            Path("fork-src".into()),
            Json(ForkRequest { name: "my-fork".into(), description: Some("test".into()) }),
        ).await.unwrap();
        assert_eq!(result.0.name, "my-fork");
        assert!(result.0.size_bytes > 0);
        // Verify fork created a persistent sandbox entry in the registry
        let registry = state.persistent_registry.lock().unwrap();
        let entry = registry.get("my-fork").unwrap();
        assert_eq!(entry.forked_from, Some("fork-src".into()));
        assert_eq!(entry.description, Some("test".into()));
        assert_eq!(entry.base_version, "0.0.0");
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
                forked_from: None,
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
                forked_from: None,
                description: None,
                suspended: false,
                checkpoint_path: None,
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
    fn provision_rejects_nonexistent_source_sandbox() {
        let (state, _dir) = make_test_state_with_tempdir();
        let result = state.provision_sandbox(ProvisionOptions {
            id: "vm1",
            ram_mb: 2048,
            cpus: 2,
            version_override: None,
            persistent: false,
            env: None,
            from: Some("ghost-sandbox".into()),
            description: None,
        });
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "expected sandbox not found, got: {err}");
    }

    // -----------------------------------------------------------------------
    // Suspend/resume registry fixes (issues #4-8)
    // -----------------------------------------------------------------------

    #[test]
    fn persistent_registry_get_mut() {
        let dir = std::env::temp_dir().join("capsem-test-registry-getmut");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_registry.json");
        let _ = std::fs::remove_file(&path);

        let mut registry = PersistentRegistry::load(path);
        registry.register(PersistentVmEntry {
            name: "mutvm".into(),
            ram_mb: 2048,
            cpus: 2,
            base_version: "0.1.0".into(),
            created_at: "12345".into(),
            session_dir: dir.join("mutvm"),
            forked_from: None,
            description: None,
            suspended: false,
            checkpoint_path: None,
        }).unwrap();

        // Mutate via get_mut
        let entry = registry.get_mut("mutvm").unwrap();
        entry.suspended = true;
        entry.checkpoint_path = Some("checkpoint.vzsave".into());
        let _ = registry.save();

        assert!(registry.get("mutvm").unwrap().suspended);
        assert_eq!(
            registry.get("mutvm").unwrap().checkpoint_path.as_deref(),
            Some("checkpoint.vzsave")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn handle_list_shows_suspended_status() {
        let (state, _dir) = make_test_state_with_tempdir();

        // Register a suspended persistent VM
        {
            let mut reg = state.persistent_registry.lock().unwrap();
            reg.data.vms.insert("susp-vm".into(), PersistentVmEntry {
                name: "susp-vm".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/susp-vm"),
                forked_from: None,
                description: None,
                suspended: true,
                checkpoint_path: Some("checkpoint.vzsave".into()),
            });
        }

        // Register a stopped (not suspended) persistent VM
        {
            let mut reg = state.persistent_registry.lock().unwrap();
            reg.data.vms.insert("stop-vm".into(), PersistentVmEntry {
                name: "stop-vm".into(),
                ram_mb: 1024,
                cpus: 1,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/stop-vm"),
                forked_from: None,
                description: None,
                suspended: false,
                checkpoint_path: None,
            });
        }

        let Json(list) = handle_list(State(state)).await;

        let susp = list.sandboxes.iter().find(|s| s.id == "susp-vm").unwrap();
        assert_eq!(susp.status, "Suspended", "suspended VM should show Suspended status");

        let stop = list.sandboxes.iter().find(|s| s.id == "stop-vm").unwrap();
        assert_eq!(stop.status, "Stopped", "non-suspended VM should show Stopped status");
    }

    #[tokio::test]
    async fn handle_info_shows_suspended_status() {
        let (state, _dir) = make_test_state_with_tempdir();

        {
            let mut reg = state.persistent_registry.lock().unwrap();
            reg.data.vms.insert("info-susp".into(), PersistentVmEntry {
                name: "info-susp".into(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: state.run_dir.join("persistent/info-susp"),
                forked_from: None,
                description: None,
                suspended: true,
                checkpoint_path: Some("checkpoint.vzsave".into()),
            });
        }

        let result = handle_info(State(state), Path("info-susp".into())).await;
        let Json(info) = result.unwrap();
        assert_eq!(info.status, "Suspended");
    }

    #[tokio::test]
    async fn handle_suspend_rejects_ephemeral_vm() {
        let (state, _dir) = make_test_state_with_tempdir();

        // Insert an ephemeral VM in instances
        {
            let mut instances = state.instances.lock().unwrap();
            instances.insert("eph-vm".into(), InstanceInfo {
                id: "eph-vm".into(),
                pid: 0,
                uds_path: state.run_dir.join("instances/eph-vm.sock"),
                session_dir: state.run_dir.join("sessions/eph-vm"),
                ram_mb: 2048,
                cpus: 2,
                start_time: std::time::Instant::now(),
                base_version: "0.0.0".into(),
                persistent: false,
                env: None,
                forked_from: None,
            });
        }

        let result = handle_suspend(State(state), Path("eph-vm".into())).await;
        let err = result.unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("ephemeral"));
    }

    #[tokio::test]
    async fn handle_suspend_returns_not_found_for_missing_vm() {
        let (state, _dir) = make_test_state_with_tempdir();
        let result = handle_suspend(State(state), Path("nonexistent".into())).await;
        let err = result.unwrap_err();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    #[test]
    fn resume_clears_suspended_flag_in_registry() {
        let dir = std::env::temp_dir().join("capsem-test-resume-flag");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_registry.json");
        let _ = std::fs::remove_file(&path);

        let mut registry = PersistentRegistry::load(path.clone());
        registry.register(PersistentVmEntry {
            name: "resumevm".into(),
            ram_mb: 2048,
            cpus: 2,
            base_version: "0.1.0".into(),
            created_at: "12345".into(),
            session_dir: dir.join("resumevm"),
            forked_from: None,
            description: None,
            suspended: true,
            checkpoint_path: Some("checkpoint.vzsave".into()),
        }).unwrap();

        // Verify suspended initially
        assert!(registry.get("resumevm").unwrap().suspended);
        assert!(registry.get("resumevm").unwrap().checkpoint_path.is_some());

        // Simulate what resume_sandbox does after spawning the process
        if let Some(entry) = registry.get_mut("resumevm") {
            entry.suspended = false;
            entry.checkpoint_path = None;
        }
        let _ = registry.save();

        // Verify cleared
        assert!(!registry.get("resumevm").unwrap().suspended);
        assert!(registry.get("resumevm").unwrap().checkpoint_path.is_none());

        // Verify persists to disk
        let registry2 = PersistentRegistry::load(path);
        assert!(!registry2.get("resumevm").unwrap().suspended);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suspended_flag_roundtrips_through_json() {
        let entry = PersistentVmEntry {
            name: "jsonvm".into(),
            ram_mb: 2048,
            cpus: 2,
            base_version: "0.1.0".into(),
            created_at: "12345".into(),
            session_dir: PathBuf::from("/tmp/jsonvm"),
            forked_from: None,
            description: None,
            suspended: true,
            checkpoint_path: Some("checkpoint.vzsave".into()),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: PersistentVmEntry = serde_json::from_str(&json).unwrap();
        assert!(parsed.suspended);
        assert_eq!(parsed.checkpoint_path.as_deref(), Some("checkpoint.vzsave"));
    }

    #[test]
    fn suspended_flag_defaults_to_false_when_missing() {
        // Old registry entries won't have the suspended field
        let json = r#"{"name":"old","ram_mb":2048,"cpus":2,"base_version":"0.1.0","created_at":"0","session_dir":"/tmp/old"}"#;
        let entry: PersistentVmEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.suspended, "suspended should default to false");
        assert!(entry.checkpoint_path.is_none(), "checkpoint_path should default to None");
    }

    // -----------------------------------------------------------------------
    // main_db_path
    // -----------------------------------------------------------------------

    #[test]
    fn main_db_path_resolves_to_sessions_dir() {
        let state = make_test_state();
        // run_dir = /tmp/capsem-test-svc => parent = /tmp => main.db = /tmp/sessions/main.db
        let path = state.main_db_path();
        assert!(path.ends_with("sessions/main.db"), "got: {}", path.display());
    }

    // -----------------------------------------------------------------------
    // SandboxInfo::new
    // -----------------------------------------------------------------------

    #[test]
    fn sandbox_info_new_defaults_telemetry_to_none() {
        let info = SandboxInfo::new("test".into(), 1, "Running".into(), false);
        assert_eq!(info.id, "test");
        assert_eq!(info.pid, 1);
        assert!(!info.persistent);
        assert!(info.total_input_tokens.is_none());
        assert!(info.total_estimated_cost.is_none());
        assert!(info.model_call_count.is_none());
        assert!(info.created_at.is_none());
        assert!(info.uptime_secs.is_none());
    }

    #[test]
    fn sandbox_info_telemetry_fields_serialize_when_present() {
        let mut info = SandboxInfo::new("test".into(), 1, "Running".into(), false);
        info.total_input_tokens = Some(1000);
        info.total_estimated_cost = Some(0.42);
        info.model_call_count = Some(5);
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"total_input_tokens\":1000"));
        assert!(json.contains("\"total_estimated_cost\":0.42"));
        assert!(json.contains("\"model_call_count\":5"));
    }

    #[test]
    fn sandbox_info_telemetry_fields_omitted_when_none() {
        let info = SandboxInfo::new("test".into(), 1, "Running".into(), false);
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("total_input_tokens"));
        assert!(!json.contains("total_estimated_cost"));
        assert!(!json.contains("model_call_count"));
        assert!(!json.contains("uptime_secs"));
    }

    #[test]
    fn sandbox_info_backwards_compatible_deserialization() {
        // Old JSON without telemetry fields should still deserialize
        let json = r#"{"id":"x","pid":1,"status":"Running","persistent":false}"#;
        let info: SandboxInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "x");
        assert!(info.total_input_tokens.is_none());
    }

    // -----------------------------------------------------------------------
    // StatsResponse
    // -----------------------------------------------------------------------

    #[test]
    fn stats_response_serializes() {
        let resp = StatsResponse {
            global: capsem_core::session::GlobalStats {
                total_sessions: 10,
                total_input_tokens: 5000,
                total_output_tokens: 2000,
                total_estimated_cost: 1.50,
                total_tool_calls: 100,
                total_mcp_calls: 20,
                total_file_events: 300,
                total_requests: 400,
                total_allowed: 380,
                total_denied: 20,
            },
            sessions: vec![],
            top_providers: vec![],
            top_tools: vec![],
            top_mcp_tools: vec![],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"total_sessions\":10"));
        assert!(json.contains("\"total_estimated_cost\":1.5"));
        assert!(json.contains("\"top_providers\":[]"));
    }

    // -----------------------------------------------------------------------
    // handle_list includes uptime_secs for running VMs
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn handle_list_includes_uptime_for_running_vms() {
        let state = make_test_state();
        insert_fake_instance(&state, "vm-1", 100);
        let resp = handle_list(State(state)).await;
        let list = resp.0;
        assert_eq!(list.sandboxes.len(), 1);
        assert!(list.sandboxes[0].uptime_secs.is_some());
    }

    // -----------------------------------------------------------------------
    // handle_stats with tempdir
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn handle_stats_returns_global_data() {
        let dir = tempfile::tempdir().unwrap();
        let run_dir = dir.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();

        // Create main.db with a test session
        let idx = capsem_core::session::SessionIndex::open(&sessions_dir.join("main.db")).unwrap();
        let record = capsem_core::session::SessionRecord {
            id: "20260412-120000-abcd".into(),
            mode: "virtiofs".into(),
            command: Some("echo hello".into()),
            status: "stopped".into(),
            created_at: "2026-04-12T12:00:00Z".into(),
            stopped_at: Some("2026-04-12T12:05:00Z".into()),
            scratch_disk_size_gb: 16,
            ram_bytes: 4294967296,
            total_requests: 50,
            allowed_requests: 45,
            denied_requests: 5,
            total_input_tokens: 10000,
            total_output_tokens: 3000,
            total_estimated_cost: 0.42,
            total_tool_calls: 25,
            total_mcp_calls: 5,
            total_file_events: 100,
            compressed_size_bytes: None,
            vacuumed_at: None,
            storage_mode: "virtiofs".into(),
            rootfs_hash: None,
            rootfs_version: None,
            forked_from: None,
            persistent: false,
        };
        idx.create_session(&record).unwrap();
        drop(idx);

        let (state, _dir) = make_test_state_with_tempdir_at(dir);
        let result = handle_stats(State(state)).await;
        assert!(result.is_ok());
        let resp = result.unwrap().0;
        assert_eq!(resp.global.total_sessions, 1);
        assert_eq!(resp.global.total_input_tokens, 10000);
        assert_eq!(resp.global.total_estimated_cost, 0.42);
        assert_eq!(resp.sessions.len(), 1);
        assert_eq!(resp.sessions[0].id, "20260412-120000-abcd");
    }

    // -----------------------------------------------------------------------
    // Settings handler tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn handle_get_settings_returns_tree() {
        let Json(val) = handle_get_settings().await;
        assert!(val.get("tree").is_some(), "response must have 'tree'");
        assert!(val.get("issues").is_some(), "response must have 'issues'");
        assert!(val.get("presets").is_some(), "response must have 'presets'");
        assert!(val["tree"].is_array());
        assert!(val["issues"].is_array());
        assert!(val["presets"].is_array());
    }

    #[tokio::test]
    async fn handle_get_presets_returns_list() {
        let Json(val) = handle_get_presets().await;
        let arr = val.as_array().expect("presets should be an array");
        assert!(!arr.is_empty(), "should have at least one preset");
        assert!(arr[0].get("id").is_some());
        assert!(arr[0].get("name").is_some());
        assert!(arr[0].get("settings").is_some());
    }

    #[tokio::test]
    async fn handle_lint_config_returns_array() {
        let Json(val) = handle_lint_config().await;
        assert!(val.is_array(), "lint response should be an array");
    }

    #[tokio::test]
    async fn handle_save_settings_rejects_unknown_key() {
        let mut changes = HashMap::new();
        changes.insert("nonexistent.setting.xyz".into(), serde_json::json!("value"));
        let result = handle_save_settings(Json(changes)).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    fn make_test_state_with_tempdir_at(dir: tempfile::TempDir) -> (Arc<ServiceState>, tempfile::TempDir) {
        let run_dir = dir.path().join("run");
        let dummy_hash = "a".repeat(64);
        let manifest_json = format!(
            r#"{{"latest":"0.0.0","releases":{{"0.0.0":{{"assets":[{{"filename":"dummy.img","hash":"{}","size":0}}]}}}}}}"#,
            dummy_hash
        );
        let manifest = capsem_core::asset_manager::Manifest::from_json(&manifest_json).unwrap();
        let am = capsem_core::asset_manager::AssetManager::from_manifest(
            &manifest, "0.0.0", run_dir.join("assets"), None
        ).unwrap();
        let registry_path = run_dir.join("persistent_registry.json");
        let state = Arc::new(ServiceState {
            instances: Mutex::new(HashMap::new()),
            persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
            process_binary: PathBuf::from("/nonexistent/capsem-process"),
            assets_dir: run_dir.join("assets"),
            run_dir,
            job_counter: AtomicU64::new(1),
            asset_manager: Arc::new(am),
            current_version: "0.0.0".into(),
        });
        (state, dir)
    }
}
