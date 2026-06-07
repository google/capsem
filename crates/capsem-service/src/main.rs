use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use capsem_core::poll::{poll_until, PollOpts};
use capsem_core::{
    net::policy_config::{
        DetectionLevel, SecurityPluginConfig, SecurityPluginMode, SecurityRule, SecurityRuleGroup,
        SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource, SettingsFile,
    },
    security_engine::{
        FileSecurityEvent, RuntimeSecurityEventType, SecurityActionRegistry, SecurityEmitError,
        SecurityEvent, SecurityEventEmitter, SecurityEventEngine, SerializableSecurityEvent,
    },
};
use capsem_proto::ipc::{FileBoundaryAction, ProcessToService, ServiceToProcess};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path as StdPath, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::UnixListener;
use tokio_unix_ipc::{channel_from_std, Receiver, Sender};
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn, Instrument};

mod startup;

use capsem_service::api;
use capsem_service::api::*;
use capsem_service::naming::{generate_tmp_name, validate_vm_name};
use capsem_service::registry::{PersistentRegistry, PersistentVmEntry};
use capsem_service::triage;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)]
    foreground: bool,
    #[arg(long)]
    uds_path: Option<PathBuf>,
    #[arg(long)]
    process_binary: Option<PathBuf>,
    #[arg(long)]
    gateway_binary: Option<PathBuf>,
    #[arg(long)]
    gateway_port: Option<u16>,
    #[arg(long)]
    tray_binary: Option<PathBuf>,
    #[arg(long)]
    assets_dir: Option<PathBuf>,
    /// When set, exit the moment this PID goes away. Used by the pytest
    /// fixture to bound service lifetime to the test runner so an aborted
    /// pytest (Ctrl-C, xdist worker crash) can't leak a service + its
    /// companions. Real users never pass this.
    #[arg(long)]
    parent_pid: Option<u32>,
}

const PROCESS_ENV_ALLOWLIST: &[&str] = &[
    "HOME",
    "PATH",
    "USER",
    "TMPDIR",
    "CAPSEM_HOME",
    "CAPSEM_USER_CONFIG",
    "CAPSEM_CORP_CONFIG",
    // Tunable: bounded MITM MCP endpoint in-flight handler cap.
    "CAPSEM_MCP_INFLIGHT",
    // Tunable: pool size for the local builtin MCP server (rmcp stdio funnel).
    "CAPSEM_MCP_BUILTIN_POOL",
    // Read by capsem-process when constructing the framed MCP endpoint.
    "CAPSEM_MCP_DEFAULT_TIMEOUT_SECS",
    "CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS",
    "CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS",
    // Experimental rootfs benchmark lane: capsem-process appends
    // capsem.rootfs=erofs-dax when booting a .erofs rootfs.
    "CAPSEM_EXPERIMENTAL_EROFS_DAX",
];

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
    /// In-memory asset reconciliation progress. Service startup and explicit
    /// /assets/ensure share this single rail so status can explain both.
    asset_reconcile: Mutex<AssetReconcileState>,
    asset_reconcile_inflight: AtomicBool,
    asset_status_path: PathBuf,
    /// Magika file-type detection session (thread-safe, shared)
    magika: Mutex<magika::Session>,
    /// Profile-owned plugin policy overrides. Effective policy is built-in
    /// plugin defaults plus overrides for the profile executing the VM.
    plugin_policy_by_profile: Mutex<HashMap<String, BTreeMap<String, SecurityPluginConfig>>>,
    /// Serializes Apple VZ save_state and restore_state calls across all VMs
    /// managed by this service. Apple's Virtualization.framework does not
    /// tolerate concurrent save/restore on sibling VMs: when two VZ instances
    /// each call saveMachineStateToURL (or one calls save_state while another
    /// is mid-restore), one of them can come back with ext4 overlay I/O
    /// errors after resume. Held for the full suspend IPC + child-exit wait,
    /// and for the resume spawn + wait_for_vm_ready window. See
    /// docs/src/content/docs/gotchas/concurrent-suspend-resume.mdx.
    save_restore_lock: tokio::sync::Mutex<()>,
    /// Serializes VM teardown (delete / stop / purge per-VM / handle_run)
    /// across all VMs managed by this service. N concurrent shutdowns starve
    /// each other of the resources each capsem-process needs to (a) let VZ
    /// tear down the guest, (b) run the DbWriter's WAL checkpoint on Drop,
    /// and (c) clean up the session UDS files. Under that contention a
    /// single teardown can exceed `wait_for_process_exit`'s 1s fast-path
    /// budget -- at which point the service SIGKILLs capsem-process mid-
    /// checkpoint, leaving a non-empty WAL and (in the worst case) orphaned
    /// sockets. Same serialization pattern as `save_restore_lock`: one
    /// critical-section operation in flight at a time, in-process only,
    /// sufficient because production runs exactly one capsem-service per
    /// user-host.
    shutdown_lock: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AssetReconcileState {
    #[serde(default)]
    in_progress: bool,
    #[serde(default)]
    current_asset: Option<String>,
    #[serde(default)]
    bytes_done: u64,
    #[serde(default)]
    bytes_total: Option<u64>,
    #[serde(default)]
    last_error: Option<String>,
    #[serde(default)]
    last_downloaded: Option<usize>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PluginScopeKind {
    Profile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PluginScope {
    kind: PluginScopeKind,
    profile_id: String,
}

#[derive(Debug, Serialize)]
struct PluginListResponse {
    scope: PluginScope,
    plugins: Vec<PluginInfo>,
}

#[derive(Debug, Serialize)]
struct PluginInfo {
    id: String,
    config: SecurityPluginConfig,
    default_config: SecurityPluginConfig,
    overridden: bool,
    scope: PluginScope,
    description: &'static str,
}

#[derive(Debug, Deserialize)]
struct PluginUpdate {
    #[serde(default)]
    mode: Option<SecurityPluginMode>,
    #[serde(default)]
    detection_level: Option<DetectionLevel>,
}

#[derive(Debug, Deserialize)]
struct McpToolEditRequest {
    #[serde(default)]
    approved: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct EnforcementEvaluateRequest {
    rules_toml: String,
    event: EnforcementEventInput,
}

impl EnforcementEvaluateRequest {
    #[cfg(test)]
    fn eicar_fixture() -> Self {
        Self {
            rules_toml: r#"
[profiles.rules.eicar]
name = "eicar_rewrite_scan"
plugin = "dummy_pre_eicar"
action = "rewrite"
detection_level = "high"
match = 'file.import.content.contains("EICAR")'
"#
            .to_string(),
            event: EnforcementEventInput {
                event_type: "file.import".to_string(),
                file_import_content: Some(
                    capsem_core::security_engine::DUMMY_EICAR_TEST_STRING.to_string(),
                ),
                http_host: None,
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct EnforcementEventInput {
    event_type: String,
    #[serde(default)]
    file_import_content: Option<String>,
    #[serde(default)]
    http_host: Option<String>,
}

#[derive(Debug, Serialize)]
struct EnforcementEvaluateResponse {
    event: SerializableSecurityEvent,
}

#[derive(Debug, Serialize)]
struct EnforcementRuleResponse {
    rule_id: String,
    compiled_rule_id: String,
    rule: SecurityRule,
}

#[derive(Debug, Serialize)]
struct EnforcementRuleDeleteResponse {
    rule_id: String,
    deleted: bool,
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
/// wait_for_vm_ready timeouts / dead-process cleanup -- and now also for
/// every clean DELETE, so post-mortem of Python-side test assertions that
/// fire after /exec but before the test's `finally: delete()` works (the
/// previous unlink-on-delete left only service.log, which doesn't show
/// what the per-VM process or guest were doing). The preserved dirs hold
/// the only host-side post-mortem signal we have (process.log,
/// mcp-aggregator.stderr.log, serial.log, session.db). 32 is enough to
/// span a 10-iteration stress suite that creates 1-3 VMs per iteration
/// without losing earlier failures to the cull.
const MAX_FAILED_SESSIONS: usize = 32;

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
            info!(
                id,
                "ephemeral VM process died, preserving session dir for post-mortem"
            );
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
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
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

    fn provision_sandbox(self: &Arc<Self>, options: ProvisionOptions) -> Result<()> {
        let ProvisionOptions {
            id,
            ram_mb,
            cpus,
            version_override,
            persistent,
            env,
            from,
            description,
        } = options;

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
                return Err(anyhow!(
                    "persistent VM \"{}\" already exists. Use `capsem resume {}` to reconnect.",
                    id,
                    id
                ));
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
                return Err(anyhow!(
                    "maximum number of concurrent VMs reached ({})",
                    max_concurrent_vms
                ));
            }
        }

        // Validate source sandbox if --from provided
        let source_entry = if let Some(ref from_name) = from {
            let registry = self.persistent_registry.lock().unwrap();
            let entry = registry
                .get(from_name)
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
            return Err(anyhow!(
                "rootfs not found at {}. Dir entries: {:?}",
                resolved.rootfs.display(),
                entries
            ));
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
        for key in PROCESS_ENV_ALLOWLIST {
            if let Ok(val) = std::env::var(key) {
                child_cmd.env(key, val);
            }
        }
        // W4: propagate trace context to the child process.
        // CAPSEM_VM_ID, CAPSEM_TRACE_ID, TRACEPARENT, TRACESTATE.
        for (k, v) in capsem_core::telemetry::child_trace_env(id) {
            child_cmd.env(k, v);
        }

        let process_spawn_span = tracing::debug_span!(
            target: "capsem.launch",
            capsem_core::telemetry::LAUNCH_PROCESS_SPAWN_SPAN,
            boot_mode = "provision",
            status = tracing::field::Empty,
        );
        let mut child = match process_spawn_span.in_scope(|| {
            child_cmd
                .env(
                    "RUST_LOG",
                    std::env::var("RUST_LOG").unwrap_or_else(|_| {
                        capsem_core::telemetry::with_subsys_targets("capsem=info")
                    }),
                )
                .arg("--id")
                .arg(id)
                .arg("--assets-dir")
                .arg(&self.assets_dir)
                .arg("--rootfs")
                .arg(&resolved.rootfs)
                .arg("--kernel")
                .arg(&resolved.kernel)
                .arg("--initrd")
                .arg(&resolved.initrd)
                .arg("--session-dir")
                .arg(&session_dir)
                .arg("--cpus")
                .arg(cpus.to_string())
                .arg("--ram-mb")
                .arg(ram_mb.to_string())
                .arg("--uds-path")
                .arg(&uds_path)
                .stdout(std::process::Stdio::from(process_log_file.try_clone()?))
                .stderr(std::process::Stdio::from(process_log_file))
                .spawn()
        }) {
            Ok(child) => {
                process_spawn_span.record("status", "ok");
                child
            }
            Err(error) => {
                process_spawn_span.record("status", "error");
                return Err(anyhow::Error::new(error).context("failed to spawn capsem-process"));
            }
        };

        let pid = child.id().unwrap_or(0);
        info!(id, pid, version, asset_version = %resolved.asset_version, "capsem-process spawned");

        let id_clone = id.to_string();
        let state_clone = Arc::clone(self);
        let uds_clone = uds_path.clone();
        let session_dir_clone = session_dir.clone();
        tokio::spawn(async move {
            let exit_status = child.wait().await.ok();
            info!(id_clone, ?exit_status, "capsem-process exited, cleaning up");

            // An ephemeral VM's removal from the instances map below is
            // the trigger for preserve_failed_session_dir; if `removed`
            // is Some, the child exited without an explicit
            // capsem-service-side shutdown removing it first.
            //
            // BUT: a guest-initiated shutdown via `capsem-sysutil
            // shutdown` (vsock:5004 -> ProcessToService::Shutdown
            // Requested) also leaves the instance in the map -- the
            // service has no listener for ShutdownRequested, the
            // process just sends Shutdown to itself and exits cleanly
            // with code 0. Treating that as "unexpected" flips the
            // persistent registry to `defunct` so `capsem list` shows
            // the VM as Defunct instead of Stopped, and the next
            // `capsem resume` is misleadingly blocked.
            //
            // Distinguish: a clean exit (code 0) from the process is a
            // graceful shutdown regardless of who initiated it. Any
            // non-zero exit code or signal-kill is a crash.
            let removed = state_clone.instances.lock().unwrap().remove(&id_clone);
            let clean_exit = exit_status.as_ref().is_some_and(|s| s.success());
            let unexpected_exit = removed.is_some() && !clean_exit;

            // Persistent-VM registry bookkeeping. Checkpoint takes
            // precedence: a graceful suspend writes checkpoint.vzsave
            // which we must honor regardless of whether the exit looked
            // "unexpected". `defunct` only fires when the process died
            // WITHOUT writing a checkpoint AND without an explicit
            // shutdown handler removing the instance first.
            {
                let mut registry = state_clone.persistent_registry.lock().unwrap();
                if let Some(entry) = registry.data.vms.get_mut(&id_clone) {
                    let checkpoint_path = session_dir_clone.join("checkpoint.vzsave");
                    if checkpoint_path.exists() {
                        info!(id_clone, "Checkpoint file found, marking VM as suspended");
                        entry.suspended = true;
                        entry.checkpoint_path = Some("checkpoint.vzsave".to_string());
                        entry.defunct = false;
                        entry.last_error = None;
                    } else {
                        entry.suspended = false;
                        entry.checkpoint_path = None;
                        if unexpected_exit {
                            entry.defunct = true;
                            entry.last_error = Some(read_process_log_tail(&session_dir_clone, 20));
                        } else {
                            // Graceful stop / delete path -- not a crash.
                            entry.defunct = false;
                            entry.last_error = None;
                        }
                    }
                    if let Err(e) = registry.save() {
                        error!(id_clone, "failed to save persistent registry: {e}");
                    }
                }
            }

            // Ephemeral session dirs: preserve on unexpected exit so
            // process.log / mcp-aggregator.stderr.log / serial.log /
            // session.db survive for post-mortem. `find_failed_session_dir`
            // + handle_logs surface them to `capsem logs`.
            if let Some(info) = removed {
                if unexpected_exit {
                    tracing::warn!(
                        id_clone,
                        ?exit_status,
                        "child exited unexpectedly, preserving session dir"
                    );
                    if !info.persistent {
                        state_clone.preserve_failed_session_dir(&info.session_dir, &id_clone);
                    }
                } else {
                    tracing::info!(id_clone, "child exited cleanly (guest-initiated shutdown)");
                }
            } else {
                tracing::debug!(
                    id_clone,
                    "child exited after explicit service-side shutdown"
                );
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
                created_at: format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                ),
                session_dir: session_dir.clone(),
                forked_from: from.clone(),
                description: description.clone(),
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: env.clone(),
            })?;
        }

        let mut instances = self.instances.lock().unwrap();
        instances.insert(
            id.to_string(),
            InstanceInfo {
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
            },
        );

        Ok(())
    }

    /// Resume a stopped persistent VM by re-spawning capsem-process against its
    /// existing session directory.
    fn resume_sandbox(
        self: &Arc<Self>,
        name: &str,
        ram_mb_override: Option<u64>,
        cpus_override: Option<u32>,
    ) -> Result<String> {
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
            registry
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow!("no persistent VM named \"{}\"", name))?
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
        child_cmd
            .arg("--env")
            .arg(format!("CAPSEM_VM_NAME={}", name));

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
        for key in PROCESS_ENV_ALLOWLIST {
            if let Ok(val) = std::env::var(key) {
                child_cmd.env(key, val);
            }
        }
        // W4: propagate trace context (resume path).
        for (k, v) in capsem_core::telemetry::child_trace_env(name) {
            child_cmd.env(k, v);
        }

        let process_spawn_span = tracing::debug_span!(
            target: "capsem.launch",
            capsem_core::telemetry::LAUNCH_PROCESS_SPAWN_SPAN,
            boot_mode = "resume",
            status = tracing::field::Empty,
        );
        let mut child = match process_spawn_span.in_scope(|| {
            child_cmd
                .env(
                    "RUST_LOG",
                    std::env::var("RUST_LOG").unwrap_or_else(|_| {
                        capsem_core::telemetry::with_subsys_targets("capsem=info")
                    }),
                )
                .arg("--id")
                .arg(name)
                .arg("--assets-dir")
                .arg(&self.assets_dir)
                .arg("--rootfs")
                .arg(&resolved.rootfs)
                .arg("--kernel")
                .arg(&resolved.kernel)
                .arg("--initrd")
                .arg(&resolved.initrd)
                .arg("--session-dir")
                .arg(&entry.session_dir)
                .arg("--cpus")
                .arg(cpus.to_string())
                .arg("--ram-mb")
                .arg(ram_mb.to_string())
                .arg("--uds-path")
                .arg(&uds_path)
                .stdout(std::process::Stdio::from(process_log_file.try_clone()?))
                .stderr(std::process::Stdio::from(process_log_file))
                .spawn()
        }) {
            Ok(child) => {
                process_spawn_span.record("status", "ok");
                child
            }
            Err(error) => {
                process_spawn_span.record("status", "error");
                return Err(anyhow::Error::new(error).context("failed to spawn capsem-process"));
            }
        };

        let pid = child.id().unwrap_or(0);
        info!(name, pid, "capsem-process resumed");

        let name_clone = name.to_string();
        let state_clone = Arc::clone(self);
        let uds_clone = uds_path.clone();
        tokio::spawn(async move {
            let exit_status = child.wait().await;
            info!(name_clone, exit_status = ?exit_status, "capsem-process (resume) exited, cleaning up");
            // Persistent VMs: remove from instances but keep session dir.
            tracing::warn!(name_clone, exit_status = ?exit_status, "resume_sandbox child exit handler removing instance");
            state_clone.instances.lock().unwrap().remove(&name_clone);
            let _ = std::fs::remove_file(&uds_clone);
            let _ = std::fs::remove_file(uds_clone.with_extension("ready"));
        });

        let mut instances = self.instances.lock().unwrap();
        instances.insert(
            name.to_string(),
            InstanceInfo {
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
            },
        );

        Ok(name.to_string())
    }

    fn has_existing_resume_checkpoint(&self, name: &str) -> bool {
        let registry = self.persistent_registry.lock().unwrap();
        registry.get(name).is_some_and(|entry| {
            entry.suspended
                && entry
                    .checkpoint_path
                    .as_ref()
                    .is_some_and(|cp| entry.session_dir.join(cp).exists())
        })
    }

    fn archive_failed_restore_checkpoint(&self, name: &str) -> Option<PathBuf> {
        let (session_dir, checkpoint_name) = {
            let registry = self.persistent_registry.lock().unwrap();
            let entry = registry.get(name)?;
            let checkpoint_name = entry.checkpoint_path.clone()?;
            (entry.session_dir.clone(), checkpoint_name)
        };

        let checkpoint_path = session_dir.join(&checkpoint_name);
        if !checkpoint_path.exists() {
            return None;
        }

        let epoch_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let archived_path =
            session_dir.join(format!("{checkpoint_name}.failed-restore-{epoch_ms}"));

        match std::fs::rename(&checkpoint_path, &archived_path) {
            Ok(()) => {
                warn!(
                    name,
                    checkpoint = %checkpoint_path.display(),
                    archived = %archived_path.display(),
                    "archived failed restore checkpoint before cold fallback"
                );
                Some(archived_path)
            }
            Err(e) => {
                error!(
                    name,
                    checkpoint = %checkpoint_path.display(),
                    archived = %archived_path.display(),
                    "failed to archive restore checkpoint: {e}"
                );
                None
            }
        }
    }

    fn clear_resume_checkpoint(&self, id: &str) {
        let mut registry = self.persistent_registry.lock().unwrap();
        if let Some(entry) = registry.get_mut(id) {
            entry.suspended = false;
            entry.checkpoint_path = None;
            entry.defunct = false;
            entry.last_error = None;
            if let Err(e) = registry.save() {
                error!(id, "failed to save persistent registry after resume: {e}");
            }
        }
    }

    /// Resolve asset file paths for a VM.
    ///
    /// In v2 mode (manifest present): resolves hash-based filenames from manifest.
    /// In dev mode (no manifest): finds assets by logical name in arch subdirs.
    fn resolve_asset_paths(&self) -> Result<capsem_core::asset_manager::ResolvedAssets> {
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x86_64"
        };

        // Resolve from v2 manifest (works for both dev and installed --
        // dev creates hash-named symlinks, installed has hash-named files)
        if let Some(ref manifest) = self.manifest {
            return manifest.resolve(&self.current_version, arch, &self.assets_dir);
        }

        // No manifest: use logical names as fallback. Prefer the release
        // rootfs format when both modern and legacy dev assets exist.
        let base = if self.assets_dir.join(arch).join("rootfs.erofs").exists()
            || self.assets_dir.join(arch).join("rootfs.squashfs").exists()
        {
            self.assets_dir.join(arch)
        } else {
            self.assets_dir.clone()
        };
        let rootfs = if base.join("rootfs.erofs").exists() {
            base.join("rootfs.erofs")
        } else {
            base.join("rootfs.squashfs")
        };
        Ok(capsem_core::asset_manager::ResolvedAssets {
            kernel: base.join("vmlinuz"),
            initrd: base.join("initrd.img"),
            rootfs,
            asset_version: "dev".to_string(),
        })
    }
}

/// Identify the launchd-cleanup-saturation transient that masquerades
/// as an "entitlement missing" error from VZ.
///
/// Apple's `Virtualization.framework` runs a per-VM XPC helper
/// (`com.apple.Virtualization.VirtualMachine.<UUID>`). When capsem-process
/// dies, launchd schedules that XPC's cleanup with a 9s delay. Under
/// rapid VM churn (~3s/cycle) the PETRIFIED-pending queue grows; once
/// `syspolicyd` saturates (we observe `Unable to get certificates
/// array: (null)` in the unified log just before the failure window),
/// the next `VZVirtualMachineConfiguration.validateWithError()`
/// returns NSError code 2 with the misleading
/// `localizedDescription = "...The process doesn't have the
/// 'com.apple.security.virtualization' entitlement."` string -- even
/// though the binary IS entitled. The error message is wrong; the
/// actual cause is launchd cleanup saturation that drains within a
/// second or two.
///
/// Pattern-match on the full VZ-specific phrase (not just the bare
/// word "entitlement") so a real codesign regression -- which we'd
/// also want to surface -- is not silently retried away. The error
/// string is stable across VZ releases since it comes from VZ's
/// localized string table, not our code.
fn is_launchd_cleanup_transient(process_log_tail: &str) -> bool {
    process_log_tail.contains("com.apple.security.virtualization")
        && process_log_tail.contains("entitlement")
}

/// Read the last `n` lines of `<session_dir>/process.log`. Returns a
/// placeholder string when the log is absent or unreadable, so callers
/// can always embed SOMETHING meaningful in a user-facing error.
fn read_process_log_tail(session_dir: &std::path::Path, n: usize) -> String {
    let log_path = session_dir.join("process.log");
    let content = match std::fs::read_to_string(&log_path) {
        Ok(c) => c,
        Err(e) => return format!("(could not read {}: {e})", log_path.display()),
    };
    let lines: Vec<&str> = content.lines().collect();
    let tail = if lines.len() > n {
        &lines[lines.len() - n..]
    } else {
        &lines[..]
    };
    tail.join("\n")
}

/// Find the most recent `sessions/<id>-failed-<suffix>/` directory for a
/// given VM id. Returns `None` when no failed session has been preserved
/// (e.g. the VM id is simply unknown). Used by `handle_logs` so a user
/// running `capsem logs <id>` after a boot crash sees the logs that
/// `preserve_failed_session_dir` saved instead of a 404.
fn find_failed_session_dir(run_dir: &std::path::Path, id: &str) -> Option<PathBuf> {
    let sessions_dir = run_dir.join("sessions");
    let entries = std::fs::read_dir(&sessions_dir).ok()?;
    let prefix = format!("{id}-failed-");
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        match &best {
            Some((_, existing)) if *existing >= mtime => {}
            _ => best = Some((path, mtime)),
        }
    }
    best.map(|(p, _)| p)
}

use axum::http::StatusCode;
use capsem_service::errors::AppError;
use capsem_service::fs_utils::{identify_file_sync, sanitize_file_path};

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
            reg.data
                .vms
                .get(id)
                .or_else(|| reg.data.vms.values().find(|e| e.name == id))
                .map(|e| e.session_dir.clone())
                .ok_or_else(|| {
                    AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}"))
                })?
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
                let canon_parent = parent.canonicalize().map_err(|e| {
                    AppError(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("canonicalize: {e}"),
                    )
                })?;
                let ws_canon = workspace_root.canonicalize().map_err(|e| {
                    AppError(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("canonicalize workspace: {e}"),
                    )
                })?;
                if !canon_parent.starts_with(&ws_canon) {
                    return Err(AppError(
                        StatusCode::FORBIDDEN,
                        "path outside workspace".into(),
                    ));
                }
                return Ok((workspace_root, target));
            }
        }
        return Ok((workspace_root, target));
    }
    .map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("canonicalize: {e}"),
        )
    })?;

    let ws_canon = workspace_root.canonicalize().map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("canonicalize workspace: {e}"),
        )
    })?;
    if !canonical.starts_with(&ws_canon) {
        return Err(AppError(
            StatusCode::FORBIDDEN,
            "path outside workspace".into(),
        ));
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

fn default_file_depth() -> u32 {
    1
}

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
        b_is_dir
            .cmp(&a_is_dir)
            .then_with(|| a.file_name().cmp(&b.file_name()))
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
        let mtime = meta
            .modified()
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
                reg.data
                    .vms
                    .get(&id)
                    .or_else(|| reg.data.vms.values().find(|e| e.name == id))
                    .map(|e| e.session_dir.clone())
                    .ok_or_else(|| {
                        AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}"))
                    })?
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
    let rel_prefix = target
        .strip_prefix(&workspace_root)
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
        })
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("list: {e}")))?
    };

    Ok(Json(FileListResponse {
        entries: magika_ref,
    }))
}

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB
const FILE_SECURITY_CONTENT_PREVIEW_MAX: usize = 64 * 1024;

