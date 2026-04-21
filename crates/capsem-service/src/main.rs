use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use anyhow::{Context, Result, anyhow};
use clap::Parser;
use tracing::{info, warn, error};
use axum::{
    routing::{get, post, delete},
    extract::{Path, Query, State},
    response::IntoResponse,
    Json, Router,
};
use tokio::net::UnixListener;
use tokio_unix_ipc::{channel_from_std, Sender, Receiver};
use capsem_proto::ipc::{ServiceToProcess, ProcessToService};
use tower_http::trace::TraceLayer;
use serde::{Deserialize, Serialize};
use serde_json::json;

mod startup;

use capsem_service::api;
use capsem_service::api::*;
use capsem_service::naming::{generate_tmp_name, validate_vm_name};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)] foreground: bool,
    #[arg(long)] uds_path: Option<PathBuf>,
    #[arg(long)] process_binary: Option<PathBuf>,
    #[arg(long)] gateway_binary: Option<PathBuf>,
    #[arg(long)] gateway_port: Option<u16>,
    #[arg(long)] tray_binary: Option<PathBuf>,
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
    /// User-provided env vars from /provision -- replayed on every resume so the
    /// guest sees the same environment after stop+resume cycles.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    env: Option<std::collections::HashMap<String, String>>,
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
    /// v2 manifest (None in dev mode where assets use logical names)
    manifest: Option<Arc<capsem_core::asset_manager::ManifestV2>>,
    current_version: String,
    /// Magika file-type detection session (thread-safe, shared)
    magika: Mutex<magika::Session>,
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

/// Maximum number of `-failed-*` session dirs preserved across crashes /
/// wait_for_vm_ready timeouts / dead-process cleanup. The preserved dirs
/// hold the only host-side post-mortem signal we have (process.log,
/// mcp-aggregator.stderr.log, serial.log, session.db), so too few is
/// useless and too many accumulates disk for rare events.
const MAX_FAILED_SESSIONS: usize = 5;

impl ServiceState {
    /// Build the Unix socket path for a VM instance.
    ///
    /// Delegates to `capsem_core::uds::instance_socket_path`, the single
    /// source of truth for the macOS `SUN_LEN` workaround. Logs when the
    /// fallback path is used so clients can correlate.
    fn instance_socket_path(&self, id: &str) -> PathBuf {
        let path = capsem_core::uds::instance_socket_path(&self.run_dir, id);
        if !path.starts_with(&self.run_dir) {
            let preferred = self.run_dir.join("instances").join(format!("{id}.sock"));
            tracing::info!(%id, original = %preferred.display(), short = %path.display(),
                           "socket path too long, using /tmp/capsem/");
        }
        path
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

    /// Probe instance PIDs and evict entries whose process is gone.
    ///
    /// Two-phase so the instances mutex is held only for the PID probe +
    /// map removal. The returned entries still have session dirs / UDS
    /// sockets on disk -- the caller is responsible for scrubbing those
    /// OUTSIDE the lock, otherwise a concurrent `instances.lock()` caller
    /// would wait for `remove_dir_all` to finish.
    #[must_use = "evicted entries still have filesystem artifacts; pass each to ServiceState::scrub_evicted_instance"]
    fn drain_dead_instances(&self) -> Vec<(String, InstanceInfo)> {
        let mut instances = self.instances.lock().unwrap();
        let dead_ids: Vec<String> = instances
            .iter()
            .filter(|(_, info)| unsafe { nix::libc::kill(info.pid as i32, 0) } != 0)
            .map(|(id, _)| id.clone())
            .collect();
        dead_ids
            .into_iter()
            .filter_map(|id| {
                tracing::warn!(id, "drain_dead_instances removing instance");
                instances.remove(&id).map(|info| (id, info))
            })
            .collect()
    }

    /// Scrub filesystem artifacts for a dead-process instance: preserve
    /// the ephemeral session dir for post-mortem (rename + cull) and
    /// clean up its UDS sockets. Persistent VMs keep their session dir
    /// untouched -- they're designed to survive.
    ///
    /// MUST be called OUTSIDE the instances mutex -- `remove_dir_all`
    /// and `rename` can block on large dirs and stall other handlers
    /// racing for the lock.
    fn scrub_evicted_instance(&self, id: &str, info: &InstanceInfo) {
        if info.persistent {
            info!(id, "persistent VM process died, preserving session dir");
        } else {
            info!(id, "ephemeral VM process died, preserving session dir for post-mortem");
            self.preserve_failed_session_dir(&info.session_dir, id);
        }
        let _ = std::fs::remove_file(&info.uds_path);
        let _ = std::fs::remove_file(info.uds_path.with_extension("ready"));
    }

    fn cleanup_stale_instances(&self) {
        for (id, info) in self.drain_dead_instances() {
            info!(id, "removing stale instance record");
            self.scrub_evicted_instance(&id, &info);
        }
    }

    /// Rename an ephemeral session dir to a `-failed-*` sibling so its
    /// logs survive for post-mortem, then cull down to
    /// `MAX_FAILED_SESSIONS`.
    ///
    /// Three loss paths converge here: (a) `handle_run`'s
    /// `wait_for_vm_ready` timeout, (b) `scrub_evicted_instance` when
    /// cleanup detects a dead capsem-process, (c) the unexpected
    /// child-exit handler in `provision_sandbox`. All three cases are
    /// "the process we wanted died" -- exactly when you need
    /// `process.log`, `mcp-aggregator.stderr.log`, `serial.log`, and
    /// `session.db` most. Call this instead of `remove_dir_all` on
    /// every such path.
    ///
    /// If the rename fails (EEXIST, permission, different filesystem,
    /// etc.) we `warn!` with the specific error and fall back to
    /// `remove_dir_all` so disk isn't leaked when the filesystem is
    /// already unhappy.
    fn preserve_failed_session_dir(&self, session_dir: &std::path::Path, id: &str) {
        let failed_id = format!(
            "{}-failed-{}",
            id,
            capsem_core::session::generate_session_id(),
        );
        let failed_dir = self.run_dir.join("sessions").join(&failed_id);
        match std::fs::rename(session_dir, &failed_dir) {
            Ok(()) => {
                info!(
                    id,
                    path = %failed_dir.display(),
                    "preserved failed session dir for post-mortem"
                );
                if let Err(e) = self.cull_failed_sessions() {
                    warn!(
                        error = %e,
                        "failed to cull old failed session dirs -- disk may grow beyond {MAX_FAILED_SESSIONS}"
                    );
                }
            }
            Err(e) => {
                warn!(
                    id,
                    from = %session_dir.display(),
                    to = %failed_dir.display(),
                    error = %e,
                    "failed to preserve session dir for post-mortem -- logs lost; removing to reclaim disk"
                );
                if let Err(e) = std::fs::remove_dir_all(session_dir) {
                    warn!(
                        id,
                        path = %session_dir.display(),
                        error = %e,
                        "also failed to remove session dir -- orphaned on disk"
                    );
                }
            }
        }
    }

    fn cull_failed_sessions(&self) -> Result<()> {
        let sessions_dir = self.run_dir.join("sessions");
        if !sessions_dir.exists() {
            return Ok(());
        }
        let mut failed_dirs: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        let entries = std::fs::read_dir(&sessions_dir)
            .with_context(|| format!("read_dir({})", sessions_dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.contains("-failed-") {
                continue;
            }
            // If we can't stat, skip rather than fail the whole cull --
            // we'd rather leave one undateable dir than abort the prune.
            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    failed_dirs.push((path, modified));
                }
            }
        }
        failed_dirs.sort_by(|a, b| a.1.cmp(&b.1));
        if failed_dirs.len() > MAX_FAILED_SESSIONS {
            let to_delete = failed_dirs.len() - MAX_FAILED_SESSIONS;
            for (path, _) in failed_dirs.iter().take(to_delete) {
                info!(path = %path.display(), "culling old failed session dir");
                if let Err(e) = std::fs::remove_dir_all(path) {
                    warn!(path = %path.display(), error = %e, "cull remove_dir_all failed");
                }
            }
        }
        Ok(())
    }