fn file_security_preview_bytes(data: &[u8]) -> Vec<u8> {
    data[..data.len().min(FILE_SECURITY_CONTENT_PREVIEW_MAX)].to_vec()
}

fn active_instance_uds_path(state: &Arc<ServiceState>, id: &str) -> Result<PathBuf, AppError> {
    let instances = state.instances.lock().unwrap();
    instances
        .get(id)
        .map(|i| i.uds_path.clone())
        .ok_or_else(|| {
            AppError(
                StatusCode::CONFLICT,
                "file import/export requires a running sandbox security ledger".into(),
            )
        })
}

async fn log_file_boundary(
    state: &Arc<ServiceState>,
    sandbox_id: &str,
    action: FileBoundaryAction,
    path: String,
    data_preview: Vec<u8>,
    size: u64,
    mime_type: Option<String>,
) -> Result<(), AppError> {
    let uds_path = active_instance_uds_path(state, sandbox_id)?;
    wait_for_vm_ready(&uds_path, 30, Some(state), Some(sandbox_id))
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let id = state.next_job_id();
    let res = send_ipc_command(
        &uds_path,
        ServiceToProcess::LogFileBoundary {
            id,
            action,
            path,
            data: data_preview,
            size,
            mime_type,
        },
        Some(5),
    )
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    match res {
        ProcessToService::LogFileBoundaryResult { success: true, .. } => Ok(()),
        ProcessToService::LogFileBoundaryResult { error, .. } => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            error.unwrap_or_else(|| "failed to log file boundary".into()),
        )),
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response for file boundary log".into(),
        )),
    }
}

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
        return Err(AppError(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "file too large: {} bytes (max {})",
                meta.len(),
                MAX_FILE_SIZE
            ),
        ));
    }

    // Read file and detect type in spawn_blocking
    let state_clone = Arc::clone(&state);
    let resolved_clone = resolved.clone();
    let (data, mime, filename) = tokio::task::spawn_blocking(move || {
        let data = std::fs::read(&resolved_clone)
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("read: {e}")))?;
        let (_, mime_str, _, _) = identify_file_sync(&state_clone.magika, &resolved_clone);
        let name = resolved_clone
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".into());
        // Sanitize the filename for Content-Disposition
        let safe_name: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
            .collect();
        Ok::<_, AppError>((data, mime_str, safe_name))
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("task: {e}")))??;

    log_file_boundary(
        &state,
        &id,
        FileBoundaryAction::Export,
        sanitized,
        file_security_preview_bytes(&data),
        data.len() as u64,
        Some(mime.clone()),
    )
    .await?;

    use axum::response::IntoResponse;
    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, mime),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
            (axum::http::header::CONTENT_LENGTH, data.len().to_string()),
        ],
        data,
    )
        .into_response())
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
    let preview = file_security_preview_bytes(&body);
    let target_for_write = target.clone();

    log_file_boundary(
        &state,
        &id,
        FileBoundaryAction::Import,
        sanitized,
        preview,
        size,
        None,
    )
    .await?;

    // Write file in spawn_blocking (blocking I/O)
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = target_for_write.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir: {e}")))?;
        }
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o644)
            .open(&target_for_write)
            .and_then(|f| {
                use std::io::Write;
                let mut f = f;
                f.write_all(&body)?;
                Ok(())
            })
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("write: {e}")))?;
        Ok::<_, AppError>(())
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("task: {e}")))??;

    Ok(Json(UploadResponse {
        success: true,
        size,
    }))
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
            return Err(AppError(
                StatusCode::CONFLICT,
                format!("sandbox '{}' already exists", name),
            ));
        }
    }

    // Find source: running instance or stopped persistent VM
    let (session_dir, ram_mb, cpus, base_version, uds_path) = {
        let instances = state.instances.lock().unwrap();
        if let Some(i) = instances.get(&id) {
            (
                i.session_dir.clone(),
                i.ram_mb,
                i.cpus,
                i.base_version.clone(),
                Some(i.uds_path.clone()),
            )
        } else {
            drop(instances);
            let registry = state.persistent_registry.lock().unwrap();
            if let Some(p) = registry.get(&id) {
                (
                    p.session_dir.clone(),
                    p.ram_mb,
                    p.cpus,
                    p.base_version.clone(),
                    None,
                )
            } else {
                return Err(AppError(
                    StatusCode::NOT_FOUND,
                    format!("source sandbox not found: {}", id),
                ));
            }
        }
    };

    // Freeze + thaw the guest root filesystem so the ext4 system overlay
    // (/dev/vdb backed by rootfs.img) is fully flushed before fork clone.
    if let Some(ref uds) = uds_path {
        let freeze_id = state.next_job_id();
        if let Err(e) = send_ipc_command(
            uds,
            ServiceToProcess::Exec {
                id: freeze_id,
                command: "fsfreeze -f / 2>/dev/null; sync; fsfreeze -u / 2>/dev/null; true"
                    .to_string(),
            },
            Some(10),
        )
        .await
        {
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
    .map_err(|e| {
        capsem_service::app_error_logged!(
            error,
            StatusCode::INTERNAL_SERVER_ERROR,
            "fork: clone-task panic: {e}"
        )
    })?
    .map_err(|e| {
        capsem_service::app_error_logged!(
            error,
            StatusCode::INTERNAL_SERVER_ERROR,
            "fork: clone failed: {e}"
        )
    })?;

    // Register as persistent VM
    {
        let mut registry = state.persistent_registry.lock().unwrap();
        registry
            .register(PersistentVmEntry {
                name: name.clone(),
                ram_mb,
                cpus,
                base_version,
                created_at: format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                ),
                session_dir: new_session_dir,
                forked_from: Some(id.clone()),
                description: payload.description.clone(),
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            })
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(Json(ForkResponse {
        name: name.clone(),
        size_bytes,
    }))
}

/// Outcome of a single provision attempt inside `handle_provision`.
/// `LaunchdTransient` is the recoverable case: VZ rejected the fresh
/// VM with the misleading entitlement string while launchd's
/// PETRIFIED-cleanup queue was draining. The poll_until loop retries
/// on this; everything else (incl. `Other`) bubbles up unchanged.
enum ProvisionAttemptOutcome {
    Ready { uds_path: PathBuf },
    StillBootingTimedOut { uds_path: PathBuf }, // 5s envelope hit; treat as success per pre-existing contract
    LaunchdTransient,
    BootCrash { tail: String },
    ProvisionError(anyhow::Error),
}

/// Decision the retry loop takes after observing one provision attempt.
/// Pure function of the outcome -- no side effects -- so the
/// retry-routing can be unit-tested without spawning a real VM.
#[derive(Debug)]
enum AttemptDecision {
    Succeed(PathBuf),
    BailWithError(AppError),
    RetryAfterCleanup,
}

/// Map a single attempt's outcome to the retry loop's next move.
/// The `LaunchdTransient` variant is the only one that triggers retry;
/// `BootCrash` and `ProvisionError` bail with structured errors that
/// match the pre-refactor handle_provision response shape.
fn classify_attempt_decision(outcome: ProvisionAttemptOutcome, id: &str) -> AttemptDecision {
    match outcome {
        ProvisionAttemptOutcome::Ready { uds_path }
        | ProvisionAttemptOutcome::StillBootingTimedOut { uds_path } => {
            AttemptDecision::Succeed(uds_path)
        }
        ProvisionAttemptOutcome::LaunchdTransient => AttemptDecision::RetryAfterCleanup,
        ProvisionAttemptOutcome::BootCrash { tail } => AttemptDecision::BailWithError(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "sandbox {id} failed to boot. process.log tail:\n\n{tail}\n\n\
                 (full logs: `capsem logs {id}`)"
            ),
        )),
        ProvisionAttemptOutcome::ProvisionError(e) => {
            let status = if e.to_string().contains("already exists") {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            AttemptDecision::BailWithError(AppError(status, format!("provision failed: {e}")))
        }
    }
}