    fn provision_sandbox(
        self: &Arc<Self>,
        options: ProvisionOptions,
    ) -> Result<()> {
        let ProvisionOptions { id, ram_mb, cpus, version_override, persistent, env, from, description } = options;

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

        // Stale-record reclamation only runs when we'd otherwise reject the
        // provision. The probe acquires the instances mutex that many other
        // handlers contend for, and with the lock-released-before-fs-io
        // contract of `cleanup_stale_instances` the cost is minimal, but
        // this still skips an avoidable acquisition on the common path.
        let cleanup_needed = {
            let instances = self.instances.lock().unwrap();
            instances.contains_key(id) || instances.len() >= max_concurrent_vms
        };
        if cleanup_needed {
            self.cleanup_stale_instances();
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

        let resolved = self.resolve_asset_paths()?;
        if !resolved.rootfs.exists() {
            let entries = std::fs::read_dir(&self.assets_dir)
                .map(|d| d.map(|e| e.unwrap().file_name()).collect::<Vec<_>>())
                .unwrap_or_default();
            error!(rootfs = %resolved.rootfs.display(), ?entries, "rootfs NOT FOUND");
            return Err(anyhow!("rootfs not found at {}. Dir entries: {:?}", resolved.rootfs.display(), entries));
        }

        info!(process_binary = %self.process_binary.display(), exists = self.process_binary.exists(), "checking process_binary");

        info!(id, version, asset_version = %resolved.asset_version, "spawning capsem-process");

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
        // CAPSEM_{USER,CORP}_CONFIG are forwarded so the child loads the
        // same settings tree as the service (tests rely on this to route
        // policy through an isolated test config without touching the
        // real ~/.capsem/user.toml).
        child_cmd.env_clear();
        for key in &["HOME", "PATH", "USER", "TMPDIR", "CAPSEM_USER_CONFIG", "CAPSEM_CORP_CONFIG"] {
            if let Ok(val) = std::env::var(key) {
                child_cmd.env(key, val);
            }
        }

        let mut child = child_cmd
            .env("RUST_LOG", "capsem=info")
            .arg("--id").arg(id)
            .arg("--assets-dir").arg(&self.assets_dir)
            .arg("--rootfs").arg(&resolved.rootfs)
            .arg("--kernel").arg(&resolved.kernel)
            .arg("--initrd").arg(&resolved.initrd)
            .arg("--session-dir").arg(&session_dir)
            .arg("--cpus").arg(cpus.to_string())
            .arg("--ram-mb").arg(ram_mb.to_string())
            .arg("--uds-path").arg(&uds_path)
            .stdout(std::process::Stdio::from(process_log_file.try_clone()?))
            .stderr(std::process::Stdio::from(process_log_file))
            .spawn()
            .context("failed to spawn capsem-process")?;

        let pid = child.id().unwrap_or(0);
        info!(id, pid, version, asset_version = %resolved.asset_version, "capsem-process spawned");

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

            // If the VM was ephemeral and died without going through
            // handle_stop / handle_run / handle_purge / handle_delete
            // (crash, SIGTERM, OOM), nothing else will ever touch its
            // session dir. Those explicit handlers all call
            // shutdown_vm_process which removes the entry from the map
            // BEFORE this handler fires -- so when `removed` is Some,
            // we're in the "died unexpectedly" case by definition.
            // That's exactly when we want to preserve process.log /
            // mcp-aggregator.stderr.log / serial.log / session.db for
            // post-mortem, rather than silently `remove_dir_all`.
            tracing::warn!(id_clone, "provision_sandbox child exit handler removing instance");
            let removed = state_clone.instances.lock().unwrap().remove(&id_clone);
            if let Some(info) = removed {
                if !info.persistent {
                    state_clone.preserve_failed_session_dir(&info.session_dir, &id_clone);
                }
            }
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
                env: env.clone(),
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

        // Clear stale UDS + ready sentinel from the prior boot. Without this,
        // wait_for_vm_ready returns instantly against the old .ready file and
        // callers race ahead before the resumed agent has reconnected.
        let _ = std::fs::remove_file(&uds_path);
        let _ = std::fs::remove_file(uds_path.with_extension("ready"));

        let resolved = self.resolve_asset_paths()?;
        if !resolved.rootfs.exists() {
            return Err(anyhow!("rootfs not found at {}", resolved.rootfs.display()));
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

        // Replay user-provided env vars so they survive stop/resume cycles.
        if let Some(ref env_vars) = entry.env {
            for (k, v) in env_vars {
                child_cmd.arg("--env").arg(format!("{}={}", k, v));
            }
        }

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
        // CAPSEM_{USER,CORP}_CONFIG are forwarded so the child loads the
        // same settings tree as the service (tests rely on this to route
        // policy through an isolated test config without touching the
        // real ~/.capsem/user.toml).
        child_cmd.env_clear();
        for key in &["HOME", "PATH", "USER", "TMPDIR", "CAPSEM_USER_CONFIG", "CAPSEM_CORP_CONFIG"] {
            if let Ok(val) = std::env::var(key) {
                child_cmd.env(key, val);
            }
        }

        let mut child = child_cmd
            .env("RUST_LOG", "capsem=info")
            .arg("--id").arg(name)
            .arg("--assets-dir").arg(&self.assets_dir)
            .arg("--rootfs").arg(&resolved.rootfs)
            .arg("--kernel").arg(&resolved.kernel)
            .arg("--initrd").arg(&resolved.initrd)
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
            tracing::warn!(name_clone, "resume_sandbox child exit handler removing instance");
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

    /// Resolve asset file paths for a VM.
    ///
    /// In v2 mode (manifest present): resolves hash-based filenames from manifest.
    /// In dev mode (no manifest): finds assets by logical name in arch subdirs.
    fn resolve_asset_paths(&self) -> Result<capsem_core::asset_manager::ResolvedAssets> {
        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };

        // Resolve from v2 manifest (works for both dev and installed --
        // dev creates hash-named symlinks, installed has hash-named files)
        if let Some(ref manifest) = self.manifest {
            return manifest.resolve(&self.current_version, arch, &self.assets_dir);
        }

        // No manifest: use logical names as fallback
        let base = if self.assets_dir.join(arch).join("rootfs.squashfs").exists() {
            self.assets_dir.join(arch)
        } else {
            self.assets_dir.clone()
        };
        Ok(capsem_core::asset_manager::ResolvedAssets {
            kernel: base.join("vmlinuz"),
            initrd: base.join("initrd.img"),
            rootfs: base.join("rootfs.squashfs"),
            asset_version: "dev".to_string(),
        })
    }
}

use axum::http::StatusCode;
use capsem_service::errors::AppError;
use capsem_service::fs_utils::{sanitize_file_path, identify_file_sync};

// ---------------------------------------------------------------------------
// Files API -- workspace path resolver (state-bound; pure helpers live in fs_utils.rs)
// ---------------------------------------------------------------------------

/// Resolve a sanitized relative path to an absolute workspace path on the host.
/// Returns (workspace_root, resolved_path). Verifies the resolved path is
/// inside the workspace via canonicalize + starts_with.
fn resolve_workspace_path(
    state: &ServiceState,
    id: &str,
    sanitized: &str,
) -> Result<(PathBuf, PathBuf), AppError> {
    let session_dir = {
        let instances = state.instances.lock().unwrap();
        if let Some(info) = instances.get(id) {
            info.session_dir.clone()
        } else {
            drop(instances);
            // Check persistent registry for stopped VMs
            let reg = state.persistent_registry.lock().unwrap();
            reg.data.vms.get(id)
                .or_else(|| reg.data.vms.values().find(|e| e.name == id))
                .map(|e| e.session_dir.clone())
                .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?
        }
    };
    let workspace_root = capsem_core::guest_share_dir(&session_dir).join("workspace");
    let target = workspace_root.join(sanitized);

    // Canonicalize requires the path to exist for files; for listing we may
    // also target the workspace root itself. Use the parent if target doesn't exist.
    let canonical = if target.exists() {
        target.canonicalize()
    } else {
        // For upload: parent must exist and be inside workspace
        if let Some(parent) = target.parent() {
            if parent.exists() {
                let canon_parent = parent.canonicalize()
                    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("canonicalize: {e}")))?;
                let ws_canon = workspace_root.canonicalize()
                    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("canonicalize workspace: {e}")))?;
                if !canon_parent.starts_with(&ws_canon) {
                    return Err(AppError(StatusCode::FORBIDDEN, "path outside workspace".into()));
                }
                return Ok((workspace_root, target));
            }
        }
        return Ok((workspace_root, target));
    }.map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("canonicalize: {e}")))?;

    let ws_canon = workspace_root.canonicalize()
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("canonicalize workspace: {e}")))?;
    if !canonical.starts_with(&ws_canon) {
        return Err(AppError(StatusCode::FORBIDDEN, "path outside workspace".into()));
    }
    Ok((workspace_root, canonical))
}

// ---------------------------------------------------------------------------
// Files API Handlers (host-side VirtioFS)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct FileListQuery {
    #[serde(default)]
    path: Option<String>,
    #[serde(default = "default_file_depth")]
    depth: u32,
}

fn default_file_depth() -> u32 { 1 }

#[derive(Deserialize)]
struct FileContentQuery {
    path: String,
}

/// Recursively list a directory up to `max_depth`.
fn list_dir_recursive(
    base: &std::path::Path,
    rel_prefix: &str,
    current_depth: u32,
    max_depth: u32,
    magika: &Mutex<magika::Session>,
) -> Vec<FileListEntry> {
    let mut entries = Vec::new();
    let read = match std::fs::read_dir(base) {
        Ok(r) => r,
        Err(_) => return entries,
    };

    let mut items: Vec<_> = read.flatten().collect();
    items.sort_by(|a, b| {
        let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        b_is_dir.cmp(&a_is_dir).then_with(|| a.file_name().cmp(&b.file_name()))
    });

    for item in items {
        let name = item.file_name().to_string_lossy().into_owned();
        // Skip the system directory (rootfs overlay, not user content)
        if name == "system" {
            continue;
        }
        let rel_path = if rel_prefix.is_empty() {
            name.clone()
        } else {
            format!("{rel_prefix}/{name}")
        };
        let meta = match item.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime = meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        if meta.is_dir() {
            let children = if current_depth < max_depth {
                Some(list_dir_recursive(
                    &base.join(&name),
                    &rel_path,
                    current_depth + 1,
                    max_depth,
                    magika,
                ))
            } else {
                None
            };
            entries.push(FileListEntry {
                name,
                path: rel_path,
                entry_type: "directory".into(),
                size: 0,
                mtime,
                mime: None,
                label: None,
                is_text: None,
                children,
            });
        } else if meta.is_file() {
            let (lbl, mime_str, _group, text) = identify_file_sync(magika, &base.join(&name));
            let (mime, label, is_text) = (Some(mime_str), Some(lbl), Some(text));
            entries.push(FileListEntry {
                name,
                path: rel_path,
                entry_type: "file".into(),
                size: meta.len(),
                mtime,
                mime,
                label,
                is_text,
                children: None,
            });
        }
    }
    entries
}