async fn handle_provision(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, AppError> {
    if let Some(reason) = vm_asset_block_reason(&state) {
        return Err(AppError(StatusCode::PRECONDITION_FAILED, reason));
    }

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
    let cpus = payload
        .cpus
        .unwrap_or_else(|| vm_settings.cpu_count.unwrap_or(4));

    // Retry budget for the launchd-cleanup transient. Failed attempts
    // fast-fail in ~500ms (capsem-process spawn -> validateWithError
    // crash -> child-exit handler -> instances-map removal observable
    // here), so 8s covers ~5-8 attempts including backoff. Successful
    // attempts return on the first poll iteration regardless of timeout.
    // Backoff lets launchd tick at least one PETRIFIED-cleanup entry
    // (9s wall-clock per entry) between retries; under a real cascade
    // the second attempt usually lands once one entry has drained.
    let opts = capsem_core::poll::PollOpts {
        label: "provision-launchd-drain",
        timeout: std::time::Duration::from_secs(8),
        initial_delay: std::time::Duration::from_millis(200),
        max_delay: std::time::Duration::from_millis(500),
    };

    let id_for_loop = id.clone();
    let attempt_num = std::sync::atomic::AtomicU32::new(0);
    let result = capsem_core::poll::poll_until(opts, || {
        let state = Arc::clone(&state);
        let id = id_for_loop.clone();
        let payload_env = payload.env.clone();
        let payload_from = payload.from.clone();
        let payload_persistent = payload.persistent;
        let attempt = attempt_num.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        async move {
            // Before retry attempts (>1), clear any state the prior
            // failed attempt left behind so provision_sandbox does not
            // reject with "already exists". The child-exit handler has
            // already done its own cleanup (instances.remove +
            // preserve_failed_session_dir) by the time we observe
            // crash-before-ready; we only need to undo registration of
            // the persistent entry.
            if attempt > 1 {
                let mut registry = state.persistent_registry.lock().unwrap();
                let _ = registry.unregister(&id);
                drop(registry);
                state.instances.lock().unwrap().remove(&id);
                warn!(
                    id,
                    attempt, "retrying provision after launchd-cleanup transient"
                );
            }

            let outcome = provision_attempt(
                &state,
                &id,
                ram_mb,
                cpus,
                payload_persistent,
                payload_env,
                payload_from,
            )
            .await;
            // Log structured context BEFORE losing the outcome to classify_*.
            // BootCrash/ProvisionError still produce a user-facing error
            // body via classify_attempt_decision; these logs are for
            // operators reading service.log.
            if matches!(&outcome, ProvisionAttemptOutcome::BootCrash { .. }) {
                error!(id, "capsem-process exited before reaching ready");
            } else if let ProvisionAttemptOutcome::ProvisionError(ref e) = outcome {
                error!(id, "provision failed: {e}");
            }
            match classify_attempt_decision(outcome, &id) {
                AttemptDecision::Succeed(uds_path) => Some(Ok(uds_path)),
                AttemptDecision::RetryAfterCleanup => None, // poll_until retries
                AttemptDecision::BailWithError(err) => Some(Err(err)),
            }
        }
    })
    .await;

    match result {
        Ok(Ok(uds_path)) => Ok(Json(ProvisionResponse {
            id,
            uds_path: Some(uds_path),
        })),
        Ok(Err(app_err)) => Err(app_err),
        Err(timed_out) => {
            // Exhausted retries on launchd transient. Surface the most
            // recent failed-attempt tail so the user sees what VZ said,
            // even though the actual cause is launchd-side saturation.
            let tail = match find_failed_session_dir(&state.run_dir, &id) {
                Some(dir) => read_process_log_tail(&dir, 20),
                None => "(no preserved log found)".to_string(),
            };
            error!(
                id,
                attempts = timed_out.attempts,
                "provision: launchd-cleanup retries exhausted"
            );
            Err(AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "sandbox {id} could not be provisioned after {} attempts ({}). \
                     This typically clears within 10s; please retry. process.log tail:\n\n{tail}\n\n\
                     (full logs: `capsem logs {id}`)",
                    timed_out.attempts, timed_out
                ),
            ))
        }
    }
}

/// Run one provision attempt: spawn capsem-process, then poll up to 5s
/// for either the `.ready` sentinel or a crash-before-ready signal.
/// Pure bookkeeping; no retry logic here -- caller drives the retry
/// loop on `ProvisionAttemptOutcome::LaunchdTransient`.
#[allow(clippy::too_many_arguments)]
async fn provision_attempt(
    state: &Arc<ServiceState>,
    id: &str,
    ram_mb: u64,
    cpus: u32,
    persistent: bool,
    env: Option<std::collections::HashMap<String, String>>,
    from: Option<String>,
) -> ProvisionAttemptOutcome {
    let state_clone = Arc::clone(state);
    let id_owned = id.to_string();
    let version = state.current_version.clone();
    let provision_result = match tokio::task::spawn_blocking(move || {
        state_clone.provision_sandbox(ProvisionOptions {
            id: &id_owned,
            ram_mb,
            cpus,
            version_override: Some(version),
            persistent,
            env,
            from,
            description: None,
        })
    })
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return ProvisionAttemptOutcome::ProvisionError(anyhow::anyhow!("provision task: {e}"))
        }
    };

    if let Err(e) = provision_result {
        return ProvisionAttemptOutcome::ProvisionError(e);
    }

    // Wait briefly for either the `.ready` sentinel or the child-exit
    // handler to remove the VM from the instances map (crash). Without
    // this poll, `capsem create` prints the id and exits 0 while the
    // guest is already dead. 5s is enough to catch synchronous boot
    // failures (missing asset, signed-manifest mismatch, Apple VZ
    // entitlement transient -- all < 1s) without penalizing slow-but-
    // valid boots; on hit we let the caller still hand the id back.
    let uds_path = state.instance_socket_path(id);
    let ready_path = uds_path.with_extension("ready");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if ready_path.exists() {
            return ProvisionAttemptOutcome::Ready { uds_path };
        }
        let still_alive = state.instances.lock().unwrap().contains_key(id);
        if !still_alive {
            // Crash before ready. Prefer the persistent entry's
            // cached last_error (already computed by the child-exit
            // handler) to avoid re-reading the log; fall back to
            // find_failed_session_dir for ephemeral VMs whose dir was
            // renamed to `-failed-*`.
            let cached = {
                let registry = state.persistent_registry.lock().unwrap();
                registry.get(id).and_then(|e| e.last_error.clone())
            };
            let tail =
                cached.unwrap_or_else(|| match find_failed_session_dir(&state.run_dir, id) {
                    Some(dir) => read_process_log_tail(&dir, 20),
                    None => "(no preserved log found)".to_string(),
                });
            return if is_launchd_cleanup_transient(&tail) {
                warn!(id, "provision: detected launchd-cleanup transient (misleading 'entitlement' error)");
                ProvisionAttemptOutcome::LaunchdTransient
            } else {
                ProvisionAttemptOutcome::BootCrash { tail }
            };
        }
        if tokio::time::Instant::now() >= deadline {
            return ProvisionAttemptOutcome::StillBootingTimedOut { uds_path };
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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

async fn handle_list(State(state): State<Arc<ServiceState>>) -> Json<ListResponse> {
    let mut sandboxes: Vec<SandboxInfo> = Vec::new();

    // Running instances (with live telemetry)
    {
        let instances = state.instances.lock().unwrap();
        for i in instances.values() {
            let mut info = SandboxInfo::new(i.id.clone(), i.pid, "Running".into(), i.persistent);
            info.name = if i.persistent {
                Some(i.id.clone())
            } else {
                None
            };
            info.ram_mb = Some(i.ram_mb);
            info.cpus = Some(i.cpus);
            info.version = Some(i.base_version.clone());
            info.forked_from = i.forked_from.clone();
            info.uptime_secs = Some(i.start_time.elapsed().as_secs());
            enrich_telemetry(&mut info, &i.session_dir);
            sandboxes.push(info);
        }
    }

    // Stopped/Suspended/Defunct persistent VMs (not in instances map).
    // `Defunct` surfaces a boot failure so users see the problem in
    // `capsem list` instead of a misleading "Stopped" -- last_error
    // carries the tail of process.log for one-line diagnosis.
    {
        let registry = state.persistent_registry.lock().unwrap();
        let instances = state.instances.lock().unwrap();
        for entry in registry.list() {
            if !instances.contains_key(&entry.name) {
                let status = if entry.defunct {
                    "Defunct"
                } else if entry.suspended {
                    "Suspended"
                } else {
                    "Stopped"
                };
                let mut info = SandboxInfo::new(entry.name.clone(), 0, status.into(), true);
                info.name = Some(entry.name.clone());
                info.ram_mb = Some(entry.ram_mb);
                info.cpus = Some(entry.cpus);
                info.version = Some(entry.base_version.clone());
                info.forked_from = entry.forked_from.clone();
                info.description = entry.description.clone();
                if entry.defunct {
                    info.last_error = entry.last_error.clone();
                }
                sandboxes.push(info);
            }
        }
    }

    // Check asset health
    let asset_health = match state.resolve_asset_paths() {
        Ok(resolved) => {
            let mut missing = Vec::new();
            if !resolved.kernel.exists() {
                missing.push("vmlinuz".to_string());
            }
            if !resolved.initrd.exists() {
                missing.push("initrd.img".to_string());
            }
            if !resolved.rootfs.exists() {
                missing.push(
                    resolved
                        .rootfs
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("rootfs")
                        .to_string(),
                );
            }
            Some(AssetHealth {
                ready: missing.is_empty(),
                version: Some(resolved.asset_version),
                missing,
            })
        }
        Err(_) => None,
    };

    Json(ListResponse {
        sandboxes,
        asset_health,
    })
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
                    let mut info =
                        SandboxInfo::new(i.id.clone(), i.pid, "Running".into(), i.persistent);
                    info.name = if i.persistent {
                        Some(i.id.clone())
                    } else {
                        None
                    };
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

    // Check stopped/suspended/defunct persistent VMs
    {
        let registry = state.persistent_registry.lock().unwrap();
        if let Some(entry) = registry.get(&id) {
            let status = if entry.defunct {
                "Defunct"
            } else if entry.suspended {
                "Suspended"
            } else {
                "Stopped"
            };
            let mut info = SandboxInfo::new(entry.name.clone(), 0, status.into(), true);
            info.name = Some(entry.name.clone());
            info.ram_mb = Some(entry.ram_mb);
            info.cpus = Some(entry.cpus);
            info.version = Some(entry.base_version.clone());
            info.forked_from = entry.forked_from.clone();
            info.description = entry.description.clone();
            if entry.defunct {
                info.last_error = entry.last_error.clone();
            }
            info.size_bytes =
                capsem_core::auto_snapshot::sandbox_disk_usage(&entry.session_dir).ok();
            return Ok(Json(info));
        }
    }

    Err(AppError(
        StatusCode::NOT_FOUND,
        format!("sandbox not found: {id}"),
    ))
}

/// GET /stats -- return full main.db aggregation in one response.
async fn handle_stats(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<StatsResponse>, AppError> {
    let db_path = state.main_db_path();
    let index = capsem_core::session::SessionIndex::open(&db_path).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to open main.db: {e}"),
        )
    })?;

    let global = index.global_stats().map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("global_stats: {e}"),
        )
    })?;
    let sessions = index
        .recent(100)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("recent: {e}")))?;
    let top_providers = index.top_providers(20).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("top_providers: {e}"),
        )
    })?;
    let top_tools = index
        .top_tools(20)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("top_tools: {e}")))?;
    let top_mcp_tools = index.top_mcp_tools(20).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("top_mcp_tools: {e}"),
        )
    })?;

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
            match registry.get(&id).map(|e| e.session_dir.clone()) {
                Some(dir) => dir,
                None => {
                    // VM might have crashed on boot. preserve_failed_session_dir
                    // renames `sessions/<id>` to `sessions/<id>-failed-<suffix>`,
                    // so the most recent `<id>-failed-*` still has the logs the
                    // user needs to debug the crash. Without this branch
                    // `capsem logs <id>` just returns 404 after a boot failure,
                    // which is exactly when logs matter most.
                    match find_failed_session_dir(&state.run_dir, &id) {
                        Some(dir) => dir,
                        None => {
                            return Err(AppError(
                                StatusCode::NOT_FOUND,
                                format!("sandbox not found: {id}"),
                            ))
                        }
                    }
                }
            }
        }
    };

    let serial_log_path = session_dir.join("serial.log");
    let process_log_path = session_dir.join("process.log");

    let (serial_logs, process_logs) = tokio::task::spawn_blocking(move || {
        let serial = std::fs::read_to_string(&serial_log_path).ok();
        let process = std::fs::read_to_string(&process_log_path).ok();
        (serial, process)
    })
    .await
    .map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("log read failed: {e}"),
        )
    })?;

    Ok(Json(LogsResponse {
        logs: serial_logs.as_deref().unwrap_or("").to_string(),
        serial_logs,
        process_logs,
    }))
}

/// `GET /panics?since=30m&limit=20` -- structured panic + backtrace
/// extractor across all host log files. Returns JSON array. Used by the
/// `capsem_panics` MCP tool.
async fn handle_panics(
    State(state): State<Arc<ServiceState>>,
    axum::extract::Query(params): axum::extract::Query<TriageQuery>,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    let since_unix = params
        .since
        .as_deref()
        .and_then(triage::parse_since)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let limit = params.limit.unwrap_or(20).min(200);

    let run_dir = state.run_dir.clone();
    let home = capsem_core::paths::capsem_home();

    let mut all_panics: Vec<triage::PanicEvent> = Vec::new();
    for binary in ["service", "mcp", "gateway", "tray"] {
        if let Some(path) = triage::host_log_path(&run_dir, binary) {
            all_panics.extend(triage::scan_panics_in_file(
                &path,
                &format!("capsem-{binary}"),
                since_unix,
            ));
        }
    }
    if let Some(path) = triage::latest_app_log(&home) {
        all_panics.extend(triage::scan_panics_in_file(&path, "capsem-app", since_unix));
    }

    all_panics.truncate(limit);
    Ok(axum::Json(serde_json::json!({ "panics": all_panics })))
}

/// `GET /triage?id=<vm>&since=30m&limit=20` -- ranked summary of recent
/// panics, errors, and slow ops across host logs (and, when `id` is
/// provided, session.db error rows). Used by the `capsem_triage` MCP
/// tool.
async fn handle_triage(
    State(state): State<Arc<ServiceState>>,
    axum::extract::Query(params): axum::extract::Query<TriageQuery>,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    let since_str = params.since.clone().unwrap_or_else(|| "30m".to_string());
    let since_unix = triage::parse_since(&since_str)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let limit = params.limit.unwrap_or(20).min(200);

    let run_dir = state.run_dir.clone();
    let home = capsem_core::paths::capsem_home();

    let mut panics: Vec<triage::PanicEvent> = Vec::new();
    let mut errors: Vec<triage::ErrorEvent> = Vec::new();
    let mut slow_ops: Vec<triage::SlowOpEvent> = Vec::new();

    for binary in ["service", "mcp", "gateway", "tray"] {
        if let Some(path) = triage::host_log_path(&run_dir, binary) {
            let bin_label = format!("capsem-{binary}");
            panics.extend(triage::scan_panics_in_file(&path, &bin_label, since_unix));
            errors.extend(triage::scan_errors_in_file(
                &path, &bin_label, since_unix, limit,
            ));
            slow_ops.extend(triage::scan_slow_ops_in_file(
                &path, &bin_label, since_unix, 500,
            ));
        }
    }
    if let Some(path) = triage::latest_app_log(&home) {
        panics.extend(triage::scan_panics_in_file(&path, "capsem-app", since_unix));
        errors.extend(triage::scan_errors_in_file(
            &path,
            "capsem-app",
            since_unix,
            limit,
        ));
    }

    panics.truncate(limit);
    errors.truncate(limit);
    slow_ops.truncate(limit);

    // F6: when `id` is set, query session.db for session-scoped error
    // signals. Best-effort -- a missing or vacuumed DB just leaves the
    // session block empty, the host-side triage still returns.
    let session_block = if let Some(ref vm_id) = params.id {
        let db_path = {
            let instances = state.instances.lock().unwrap();
            instances
                .get(vm_id)
                .map(|i| i.session_dir.join("session.db"))
        };
        if let Some(path) = db_path {
            session_db_triage(&path, limit).unwrap_or_else(|e| {
                tracing::warn!(target: "service", vm = %vm_id, error = %e, "session-db triage skipped");
                serde_json::json!({})
            })
        } else {
            serde_json::json!({ "missing": true, "reason": "session not found" })
        }
    } else {
        serde_json::json!({})
    };

    // Build a deterministic ranked-list of the highest-blast-radius items
    // first: panics > unhandled-enum warns > slow_op events > everything else.
    let mut rank: Vec<String> = Vec::new();
    for p in panics.iter().take(5) {
        rank.push(format!(
            "panic {} in {} at {} -- {}",
            p.ts.as_str().chars().take(19).collect::<String>(),
            p.binary,
            p.location.clone().unwrap_or_else(|| "?".into()),
            p.message.chars().take(120).collect::<String>(),
        ));
    }
    for e in errors
        .iter()
        .filter(|e| e.target.as_deref() == Some("ipc"))
        .take(3)
    {
        rank.push(format!(
            "ipc-warn {} in {} -- {}",
            e.ts.as_str().chars().take(19).collect::<String>(),
            e.binary,
            e.message.chars().take(120).collect::<String>(),
        ));
    }
    for s in slow_ops.iter().take(3) {
        rank.push(format!(
            "slow_op {} {} {}ms in {}",
            s.ts.as_str().chars().take(19).collect::<String>(),
            s.op,
            s.duration_ms,
            s.binary,
        ));
    }

    let out = serde_json::json!({
        "since": since_str,
        "session_id": params.id,
        "host": {
            "panics": panics,
            "errors": errors,
            "slow_ops": slow_ops,
        },
        "session": session_block,
        "rank": rank,
    });
    Ok(axum::Json(out))
}