async fn handle_list_files(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<FileListQuery>,
) -> Result<Json<FileListResponse>, AppError> {
    let depth = params.depth.min(6);
    let rel_path = match params.path.as_deref() {
        Some(p) if !p.is_empty() => sanitize_file_path(p)?,
        _ => String::new(),
    };

    let (workspace_root, target) = if rel_path.is_empty() {
        // List workspace root -- get session_dir directly
        let session_dir = {
            let instances = state.instances.lock().unwrap();
            if let Some(info) = instances.get(&id) {
                info.session_dir.clone()
            } else {
                drop(instances);
                let reg = state.persistent_registry.lock().unwrap();
                reg.data.vms.get(&id)
                    .or_else(|| reg.data.vms.values().find(|e| e.name == id))
                    .map(|e| e.session_dir.clone())
                    .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?
            }
        };
        let ws = capsem_core::guest_share_dir(&session_dir).join("workspace");
        (ws.clone(), ws)
    } else {
        resolve_workspace_path(&state, &id, &rel_path)?
    };

    if !target.exists() {
        return Err(AppError(StatusCode::NOT_FOUND, "path not found".into()));
    }

    // Compute relative prefix for the listing
    let rel_prefix = target.strip_prefix(&workspace_root)
        .unwrap_or(std::path::Path::new(""))
        .to_string_lossy()
        .into_owned();

    // read_dir + metadata are blocking I/O -- run in spawn_blocking
    let magika = state.magika.lock().unwrap();
    // We can't send MutexGuard across threads; re-acquire inside spawn_blocking
    drop(magika);
    let magika_ref = {
        // Clone Arc to move into blocking task
        let state_clone = Arc::clone(&state);
        let target = target.clone();
        tokio::task::spawn_blocking(move || {
            list_dir_recursive(&target, &rel_prefix, 1, depth, &state_clone.magika)
        }).await.map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("list: {e}")))?
    };

    Ok(Json(FileListResponse { entries: magika_ref }))
}

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB

async fn handle_download_file(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<FileContentQuery>,
) -> Result<axum::response::Response, AppError> {
    let sanitized = sanitize_file_path(&params.path)?;
    let (_ws_root, resolved) = resolve_workspace_path(&state, &id, &sanitized)?;

    if !resolved.is_file() {
        return Err(AppError(StatusCode::NOT_FOUND, "file not found".into()));
    }

    let meta = std::fs::metadata(&resolved)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("metadata: {e}")))?;
    if meta.len() > MAX_FILE_SIZE {
        return Err(AppError(StatusCode::PAYLOAD_TOO_LARGE, format!(
            "file too large: {} bytes (max {})", meta.len(), MAX_FILE_SIZE
        )));
    }

    // Read file and detect type in spawn_blocking
    let state_clone = Arc::clone(&state);
    let resolved_clone = resolved.clone();
    let (data, mime, filename) = tokio::task::spawn_blocking(move || {
        let data = std::fs::read(&resolved_clone)
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("read: {e}")))?;
        let (_, mime_str, _, _) = identify_file_sync(&state_clone.magika, &resolved_clone);
        let name = resolved_clone.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".into());
        // Sanitize the filename for Content-Disposition
        let safe_name: String = name.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
            .collect();
        Ok::<_, AppError>((data, mime_str, safe_name))
    }).await.map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("task: {e}")))??;

    use axum::response::IntoResponse;
    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, mime),
            (axum::http::header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\"")),
            (axum::http::header::CONTENT_LENGTH, data.len().to_string()),
        ],
        data,
    ).into_response())
}

async fn handle_upload_file(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<FileContentQuery>,
    body: axum::body::Bytes,
) -> Result<Json<UploadResponse>, AppError> {
    let sanitized = sanitize_file_path(&params.path)?;
    let (_ws_root, target) = resolve_workspace_path(&state, &id, &sanitized)?;

    let size = body.len() as u64;

    // Write file in spawn_blocking (blocking I/O)
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir: {e}")))?;
        }
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o644)
            .open(&target)
            .and_then(|f| {
                use std::io::Write;
                let mut f = f;
                f.write_all(&body)?;
                Ok(())
            })
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("write: {e}")))?;
        Ok::<_, AppError>(())
    }).await.map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("task: {e}")))??;

    Ok(Json(UploadResponse { success: true, size }))
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

    // clone_sandbox_state does fsync + APFS clonefile + walkdir -- all blocking.
    // Offload to the blocking pool so axum worker threads aren't starved under
    // concurrent fork load.
    let clone_dst = new_session_dir.clone();
    let size_bytes = tokio::task::spawn_blocking(move || {
        capsem_core::auto_snapshot::clone_sandbox_state(&session_dir, &clone_dst)
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("clone task: {e}")))?
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
            env: None,
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
    let id = payload.name.clone().unwrap_or_else(|| {
        let existing: Vec<String> = state.instances.lock().unwrap().keys().cloned().collect();
        generate_tmp_name(existing.iter().map(|s| s.as_str()))
    });

    // Missing ram_mb/cpus fall back to merged VM settings. This keeps
    // "new ephemeral VM" callers (tray, MCP one-shots) honoring the user's
    // configured defaults without having to fetch settings first.
    let vm_settings = capsem_core::net::policy_config::load_merged_vm_settings();
    let ram_mb = payload
        .ram_mb
        .unwrap_or_else(|| vm_settings.ram_gb.unwrap_or(4) as u64 * 1024);
    let cpus = payload.cpus.unwrap_or_else(|| vm_settings.cpu_count.unwrap_or(4));

    // provision_sandbox is synchronous and performs heavy I/O (APFS clonefile,
    // rootfs.img fsync, walkdir-based disk_usage_bytes, child process spawn).
    // Offload to the blocking pool so the axum worker thread stays free.
    // tokio::process::Command::spawn inside still works -- spawn_blocking
    // preserves the runtime handle via thread-locals.
    let state_clone = Arc::clone(&state);
    let id_clone = id.clone();
    let version = state.current_version.clone();
    let provision_result = tokio::task::spawn_blocking(move || {
        state_clone.provision_sandbox(ProvisionOptions {
            id: &id_clone,
            ram_mb,
            cpus,
            version_override: Some(version),
            persistent: payload.persistent,
            env: payload.env,
            from: payload.from,
            description: None,
        })
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("provision task: {e}")))?;

    match provision_result {
        Ok(_) => {
            let uds_path = state.instance_socket_path(&id);
            Ok(Json(ProvisionResponse { id, uds_path: Some(uds_path) }))
        }
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
            info.total_file_events = Some(fc);
        }
        if let Ok(mcp) = reader.mcp_call_stats() {
            info.total_mcp_calls = Some(mcp.total);
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

    // Check asset health
    let asset_health = match state.resolve_asset_paths() {
        Ok(resolved) => {
            let mut missing = Vec::new();
            if !resolved.kernel.exists() { missing.push("vmlinuz".to_string()); }
            if !resolved.initrd.exists() { missing.push("initrd.img".to_string()); }
            if !resolved.rootfs.exists() { missing.push("rootfs.squashfs".to_string()); }
            Some(AssetHealth {
                ready: missing.is_empty(),
                version: Some(resolved.asset_version),
                missing,
            })
        }
        Err(_) => None,
    };

    Json(ListResponse { sandboxes, asset_health })
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
            info.size_bytes = capsem_core::auto_snapshot::sandbox_disk_usage(&entry.session_dir).ok();
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

async fn handle_service_logs(
    State(state): State<Arc<ServiceState>>,
) -> Result<String, AppError> {
    let log_path = state.run_dir.join("service.log");

    let text = tokio::task::spawn_blocking(move || -> Result<String, String> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(&log_path).map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        // Read last 100KB
        let max = 100 * 1024u64;
        if len > max {
            file.seek(SeekFrom::End(-(max as i64))).map_err(|e| e.to_string())?;
        }
        let mut buf = String::new();
        file.read_to_string(&mut buf).map_err(|e| e.to_string())?;
        // If we seeked into the middle, skip the first partial line
        if len > max {
            if let Some(pos) = buf.find('\n') {
                buf = buf[pos + 1..].to_string();
            }
        }
        Ok(buf)
    }).await.map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("log read failed: {e}")))?
      .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(text)
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

/// POST /settings/validate-key -- validate an API key against a provider endpoint.
async fn handle_validate_key(
    Json(payload): Json<ValidateKeyRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = capsem_core::host_config::validate_api_key(&payload.provider, &payload.key)
        .await
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(serde_json::to_value(result).unwrap_or_default()))
}

// ---------------------------------------------------------------------------
// Setup / Onboarding API Handlers
// ---------------------------------------------------------------------------

/// GET /setup/state -- return onboarding state from setup-state.json.
async fn handle_get_setup_state() -> Json<serde_json::Value> {
    let state = match capsem_core::setup_state::default_state_path() {
        Some(path) => capsem_core::setup_state::load_state(&path),
        None => capsem_core::setup_state::SetupState::default(),
    };
    // `needs_onboarding` is computed server-side so the frontend never has to
    // mirror the version constant. `install_completed` is surfaced so the app
    // can render an "install incomplete" banner if the CLI setup never finished.
    Json(json!({
        "schema_version": state.schema_version,
        "completed_steps": state.completed_steps,
        "security_preset": state.security_preset,
        "providers_done": state.providers_done,
        "repositories_done": state.repositories_done,
        "service_installed": state.service_installed,
        "install_completed": state.install_completed,
        "onboarding_completed": state.onboarding_completed,
        "onboarding_version": state.onboarding_version,
        "needs_onboarding": state.needs_onboarding(),
        "corp_config_source": state.corp_config_source,
    }))
}