/// F6: scoped session.db queries for triage. Returns the JSON object
/// embedded under `session` in the /triage response.
fn session_db_triage(db_path: &std::path::Path, limit: usize) -> anyhow::Result<serde_json::Value> {
    let reader = capsem_logger::DbReader::open(db_path)?;
    let denied_net_sql = format!(
        "SELECT timestamp, domain, decision, status_code, duration_ms \
         FROM net_events WHERE decision = 'denied' OR status_code >= 500 \
         ORDER BY timestamp DESC LIMIT {limit}"
    );
    let mcp_errors_sql = format!(
        "SELECT timestamp, server_name, method, decision, policy_mode, policy_action, \
                policy_rule, policy_reason, error_message, duration_ms \
         FROM mcp_calls WHERE decision IN ('denied','error') OR error_message IS NOT NULL \
         ORDER BY timestamp DESC LIMIT {limit}"
    );
    let exec_failures_sql = format!(
        "SELECT timestamp, exec_id, command, exit_code, duration_ms \
         FROM exec_events WHERE exit_code IS NOT NULL AND exit_code != 0 \
         ORDER BY timestamp DESC LIMIT {limit}"
    );

    let denied_net = reader
        .query_raw(&denied_net_sql)
        .unwrap_or_else(|_| "[]".into());
    let mcp_errors = reader
        .query_raw(&mcp_errors_sql)
        .unwrap_or_else(|_| "[]".into());
    let exec_failures = reader
        .query_raw(&exec_failures_sql)
        .unwrap_or_else(|_| "[]".into());

    let denied_net_v: serde_json::Value = serde_json::from_str(&denied_net).unwrap_or_default();
    let mcp_errors_v: serde_json::Value = serde_json::from_str(&mcp_errors).unwrap_or_default();
    let exec_failures_v: serde_json::Value =
        serde_json::from_str(&exec_failures).unwrap_or_default();

    Ok(serde_json::json!({
        "denied_net": denied_net_v,
        "mcp_errors": mcp_errors_v,
        "exec_failures": exec_failures_v,
    }))
}

#[derive(Deserialize, Debug, Default)]
struct TriageQuery {
    /// Lookback window. Default "30m". Accepts "5m", "1h", "24h", or
    /// RFC3339 ("2026-05-02T17:30:00Z").
    since: Option<String>,
    /// Max items per category. Default 20, capped at 200.
    limit: Option<usize>,
    /// Optional session id (reserved for the future session.db query).
    id: Option<String>,
}

/// `GET /host-logs/{name}?grep=&tail=&max_bytes=` -- read a host-side log
/// file by symbolic name. Hard-coded allowlist (no path traversal). Used
/// by the `capsem_host_logs` MCP tool (T3) but the endpoint already lands
/// in this commit so a future T3 sub-sprint can wire the MCP tool without
/// touching the service.
async fn handle_host_logs(
    State(state): State<Arc<ServiceState>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<HostLogsQuery>,
) -> Result<String, AppError> {
    let path = if name == "app" {
        triage::latest_app_log(&capsem_core::paths::capsem_home())
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, "no app log found".into()))?
    } else {
        triage::host_log_path(&state.run_dir, &name)
            .ok_or_else(|| AppError(StatusCode::BAD_REQUEST, format!("unknown log name: {name}")))?
    };
    let max_bytes = params.max_bytes.unwrap_or(100 * 1024).min(5 * 1024 * 1024);
    let text = tokio::task::spawn_blocking(move || -> Result<String, String> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        if len > max_bytes {
            file.seek(SeekFrom::End(-(max_bytes as i64)))
                .map_err(|e| e.to_string())?;
        }
        let mut buf = String::new();
        file.read_to_string(&mut buf).map_err(|e| e.to_string())?;
        if len > max_bytes {
            if let Some(pos) = buf.find('\n') {
                buf = buf[pos + 1..].to_string();
            }
        }
        Ok(buf)
    })
    .await
    .map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("log read failed: {e}"),
        )
    })?
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Apply grep + tail post-filters here so the wire surface to the
    // capsem_host_logs MCP tool can avoid two round-trips.
    let mut text = text;
    if let Some(pat) = &params.grep {
        text = text
            .lines()
            .filter(|l| l.contains(pat))
            .collect::<Vec<_>>()
            .join("\n");
    }
    if let Some(n) = params.tail {
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(n);
        text = lines[start..].join("\n");
    }
    Ok(text)
}

#[derive(Deserialize, Debug, Default)]
struct HostLogsQuery {
    grep: Option<String>,
    tail: Option<usize>,
    max_bytes: Option<u64>,
}

async fn handle_service_logs(State(state): State<Arc<ServiceState>>) -> Result<String, AppError> {
    let log_path = state.run_dir.join("service.log");

    let text = tokio::task::spawn_blocking(move || -> Result<String, String> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(&log_path).map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        // Read last 100KB
        let max = 100 * 1024u64;
        if len > max {
            file.seek(SeekFrom::End(-(max as i64)))
                .map_err(|e| e.to_string())?;
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
    })
    .await
    .map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("log read failed: {e}"),
        )
    })?
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(text)
}

#[tracing::instrument(skip_all, fields(cmd = ?std::mem::discriminant(&cmd), timeout_secs = ?timeout_secs))]
async fn send_ipc_command(
    uds_path: &std::path::Path,
    cmd: ServiceToProcess,
    timeout_secs: Option<u64>,
) -> Result<ProcessToService, String> {
    let stream = tokio::net::UnixStream::connect(uds_path)
        .await
        .map_err(|e| format!("failed to connect to sandbox: {e}"))?;
    let mut std_stream = stream
        .into_std()
        .map_err(|e| format!("failed to convert stream: {e}"))?;
    capsem_core::ipc_handshake::negotiate_initiator(
        &mut std_stream,
        "capsem-service",
        capsem_core::telemetry::current_parent_traceparent(),
    )
    .map_err(|e| format!("IPC handshake failed: {e}"))?;
    let (tx, rx): (Sender<ServiceToProcess>, Receiver<ProcessToService>) =
        channel_from_std(std_stream).map_err(|e| format!("failed to create IPC channel: {e}"))?;

    tx.send(cmd.clone())
        .await
        .map_err(|e| format!("failed to send IPC command: {e}"))?;

    let deadline =
        timeout_secs.map(|secs| tokio::time::Instant::now() + std::time::Duration::from_secs(secs));
    loop {
        let msg = match deadline {
            Some(deadline) => match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(msg)) => msg,
                Ok(Err(e)) => {
                    error!(?e, "IPC receive error");
                    return Err(format!("IPC connection closed: {e}"));
                }
                Err(_) => {
                    let secs = timeout_secs.unwrap_or_default();
                    return Err(format!("IPC command timed out after {secs}s"));
                }
            },
            None => match rx.recv().await {
                Ok(msg) => msg,
                Err(e) => {
                    error!(?e, "IPC receive error");
                    return Err(format!("IPC connection closed: {e}"));
                }
            },
        };

        match msg {
            ProcessToService::Pong => {
                if matches!(cmd, ServiceToProcess::Ping | ServiceToProcess::ReloadConfig) {
                    return Ok(ProcessToService::Pong);
                }
                continue;
            }
            ProcessToService::TerminalOutput { .. } => continue,
            ProcessToService::StateChanged { .. } => continue,
            res => return Ok(res),
        }
    }
}

/// Wait until a VM signals readiness via a `.ready` sentinel file.
/// The capsem-process creates this file once the guest handshake completes.
///
/// If `state` and `id` are provided, also checks on every poll iteration that
/// the VM is still in the instance registry. The resume_sandbox / spawn child-
/// exit handlers remove the instance when capsem-process dies; observing that
/// removal lets us fail fast (within ~50ms) instead of polling the dead
/// sentinel for the full timeout. Without this, a capsem-process that crashes
/// or exits during boot/restore would hang the API for `timeout_secs` (was
/// reproducibly 30s under heavy suspend/resume churn).
#[tracing::instrument(skip_all, fields(timeout_secs))]
async fn wait_for_vm_ready(
    uds_path: &std::path::Path,
    timeout_secs: u64,
    state: Option<&Arc<ServiceState>>,
    id: Option<&str>,
) -> Result<(), String> {
    let ready_span = tracing::debug_span!(
        target: "capsem.launch",
        capsem_core::telemetry::LAUNCH_VSOCK_READY_SPAN,
        status = tracing::field::Empty,
    );
    let ready_path = uds_path.with_extension("ready");
    // Override the PollOpts::new defaults (50ms / 500ms): VM ready-time is
    // sub-second in the common case and the sentinel check is a single stat,
    // so 500ms max_delay overshoots readiness by ~500ms and blows the
    // exec_ready / boot_ready latency gates. Peer callers (service-connect,
    // gateway-ready) wait for remote processes with seconds-scale startup
    // where 500ms is appropriate; this poll is different.
    let opts = capsem_core::poll::PollOpts {
        initial_delay: std::time::Duration::from_millis(5),
        max_delay: std::time::Duration::from_millis(50),
        ..capsem_core::poll::PollOpts::new("vm-ready", std::time::Duration::from_secs(timeout_secs))
    };
    let died: Arc<std::sync::atomic::AtomicBool> =
        Arc::new(std::sync::atomic::AtomicBool::new(false));
    let res = capsem_core::poll::poll_until(opts, || {
        let ready = ready_path.clone();
        let state = state.cloned();
        let id = id.map(|s| s.to_string());
        let died = Arc::clone(&died);
        async move {
            if ready.exists() {
                return Some(());
            }
            if let (Some(st), Some(name)) = (state.as_ref(), id.as_ref()) {
                if !st.instances.lock().unwrap().contains_key(name) {
                    died.store(true, std::sync::atomic::Ordering::Release);
                    // Returning Some short-circuits the poll loop; the
                    // outer caller distinguishes via `died`.
                    return Some(());
                }
            }
            None
        }
    })
    .instrument(ready_span.clone())
    .await;
    if died.load(std::sync::atomic::Ordering::Acquire) {
        ready_span.record("status", "error");
        return Err("capsem-process exited before signalling ready".into());
    }
    match res {
        Ok(()) => {
            ready_span.record("status", "ok");
            Ok(())
        }
        Err(error) => {
            ready_span.record("status", "error");
            Err(format!("{error}"))
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
        let i = instances
            .get(&id)
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        i.uds_path.clone()
    };

    wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&id))
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let id_val = state.next_job_id();
    let res = send_ipc_command(
        &uds_path,
        ServiceToProcess::Exec {
            id: id_val,
            command: payload.command,
        },
        payload.timeout_secs,
    )
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    match res {
        ProcessToService::ExecResult {
            stdout,
            stderr,
            exit_code,
            ..
        } => Ok(Json(ExecResponse {
            stdout: String::from_utf8(stdout)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            stderr: String::from_utf8(stderr)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            exit_code,
        })),
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response for exec".to_string(),
        )),
    }
}

async fn handle_write_file(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<WriteFileRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let uds_path = {
        let instances = state.instances.lock().unwrap();
        let i = instances
            .get(&id)
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        i.uds_path.clone()
    };

    let data = payload.content.into_bytes();
    let path = payload.path;
    log_file_boundary(
        &state,
        &id,
        FileBoundaryAction::Import,
        path.clone(),
        file_security_preview_bytes(&data),
        data.len() as u64,
        None,
    )
    .await?;

    let id_val = state.next_job_id();
    let res = send_ipc_command(
        &uds_path,
        ServiceToProcess::WriteFile {
            id: id_val,
            path,
            data,
        },
        Some(30),
    )
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    match res {
        ProcessToService::WriteFileResult { success, error, .. } => {
            if success {
                Ok(Json(json!({ "success": true })))
            } else {
                Err(AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    error.unwrap_or_else(|| "unknown write error".into()),
                ))
            }
        }
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response for write_file".to_string(),
        )),
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
        let i = instances
            .get(&id)
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        i.uds_path.clone()
    };

    wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&id))
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let id_val = state.next_job_id();
    let res = send_ipc_command(
        &uds_path,
        ServiceToProcess::ReadFile {
            id: id_val,
            path: path.clone(),
        },
        Some(30),
    )
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    match res {
        ProcessToService::ReadFileResult { data, error, .. } => {
            if let Some(d) = data {
                Ok(Json(ReadFileResponse {
                    content: String::from_utf8(d)
                        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
                }))
            } else {
                Err(AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    error.unwrap_or_else(|| "unknown read error".into()),
                ))
            }
        }
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response for read_file".to_string(),
        )),
    }
}

async fn handle_reload_config(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Collect paths to broadcast to.
    let uds_paths = {
        let instances = state.instances.lock().unwrap();
        instances
            .iter()
            .map(|(id, info)| (id.clone(), info.uds_path.clone()))
            .collect::<Vec<_>>()
    };

    let results = futures::future::join_all(uds_paths.iter().map(|(id, uds_path)| {
        let id = id.clone();
        async move {
            match send_ipc_command(uds_path, ServiceToProcess::ReloadConfig, Some(5)).await {
                Ok(ProcessToService::Pong) => None,
                Ok(_) => Some(format!("{id}: unexpected response")),
                Err(e) => Some(format!("{id}: {e}")),
            }
        }
    }))
    .await;
    let failures: Vec<String> = results.into_iter().flatten().collect();

    if failures.is_empty() {
        Ok(Json(
            serde_json::json!({ "success": true, "reloaded": uds_paths.len() }),
        ))
    } else {
        Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "failed to reload config in some instances: {}",
                failures.join(", ")
            ),
        ))
    }
}

async fn handle_profile_reload(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _profile_id = validate_profile_route_id(profile_id)?;
    handle_reload_config(State(state)).await
}

// ---------------------------------------------------------------------------
// Settings endpoints
// ---------------------------------------------------------------------------

/// GET /settings/info -- unified settings tree + issues + presets.
async fn handle_get_settings() -> Json<serde_json::Value> {
    let resp = capsem_core::net::policy_config::load_settings_response();
    Json(serde_json::to_value(resp).unwrap_or_default())
}

/// PATCH /settings/edit -- batch-update settings and return the refreshed tree.
async fn handle_save_settings(
    Json(raw): Json<HashMap<String, serde_json::Value>>,
) -> Result<Json<serde_json::Value>, AppError> {
    capsem_core::net::policy_config::batch_update_settings_json(&raw)
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
async fn handle_apply_preset(Path(id): Path<String>) -> Result<Json<serde_json::Value>, AppError> {
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

fn asset_status_value(state: &ServiceState) -> serde_json::Value {
    let reconcile = state
        .asset_reconcile
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();
    match state.resolve_asset_paths() {
        Ok(resolved) => {
            let assets = vec![
                json!({ "name": "vmlinuz", "path": resolved.kernel.display().to_string(), "status": if resolved.kernel.exists() { "present" } else { "missing" } }),
                json!({ "name": "initrd.img", "path": resolved.initrd.display().to_string(), "status": if resolved.initrd.exists() { "present" } else { "missing" } }),
                json!({ "name": resolved.rootfs.file_name().and_then(|name| name.to_str()).unwrap_or("rootfs"), "path": resolved.rootfs.display().to_string(), "status": if resolved.rootfs.exists() { "present" } else { "missing" } }),
            ];
            let all_ready = assets.iter().all(|a| a["status"] == "present");
            let mut value = json!({
                "ready": all_ready,
                "downloading": reconcile.in_progress,
                "asset_version": resolved.asset_version,
                "assets": assets,
            });
            append_asset_reconcile_status(&mut value, &reconcile);
            value
        }
        Err(e) => {
            let mut value = json!({
                "ready": false,
                "downloading": reconcile.in_progress,
                "error": e.to_string(),
                "assets": [],
            });
            append_asset_reconcile_status(&mut value, &reconcile);
            value
        }
    }
}

fn append_asset_reconcile_status(value: &mut serde_json::Value, reconcile: &AssetReconcileState) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if let Some(asset) = &reconcile.current_asset {
        obj.insert("current_asset".to_string(), json!(asset));
        obj.insert("bytes_done".to_string(), json!(reconcile.bytes_done));
        if let Some(total) = reconcile.bytes_total {
            obj.insert("bytes_total".to_string(), json!(total));
        }
    }
    if let Some(downloaded) = reconcile.last_downloaded {
        obj.insert("downloaded".to_string(), json!(downloaded));
    }
    if let Some(error) = &reconcile.last_error {
        obj.insert("reconcile_error".to_string(), json!(error));
    }
}

fn vm_asset_block_reason(state: &ServiceState) -> Option<String> {
    let resolved = match state.resolve_asset_paths() {
        Ok(resolved) => resolved,
        Err(error) => return Some(format!("VM assets are not ready: {error}")),
    };
    let mut missing = Vec::new();
    if !resolved.kernel.exists() {
        missing.push("vmlinuz".to_string());
    }
    if !resolved.initrd.exists() {
        missing.push("initrd.img".to_string());
    }
    if !resolved.rootfs.exists() {
        missing.push(
            resolved
                .rootfs
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("rootfs")
                .to_string(),
        );
    }
    if missing.is_empty() {
        return None;
    }
    let prefix = state
        .asset_reconcile
        .lock()
        .ok()
        .filter(|status| status.in_progress)
        .map(|_| "VM assets are still downloading")
        .unwrap_or("VM assets are not ready");
    Some(format!("{prefix}: missing {}", missing.join(", ")))
}

fn asset_status_path_for_run_dir(run_dir: &StdPath) -> PathBuf {
    run_dir
        .parent()
        .unwrap_or(run_dir)
        .join("asset-status.json")
}

fn load_asset_reconcile_state(path: &StdPath) -> AssetReconcileState {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return AssetReconcileState::default();
    };
    let mut status = match serde_json::from_str::<AssetReconcileState>(&contents) {
        Ok(status) => status,
        Err(error) => {
            warn!(
                path = %path.display(),
                error = %error,
                "failed to parse asset status"
            );
            return AssetReconcileState::default();
        }
    };
    status.in_progress = false;
    status.current_asset = None;
    status.bytes_done = 0;
    status.bytes_total = None;
    status
}

fn persist_asset_reconcile_state(
    path: &StdPath,
    status: &AssetReconcileState,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(status)
        .map_err(|e| format!("serialize asset status {}: {e}", path.display()))?;
    std::fs::write(&tmp, json).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))?;
    Ok(())
}

fn update_asset_reconcile_state<F>(
    state: &ServiceState,
    update: F,
) -> Result<AssetReconcileState, String>
where
    F: FnOnce(&mut AssetReconcileState),
{
    let snapshot = {
        let mut status = state
            .asset_reconcile
            .lock()
            .map_err(|e| format!("asset reconcile lock poisoned: {e}"))?;
        update(&mut status);
        status.clone()
    };
    persist_asset_reconcile_state(&state.asset_status_path, &snapshot)?;
    Ok(snapshot)
}

async fn ensure_assets_for_state(state: Arc<ServiceState>) -> Result<usize, String> {
    if state
        .asset_reconcile_inflight
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("asset reconciliation already in progress".to_string());
    }

    let result: Result<usize, String> = async {
        let Some(manifest) = state.manifest.as_ref().cloned() else {
            return Ok(0);
        };
        update_asset_reconcile_state(&state, |status| {
            *status = AssetReconcileState {
                in_progress: true,
                ..Default::default()
            };
        })?;
        let arch = capsem_core::asset_manager::host_manifest_arch();
        let downloaded = capsem_core::asset_manager::download_missing_assets(
            &manifest,
            &state.current_version,
            arch,
            &state.assets_dir,
            {
                let state = Arc::clone(&state);
                move |progress| {
                    if let Ok(mut status) = state.asset_reconcile.lock() {
                        status.in_progress = true;
                        status.current_asset = Some(progress.logical_name.clone());
                        status.bytes_done = progress.bytes_done;
                        status.bytes_total = progress.bytes_total;
                    }
                    if progress.done {
                        let snapshot = state
                            .asset_reconcile
                            .lock()
                            .map(|status| status.clone())
                            .ok();
                        if let Some(snapshot) = snapshot {
                            if let Err(error) =
                                persist_asset_reconcile_state(&state.asset_status_path, &snapshot)
                            {
                                warn!(error = %error, "failed to persist asset progress");
                            }
                        }
                        tracing::info!(
                            asset = progress.logical_name.as_str(),
                            bytes = progress.bytes_done,
                            "asset ensure progress"
                        );
                    }
                }
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(downloaded.len())
    }
    .await;

    let final_status = update_asset_reconcile_state(&state, |status| {
        status.in_progress = false;
        status.current_asset = None;
        status.bytes_done = 0;
        status.bytes_total = None;
        match &result {
            Ok(downloaded) => {
                status.last_downloaded = Some(*downloaded);
                status.last_error = None;
            }
            Err(error) => {
                status.last_downloaded = Some(0);
                status.last_error = Some(error.clone());
            }
        }
    });
    if let Err(error) = final_status {
        warn!(error = %error, "failed to persist final asset status");
    }
    state
        .asset_reconcile_inflight
        .store(false, Ordering::Release);
    result
}

/// GET /assets/status -- query VM asset readiness.
async fn handle_assets_status(State(state): State<Arc<ServiceState>>) -> Json<serde_json::Value> {
    Json(asset_status_value(&state))
}

/// POST /assets/ensure -- download missing/corrupt assets when a manifest is
/// available, then return the refreshed status shape.
async fn handle_assets_ensure(State(state): State<Arc<ServiceState>>) -> Json<serde_json::Value> {
    let ensure_result = ensure_assets_for_state(Arc::clone(&state)).await;
    let mut status = asset_status_value(&state);
    if let Some(obj) = status.as_object_mut() {
        match ensure_result {
            Ok(downloaded) => {
                obj.insert("ensured".to_string(), json!(true));
                obj.insert("downloaded".to_string(), json!(downloaded));
            }
            Err(error) => {
                obj.insert("ensured".to_string(), json!(false));
                obj.insert("downloaded".to_string(), json!(0));
                obj.insert("error".to_string(), json!(error.to_string()));
            }
        }
    }
    Json(status)
}

/// PUT /corp/edit -- apply corporate config from URL or inline TOML.
async fn handle_corp_config(
    Json(payload): Json<CorpConfigRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    use capsem_core::net::policy_config::corp_provision;

    let capsem_dir = capsem_core::paths::capsem_home_opt().ok_or(AppError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "HOME not set".into(),
    ))?;

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
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "provide either 'source' (URL) or 'toml' (inline content)".into(),
        ));
    }

    Ok(Json(json!({ "success": true })))
}

// ---------------------------------------------------------------------------
// MCP API Handlers
// ---------------------------------------------------------------------------

fn validate_profile_route_id(profile_id: String) -> Result<String, AppError> {
    if profile_id.is_empty() {
        Err(AppError(
            StatusCode::BAD_REQUEST,
            "profile id must not be empty".to_string(),
        ))
    } else {
        Ok(profile_id)
    }
}

fn resolve_mcp_tool_id(server_id: &str, tool_id: &str) -> Result<String, AppError> {
    if server_id.is_empty() || tool_id.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "server id and tool id must not be empty".to_string(),
        ));
    }
    if let Some((prefix, _)) = tool_id.split_once("__") {
        if prefix != server_id {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                format!("tool id {tool_id} does not belong to MCP server {server_id}"),
            ));
        }
        Ok(tool_id.to_string())
    } else {
        Ok(format!("{server_id}__{tool_id}"))
    }
}

/// GET /profiles/:profile_id/mcp/servers/list -- list profile MCP servers with status.
async fn handle_profile_mcp_servers(
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _profile_id = validate_profile_route_id(profile_id)?;
    use capsem_core::mcp::policy::McpUserConfig;
    use capsem_core::mcp::{build_server_list_with_builtin, load_tool_cache};

    let (user_sf, corp_sf) = capsem_core::net::policy_config::load_settings_files();
    let user_mcp = user_sf.mcp.unwrap_or_default();
    let corp_mcp = corp_sf.mcp.unwrap_or(McpUserConfig::default());

    // Include the "local" builtin server if the binary exists.
    let builtin_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("capsem-mcp-builtin")));
    let servers = build_server_list_with_builtin(
        &user_mcp,
        &corp_mcp,
        builtin_bin.as_deref(),
        std::collections::HashMap::new(),
    );
    let cache = load_tool_cache();

    let resp: Vec<api::McpServerInfoResponse> = servers
        .iter()
        .map(|s| {
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
        })
        .collect();
    Ok(Json(serde_json::to_value(resp).unwrap_or_default()))
}

/// GET /profiles/:profile_id/mcp/servers/:server_id/tools/list -- list one server's tools.
async fn handle_profile_mcp_server_tools(
    Path((profile_id, server_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _profile_id = validate_profile_route_id(profile_id)?;
    if server_id.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "MCP server id must not be empty".to_string(),
        ));
    }
    use capsem_core::mcp::load_tool_cache;

    let cache = load_tool_cache();
    let resp: Vec<api::McpToolInfoResponse> = cache
        .iter()
        .filter(|entry| entry.server_name == server_id)
        .map(|entry| {
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
        })
        .collect();
    Ok(Json(serde_json::to_value(resp).unwrap_or_default()))
}

/// POST /profiles/:profile_id/mcp/servers/:server_id/refresh -- refresh one server's tool discovery.
async fn handle_profile_mcp_server_refresh(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, server_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _profile_id = validate_profile_route_id(profile_id)?;
    if server_id.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "MCP server id must not be empty".to_string(),
        ));
    }
    // Send McpRefreshTools to all running instances.
    let uds_paths = {
        let instances = state.instances.lock().unwrap();
        instances
            .values()
            .map(|info| info.uds_path.clone())
            .collect::<Vec<_>>()
    };
    for uds_path in &uds_paths {
        let id = state.next_job_id();
        let _ =
            send_ipc_command(uds_path, ServiceToProcess::McpRefreshTools { id }, Some(30)).await;
    }
    Ok(Json(
        serde_json::json!({"success": true, "server_id": server_id, "instances": uds_paths.len()}),
    ))
}

/// PATCH /profiles/:profile_id/mcp/servers/:server_id/tools/:tool_id/edit -- edit tool mechanics.
async fn handle_profile_mcp_tool_edit(
    Path((profile_id, server_id, tool_id)): Path<(String, String, String)>,
    Json(update): Json<McpToolEditRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _profile_id = validate_profile_route_id(profile_id)?;
    let namespaced_name = resolve_mcp_tool_id(&server_id, &tool_id)?;
    use capsem_core::mcp::{load_tool_cache, save_tool_cache};

    let mut cache = load_tool_cache();
    let found = cache.iter_mut().find(|entry| {
        entry.server_name == server_id
            && (entry.namespaced_name == namespaced_name || entry.original_name == tool_id)
    });
    match found {
        Some(entry) => {
            if let Some(approved) = update.approved {
                entry.approved = approved;
            }
            save_tool_cache(&cache).map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
            Ok(Json(serde_json::json!({
                "server_id": server_id,
                "tool_id": tool_id,
                "namespaced_name": namespaced_name,
                "approved": update.approved,
            })))
        }
        None => Err(AppError(
            StatusCode::NOT_FOUND,
            format!("tool not found: {server_id}/{tool_id}"),
        )),
    }
}

/// POST /profiles/:profile_id/mcp/servers/:server_id/tools/:tool_id/call -- call a tool via a VM aggregator.
async fn handle_profile_mcp_tool_call(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, server_id, tool_id)): Path<(String, String, String)>,
    Json(arguments): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _profile_id = validate_profile_route_id(profile_id)?;
    let namespaced_name = resolve_mcp_tool_id(&server_id, &tool_id)?;
    // Find any running instance to route the call through.
    let uds_path = {
        let instances = state.instances.lock().unwrap();
        instances.values().next().map(|i| i.uds_path.clone())
    };
    let uds_path = uds_path.ok_or_else(|| {
        AppError(
            StatusCode::SERVICE_UNAVAILABLE,
            "no running sessions".into(),
        )
    })?;

    let arguments_json = serde_json::to_string(&arguments)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("invalid arguments: {e}")))?;
    let msg = ServiceToProcess::McpCallTool {
        id: state.next_job_id(),
        namespaced_name,
        arguments_json,
    };
    let resp = send_ipc_command(&uds_path, msg, Some(60))
        .await
        .map_err(|e| AppError(StatusCode::BAD_GATEWAY, e))?;

    match resp {
        ProcessToService::McpCallToolResult {
            result_json, error, ..
        } => {
            if let Some(err) = error {
                Err(AppError(StatusCode::BAD_GATEWAY, err))
            } else {
                let result = match result_json {
                    Some(s) => serde_json::from_str(&s).map_err(|e| {
                        AppError(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("bad result_json from process: {e}"),
                        )
                    })?,
                    None => serde_json::Value::Null,
                };
                Ok(Json(result))
            }
        }
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response".into(),
        )),
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
        let index = capsem_core::session::SessionIndex::open(&db_path).map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to open main.db: {e}"),
            )
        })?;
        let json_str = index.query_raw(&payload.sql, &[]).map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("query failed: {e}"),
            )
        })?;
        return Ok((
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            json_str,
        ));
    }

    let db_path = {
        let instances = state.instances.lock().unwrap();
        let i = instances
            .get(&id)
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        i.session_dir.join("session.db")
    };

    let reader = capsem_logger::DbReader::open(&db_path).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to open DB: {e}"),
        )
    })?;

    let json_str = reader.query_raw(&payload.sql).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("query failed: {e}"),
        )
    })?;

    Ok((
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        json_str,
    ))
}

/// `GET /timeline/{id}?trace_id=<X>&since=10m&limit=200&layers=mcp,exec,...`
/// -- unified time-ordered event stream for one session, joining
/// `exec_events`, `mcp_calls`, `net_events`, `fs_events`, and
/// `model_calls` via UNION ALL. Used by the `capsem_timeline` MCP tool.
///
/// W6 added `trace_id` to every layer; this handler filters with
/// `WHERE trace_id = ? OR trace_id IS NULL` so rows that pre-date W4's
/// trace propagation still surface for the user.
async fn handle_timeline(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<TimelineQuery>,
) -> Result<impl IntoResponse, AppError> {
    let db_path = {
        let instances = state.instances.lock().unwrap();
        let i = instances
            .get(&id)
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        i.session_dir.join("session.db")
    };

    let limit = params.limit.unwrap_or(200).min(2000);
    let since_filter = params
        .since
        .as_deref()
        .and_then(triage::parse_since)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    // Layers the caller wants. Default to all five. C1: filter against
    // a hard allowlist BEFORE building SQL so even a future careless
    // copy-paste of this format!() can't leak attacker-supplied
    // tokens into the query string.
    const ALLOWED_LAYERS: &[&str] = &["exec", "mcp", "net", "fs", "model"];
    let layers: Vec<&str> = params
        .layers
        .as_deref()
        .map(|s| {
            s.split(',')
                .filter(|x| !x.is_empty())
                .filter(|x| ALLOWED_LAYERS.contains(x))
                .collect()
        })
        .unwrap_or_else(|| ALLOWED_LAYERS.to_vec());

    let mut parts: Vec<String> = Vec::new();
    if layers.contains(&"exec") {
        parts.push(
            "SELECT timestamp, 'exec' AS layer, exec_id AS ref, command AS summary, \
             exit_code AS status, duration_ms, trace_id FROM exec_events"
                .to_string(),
        );
    }
    if layers.contains(&"mcp") {
        // F7: include the originating model_call's tool_calls.call_id when
        // an mcp_call serviced a model tool_use, so the timeline shows
        // "model X tool_use Y -> mcp_call Z" inline. Best-effort LEFT JOIN
        // -- mcp_calls without a tool_calls peer just show NULL.
        parts.push(
            "SELECT m.timestamp AS timestamp, 'mcp' AS layer, m.id AS ref, \
             m.server_name || '/' || COALESCE(m.tool_name, m.method) || \
                COALESCE(' (call_id=' || tc.call_id || ')', '') AS summary, \
             NULL AS status, m.duration_ms AS duration_ms, m.trace_id AS trace_id \
             FROM mcp_calls m \
             LEFT JOIN tool_calls tc ON tc.mcp_call_id = m.id"
                .to_string(),
        );
    }
    if layers.contains(&"net") {
        parts.push(
            "SELECT timestamp, 'net' AS layer, id AS ref, \
             COALESCE(method, 'GET') || ' ' || domain || COALESCE(path, '') AS summary, \
             status_code AS status, duration_ms, trace_id FROM net_events"
                .to_string(),
        );
    }
    if layers.contains(&"fs") {
        parts.push(
            "SELECT timestamp, 'fs' AS layer, id AS ref, action || ' ' || path AS summary, \
             NULL AS status, NULL AS duration_ms, trace_id FROM fs_events"
                .to_string(),
        );
    }
    if layers.contains(&"model") {
        parts.push(
            "SELECT timestamp, 'model' AS layer, id AS ref, \
             provider || '/' || COALESCE(model, '?') AS summary, \
             status_code AS status, duration_ms, trace_id FROM model_calls"
                .to_string(),
        );
    }

    if parts.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "no layers selected".into(),
        ));
    }

    let mut sql = parts.join(" UNION ALL ");
    let mut filters: Vec<String> = Vec::new();
    if let Some(t) = &params.trace_id {
        // Match the row's trace_id OR pre-W4 NULL rows. Quote/escape via
        // SQLite's standard string-literal doubling.
        let safe = t.replace('\'', "''");
        filters.push(format!("(trace_id = '{safe}' OR trace_id IS NULL)"));
    }
    if let Some(s) = since_filter {
        // RFC3339 string comparison works because timestamps share format.
        let cutoff = secs_to_rfc3339(s);
        filters.push(format!("timestamp >= '{cutoff}'"));
    }
    if !filters.is_empty() {
        sql = format!("SELECT * FROM ({sql}) WHERE {}", filters.join(" AND "));
    }
    sql.push_str(&format!(" ORDER BY timestamp ASC LIMIT {limit}"));

    let reader = capsem_logger::DbReader::open(&db_path).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to open DB: {e}"),
        )
    })?;
    let json_str = reader.query_raw(&sql).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("timeline query failed: {e}"),
        )
    })?;

    Ok((
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        json_str,
    ))
}