/// GET /setup/detect -- detect host config, write to settings, return summary.
async fn handle_detect_host_config() -> Json<serde_json::Value> {
    // Detection involves blocking I/O (file reads, subprocess calls for gh token).
    let summary = tokio::task::spawn_blocking(|| {
        capsem_core::host_config::detect_and_write_to_settings()
    })
    .await
    .unwrap_or_else(|_| capsem_core::host_config::DetectedConfigSummary::from(
        &capsem_core::host_config::HostConfig::default()
    ));
    Json(serde_json::to_value(summary).unwrap_or_default())
}

/// POST /setup/retry -- re-run `capsem setup --non-interactive --accept-detected`.
/// Used by the app when `install_completed=false` so the user can retry without
/// a terminal. Invokes the installed capsem CLI as a subprocess rather than
/// pulling setup logic into capsem-core (the CLI owns provider detection, corp
/// config, asset download, etc.).
async fn handle_setup_retry() -> Result<Json<serde_json::Value>, AppError> {
    let home = capsem_core::paths::capsem_home_opt()
        .ok_or_else(|| AppError(StatusCode::INTERNAL_SERVER_ERROR, "HOME not set".into()))?;
    let capsem_bin = home.join("bin").join("capsem");
    if !capsem_bin.exists() {
        return Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("capsem binary not found at {}", capsem_bin.display()),
        ));
    }
    let output = tokio::process::Command::new(&capsem_bin)
        .args(["setup", "--non-interactive", "--accept-detected"])
        .output()
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to spawn capsem setup: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let code = output.status.code().unwrap_or(-1);
        warn!(exit_code = code, stderr = %stderr, "capsem setup retry failed");
        return Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("setup exited {code}: {}", stderr.lines().last().unwrap_or("(no output)")),
        ));
    }
    Ok(Json(json!({ "success": true })))
}

/// POST /setup/complete -- mark GUI onboarding as completed.
async fn handle_complete_onboarding() -> Result<Json<serde_json::Value>, AppError> {
    let path = capsem_core::setup_state::default_state_path()
        .ok_or_else(|| AppError(StatusCode::INTERNAL_SERVER_ERROR, "HOME not set".into()))?;
    let mut state = capsem_core::setup_state::load_state(&path);
    state.onboarding_completed = true;
    // Record which wizard version the user saw, so a future bump re-triggers it.
    state.onboarding_version = capsem_core::setup_state::CURRENT_ONBOARDING_VERSION;
    capsem_core::setup_state::save_state(&path, &state)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "success": true })))
}

/// GET /setup/assets -- query asset download status.
async fn handle_asset_status(
    State(state): State<Arc<ServiceState>>,
) -> Json<serde_json::Value> {
    match state.resolve_asset_paths() {
        Ok(resolved) => {
            let assets = vec![
                json!({ "name": "vmlinuz", "path": resolved.kernel.display().to_string(), "status": if resolved.kernel.exists() { "present" } else { "missing" } }),
                json!({ "name": "initrd.img", "path": resolved.initrd.display().to_string(), "status": if resolved.initrd.exists() { "present" } else { "missing" } }),
                json!({ "name": "rootfs.squashfs", "path": resolved.rootfs.display().to_string(), "status": if resolved.rootfs.exists() { "present" } else { "missing" } }),
            ];
            let all_ready = assets.iter().all(|a| a["status"] == "present");
            Json(json!({
                "ready": all_ready,
                "downloading": false,
                "asset_version": resolved.asset_version,
                "assets": assets,
            }))
        }
        Err(e) => {
            Json(json!({
                "ready": false,
                "downloading": false,
                "error": e.to_string(),
                "assets": [],
            }))
        }
    }
}

/// POST /setup/corp-config -- apply corporate config from URL or inline TOML.
async fn handle_corp_config(
    Json(payload): Json<CorpConfigRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    use capsem_core::net::policy_config::corp_provision;

    let capsem_dir = capsem_core::paths::capsem_home_opt()
        .ok_or(AppError(StatusCode::INTERNAL_SERVER_ERROR, "HOME not set".into()))?;

    if let Some(source) = &payload.source {
        // Use the existing provision function which handles fetch + install
        corp_provision::provision_from_source(&capsem_dir, source)
            .await
            .map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;
    } else if let Some(toml_content) = &payload.toml {
        corp_provision::validate_corp_toml(toml_content)
            .map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;
        corp_provision::install_inline_corp_config(&capsem_dir, toml_content)
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    } else {
        return Err(AppError(StatusCode::BAD_REQUEST, "provide either 'source' (URL) or 'toml' (inline content)".into()));
    }

    Ok(Json(json!({ "success": true })))
}

// ---------------------------------------------------------------------------
// MCP API Handlers
// ---------------------------------------------------------------------------

/// GET /mcp/servers -- list configured MCP servers with status.
async fn handle_mcp_servers() -> Json<serde_json::Value> {
    use capsem_core::mcp::{build_server_list_with_builtin, load_tool_cache};
    use capsem_core::mcp::policy::McpUserConfig;

    let (user_sf, corp_sf) = capsem_core::net::policy_config::load_settings_files();
    let user_mcp = user_sf.mcp.unwrap_or_default();
    let corp_mcp = corp_sf.mcp.unwrap_or(McpUserConfig::default());

    // Include the "local" builtin server if the binary exists.
    let builtin_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("capsem-mcp-builtin")));
    let servers = build_server_list_with_builtin(
        &user_mcp, &corp_mcp, builtin_bin.as_deref(), std::collections::HashMap::new(),
    );
    let cache = load_tool_cache();

    let resp: Vec<api::McpServerInfoResponse> = servers.iter().map(|s| {
        let tool_count = cache.iter().filter(|t| t.server_name == s.name).count();
        api::McpServerInfoResponse {
            name: s.name.clone(),
            url: s.url.clone(),
            has_bearer_token: s.bearer_token.is_some(),
            custom_header_count: s.headers.len(),
            source: s.source.clone(),
            enabled: s.enabled,
            running: false, // Config-level only; runtime status requires IPC.
            tool_count,
            is_stdio: s.is_stdio(),
        }
    }).collect();
    Json(serde_json::to_value(resp).unwrap_or_default())
}

/// GET /mcp/tools -- list discovered MCP tools with pin/approval status.
async fn handle_mcp_tools() -> Json<serde_json::Value> {
    use capsem_core::mcp::load_tool_cache;

    let cache = load_tool_cache();
    let resp: Vec<api::McpToolInfoResponse> = cache.iter().map(|entry| {
        api::McpToolInfoResponse {
            namespaced_name: entry.namespaced_name.clone(),
            original_name: entry.original_name.clone(),
            description: entry.description.clone(),
            server_name: entry.server_name.clone(),
            annotations: entry.annotations.as_ref().map(|a| a.to_mcp_json()),
            pin_hash: Some(entry.pin_hash.clone()),
            approved: entry.approved,
            pin_changed: false, // Would need live catalog comparison.
        }
    }).collect();
    Json(serde_json::to_value(resp).unwrap_or_default())
}

/// GET /mcp/policy -- return the merged MCP policy.
async fn handle_mcp_policy() -> Json<serde_json::Value> {
    use capsem_core::mcp::policy::McpUserConfig;

    let (user_sf, corp_sf) = capsem_core::net::policy_config::load_settings_files();
    let user_mcp = user_sf.mcp.unwrap_or_default();
    let corp_mcp = corp_sf.mcp.unwrap_or(McpUserConfig::default());

    let resp = api::McpPolicyInfoResponse {
        global_policy: user_mcp.global_policy.clone(),
        default_tool_permission: user_mcp.default_tool_permission
            .map(|d| format!("{d:?}").to_lowercase())
            .unwrap_or_else(|| "allow".into()),
        blocked_servers: {
            let policy = user_mcp.to_policy(&corp_mcp);
            policy.blocked_servers
        },
        tool_permissions: user_mcp.tool_permissions.iter()
            .map(|(k, v)| (k.clone(), format!("{v:?}").to_lowercase()))
            .collect(),
    };
    Json(serde_json::to_value(resp).unwrap_or_default())
}

/// POST /mcp/tools/refresh -- reload MCP servers from config.
async fn handle_mcp_refresh(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Send McpRefreshTools to all running instances.
    let uds_paths = {
        let instances = state.instances.lock().unwrap();
        instances.values().map(|info| info.uds_path.clone()).collect::<Vec<_>>()
    };
    for uds_path in &uds_paths {
        let id = state.next_job_id();
        let _ = send_ipc_command(uds_path, ServiceToProcess::McpRefreshTools { id }, 30).await;
    }
    Ok(Json(serde_json::json!({"success": true, "instances": uds_paths.len()})))
}

/// POST /mcp/tools/:name/approve -- approve a tool (mark approved in cache).
async fn handle_mcp_approve(
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use capsem_core::mcp::{load_tool_cache, save_tool_cache};

    let mut cache = load_tool_cache();
    let found = cache.iter_mut().find(|e| e.namespaced_name == name);
    match found {
        Some(entry) => {
            entry.approved = true;
            save_tool_cache(&cache)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
            Ok(Json(serde_json::json!({"approved": true})))
        }
        None => Err(AppError(StatusCode::NOT_FOUND, format!("tool not found: {name}"))),
    }
}

/// POST /mcp/tools/:name/call -- call an MCP tool via a running VM's aggregator.
async fn handle_mcp_call(
    State(state): State<Arc<ServiceState>>,
    Path(name): Path<String>,
    Json(arguments): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Find any running instance to route the call through.
    let uds_path = {
        let instances = state.instances.lock().unwrap();
        instances.values().next().map(|i| i.uds_path.clone())
    };
    let uds_path = uds_path
        .ok_or_else(|| AppError(StatusCode::SERVICE_UNAVAILABLE, "no running sessions".into()))?;

    let arguments_json = serde_json::to_string(&arguments)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("invalid arguments: {e}")))?;
    let msg = ServiceToProcess::McpCallTool {
        id: state.next_job_id(),
        namespaced_name: name.clone(),
        arguments_json,
    };
    let resp = send_ipc_command(&uds_path, msg, 60).await
        .map_err(|e| AppError(StatusCode::BAD_GATEWAY, e))?;

    match resp {
        ProcessToService::McpCallToolResult { result_json, error, .. } => {
            if let Some(err) = error {
                Err(AppError(StatusCode::BAD_GATEWAY, err))
            } else {
                let result = match result_json {
                    Some(s) => serde_json::from_str(&s).map_err(|e| AppError(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("bad result_json from process: {e}"),
                    ))?,
                    None => serde_json::Value::Null,
                };
                Ok(Json(result))
            }
        }
        _ => Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, "unexpected IPC response".into())),
    }
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

// ---------------------------------------------------------------------------
// History endpoints
// ---------------------------------------------------------------------------

/// Helper: resolve session_dir from instance ID (running or persistent).
fn resolve_session_dir(state: &ServiceState, id: &str) -> Result<PathBuf, AppError> {
    let instances = state.instances.lock().unwrap();
    if let Some(i) = instances.get(id) {
        return Ok(i.session_dir.clone());
    }
    drop(instances);
    let registry = state.persistent_registry.lock().unwrap();
    if let Some(entry) = registry.get(id) {
        return Ok(entry.session_dir.clone());
    }
    Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))
}

/// GET /history/{id} -- unified command history (exec + audit events).
async fn handle_history(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<api::HistoryQuery>,
) -> Result<Json<api::HistoryResponse>, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");

    let reader = capsem_logger::DbReader::open(&db_path)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to open DB: {e}")))?;

    let (commands, total) = reader.history(
        params.limit,
        params.offset,
        params.search.as_deref(),
        &params.layer,
    ).map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("query failed: {e}")))?;

    let has_more = (params.offset + commands.len()) < total as usize;
    Ok(Json(api::HistoryResponse { commands, total, has_more }))
}

/// GET /history/{id}/processes -- process-centric view of audit events.
async fn handle_history_processes(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::HistoryProcessesResponse>, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");

    let reader = capsem_logger::DbReader::open(&db_path)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to open DB: {e}")))?;

    let processes = reader.history_processes(100)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("query failed: {e}")))?;

    Ok(Json(api::HistoryProcessesResponse { processes }))
}

/// GET /history/{id}/counts -- exec and audit event counts.
async fn handle_history_counts(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::HistoryCountsResponse>, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");

    let reader = capsem_logger::DbReader::open(&db_path)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to open DB: {e}")))?;

    let counts = reader.history_counts()
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("query failed: {e}")))?;

    Ok(Json(api::HistoryCountsResponse {
        exec_count: counts.exec_count,
        audit_count: counts.audit_count,
    }))
}