#[derive(Deserialize, Debug, Default)]
struct SecurityLedgerQuery {
    /// Max rows. Default 100, capped at 2000.
    limit: Option<usize>,
}

/// GET /vms/{id}/security/latest -- latest security rule ledger rows.
///
/// This is intentionally regenerated from the session DB. It returns the full
/// stored row, including the rule snapshot and normalized SecurityEvent
/// payload that matched, because active rules may have changed by the time a
/// responder investigates the event.
async fn handle_security_latest(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<SecurityLedgerQuery>,
) -> Result<Json<Vec<capsem_logger::SecurityRuleEvent>>, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    let limit = params.limit.unwrap_or(100).min(2000);

    let reader = capsem_logger::DbReader::open(&db_path).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to open DB: {e}"),
        )
    })?;
    let items = reader.recent_security_rule_events(limit).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("query failed: {e}"),
        )
    })?;

    Ok(Json(items))
}

/// GET /vms/{id}/security/status -- security rule ledger aggregates.
async fn handle_security_info(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<capsem_logger::SecurityRuleStats>, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");

    let reader = capsem_logger::DbReader::open(&db_path).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to open DB: {e}"),
        )
    })?;
    let stats = reader.security_rule_stats().map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("query failed: {e}"),
        )
    })?;

    Ok(Json(stats))
}

fn default_plugin_config(mode: SecurityPluginMode) -> SecurityPluginConfig {
    SecurityPluginConfig {
        mode,
        detection_level: DetectionLevel::Informational,
    }
}

fn plugin_catalog() -> BTreeMap<String, (&'static str, SecurityPluginConfig)> {
    BTreeMap::from([
        (
            "credential_broker".to_string(),
            (
                "captures observed credentials into brokered credential references",
                default_plugin_config(SecurityPluginMode::Rewrite),
            ),
        ),
        (
            "dummy_pre_eicar".to_string(),
            (
                "debug preprocess plugin that blocks harmless EICAR test content",
                default_plugin_config(SecurityPluginMode::Rewrite),
            ),
        ),
        (
            "dummy_post_allow".to_string(),
            (
                "debug postprocess plugin that requests allow to prove block is absolute",
                default_plugin_config(SecurityPluginMode::Allow),
            ),
        ),
    ])
}

fn profile_plugin_scope(profile_id: String) -> Result<PluginScope, AppError> {
    if profile_id.is_empty() {
        Err(AppError(
            StatusCode::BAD_REQUEST,
            "profile plugin scope id must not be empty".to_string(),
        ))
    } else {
        Ok(PluginScope {
            kind: PluginScopeKind::Profile,
            profile_id,
        })
    }
}

fn effective_plugin_policy(
    state: &ServiceState,
    profile_id: &str,
) -> BTreeMap<String, SecurityPluginConfig> {
    let mut policy: BTreeMap<_, _> = plugin_catalog()
        .into_iter()
        .map(|(id, (_, config))| (id, config))
        .collect();
    if let Some(overrides) = state
        .plugin_policy_by_profile
        .lock()
        .unwrap()
        .get(profile_id)
    {
        for (id, config) in overrides {
            policy.insert(id.clone(), *config);
        }
    }
    policy
}

fn plugin_info_for(
    state: &ServiceState,
    plugin_id: &str,
    scope: PluginScope,
) -> Result<PluginInfo, AppError> {
    let catalog = plugin_catalog();
    let Some((description, default_config)) = catalog.get(plugin_id).copied() else {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            format!("unknown plugin: {plugin_id}"),
        ));
    };
    let effective = effective_plugin_policy(state, &scope.profile_id);
    let config = effective.get(plugin_id).copied().unwrap_or(default_config);
    let overridden = state
        .plugin_policy_by_profile
        .lock()
        .unwrap()
        .get(&scope.profile_id)
        .is_some_and(|policy| policy.contains_key(plugin_id));
    Ok(PluginInfo {
        id: plugin_id.to_string(),
        config,
        default_config,
        overridden,
        scope,
        description,
    })
}

async fn handle_profile_plugins(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<PluginListResponse>, AppError> {
    list_plugins_for_scope(&state, profile_plugin_scope(profile_id)?)
}

fn list_plugins_for_scope(
    state: &Arc<ServiceState>,
    scope: PluginScope,
) -> Result<Json<PluginListResponse>, AppError> {
    let mut plugins = Vec::new();
    for plugin_id in plugin_catalog().keys() {
        plugins.push(plugin_info_for(&state, plugin_id, scope.clone())?);
    }
    Ok(Json(PluginListResponse { scope, plugins }))
}

async fn handle_profile_plugin_info(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, plugin_id)): Path<(String, String)>,
) -> Result<Json<PluginInfo>, AppError> {
    Ok(Json(plugin_info_for(
        &state,
        &plugin_id,
        profile_plugin_scope(profile_id)?,
    )?))
}

async fn handle_profile_plugin_update(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, plugin_id)): Path<(String, String)>,
    Json(update): Json<PluginUpdate>,
) -> Result<Json<PluginInfo>, AppError> {
    update_plugin_for_scope(&state, plugin_id, profile_plugin_scope(profile_id)?, update)
}

fn update_plugin_for_scope(
    state: &Arc<ServiceState>,
    plugin_id: String,
    scope: PluginScope,
    update: PluginUpdate,
) -> Result<Json<PluginInfo>, AppError> {
    if !plugin_catalog().contains_key(&plugin_id) {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            format!("unknown plugin: {plugin_id}"),
        ));
    }
    let mut config = effective_plugin_policy(state, &scope.profile_id)
        .get(&plugin_id)
        .copied()
        .unwrap_or_else(|| default_plugin_config(SecurityPluginMode::Allow));
    if let Some(mode) = update.mode {
        config.mode = mode;
    }
    if let Some(detection_level) = update.detection_level {
        config.detection_level = detection_level;
    }
    state
        .plugin_policy_by_profile
        .lock()
        .unwrap()
        .entry(scope.profile_id.clone())
        .or_default()
        .insert(plugin_id.clone(), config);
    Ok(Json(plugin_info_for(&state, &plugin_id, scope)?))
}

#[derive(Debug, Default)]
struct ServiceEvaluateEmitter;

impl SecurityEventEmitter for ServiceEvaluateEmitter {
    fn emit(&self, _event: SecurityEvent) -> Result<(), SecurityEmitError> {
        Ok(())
    }
}

async fn handle_enforcement_evaluate(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
    Json(request): Json<EnforcementEvaluateRequest>,
) -> Result<Json<EnforcementEvaluateResponse>, AppError> {
    let profile_id = validate_profile_route_id(profile_id)?;
    let profile = SecurityRuleProfile::parse_toml(&request.rules_toml).map_err(|error| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("invalid enforcement rules: {error}"),
        )
    })?;
    let rules =
        SecurityRuleProfile::compile(&profile, SecurityRuleSource::User).map_err(|error| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("invalid enforcement rules: {error}"),
            )
        })?;
    let rule_set = SecurityRuleSet::new(rules);
    let event = request.event.into_security_event()?;
    let policy = effective_plugin_policy(&state, &profile_id);
    let engine = SecurityEventEngine::new(
        SecurityActionRegistry::with_builtin_actions().with_plugin_policy(policy),
        Arc::new(ServiceEvaluateEmitter),
    );
    let event = engine
        .apply_matching_rules_and_emit(&rule_set, event)
        .map_err(|error| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("enforcement evaluation failed: {error}"),
            )
        })?;
    Ok(Json(EnforcementEvaluateResponse {
        event: event.serializable(),
    }))
}

async fn handle_enforcement_rule_upsert(
    Path((profile_id, rule_id)): Path<(String, String)>,
    Json(rule): Json<SecurityRule>,
) -> Result<Json<EnforcementRuleResponse>, AppError> {
    let _profile_id = validate_profile_route_id(profile_id)?;
    if rule.corp_locked {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "enforcement rule endpoint writes user profile rules only; corp_locked rules must come from corp config"
                .to_string(),
        ));
    }
    let compiled = validate_single_user_profile_rule(&rule_id, &rule)?;
    let (path, mut settings) = load_user_settings_for_enforcement_write()?;
    settings
        .profiles
        .rules
        .insert(rule_id.clone(), rule.clone());
    validate_user_profile_rules(&settings)?;
    capsem_core::net::policy_config::write_settings_file(&path, &settings).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to write enforcement rule: {error}"),
        )
    })?;
    Ok(Json(EnforcementRuleResponse {
        rule_id,
        compiled_rule_id: compiled.rule_id,
        rule,
    }))
}

async fn handle_enforcement_rule_delete(
    Path((profile_id, rule_id)): Path<(String, String)>,
) -> Result<Json<EnforcementRuleDeleteResponse>, AppError> {
    let _profile_id = validate_profile_route_id(profile_id)?;
    let (path, mut settings) = load_user_settings_for_enforcement_write()?;
    if settings.profiles.rules.remove(&rule_id).is_none() {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            format!("enforcement rule not found: {rule_id}"),
        ));
    }
    validate_user_profile_rules(&settings)?;
    capsem_core::net::policy_config::write_settings_file(&path, &settings).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to delete enforcement rule: {error}"),
        )
    })?;
    Ok(Json(EnforcementRuleDeleteResponse {
        rule_id,
        deleted: true,
    }))
}

async fn handle_enforcement_reload(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _profile_id = validate_profile_route_id(profile_id)?;
    handle_reload_config(State(state)).await
}

fn load_user_settings_for_enforcement_write() -> Result<(PathBuf, SettingsFile), AppError> {
    let path = capsem_core::net::policy_config::user_config_path().ok_or_else(|| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "HOME not set; cannot resolve user settings path".to_string(),
        )
    })?;
    let settings = capsem_core::net::policy_config::load_settings_file(&path).map_err(|error| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("failed to load user settings: {error}"),
        )
    })?;
    Ok((path, settings))
}

fn validate_single_user_profile_rule(
    rule_id: &str,
    rule: &SecurityRule,
) -> Result<capsem_core::net::policy_config::CompiledSecurityRule, AppError> {
    let profile = SecurityRuleProfile {
        profiles: SecurityRuleGroup {
            rules: BTreeMap::from([(rule_id.to_string(), rule.clone())]),
            defaults: BTreeMap::new(),
        },
        ..SecurityRuleProfile::default()
    };
    let mut compiled = profile.compile(SecurityRuleSource::User).map_err(|error| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("invalid enforcement rule: {error}"),
        )
    })?;
    compiled.pop().ok_or_else(|| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "valid enforcement rule did not compile".to_string(),
        )
    })
}

fn validate_user_profile_rules(settings: &SettingsFile) -> Result<(), AppError> {
    SecurityRuleProfile {
        profiles: settings.profiles.clone(),
        ..SecurityRuleProfile::default()
    }
    .compile(SecurityRuleSource::User)
    .map_err(|error| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("invalid user profile enforcement rules: {error}"),
        )
    })?;
    Ok(())
}

impl EnforcementEventInput {
    fn into_security_event(self) -> Result<SecurityEvent, AppError> {
        match self.event_type.as_str() {
            "file.import" => Ok(SecurityEvent::new(RuntimeSecurityEventType::FileImport)
                .with_file(FileSecurityEvent {
                    import_content: self.file_import_content,
                    ..Default::default()
                })),
            "http.request" => Ok(SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
                .with_http(capsem_core::security_engine::HttpSecurityEvent {
                    host: self.http_host,
                    ..Default::default()
                })),
            other => Err(AppError(
                StatusCode::BAD_REQUEST,
                format!("unsupported enforcement event_type: {other}"),
            )),
        }
    }
}

#[derive(Deserialize, Debug, Default)]
struct TimelineQuery {
    /// Filter to one trace_id. Rows with NULL trace_id are also returned
    /// (they pre-date W4's trace propagation).
    trace_id: Option<String>,
    /// Lookback window. "30m", "1h", "24h", "7d", "300s", or RFC3339.
    since: Option<String>,
    /// Max rows. Default 200, capped at 2000.
    limit: Option<usize>,
    /// Comma-separated subset of layers to include. Default all:
    /// "exec,mcp,net,fs,model".
    layers: Option<String>,
}

fn secs_to_rfc3339(secs: u64) -> String {
    // Pure-stdlib RFC3339 (UTC, second precision). Mirrors the helper in
    // the support_bundle crate; we pay the duplication tax to keep
    // capsem-service free of `chrono`.
    let secs = secs as i64;
    let days = secs.div_euclid(86400);
    let secs_in_day = secs.rem_euclid(86400);
    let hh = (secs_in_day / 3600) as u32;
    let mm = ((secs_in_day % 3600) / 60) as u32;
    let ss = (secs_in_day % 60) as u32;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
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
    Err(AppError(
        StatusCode::NOT_FOUND,
        format!("sandbox not found: {id}"),
    ))
}

/// GET /history/{id} -- unified command history (exec + audit events).
async fn handle_history(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<api::HistoryQuery>,
) -> Result<Json<api::HistoryResponse>, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");

    let reader = capsem_logger::DbReader::open(&db_path).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to open DB: {e}"),
        )
    })?;

    let (commands, total) = reader
        .history(
            params.limit,
            params.offset,
            params.search.as_deref(),
            &params.layer,
        )
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("query failed: {e}"),
            )
        })?;

    let has_more = (params.offset + commands.len()) < total as usize;
    Ok(Json(api::HistoryResponse {
        commands,
        total,
        has_more,
    }))
}

/// GET /history/{id}/processes -- process-centric view of audit events.
async fn handle_history_processes(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::HistoryProcessesResponse>, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");

    let reader = capsem_logger::DbReader::open(&db_path).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to open DB: {e}"),
        )
    })?;

    let processes = reader.history_processes(100).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("query failed: {e}"),
        )
    })?;

    Ok(Json(api::HistoryProcessesResponse { processes }))
}

/// GET /history/{id}/counts -- exec and audit event counts.
async fn handle_history_counts(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::HistoryCountsResponse>, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");

    let reader = capsem_logger::DbReader::open(&db_path).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to open DB: {e}"),
        )
    })?;

    let counts = reader.history_counts().map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("query failed: {e}"),
        )
    })?;

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

    let output = std::fs::read(&pty_log_path).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read pty.log: {e}"),
        )
    })?;

    let encoded = base64::engine::general_purpose::STANDARD.encode(&output);
    Ok(Json(api::TranscriptResponse {
        bytes: output.len(),
        content: encoded,
    }))
}

/// Acquire the host-wide VZ save/restore flock (`startup::VzHostLock`)
/// from an async context. The underlying `flock(2)` syscall is blocking
/// and can wait on a sibling service; wrap in `spawn_blocking` so we
/// don't stall a tokio worker.
///
/// Default wait budget is 60s -- the longest single suspend under `-n 4`
/// test load observed is ~15s, so 60s absorbs the typical p99. Returning
/// 503 on timeout tells the caller "try again" instead of blocking
/// indefinitely.
async fn acquire_vz_host_lock() -> Result<startup::VzHostLock, AppError> {
    let result = tokio::task::spawn_blocking(|| {
        startup::VzHostLock::acquire(std::time::Duration::from_secs(60))
    })
    .await
    .map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("vz host lock task panicked: {e}"),
        )
    })?;
    match result {
        Ok(Some(guard)) => Ok(guard),
        Ok(None) => Err(AppError(
            StatusCode::SERVICE_UNAVAILABLE,
            "another process holds the Apple VZ save/restore lock; retry shortly".into(),
        )),
        Err(e) => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("vz host lock acquire failed: {e:#}"),
        )),
    }
}