/// GET /history/{id}/transcript -- raw PTY output (base64-encoded).
async fn handle_history_transcript(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(_params): Query<api::TranscriptQuery>,
) -> Result<Json<api::TranscriptResponse>, AppError> {
    use base64::Engine;
    let session_dir = resolve_session_dir(&state, &id)?;
    let pty_log_path = session_dir.join("pty.log");

    if !pty_log_path.exists() {
        return Ok(Json(api::TranscriptResponse {
            content: String::new(),
            bytes: 0,
        }));
    }

    let output = std::fs::read(&pty_log_path)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to read pty.log: {e}")))?;

    let encoded = base64::engine::general_purpose::STANDARD.encode(&output);
    Ok(Json(api::TranscriptResponse {
        bytes: output.len(),
        content: encoded,
    }))
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
    tracing::warn!(id, "shutdown_vm_process removing instance");
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
        // The guest never acknowledged suspend. Leaving the process alive
        // would leak a wedged Apple VZ instance (seen in the wild: 945
        // orphan temp dirs accumulated over one test run). SIGKILL the
        // child, reclaim the instance slot, and surface the error.
        if pid > 0 {
            let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGKILL);
        }
        tracing::warn!(id, "handle_suspend (timeout) removing instance");
        state.instances.lock().unwrap().remove(&id);
        let _ = std::fs::remove_file(&uds_path);
        let _ = std::fs::remove_file(uds_path.with_extension("ready"));
        return Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "suspend timed out: VM did not confirm suspended state (process killed)".into(),
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

    tracing::warn!(id, "handle_suspend (success) removing instance");
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
        Ok(id) => {
            let uds_path = state.instance_socket_path(&id);
            Ok(Json(ProvisionResponse { id, uds_path: Some(uds_path) }))
        }
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
    let (old_session_dir, ram_mb, cpus, base_version, forked_from, env) = {
        let instances = state.instances.lock().unwrap();
        let i = instances.get(&id).ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        if i.persistent {
            return Err(AppError(StatusCode::BAD_REQUEST, format!("VM \"{}\" is already persistent", id)));
        }
        (i.session_dir.clone(), i.ram_mb, i.cpus, i.base_version.clone(), i.forked_from.clone(), i.env.clone())
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
            env: env.clone(),
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
    let id = {
        let existing: Vec<String> = state.instances.lock().unwrap().keys().cloned().collect();
        generate_tmp_name(existing.iter().map(|s| s.as_str()))
    };

    // Resolve ram/cpu from merged VM settings if the caller didn't specify,
    // matching handle_provision. Keeps `capsem run` settings-driven.
    let vm_settings = capsem_core::net::policy_config::load_merged_vm_settings();
    let ram_mb = payload
        .ram_mb
        .unwrap_or_else(|| vm_settings.ram_gb.unwrap_or(4) as u64 * 1024);
    let cpus = payload.cpus.unwrap_or_else(|| vm_settings.cpu_count.unwrap_or(4));

    let ram_bytes = ram_mb * 1024 * 1024;
    let session_dir = state.run_dir.join("sessions").join(&id);

    // 1. Provision ephemeral VM. `provision_sandbox` is synchronous and
    // does heavy I/O (APFS clonefile, rootfs.img fsync, child spawn);
    // offload to the blocking pool, matching `handle_provision` -- the
    // tokio::process::Command::spawn inside still works because
    // spawn_blocking preserves the runtime handle via thread-locals.
    let state_clone = Arc::clone(&state);
    let id_clone = id.clone();
    let version = state.current_version.clone();
    let env = payload.env.clone();
    let provision_result = tokio::task::spawn_blocking(move || {
        state_clone.provision_sandbox(ProvisionOptions {
            id: &id_clone,
            ram_mb,
            cpus,
            version_override: Some(version),
            persistent: false,
            env,
            from: None,
            description: None,
        })
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("provision task: {e}")))?;
    provision_result
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
            exec_count: 0,
            audit_event_count: 0,
        };
        if let Err(e) = idx.create_session(&record) {
            tracing::warn!("failed to register session in main.db: {e}");
        }
    }

    // 3. Wait for VM socket to appear
    let uds_path = state.instance_socket_path(&id);
    if let Err(e) = wait_for_vm_ready(&uds_path, 30).await {
        // Send Shutdown IPC / SIGTERM and then wait for the child to
        // actually exit before renaming. Rename on an open-for-write
        // dir is safe (fds survive) but any path-based reopens the
        // child might do during shutdown (log rotation, db reopen)
        // would ENOENT -- so we let it finish flushing first. The
        // 5s bound matches shutdown_vm_process's own reaper.
        let pid = shutdown_vm_process(&state, &id)
            .await
            .map(|(_, _, p)| p)
            .unwrap_or(0);
        if pid > 0 {
            wait_for_process_exit(pid, std::time::Duration::from_secs(5)).await;
        }
        let dir = session_dir;
        let state_clone = Arc::clone(&state);
        let id_owned = id.clone();
        tokio::task::spawn_blocking(move || {
            state_clone.preserve_failed_session_dir(&dir, &id_owned);
        });
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

    let run_dir = capsem_core::paths::capsem_run_dir();

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

    // Self-idempotent startup. Four parallel `capsem-service --uds-path X`
    // invocations must converge on exactly one running service.
    //
    //   1. Fast probe without locking: if someone matching our version is
    //      already serving, exit 0 (happy path for tests and re-runs).
    //   2. Take an flock next to the socket for the critical section:
    //      probe again (double-check), remove any stale socket, bind.
    //      Drop the lock the moment bind() succeeds so peers waiting for
    //      the lock can fast-probe us on their next iteration.
    //   3. Version mismatch refuses to start (do not auto-kill -- destructive).
    //
    // On crash the flock releases automatically (fd close), so failed
    // startups never wedge subsequent ones.
    let current_version = env!("CARGO_PKG_VERSION");
    let probe_timeout = std::time::Duration::from_millis(500);

    // Fast path: someone else already serves a compatible version.
    if service_sock.exists() {
        if let Ok(Some(running)) =
            startup::probe_running_version(&service_sock, probe_timeout).await
        {
            if running == current_version {
                info!(
                    socket = %service_sock.display(),
                    version = %running,
                    "compatible capsem-service already running; exiting 0"
                );
                return Ok(());
            }
            eprintln!(
                "capsem-service {} is already running at {}, but this binary is {}.\n\
                 Stop the running service before starting a new one.",
                running,
                service_sock.display(),
                current_version
            );
            return Err(anyhow::anyhow!(
                "version mismatch with running service (running: {}, this: {})",
                running,
                current_version
            ));
        }
    }

    let lock_path = service_sock.with_extension("lock");
    let startup_lock =
        match startup::StartupLock::acquire(&lock_path, std::time::Duration::from_secs(30))? {
            Some(lock) => lock,
            None => {
                return Err(anyhow::anyhow!(
                    "another capsem-service startup holds {} after 30s; aborting",
                    lock_path.display()
                ));
            }
        };

    // Under lock: double-check a peer didn't finish starting while we waited.
    if service_sock.exists() {
        match startup::probe_running_version(&service_sock, probe_timeout).await {
            Ok(Some(running)) if running == current_version => {
                info!(
                    socket = %service_sock.display(),
                    version = %running,
                    "peer starter won the race; exiting 0"
                );
                return Ok(());
            }
            Ok(Some(running)) => {
                return Err(anyhow::anyhow!(
                    "version mismatch with running service (running: {}, this: {})",
                    running,
                    current_version
                ));
            }
            Ok(None) => {
                info!(socket = %service_sock.display(), "removing stale socket");
                let _ = std::fs::remove_file(&service_sock);
            }
            Err(e) => {
                warn!(error = %e, socket = %service_sock.display(),
                    "probe failed under lock; removing socket and continuing");
                let _ = std::fs::remove_file(&service_sock);
            }
        }
    }
    // Keep `startup_lock` alive until after UnixListener::bind below. Released
    // where we explicitly drop it, right after bind succeeds.
    let startup_lock_guard = startup_lock;

    let process_binary = args.process_binary.unwrap_or_else(|| PathBuf::from("target/debug/capsem-process"));
    let assets_base_dir = args.assets_dir.unwrap_or_else(|| run_dir.parent().unwrap().join("assets"));

    // Load v2 manifest if available. In dev mode (no manifest or v1), use None.
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let manifest_path = if assets_base_dir.join("manifest.json").exists() {
        Some(assets_base_dir.join("manifest.json"))
    } else if assets_base_dir.parent().unwrap().join("manifest.json").exists() {
        Some(assets_base_dir.parent().unwrap().join("manifest.json"))
    } else {
        None
    };

    let manifest = manifest_path.and_then(|path| {
        let content = std::fs::read_to_string(&path).ok()?;
        match capsem_core::asset_manager::ManifestV2::from_json(&content) {
            Ok(m) => {
                info!(asset_version = %m.assets.current, "loaded manifest");
                Some(Arc::new(m))
            }
            Err(e) => {
                warn!(error = %e, "failed to parse manifest");
                None
            }
        }
    });

    // Clean up stale assets (legacy v*/ dirs, unreferenced hash-named files)
    if let Some(ref m) = manifest {
        match capsem_core::asset_manager::cleanup_unused_assets(&assets_base_dir, m) {
            Ok(removed) if !removed.is_empty() => {
                info!(count = removed.len(), "cleaned up stale assets");
            }
            Err(e) => warn!(error = %e, "asset cleanup failed"),
            _ => {}
        }
    }

    let registry_path = run_dir.join("persistent_registry.json");
    let persistent_registry = PersistentRegistry::load(registry_path);
    info!(persistent_vms = persistent_registry.data.vms.len(), "loaded persistent VM registry");

    let magika_session = magika::Session::builder()
        .with_inter_threads(1)
        .with_intra_threads(1)
        .build()
        .expect("failed to init magika file-type detection");

    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(persistent_registry),
        process_binary: process_binary.clone(),
        assets_dir: assets_base_dir,
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        manifest,
        current_version,
        magika: Mutex::new(magika_session),
    });

    // Check for running instances to reattach
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
        .route("/version", get(|| async {
            Json(serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }))
        }))
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
        .route("/service-logs", get(handle_service_logs))
        .route("/reload-config", post(handle_reload_config))
        .route("/fork/{id}", post(handle_fork))
        .route("/settings", get(handle_get_settings).post(handle_save_settings))
        .route("/settings/presets", get(handle_get_presets))
        .route("/settings/presets/{id}", post(handle_apply_preset))
        .route("/settings/lint", post(handle_lint_config))
        .route("/settings/validate-key", post(handle_validate_key))
        .route("/setup/state", get(handle_get_setup_state))
        .route("/setup/detect", get(handle_detect_host_config))
        .route("/setup/complete", post(handle_complete_onboarding))
        .route("/setup/retry", post(handle_setup_retry))
        .route("/setup/assets", get(handle_asset_status))
        .route("/setup/corp-config", post(handle_corp_config))
        .route("/mcp/servers", get(handle_mcp_servers))
        .route("/mcp/tools", get(handle_mcp_tools))
        .route("/mcp/policy", get(handle_mcp_policy))
        .route("/mcp/tools/refresh", post(handle_mcp_refresh))
        .route("/mcp/tools/{name}/approve", post(handle_mcp_approve))
        .route("/mcp/tools/{name}/call", post(handle_mcp_call))
        .route("/history/{id}", get(handle_history))
        .route("/history/{id}/processes", get(handle_history_processes))
        .route("/history/{id}/counts", get(handle_history_counts))
        .route("/history/{id}/transcript", get(handle_history_transcript))
        .route("/files/{id}", get(handle_list_files))
        .route("/files/{id}/content", get(handle_download_file).post(handle_upload_file))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    info!(socket = %service_sock.display(), "listening on UDS");

    let uds = UnixListener::bind(&service_sock).context("failed to bind UDS")?;
    // Socket is bound; release the startup lock so any peer starter still in
    // its flock wait can fast-probe us and exit 0.
    drop(startup_lock_guard);

    // Spawn companion processes (gateway + tray) in the background so the UDS
    // starts accepting immediately. The previous .await here delayed accept()
    // by up to 5s on every startup while polling gateway.token into existence
    // -- fatal under parallel test load. Companions are stateless and can come
    // up after the service is already serving clients.
    let companions = Arc::new(std::sync::Mutex::new(Vec::<tokio::process::Child>::new()));
    let companions_for_spawn = Arc::clone(&companions);
    let service_sock_for_spawn = service_sock.clone();
    let run_dir_for_spawn = run_dir.clone();
    let gateway_binary = args.gateway_binary;
    let gateway_port = args.gateway_port;
    let tray_binary = args.tray_binary;
    tokio::spawn(async move {
        let spawned = spawn_companions(
            &service_sock_for_spawn,
            &run_dir_for_spawn,
            gateway_binary,
            gateway_port,
            tray_binary,
        )
        .await;
        companions_for_spawn.lock().unwrap().extend(spawned);
    });

    let shutdown_state = state.clone();
    let companions_for_shutdown = Arc::clone(&companions);
    axum::serve(uds, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            info!("service shutting down, killing companions and VM processes");
            // Companions FIRST. kill_all_vm_processes has an unconditional
            // 500ms SIGTERM grace sleep; if companion-kill ran after it, a
            // downstream `_ensure-service` (which itself sleeps 500ms before
            // spawning the next service) would race with companion exit and
            // the new gateway would fail to bind :19222.
            let mut children = std::mem::take(&mut *companions_for_shutdown.lock().unwrap());
            for child in &mut children {
                let _ = child.kill().await;
            }
            kill_all_vm_processes(&shutdown_state);
        })
        .await
        .context("server error")?;

    Ok(())
}