/// Wait for a process to exit, force-killing after timeout.
async fn wait_for_process_exit(pid: u32, timeout: std::time::Duration) {
    if pid == 0 {
        return;
    }
    let pid_i32 = pid as i32;
    let exited = || async move { (unsafe { nix::libc::kill(pid_i32, 0) } != 0).then_some(()) };
    if poll_until(PollOpts::new("vm-process-exit", timeout), exited)
        .await
        .is_ok()
    {
        return;
    }
    tracing::warn!(
        pid,
        "VM process did not exit within timeout, sending SIGKILL"
    );
    let _ = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid_i32),
        nix::sys::signal::Signal::SIGKILL,
    );
    if poll_until(
        PollOpts::new("vm-process-sigkill", std::time::Duration::from_secs(2)),
        exited,
    )
    .await
    .is_err()
    {
        tracing::error!(pid, "VM process survived SIGKILL");
    }
}

/// Shutdown a running VM process by ID. Returns (session_dir, persistent, pid).
///
/// When `graceful` is true: sends `ServiceToProcess::Shutdown` via IPC so
/// the guest agent can `sync()` and bash can run traps / save history, then
/// waits up to 5s for natural exit. The in-process 2.5s self-timer in
/// capsem-process (capsem-process/src/vsock.rs, ServiceToProcess::Shutdown
/// branch) sets the floor at ~2.5s. Required for `handle_stop` on
/// persistent VMs (preserves workspace state) and `handle_run` (session DB
/// rollup reads main.db after exit).
///
/// When `graceful` is false: skips the IPC and sends SIGTERM directly to
/// capsem-process. Its SIGTERM handler (capsem-process/src/main.rs, added
/// in 9b14618) calls `CFRunLoopStop` so the process exits as soon as the
/// main runloop returns -- typically well under 500ms. VZ tears down the
/// VM when capsem-process exits, which kills the agent and bash without
/// grace. Use for `delete` / `purge`: the workspace is about to be removed,
/// so guest `sync()` and bash history are irrelevant. Polls up to 1s and
/// escalates to SIGKILL on miss.
///
/// Either way, UDS socket / `.ready` files are removed inline and the
/// instance is removed from the registry before return. The leak detector
/// and suspend/resume both rely on "process is gone when this returns".
async fn shutdown_vm_process(
    state: &ServiceState,
    id: &str,
    graceful: bool,
) -> Option<(PathBuf, bool, u32)> {
    // Serialize VM teardown across the service. Concurrent deletes under
    // load starve each other: VZ guest teardown + DbWriter WAL checkpoint +
    // socket cleanup all compete, and a single shutdown can exceed the 1s
    // fast-path exit budget, which SIGKILLs capsem-process mid-checkpoint
    // and leaves a non-empty session.db-wal on disk (see
    // tests/capsem-session-lifecycle/test_wal_cleanup.py).
    // See docs/src/content/docs/gotchas/serialized-vm-shutdown.md.
    let _shutdown_guard = state.shutdown_lock.lock().await;

    let (uds_path, session_dir, pid, persistent) = {
        let instances = state.instances.lock().unwrap();
        let i = instances.get(id)?;
        (
            i.uds_path.clone(),
            i.session_dir.clone(),
            i.pid,
            i.persistent,
        )
    };

    if graceful {
        // Send shutdown command via IPC (or SIGTERM as fallback).
        let stream_res = tokio::net::UnixStream::connect(&uds_path).await;
        if let Ok(stream) = stream_res {
            if let Ok(mut std_stream) = stream.into_std() {
                if capsem_core::ipc_handshake::negotiate_initiator(
                    &mut std_stream,
                    "capsem-service",
                    capsem_core::telemetry::current_parent_traceparent(),
                )
                .is_ok()
                {
                    if let Ok((tx, _)) =
                        channel_from_std::<ServiceToProcess, ProcessToService>(std_stream)
                    {
                        capsem_core::try_send!(
                            "ipc_graceful_shutdown",
                            tx.send(ServiceToProcess::Shutdown).await
                        );
                    }
                }
            }
        } else if pid > 0 {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
    } else if pid > 0 {
        // Fast path: SIGTERM capsem-process directly. CFRunLoopStop fires
        // before the guest's SHUTDOWN_GRACE_SECS sleep or the 2.5s in-process
        // self-timer would, so delete/purge don't pay for bash's graceful
        // exit when the VM is about to be destroyed anyway.
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGTERM,
        );
    }

    // Remove from active instances immediately so the service considers this
    // VM gone. The child-exit handler at spawn time may also call remove
    // (idempotent).
    tracing::debug!(id, "shutdown_vm_process removing instance");
    state.instances.lock().unwrap().remove(id);

    // Wait for actual exit (poll_until + SIGKILL fallback), then clean up
    // sockets. Synchronous: callers must not see "shutdown returned" while
    // the process is still alive (leak detector + suspend/resume rely on it).
    let exit_timeout = if graceful {
        std::time::Duration::from_secs(5)
    } else {
        std::time::Duration::from_secs(1)
    };
    wait_for_process_exit(pid, exit_timeout).await;
    let _ = std::fs::remove_file(&uds_path);
    let _ = std::fs::remove_file(uds_path.with_extension("ready"));

    Some((session_dir, persistent, pid))
}

#[tracing::instrument(skip_all, fields(vm_id = %id))]
async fn handle_suspend(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Apple VZ corrupts the VirtioFS-backed overlay of a sibling VM if two
    // save_state / restore_state calls overlap. Serialize across all VMs
    // managed by this service. Held for the whole handler; released when
    // the child has exited and the checkpoint is durable.
    let _vz_guard = state.save_restore_lock.lock().await;
    // Plus a host-wide flock so serialization survives pytest-xdist's
    // per-worker `capsem-service` processes. See `VzHostLock`.
    let _vz_host_guard = acquire_vz_host_lock().await?;

    let (uds_path, pid) = {
        let mut instances = state.instances.lock().unwrap();
        let i = instances
            .get_mut(&id)
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        if !i.persistent {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                "ephemeral VMs cannot be suspended (persist first)".into(),
            ));
        }
        (i.uds_path.clone(), i.pid)
    };

    let stream = tokio::net::UnixStream::connect(&uds_path)
        .await
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to connect to VM IPC: {e}"),
            )
        })?;
    let mut std_stream = stream.into_std().map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to convert stream: {e}"),
        )
    })?;
    capsem_core::ipc_handshake::negotiate_initiator(
        &mut std_stream,
        "capsem-service",
        capsem_core::telemetry::current_parent_traceparent(),
    )
    .map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("IPC handshake failed: {e}"),
        )
    })?;
    let (tx, rx) =
        channel_from_std::<ServiceToProcess, ProcessToService>(std_stream).map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to create IPC channel: {e}"),
            )
        })?;

    let checkpoint_path = "checkpoint.vzsave".to_string();
    tx.send(ServiceToProcess::Suspend { checkpoint_path })
        .await
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to send suspend command: {e}"),
            )
        })?;

    // Wait for process exit (channel closed). The process sends StateChanged {"Suspended"}
    // right before exiting. We must wait for full exit to avoid a race condition where
    // a subsequent resume request fails with permission denied because the old process
    // hasn't released the checkpoint file yet.
    let mut suspended = false;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while let Ok(msg) = rx.recv().await {
            if let ProcessToService::StateChanged { state, .. } = msg {
                if state == "Suspended" {
                    suspended = true;
                }
            }
        }
    })
    .await;

    if !suspended {
        // The guest never acknowledged suspend. Leaving the process alive
        // would leak a wedged Apple VZ instance (seen in the wild: 945
        // orphan temp dirs accumulated over one test run). SIGKILL the
        // child, reclaim the instance slot, and surface the error.
        if pid > 0 {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
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
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
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
    // shutdown_vm_process now waits for actual process exit and cleans the
    // socket inline -- when it returns, resume can immediately reuse the
    // path without a SO_REUSEADDR-style race. Graceful so persistent VMs
    // get bash history + filesystem sync before teardown.
    if let Some((session_dir, persistent, _pid)) = shutdown_vm_process(&state, &id, true).await {
        if !persistent {
            let dir = session_dir;
            tokio::task::spawn_blocking(move || {
                let _ = std::fs::remove_dir_all(&dir);
            });
        }
        Ok(Json(json!({ "success": true, "persistent": persistent })))
    } else {
        Err(AppError(
            StatusCode::NOT_FOUND,
            format!("sandbox not found: {id}"),
        ))
    }
}

async fn handle_delete(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Delete fast-paths through SIGTERM + 1s poll: session dir is about
    // to be removed, guest sync() and bash history don't matter.
    let session_dir =
        if let Some((session_dir, _, _pid)) = shutdown_vm_process(&state, &id, false).await {
            session_dir
        } else {
            // Not running -- check persistent registry for stopped VM
            let registry = state.persistent_registry.lock().unwrap();
            if let Some(entry) = registry.get(&id) {
                entry.session_dir.clone()
            } else {
                return Err(AppError(
                    StatusCode::NOT_FOUND,
                    format!("sandbox not found: {id}"),
                ));
            }
        };

    // Unregister from persistent registry if applicable
    {
        let mut registry = state.persistent_registry.lock().unwrap();
        if registry.contains(&id) {
            let _ = registry.unregister(&id);
        }
    }

    // Preserve the session dir under sessions/<id>-failed-<rand>/ instead
    // of unlinking it outright. preserve_failed_session_dir renames + culls
    // down to MAX_FAILED_SESSIONS so disk stays bounded, but each delete
    // still leaves a fresh process.log / serial.log / session.db window for
    // post-mortem (e.g. when a Python-side test assertion fails after
    // /exec but before the test's `finally: delete()` -- the existing
    // failure-path preservation only fires on host-side error routes,
    // never on a clean DELETE, so without this the only artifact left is
    // service.log, which doesn't show what the per-VM process or guest
    // were doing). The cull keeps the most recent N around.
    let state_clone = Arc::clone(&state);
    let id_clone = id.clone();
    tokio::task::spawn_blocking(move || {
        state_clone.preserve_failed_session_dir(&session_dir, &id_clone);
    });

    Ok(Json(json!({ "success": true })))
}

async fn handle_resume(
    State(state): State<Arc<ServiceState>>,
    Path(name): Path<String>,
) -> Result<Json<ProvisionResponse>, AppError> {
    // See handle_suspend: same lock, same reason. Restore happens in the
    // freshly spawned capsem-process's boot, so the lock must bridge the
    // spawn and the readiness sentinel for a sibling save_state not to
    // overlap with the restoreMachineStateFromURL call.
    let _vz_guard = state.save_restore_lock.lock().await;
    let _vz_host_guard = acquire_vz_host_lock().await?;

    let attempted_checkpoint = state.has_existing_resume_checkpoint(&name);

    match state.resume_sandbox(&name, None, None) {
        Ok(id) => {
            let uds_path = state.instance_socket_path(&id);
            if let Err(e) = wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&id)).await {
                error!(name, "resume ready-wait failed: {e}");
                if attempted_checkpoint {
                    warn!(
                        name,
                        "warm restore failed; archiving checkpoint and retrying as a cold persistent boot"
                    );
                    state.archive_failed_restore_checkpoint(&id);

                    match state.resume_sandbox(&name, None, None) {
                        Ok(cold_id) => {
                            let cold_uds_path = state.instance_socket_path(&cold_id);
                            if let Err(cold_e) =
                                wait_for_vm_ready(&cold_uds_path, 30, Some(&state), Some(&cold_id))
                                    .await
                            {
                                error!(
                                    name,
                                    "cold resume fallback failed after warm restore failure: {cold_e}"
                                );
                                return Err(AppError(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    format!(
                                        "resume failed: warm restore failed ({e}); cold fallback failed ({cold_e})"
                                    ),
                                ));
                            }
                            state.clear_resume_checkpoint(&cold_id);
                            return Ok(Json(ProvisionResponse {
                                id: cold_id,
                                uds_path: Some(cold_uds_path),
                            }));
                        }
                        Err(cold_e) => {
                            error!(
                                name,
                                "cold resume fallback spawn failed after warm restore failure: {cold_e}"
                            );
                            return Err(AppError(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!(
                                    "resume failed: warm restore failed ({e}); cold fallback failed ({cold_e})"
                                ),
                            ));
                        }
                    }
                }
                return Err(AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("resume failed: {e}"),
                ));
            }
            state.clear_resume_checkpoint(&id);
            Ok(Json(ProvisionResponse {
                id,
                uds_path: Some(uds_path),
            }))
        }
        Err(e) => {
            error!(name, "resume failed: {e}");
            Err(AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("resume failed: {e}"),
            ))
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
            return Err(AppError(
                StatusCode::CONFLICT,
                format!("persistent VM \"{}\" already exists", name),
            ));
        }
    }

    // Find the running ephemeral instance
    let (old_session_dir, ram_mb, cpus, base_version, forked_from, env) = {
        let instances = state.instances.lock().unwrap();
        let i = instances
            .get(&id)
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?;
        if i.persistent {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                format!("VM \"{}\" is already persistent", id),
            ));
        }
        (
            i.session_dir.clone(),
            i.ram_mb,
            i.cpus,
            i.base_version.clone(),
            i.forked_from.clone(),
            i.env.clone(),
        )
    };

    // Move session dir to persistent location
    let new_session_dir = state.run_dir.join("persistent").join(name);
    let _ = std::fs::create_dir_all(state.run_dir.join("persistent"));
    std::fs::rename(&old_session_dir, &new_session_dir).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to move session dir: {e}"),
        )
    })?;

    // Register in persistent registry
    {
        let mut registry = state.persistent_registry.lock().unwrap();
        registry
            .register(PersistentVmEntry {
                name: name.clone(),
                ram_mb,
                cpus,
                base_version: base_version.clone(),
                created_at: format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                ),
                session_dir: new_session_dir.clone(),
                forked_from: forked_from.clone(),
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: env.clone(),
            })
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    // Update instance info in-place
    {
        let mut instances = state.instances.lock().unwrap();
        if let Some(info) = instances.remove(&id) {
            instances.insert(
                name.clone(),
                InstanceInfo {
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
                },
            );
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
        instances
            .values()
            .filter(|i| !i.persistent || payload.all)
            .map(|i| (i.id.clone(), i.persistent))
            .collect()
    };

    let results = futures::future::join_all(to_purge.iter().map(|(id, persistent)| {
        let state_ref = &state;
        let id = id.clone();
        let persistent = *persistent;
        async move {
            // Purge fast-paths for the same reason as delete: every VM
            // here is being destroyed, so the 2.5s graceful floor is pure
            // waste per VM. join_all still runs them concurrently.
            if let Some((session_dir, _, _pid)) = shutdown_vm_process(state_ref, &id, false).await {
                Some((id, session_dir, persistent))
            } else {
                None
            }
        }
    }))
    .await;

    for item in results.into_iter().flatten() {
        let (id, session_dir, persistent) = item;
        if persistent {
            let mut registry = state.persistent_registry.lock().unwrap();
            let _ = registry.unregister(&id);
        }
        let dir = session_dir;
        tokio::task::spawn_blocking(move || {
            let _ = std::fs::remove_dir_all(&dir);
        });
        if persistent {
            persistent_purged += 1;
        } else {
            ephemeral_purged += 1;
        }
    }

    // If --all, also purge stopped persistent VMs
    if payload.all {
        let stopped_names: Vec<String> = {
            let registry = state.persistent_registry.lock().unwrap();
            let instances = state.instances.lock().unwrap();
            registry
                .list()
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
                tokio::task::spawn_blocking(move || {
                    let _ = std::fs::remove_dir_all(&dir);
                });
            }
            let mut registry = state.persistent_registry.lock().unwrap();
            let _ = registry.unregister(name);
            persistent_purged += 1;
        }
    }

    let purged = ephemeral_purged + persistent_purged;
    Ok(Json(PurgeResponse {
        purged,
        persistent_purged,
        ephemeral_purged,
    }))
}