/// Kill every per-VM `capsem-process` the service has spawned.
///
/// Called from the graceful-shutdown path so a SIGTERM to capsem-service does
/// NOT orphan running guests. Without this, each service shutdown leaked one
/// `capsem-process` per live VM, which in turn held Apple VZ memory -- making
/// long test runs increasingly slow until boots timed out.
fn kill_all_vm_processes(state: &ServiceState) {
    let pids_and_sockets: Vec<(u32, PathBuf, PathBuf, bool)> = {
        let instances = state.instances.lock().unwrap();
        instances.values()
            .map(|i| (i.pid, i.uds_path.clone(), i.session_dir.clone(), i.persistent))
            .collect()
    };
    // Nothing to reap -- skip the grace sleep. `_ensure-service` only waits
    // 500ms before respawning the service, so every unnecessary ms here
    // widens the orphan-gateway race.
    if pids_and_sockets.is_empty() {
        return;
    }
    let mut signaled_any_vm = false;
    for (pid, uds_path, session_dir, persistent) in &pids_and_sockets {
        let pid = *pid;
        if pid > 0 {
            // SIGTERM first so capsem-process gets a chance to run its own cleanup
            // (save state, unmount virtiofs). Graceful_shutdown is already holding
            // the axum server open briefly so a short wait is acceptable.
            let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGTERM);
            signaled_any_vm = true;
        }
        let _ = std::fs::remove_file(&uds_path);
        let _ = std::fs::remove_file(uds_path.with_extension("ready"));
        if !persistent {
            let _ = std::fs::remove_dir_all(&session_dir);
        }
    }
    if !signaled_any_vm {
        return;
    }
    
    // Bounded wait: poll for up to 2 seconds
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(2);
    let poll_interval = std::time::Duration::from_millis(100);
    
    loop {
        let survivors: Vec<u32> = pids_and_sockets.iter()
            .map(|(pid, _, _, _)| *pid)
            .filter(|&pid| pid > 0 && unsafe { nix::libc::kill(pid as i32, 0) } == 0)
            .collect();
            
        if survivors.is_empty() {
            break;
        }
        
        if start.elapsed() >= timeout {
            tracing::warn!(count = survivors.len(), "some VMs survived SIGTERM, escalating to SIGKILL");
            for pid in survivors {
                let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGKILL);
            }
            break;
        }
        
        std::thread::sleep(poll_interval);
    }
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
    gateway_bin: Option<PathBuf>,
    gateway_port: Option<u16>,
    tray_bin: Option<PathBuf>,
) -> Vec<tokio::process::Child> {
    // tray_bin is only consumed by the macOS-gated tray-spawn block below.
    // On Linux there's no system tray, so the parameter is intentionally
    // unused -- silence the unused-variable warning without breaking the
    // platform-agnostic signature.
    #[cfg(not(target_os = "macos"))]
    let _ = tray_bin;

    let mut children = Vec::new();

    // Log files for companion processes. Tests set CAPSEM_RUN_DIR for isolation;
    // when it is set, keep logs under that run_dir so parallel test workers do
    // not trample each other's gateway.log in ~/Library/Logs/capsem.
    let log_dir = if std::env::var("CAPSEM_RUN_DIR").is_ok() {
        run_dir.join("logs")
    } else {
        std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join("Library/Logs/capsem"))
            .unwrap_or_else(|_| run_dir.join("logs"))
    };
    let _ = std::fs::create_dir_all(&log_dir);

    // 1. Spawn capsem-gateway (TCP reverse proxy -> UDS)
    let gateway_bin = gateway_bin.unwrap_or_else(|| find_sibling_binary("capsem-gateway"));
    let (gw_out, gw_err) = companion_stdio(&log_dir.join("gateway.log"));
    info!(binary = %gateway_bin.display(), "spawning capsem-gateway");

    let mut gw_cmd = tokio::process::Command::new(&gateway_bin);
    gw_cmd.arg("--uds-path").arg(service_sock);
    // Pin the gateway to the service's run_dir so gateway.{token,port,pid} land
    // in the same place we poll for them below and the same place clients read.
    gw_cmd.arg("--run-dir").arg(run_dir);
    // Parent-watch: the gateway exits the moment we die, even if we die
    // ungracefully (SIGKILL/OOM). capsem-guard enforces this on the gateway
    // side; we just have to hand it our PID.
    gw_cmd.arg("--parent-pid").arg(std::process::id().to_string());
    if let Some(port) = gateway_port {
        gw_cmd.arg("--port").arg(port.to_string());
    }
    match gw_cmd
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
                let tray_bin = tray_bin.unwrap_or_else(|| find_sibling_binary("capsem-tray"));
                let (tray_out, tray_err) = companion_stdio(&log_dir.join("tray.log"));
                info!(binary = %tray_bin.display(), "spawning capsem-tray");
                match tokio::process::Command::new(&tray_bin)
                    .arg("--parent-pid").arg(std::process::id().to_string())
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

    fn test_magika() -> Mutex<magika::Session> {
        Mutex::new(
            magika::Session::builder()
                .with_inter_threads(1)
                .with_intra_threads(1)
                .build()
                .expect("magika init"),
        )
    }

    fn make_test_state() -> Arc<ServiceState> {
        let registry_path = PathBuf::from("/tmp/capsem-test-svc/persistent_registry.json");
        Arc::new(ServiceState {
            instances: Mutex::new(HashMap::new()),
            persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
            process_binary: PathBuf::from("/nonexistent/capsem-process"),
            assets_dir: PathBuf::from("/nonexistent/assets"),
            run_dir: PathBuf::from("/tmp/capsem-test-svc"),
            job_counter: AtomicU64::new(1),
            manifest: None,
            current_version: "0.0.0".into(),
            magika: test_magika(),
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
    // drain_dead_instances: probe-and-evict contract, filesystem work is the
    // caller's responsibility. Exists so `cleanup_stale_instances` can release
    // the instances mutex BEFORE performing remove_dir_all -- otherwise every
    // handler that touches instances.lock() blocks on slow fs I/O.
    // -----------------------------------------------------------------------

    #[test]
    fn drain_dead_instances_returns_only_dead_entries() {
        let state = make_test_state();
        insert_fake_instance(&state, "live", std::process::id());
        insert_fake_instance(&state, "dead", 99999999);

        let evicted = state.drain_dead_instances();

        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0].0, "dead");
        let map = state.instances.lock().unwrap();
        assert!(map.contains_key("live"));
        assert!(!map.contains_key("dead"));
    }

    #[test]
    fn drain_dead_instances_empty_when_all_alive() {
        let state = make_test_state();
        insert_fake_instance(&state, "live-1", std::process::id());
        insert_fake_instance(&state, "live-2", std::process::id());

        let evicted = state.drain_dead_instances();

        assert!(evicted.is_empty());
        assert_eq!(state.instances.lock().unwrap().len(), 2);
    }

    #[test]
    fn drain_dead_instances_releases_mutex_before_returning() {
        // Regression guard: the whole point of splitting drain from the
        // filesystem scrub is that the mutex must be FREE by the time
        // drain returns. If this test ever fails, the locking protocol
        // has regressed and concurrent handlers will block on cleanup I/O.
        let state = make_test_state();
        insert_fake_instance(&state, "dead", 99999999);

        let _evicted = state.drain_dead_instances();

        assert!(
            state.instances.try_lock().is_ok(),
            "mutex still held after drain_dead_instances returned"
        );
    }

    // -----------------------------------------------------------------------
    // preserve_failed_session_dir + cull_failed_sessions
    //
    // The post-mortem pipeline: when any of the three loss paths
    // (wait_for_vm_ready timeout, dead-process cleanup, unexpected
    // child exit) would have silently `remove_dir_all`'d a session dir,
    // it's renamed to a `-failed-*` sibling instead so process.log,
    // mcp-aggregator.stderr.log, serial.log, and session.db survive.
    // Cap: MAX_FAILED_SESSIONS (5).
    // -----------------------------------------------------------------------

    fn make_state_in(run_dir: PathBuf) -> Arc<ServiceState> {
        let registry_path = run_dir.join("persistent_registry.json");
        std::fs::create_dir_all(run_dir.join("sessions")).unwrap();
        Arc::new(ServiceState {
            instances: Mutex::new(HashMap::new()),
            persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
            process_binary: PathBuf::from("/nonexistent/capsem-process"),
            assets_dir: PathBuf::from("/nonexistent/assets"),
            run_dir,
            job_counter: AtomicU64::new(1),
            manifest: None,
            current_version: "0.0.0".into(),
            magika: test_magika(),
        })
    }

    #[test]
    fn preserve_renames_session_dir_and_keeps_logs() {
        let dir = tempfile::tempdir().unwrap();
        let state = make_state_in(dir.path().to_path_buf());
        let session_dir = state.run_dir.join("sessions").join("vm-abc");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(session_dir.join("process.log"), b"boot failed: ...").unwrap();
        std::fs::write(session_dir.join("serial.log"), b"kernel panic").unwrap();

        state.preserve_failed_session_dir(&session_dir, "vm-abc");

        assert!(!session_dir.exists(), "original dir should have been renamed");
        let entries: Vec<_> = std::fs::read_dir(state.run_dir.join("sessions"))
            .unwrap()
            .flatten()
            .collect();
        let failed = entries
            .iter()
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("vm-abc-failed-")
            })
            .expect("a vm-abc-failed-* dir must exist");
        let preserved = failed.path().join("process.log");
        assert_eq!(std::fs::read(&preserved).unwrap(), b"boot failed: ...");
        let preserved_serial = failed.path().join("serial.log");
        assert_eq!(std::fs::read(&preserved_serial).unwrap(), b"kernel panic");
    }

    #[test]
    fn cull_keeps_newest_and_prunes_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let state = make_state_in(dir.path().to_path_buf());
        let sessions = state.run_dir.join("sessions");

        // Create MAX_FAILED_SESSIONS + 2 failed dirs with staggered mtimes.
        // Using filetime to set mtime lets us assert deterministically
        // which ones get pruned (oldest) vs kept (newest).
        let total = MAX_FAILED_SESSIONS + 2;
        for i in 0..total {
            let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
            let p = sessions.join(&name);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("process.log"), format!("run {i}")).unwrap();
            // Older i -> older mtime.
            let when = std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(1_700_000_000 + i as u64 * 10);
            filetime::set_file_mtime(&p, filetime::FileTime::from_system_time(when)).unwrap();
        }

        state.cull_failed_sessions().unwrap();

        let remaining: std::collections::HashSet<String> = std::fs::read_dir(&sessions)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();

        assert_eq!(
            remaining.len(),
            MAX_FAILED_SESSIONS,
            "should keep exactly MAX_FAILED_SESSIONS, got {remaining:?}"
        );
        // Oldest two (i=0, i=1) must be pruned; newest MAX_FAILED_SESSIONS kept.
        for i in 0..2 {
            let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
            assert!(
                !remaining.contains(&name),
                "oldest dir {name} should have been culled"
            );
        }
        for i in 2..total {
            let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
            assert!(
                remaining.contains(&name),
                "newer dir {name} should have been kept"
            );
        }
    }

    #[test]
    fn cull_is_noop_when_under_cap() {
        let dir = tempfile::tempdir().unwrap();
        let state = make_state_in(dir.path().to_path_buf());
        let sessions = state.run_dir.join("sessions");

        for i in 0..3 {
            let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
            std::fs::create_dir_all(sessions.join(&name)).unwrap();
        }

        state.cull_failed_sessions().unwrap();

        assert_eq!(std::fs::read_dir(&sessions).unwrap().count(), 3);
    }

    #[test]
    fn cull_ignores_non_failed_dirs() {
        // Running sessions (no `-failed-` in the name) must never be
        // culled. This is the safety property: a misnamed cull is a
        // production outage.
        let dir = tempfile::tempdir().unwrap();
        let state = make_state_in(dir.path().to_path_buf());
        let sessions = state.run_dir.join("sessions");

        std::fs::create_dir_all(sessions.join("vm-alive")).unwrap();
        for i in 0..(MAX_FAILED_SESSIONS + 3) {
            let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
            std::fs::create_dir_all(sessions.join(&name)).unwrap();
        }

        state.cull_failed_sessions().unwrap();

        assert!(sessions.join("vm-alive").exists(), "active VM dir must not be culled");
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
        env: None,
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
        env: None,
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
        env: None,
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
            env: None,
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
        let registry_path = dir.path().join("persistent_registry.json");
        let state = Arc::new(ServiceState {
            instances: Mutex::new(HashMap::new()),
            persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
            process_binary: PathBuf::from("/nonexistent/capsem-process"),
            assets_dir: dir.path().join("assets"),
            run_dir: dir.path().to_path_buf(),
            job_counter: AtomicU64::new(1),
            manifest: None,
            current_version: "0.0.0".into(),
            magika: test_magika(),
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
            env: None,
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
        env: None,
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
            env: None,
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
            env: None,
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
            env: None,
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
        env: None,
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
        env: None,
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
            exec_count: 0,
            audit_event_count: 0,
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
        let registry_path = run_dir.join("persistent_registry.json");
        let state = Arc::new(ServiceState {
            instances: Mutex::new(HashMap::new()),
            persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
            process_binary: PathBuf::from("/nonexistent/capsem-process"),
            assets_dir: run_dir.join("assets"),
            run_dir,
            job_counter: AtomicU64::new(1),
            manifest: None,
            current_version: "0.0.0".into(),
            magika: test_magika(),
        });
        (state, dir)
    }

    // -----------------------------------------------------------------------
    // resolve_workspace_path
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_rejects_unknown_vm() {
        let state = make_test_state();
        let r = resolve_workspace_path(&state, "nonexistent", "src/main.rs");
        assert!(r.is_err());
    }

    #[test]
    fn resolve_rejects_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("session");
        let workspace = session_dir.join("guest/workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        // Create a symlink that points outside workspace
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(&outside, workspace.join("escape")).unwrap();

        let (state, _dir2) = make_test_state_with_tempdir();
        state.instances.lock().unwrap().insert(
            "test-vm".into(),
            InstanceInfo {
                id: "test-vm".into(),
                pid: 1,
                uds_path: PathBuf::from("/tmp/test.sock"),
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

        let r = resolve_workspace_path(&state, "test-vm", "escape/secret.txt");
        assert!(r.is_err());
    }

    #[test]
    fn resolve_valid_path_inside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("session");
        let workspace = session_dir.join("guest/workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("hello.txt"), "world").unwrap();

        let (state, _dir2) = make_test_state_with_tempdir();
        state.instances.lock().unwrap().insert(
            "test-vm".into(),
            InstanceInfo {
                id: "test-vm".into(),
                pid: 1,
                uds_path: PathBuf::from("/tmp/test.sock"),
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

        let r = resolve_workspace_path(&state, "test-vm", "hello.txt");
        assert!(r.is_ok());
        let (ws_root, resolved) = r.unwrap();
        assert!(resolved.starts_with(ws_root.canonicalize().unwrap()));
    }

    // -----------------------------------------------------------------------
    // list_dir_recursive
    // -----------------------------------------------------------------------

    #[test]
    fn list_dir_returns_correct_structure() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(ws.join("README.md"), "# Hello").unwrap();

        let magika = test_magika();
        let entries = list_dir_recursive(ws, "", 1, 2, &magika);

        // Should have src/ dir and README.md file
        assert!(entries.len() >= 2);
        let dir_entry = entries.iter().find(|e| e.name == "src").unwrap();
        assert_eq!(dir_entry.entry_type, "directory");
        assert!(dir_entry.children.is_some());
        let children = dir_entry.children.as_ref().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "main.rs");
        assert_eq!(children[0].entry_type, "file");

        let file_entry = entries.iter().find(|e| e.name == "README.md").unwrap();
        assert_eq!(file_entry.entry_type, "file");
        assert!(file_entry.size > 0);
    }

    #[test]
    fn list_dir_respects_depth_limit() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        std::fs::create_dir_all(ws.join("a/b/c")).unwrap();
        std::fs::write(ws.join("a/b/c/deep.txt"), "deep").unwrap();

        let magika = test_magika();
        // depth 1: should list "a" but not recurse into "a/b"
        let entries = list_dir_recursive(ws, "", 1, 1, &magika);
        let a = entries.iter().find(|e| e.name == "a").unwrap();
        assert!(a.children.is_none());
    }

    #[test]
    fn list_dir_skips_system_but_shows_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        std::fs::create_dir_all(ws.join(".hidden")).unwrap();
        std::fs::create_dir_all(ws.join("system")).unwrap();
        std::fs::write(ws.join("visible.txt"), "yes").unwrap();

        let magika = test_magika();
        let entries = list_dir_recursive(ws, "", 1, 1, &magika);
        // .hidden + visible.txt shown; system/ filtered out
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.name == ".hidden"));
        assert!(entries.iter().any(|e| e.name == "visible.txt"));
        assert!(!entries.iter().any(|e| e.name == "system"));
    }

    #[test]
    fn list_dir_sorts_dirs_first_then_alphabetical() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        std::fs::write(ws.join("zebra.txt"), "z").unwrap();
        std::fs::create_dir_all(ws.join("alpha")).unwrap();
        std::fs::write(ws.join("apple.txt"), "a").unwrap();
        std::fs::create_dir_all(ws.join("beta")).unwrap();

        let magika = test_magika();
        let entries = list_dir_recursive(ws, "", 1, 1, &magika);
        // Dirs first (alpha, beta), then files (apple.txt, zebra.txt)
        assert_eq!(entries[0].name, "alpha");
        assert_eq!(entries[1].name, "beta");
        assert_eq!(entries[2].name, "apple.txt");
        assert_eq!(entries[3].name, "zebra.txt");
    }

    // -----------------------------------------------------------------------
    // Download / Upload via resolve_workspace_path
    // -----------------------------------------------------------------------

    fn setup_vm_with_workspace(state: &ServiceState, dir: &std::path::Path, vm_id: &str) {
        let session_dir = dir.join("session");
        let workspace = session_dir.join("guest/workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        state.instances.lock().unwrap().insert(
            vm_id.into(),
            InstanceInfo {
                id: vm_id.into(),
                pid: 1,
                uds_path: PathBuf::from("/tmp/test.sock"),
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
    }

    #[test]
    fn download_reads_correct_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let (state, _dir2) = make_test_state_with_tempdir();
        setup_vm_with_workspace(&state, dir.path(), "dl-vm");

        let ws = dir.path().join("session/guest/workspace");
        let content = b"hello world\nline 2\n";
        std::fs::write(ws.join("test.txt"), content).unwrap();

        let (_, resolved) = resolve_workspace_path(&state, "dl-vm", "test.txt").unwrap();
        let data = std::fs::read(&resolved).unwrap();
        assert_eq!(data, content);
    }

    #[test]
    fn download_binary_preserves_content() {
        let dir = tempfile::tempdir().unwrap();
        let (state, _dir2) = make_test_state_with_tempdir();
        setup_vm_with_workspace(&state, dir.path(), "bin-vm");

        let ws = dir.path().join("session/guest/workspace");
        let binary: Vec<u8> = (0..256).map(|i| i as u8).collect();
        std::fs::write(ws.join("data.bin"), &binary).unwrap();

        let (_, resolved) = resolve_workspace_path(&state, "bin-vm", "data.bin").unwrap();
        let data = std::fs::read(&resolved).unwrap();
        assert_eq!(data, binary);
    }

    #[test]
    fn upload_creates_file_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let (state, _dir2) = make_test_state_with_tempdir();
        setup_vm_with_workspace(&state, dir.path(), "up-vm");

        let ws = dir.path().join("session/guest/workspace");
        let (_, target) = resolve_workspace_path(&state, "up-vm", "new.txt").unwrap();
        std::fs::write(&target, b"uploaded").unwrap();

        assert_eq!(std::fs::read_to_string(ws.join("new.txt")).unwrap(), "uploaded");
    }

    #[test]
    fn upload_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let (state, _dir2) = make_test_state_with_tempdir();
        setup_vm_with_workspace(&state, dir.path(), "mkdir-vm");

        let ws = dir.path().join("session/guest/workspace");
        // resolve_workspace_path should succeed even for non-existing nested paths
        let (_, target) = resolve_workspace_path(&state, "mkdir-vm", "deep/nested/file.txt").unwrap();
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"deep content").unwrap();

        assert_eq!(std::fs::read_to_string(ws.join("deep/nested/file.txt")).unwrap(), "deep content");
    }

    #[test]
    fn upload_path_traversal_blocked() {
        let r = sanitize_file_path("../../etc/passwd");
        assert!(r.is_err());
    }

    #[test]
    fn download_nonexistent_file_resolve_ok_but_not_exists() {
        let dir = tempfile::tempdir().unwrap();
        let (state, _dir2) = make_test_state_with_tempdir();
        setup_vm_with_workspace(&state, dir.path(), "404-vm");

        // Resolving a non-existent file path still works (for upload target)
        let result = resolve_workspace_path(&state, "404-vm", "nonexistent.txt");
        assert!(result.is_ok());
        let (_, resolved) = result.unwrap();
        assert!(!resolved.exists());
    }
}