/// One-shot exec: provision a temp VM, run a command, return output, destroy VM.
async fn handle_run(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<RunRequest>,
) -> Result<Json<ExecResponse>, AppError> {
    if let Some(reason) = vm_asset_block_reason(&state) {
        return Err(AppError(StatusCode::PRECONDITION_FAILED, reason));
    }

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
    let cpus = payload
        .cpus
        .unwrap_or_else(|| vm_settings.cpu_count.unwrap_or(4));

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
    .map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("provision task: {e}"),
        )
    })?;
    provision_result.map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("provision failed: {e}"),
        )
    })?;

    // 2. Register session in main.db
    let sessions_db_dir = state
        .run_dir
        .parent()
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
    if let Err(e) = wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&id)).await {
        // Wait for the child to actually exit before renaming. Rename on
        // an open-for-write dir is safe (fds survive) but any path-based
        // reopens the child might do during shutdown (log rotation, db
        // reopen) would ENOENT -- so we let it finish flushing first.
        // shutdown_vm_process now blocks until exit (5s budget, SIGKILL
        // fallback) and cleans the UDS socket inline. Graceful because
        // preserve_failed_session_dir inspects session logs that capsem-process
        // is still flushing.
        let _ = shutdown_vm_process(&state, &id, true).await;
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
        ServiceToProcess::Exec {
            id: job_id,
            command: payload.command,
        },
        payload.timeout_secs,
    )
    .await;

    // 5. Tear down VM process and build response. shutdown_vm_process
    // blocks until the process is actually gone -- the leak detector
    // (and downstream session-DB reads) need that guarantee. Graceful so
    // the DbWriter has a chance to flush before we read session.db at step 6.
    let _ = shutdown_vm_process(&state, &id, true).await;

    let response = match exec_result {
        Ok(ProcessToService::ExecResult {
            stdout,
            stderr,
            exit_code,
            ..
        }) => Ok(Json(ExecResponse {
            stdout: String::from_utf8(stdout)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            stderr: String::from_utf8(stderr)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            exit_code,
        })),
        Ok(_) => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response".into(),
        )),
        Err(e) => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("exec failed: {e}"),
        )),
    };

    // 6. Roll up session counters before returning, so callers see consistent
    //    data in main.db. shutdown_vm_process above already awaited exit, so
    //    the DbWriter has flushed.
    if let Some(idx) = index {
        let session_db_path = session_dir.join("session.db");
        if session_db_path.exists() {
            if let Ok(reader) = capsem_logger::DbReader::open(&session_db_path) {
                if let Ok(counts) = reader.net_event_counts() {
                    let _ = idx.update_request_counts(
                        &id,
                        counts.total as u64,
                        counts.allowed as u64,
                        counts.denied as u64,
                    );
                }
                let file_events = reader.file_event_count().unwrap_or(0);
                let mcp_calls = reader.mcp_call_stats().map(|s| s.total).unwrap_or(0);
                let _ = idx.update_session_summary(&id, 0, 0, 0.0, 0, mcp_calls, file_events);
            }
        }
        let _ = idx.update_status(&id, "stopped", Some(&capsem_core::session::now_iso()));
    }

    response
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let mut run_dir = capsem_core::paths::capsem_run_dir();
    let _ = std::fs::create_dir_all(&run_dir);
    if let Ok(resolved) = run_dir.canonicalize() {
        run_dir = resolved;
    }

    let _telemetry_guard = capsem_core::telemetry::init(capsem_core::telemetry::TelemetryConfig {
        service: "capsem-service",
        sink: capsem_core::telemetry::LogSink::File {
            path: run_dir.join("service.log"),
        },
        default_filter: "info",
    })?;
    let service_launch_span = tracing::info_span!(
        target: "capsem.launch",
        capsem_core::telemetry::LAUNCH_SERVICE_SPAN,
        status = tracing::field::Empty,
    );

    service_launch_span.in_scope(|| info!("capsem-service starting up"));
    info!(args = ?args, run_dir = %run_dir.display(), "environment initialized");

    // Optional parent-watch. Symmetric with the companion (tray/gateway)
    // reaper: if the test harness that spawned us dies abruptly, bail
    // rather than linger. Only armed when --parent-pid is passed.
    if let Some(ppid) = args.parent_pid {
        match capsem_guard::watch_parent_or_exit(Some(ppid)) {
            Ok(()) => {}
            Err(e) => {
                info!(parent_pid = ppid, "parent watch not armed: {e}; exiting 0");
                return Ok(());
            }
        }
    }

    let instances_dir = run_dir.join("instances");
    let sessions_dir = run_dir.join("sessions");
    let persistent_dir = run_dir.join("persistent");
    let _ = std::fs::create_dir_all(&instances_dir);
    let _ = std::fs::create_dir_all(&sessions_dir);
    let _ = std::fs::create_dir_all(&persistent_dir);

    let service_sock = args
        .uds_path
        .unwrap_or_else(|| run_dir.join("service.sock"));

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

    let process_binary = args
        .process_binary
        .unwrap_or_else(|| PathBuf::from("target/debug/capsem-process"));
    let assets_base_dir = args
        .assets_dir
        .unwrap_or_else(|| run_dir.parent().unwrap().join("assets"));

    // Load v2 manifest if available. In dev mode (no manifest or v1), use None.
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let manifest_path = if assets_base_dir.join("manifest.json").exists() {
        Some(assets_base_dir.join("manifest.json"))
    } else if assets_base_dir
        .parent()
        .unwrap()
        .join("manifest.json")
        .exists()
    {
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
    info!(
        persistent_vms = persistent_registry.data.vms.len(),
        "loaded persistent VM registry"
    );

    let magika_session = magika::Session::builder()
        .with_inter_threads(1)
        .with_intra_threads(1)
        .build()
        .expect("failed to init magika file-type detection");

    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    let asset_reconcile = load_asset_reconcile_state(&asset_status_path);
    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(persistent_registry),
        process_binary: process_binary.clone(),
        assets_dir: assets_base_dir,
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        manifest,
        current_version,
        asset_reconcile: Mutex::new(asset_reconcile),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: Mutex::new(magika_session),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    });

    {
        let state_for_assets = Arc::clone(&state);
        tokio::spawn(async move {
            match ensure_assets_for_state(Arc::clone(&state_for_assets)).await {
                Ok(downloaded) => {
                    info!(downloaded, "startup asset reconciliation finished");
                }
                Err(error) => {
                    warn!(error = %error, "startup asset reconciliation failed");
                }
            }
        });
    }

    // Reap capsem-process orphans from any prior service run sharing this
    // run_dir. A previous service that crashed (SIGKILL) or was killed by
    // tests left its per-VM processes alive; they still reference our
    // run_dir via --session-dir and will never die on their own. Do this
    // BEFORE stale-socket removal so the orphans get a chance to clean up
    // their own sockets on SIGTERM.
    reap_orphan_capsem_processes(&run_dir);

    // Check for running instances to reattach
    info!(
        "scanning for existing sandboxes in {}",
        instances_dir.display()
    );
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
        .route(
            "/version",
            get(|| async { Json(serde_json::json!({ "version": env!("CARGO_PKG_VERSION") })) }),
        )
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
        .route("/triage", get(handle_triage))
        .route("/panics", get(handle_panics))
        .route("/host-logs/{name}", get(handle_host_logs))
        .route("/timeline/{id}", get(handle_timeline))
        .route("/vms/{id}/security/latest", get(handle_security_latest))
        .route("/vms/{id}/security/status", get(handle_security_info))
        .route("/vms/{id}/detection/latest", get(handle_security_latest))
        .route("/vms/{id}/detection/status", get(handle_security_info))
        .route("/vms/{id}/enforcement/latest", get(handle_security_latest))
        .route("/vms/{id}/enforcement/status", get(handle_security_info))
        .route(
            "/profiles/{profile_id}/enforcement/evaluate",
            post(handle_enforcement_evaluate),
        )
        .route(
            "/profiles/{profile_id}/enforcement/rules/{rule_id}/edit",
            put(handle_enforcement_rule_upsert),
        )
        .route(
            "/profiles/{profile_id}/enforcement/rules/{rule_id}/delete",
            delete(handle_enforcement_rule_delete),
        )
        .route(
            "/profiles/{profile_id}/enforcement/reload",
            post(handle_enforcement_reload),
        )
        .route(
            "/profiles/{profile_id}/plugins/list",
            get(handle_profile_plugins),
        )
        .route(
            "/profiles/{profile_id}/plugins/{plugin_id}/info",
            get(handle_profile_plugin_info),
        )
        .route(
            "/profiles/{profile_id}/plugins/{plugin_id}/edit",
            patch(handle_profile_plugin_update),
        )
        .route("/profiles/{profile_id}/reload", post(handle_profile_reload))
        .route("/fork/{id}", post(handle_fork))
        .route("/settings/info", get(handle_get_settings))
        .route("/settings/edit", patch(handle_save_settings))
        .route("/settings/presets", get(handle_get_presets))
        .route("/settings/presets/{id}", post(handle_apply_preset))
        .route("/settings/lint", post(handle_lint_config))
        .route("/settings/validate-key", post(handle_validate_key))
        .route("/assets/status", get(handle_assets_status))
        .route("/assets/ensure", post(handle_assets_ensure))
        .route("/corp/edit", put(handle_corp_config))
        .route(
            "/profiles/{profile_id}/mcp/servers/list",
            get(handle_profile_mcp_servers),
        )
        .route(
            "/profiles/{profile_id}/mcp/servers/{server_id}/tools/list",
            get(handle_profile_mcp_server_tools),
        )
        .route(
            "/profiles/{profile_id}/mcp/servers/{server_id}/refresh",
            post(handle_profile_mcp_server_refresh),
        )
        .route(
            "/profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/edit",
            patch(handle_profile_mcp_tool_edit),
        )
        .route(
            "/profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/call",
            post(handle_profile_mcp_tool_call),
        )
        .route("/history/{id}", get(handle_history))
        .route("/history/{id}/processes", get(handle_history_processes))
        .route("/history/{id}/counts", get(handle_history_counts))
        .route("/history/{id}/transcript", get(handle_history_transcript))
        .route("/files/{id}", get(handle_list_files))
        .route(
            "/files/{id}/content",
            get(handle_download_file).post(handle_upload_file),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    info!(socket = %service_sock.display(), "listening on UDS");

    let uds = match service_launch_span
        .in_scope(|| UnixListener::bind(&service_sock).context("failed to bind UDS"))
    {
        Ok(uds) => {
            service_launch_span.record("status", "ok");
            uds
        }
        Err(error) => {
            service_launch_span.record("status", "error");
            return Err(error);
        }
    };
    // Socket is bound; release the startup lock so any peer starter still in
    // its flock wait can fast-probe us and exit 0.
    drop(startup_lock_guard);

    // Spawn companion processes (gateway + tray) in the background so the UDS
    // starts accepting immediately. The previous .await here delayed accept()
    // by up to 5s on every startup while polling gateway.token into existence
    // -- fatal under parallel test load. Companions are stateless and can come
    // up after the service is already serving clients.
    struct CompanionManager {
        children: Vec<tokio::process::Child>,
        spawn_task: Option<tokio::task::JoinHandle<()>>,
    }
    let companions = Arc::new(std::sync::Mutex::new(CompanionManager {
        children: Vec::new(),
        spawn_task: None,
    }));
    let companions_for_spawn = Arc::clone(&companions);
    let service_sock_for_spawn = service_sock.clone();
    let run_dir_for_spawn = run_dir.clone();
    let gateway_binary = args.gateway_binary;
    let gateway_port = args.gateway_port;
    let tray_binary = args.tray_binary;

    let spawn_task = tokio::spawn(async move {
        let spawned = spawn_companions(
            &service_sock_for_spawn,
            &run_dir_for_spawn,
            gateway_binary,
            gateway_port,
            tray_binary,
        )
        .await;
        companions_for_spawn
            .lock()
            .unwrap()
            .children
            .extend(spawned);
    });
    companions.lock().unwrap().spawn_task = Some(spawn_task);

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

            // Scoped so the MutexGuard is definitely dropped before the
            // awaits below; relying on `drop(manager)` alone was fragile
            // enough that the compiler's Send analysis tripped once the
            // surrounding future gained other Send requirements.
            let children = {
                let mut manager = companions_for_shutdown.lock().unwrap();
                if let Some(task) = manager.spawn_task.take() {
                    task.abort();
                }
                std::mem::take(&mut manager.children)
            };

            info!(count = children.len(), "killing companions");
            for mut child in children {
                info!(pid = child.id(), "killing companion process");
                let _ = child.kill().await;
            }
            info!("killing all VM processes");
            kill_all_vm_processes(&shutdown_state);
            info!("shutdown complete");
        })
        .await
        .context("server error")?;

    Ok(())
}

/// Parse `ps -ax -o pid=,command=` output and return the PIDs of every
/// `capsem-process` instance whose `--session-dir` lives inside `run_dir`.
///
/// A SIGKILL to capsem-service (crash, OOM, `svc.proc.kill()` in recovery
/// tests) does not propagate to children, so every per-VM `capsem-process`
/// it spawned becomes an orphan with its `--session-dir` still pointing
/// under the dead service's run_dir. When a replacement service starts on
/// the same run_dir it must reap these orphans or the host accumulates
/// wedged Apple VZ instances and leaked vsock ports.
///
/// Matches on the `--session-dir <run_dir>/` prefix because the spawn-side
/// always writes the absolute session dir as `<run_dir>/sessions/<id>` or
/// `<run_dir>/persistent/<id>`. Pure -- no side effects -- so the matching
/// is unit-testable without spawning real processes.
fn find_orphan_capsem_pids(ps_output: &str, run_dir: &std::path::Path) -> Vec<i32> {
    let run_dir_str = run_dir.display().to_string();
    let marker = format!("--session-dir {run_dir_str}");
    let mut pids = Vec::new();
    for line in ps_output.lines() {
        let line = line.trim_start();
        if !line.contains("capsem-process") {
            continue;
        }
        if !line.contains(&marker) {
            continue;
        }
        let Some((pid_str, _)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if let Ok(pid) = pid_str.parse::<i32>() {
            pids.push(pid);
        }
    }
    pids
}

/// Reap `capsem-process` orphans from a prior service run that shared this
/// run_dir. See [`find_orphan_capsem_pids`] for the why; this wrapper shells
/// out to `ps`, applies the match, and escalates SIGTERM -> 2s poll ->
/// SIGKILL. Best effort: silent if `ps` is missing or nothing matches.
fn reap_orphan_capsem_processes(run_dir: &std::path::Path) {
    let output = match std::process::Command::new("ps")
        .args(["-ax", "-o", "pid=,command="])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let orphan_pids = find_orphan_capsem_pids(&stdout, run_dir);
    if orphan_pids.is_empty() {
        return;
    }

    tracing::warn!(
        count = orphan_pids.len(),
        ?orphan_pids,
        "reaping capsem-process orphans from previous service run"
    );

    for pid in &orphan_pids {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(*pid),
            nix::sys::signal::Signal::SIGTERM,
        );
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let survivors: Vec<i32> = orphan_pids
            .iter()
            .copied()
            .filter(|&pid| unsafe { nix::libc::kill(pid, 0) } == 0)
            .collect();
        if survivors.is_empty() {
            return;
        }
        if std::time::Instant::now() >= deadline {
            tracing::warn!(
                count = survivors.len(),
                ?survivors,
                "orphan capsem-process did not exit, SIGKILLing"
            );
            for pid in survivors {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
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
        instances
            .values()
            .map(|i| {
                (
                    i.pid,
                    i.uds_path.clone(),
                    i.session_dir.clone(),
                    i.persistent,
                )
            })
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
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
            signaled_any_vm = true;
        }
        let _ = std::fs::remove_file(uds_path);
        let _ = std::fs::remove_file(uds_path.with_extension("ready"));
        if !persistent {
            let _ = std::fs::remove_dir_all(session_dir);
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
        let survivors: Vec<u32> = pids_and_sockets
            .iter()
            .map(|(pid, _, _, _)| *pid)
            .filter(|&pid| pid > 0 && unsafe { nix::libc::kill(pid as i32, 0) } == 0)
            .collect();

        if survivors.is_empty() {
            break;
        }

        if start.elapsed() >= timeout {
            tracing::warn!(
                count = survivors.len(),
                "some VMs survived SIGTERM, escalating to SIGKILL"
            );
            for pid in survivors {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid as i32),
                    nix::sys::signal::Signal::SIGKILL,
                );
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
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
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
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
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
    gw_cmd
        .arg("--parent-pid")
        .arg(std::process::id().to_string());
    if let Some(port) = gateway_port {
        gw_cmd.arg("--port").arg(port.to_string());
    }
    let gateway_span = tracing::debug_span!(
        target: "capsem.launch",
        capsem_core::telemetry::LAUNCH_GATEWAY_SPAN,
        status = tracing::field::Empty,
    );
    match gateway_span.in_scope(|| {
        gw_cmd
            .stdout(gw_out)
            .stderr(gw_err)
            .kill_on_drop(true)
            .spawn()
    }) {
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
                    capsem_core::poll::PollOpts::new(
                        "gateway-ready",
                        std::time::Duration::from_secs(5),
                    ),
                    || {
                        let tp = tp.clone();
                        let pp = pp.clone();
                        async move {
                            if tp.exists() && pp.exists() {
                                Some(())
                            } else {
                                None
                            }
                        }
                    },
                )
                .instrument(gateway_span.clone())
                .await;
            }
            if token_path.exists() && port_path.exists() {
                gateway_span.record("status", "ok");
            } else {
                gateway_span.record("status", "error");
            }

            // 2. Spawn capsem-tray (menu bar) -- only on macOS, only after gateway ready
            #[cfg(target_os = "macos")]
            if token_path.exists() {
                let tray_bin = tray_bin.unwrap_or_else(|| find_sibling_binary("capsem-tray"));
                let (tray_out, tray_err) = companion_stdio(&log_dir.join("tray.log"));
                info!(binary = %tray_bin.display(), "spawning capsem-tray");
                match tokio::process::Command::new(&tray_bin)
                    .arg("--parent-pid")
                    .arg(std::process::id().to_string())
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
            gateway_span.record("status", "error");
            tracing::warn!("failed to spawn capsem-gateway: {e} (non-fatal)");
        }
    }

    children
}

#[cfg(test)]
mod tests;
