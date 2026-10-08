use anyhow::{anyhow, Context, Result};
use axum::http::StatusCode;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    response::IntoResponse,
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use capsem_core::{
    mcp::ToolCacheEntry,
    net::policy_config::{DetectionLevel, SecurityPluginConfig, SecurityPluginMode, SecurityRuleAction, SettingsFile},
};
use capsem_foundation::ipc_channel::{channel_from_std, Receiver, Sender};
use capsem_foundation::poll::{poll_until, PollOpts};
use capsem_proto::ipc::{FileBoundaryAction, ProcessToService, ServiceToProcess};
use capsem_service::errors::AppError;
use clap::Parser;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path as StdPath, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use tokio::net::UnixListener;
use tokio::process::Command;
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn, Instrument};
mod active_policy;
use active_policy::push_policy_to_running_instances;
mod asset_background;
mod asset_routes;
mod blocking;
mod container_setup;
mod host_ledger;
mod instance;
mod instance_reaper;
pub mod managed_sessions;
use instance::InstanceInfo;
mod network_routes;
mod policy_mutation;
use policy_mutation::{apply_policy_mutation, MutationRoute, PolicyMutation};
mod credential_routes;
mod mcp_routes;
mod plugin_routes;
mod private_routes;
mod process_control;
mod sandbox_info;
mod session_cleanup;
mod session_db_handles;
mod switches;
use session_db_handles::session_db_path_for_session_dir;
mod session_housekeeping;
use session_cleanup::{finalize_one_shot_session, handle_preserve_failure, preserve_failed_run_shutdown_result};
mod ledger_routes;
mod router_runtime;
mod service_runtime;
mod settings_routes;
mod shutdown_policy;
mod startup;
mod suspend_confirmation;
mod update_command;
mod update_status;
mod vm_files;
mod vm_lifecycle;
use asset_routes::*;
use ledger_routes::*;
use router_runtime::*;
use service_runtime::*;
use settings_routes::*;
use shutdown_policy::*;
use suspend_confirmation::{observe_suspend_message, suspend_channel_closed, suspend_failure, SuspendConfirmation};
use update_command::{update_command_plan, UpdateCommandKind};
use update_status::update_status_response_from_paths;
use vm_files::*;
use vm_lifecycle::*;

/// Ceiling on a session log tail returned over the API. `serial.log` is guest
/// console output written through `CappedLogWriter`, so its size is the guest's
/// choice, not ours; every reader takes a bounded tail.
const SESSION_LOG_TAIL_MAX_BYTES: usize = 5 * 1024 * 1024;

const RESUME_CHECKPOINT_NAME: &str = "checkpoint.vzsave";
const SUSPEND_CONFIRM_TIMEOUT_SECS: u64 = 45;
const AUTOMATIC_UPDATE_INITIAL_DELAY_SECS: u64 = 60;
const AUTOMATIC_UPDATE_POLL_SECS: u64 = 60 * 60;
const AUTOMATIC_UPDATE_BUSY_RETRY_SECS: u64 = 5 * 60;
const AUTOMATIC_UPDATE_MAX_BACKOFF_SECS: u64 = 24 * 60 * 60;
const AUTOMATIC_UPDATE_INITIAL_DELAY_ENV: &str = "CAPSEM_AUTOMATIC_UPDATE_INITIAL_DELAY_SECS";
const AUTOMATIC_UPDATE_POLL_ENV: &str = "CAPSEM_AUTOMATIC_UPDATE_POLL_SECS";

use capsem_foundation::paths::checkpoint_complete_path;

/// Owns `$run_dir/service.pid` -- the only handle a harness has on a detached
/// service. Written when this process takes the socket, removed on clean
/// shutdown so a stale pid cannot make a dead service look alive to whatever
/// is waiting to reap it.
///
/// Removal is ownership-checked. A pidfile that no longer records our pid
/// belongs to a successor that claimed the run directory while we were shutting
/// down, and erasing it strands that successor exactly as this guard exists to
/// prevent: `stop_gate_pidfile` on a missing file is indistinguishable from a
/// successful reap.
struct ServicePidfile {
    path: std::path::PathBuf,
    pid: u32,
}

impl ServicePidfile {
    /// Claim the pidfile for this process.
    ///
    /// Call only once we own the service socket. A starter that claims before
    /// the startup race resolves and then loses it -- the "compatible
    /// capsem-service already running; exiting 0" path -- drops this guard on
    /// the way out and takes the winner's pid with it.
    fn claim(path: std::path::PathBuf) -> Self {
        let pid = std::process::id();
        if let Err(error) = std::fs::write(&path, pid.to_string()) {
            warn!(path = %path.display(), %error, "failed to write service pidfile");
        }
        Self { path, pid }
    }

    fn records_us(&self) -> bool {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|raw| raw.trim().parse::<u32>().ok())
            .is_some_and(|recorded| recorded == self.pid)
    }
}

impl Drop for ServicePidfile {
    fn drop(&mut self) {
        if self.records_us() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

use capsem_service::api;
use capsem_service::api::*;
use capsem_service::naming::{generate_session_name, validate_vm_name};
use capsem_service::registry::{
    new_persistent_vm_id, BootAssetPin, BootAssetPins, PersistentRegistry, PersistentVmEntry, SharedRegistry,
};
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
    "CAPSEM_CORP_CONFIG",
    // Ironbank uses an isolated credential store while exercising the real broker.
    "CAPSEM_CREDENTIAL_STORE_PATH",
    // Tunable: bounded MITM MCP endpoint in-flight handler cap.
    "CAPSEM_MCP_INFLIGHT",
    // Tunable: pool size for the local builtin MCP server (rmcp stdio funnel).
    "CAPSEM_MCP_BUILTIN_POOL",
    // Read by capsem-process when constructing the framed MCP endpoint.
    "CAPSEM_MCP_DEFAULT_TIMEOUT_SECS",
    "CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS",
    "CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS",
    // Experimental benchmark lane: capsem-process enables EROFS DAX at boot.
    "CAPSEM_EXPERIMENTAL_EROFS_DAX",
];

const ACTIVE_POLICY_DIR: &str = "vm";
const ACTIVE_POLICY_FILE: &str = "active_policy.toml";

pub struct ServiceState {
    instances: Mutex<HashMap<String, InstanceInfo>>, // instance id to process info
    retirements: instance_reaper::Retirements,
    /// Logger-owned DB handles keyed by session/VM id. Logged-data routes
    /// resolve a handle here and call `ready/query`; they do not open SQLite
    /// readers or create per-route projection caches.
    session_db_handles: Mutex<HashMap<String, Arc<capsem_logger::DbHandle>>>,
    persistent_registry: SharedRegistry,
    /// Named networks as groups of VMs, durable in each network's database.
    networks: tokio::sync::Mutex<capsem_core::net::network_registry::NetworkRegistry>,
    process_binary: PathBuf,
    assets_dir: PathBuf,
    run_dir: PathBuf,
    service_socket: PathBuf,      // this service's own, where an owner asks on a guest's behalf
    switches: switches::Switches, // one confined switch per network, and its links
    job_counter: AtomicU64,
    /// v2 manifest (None in dev mode where assets use logical names)
    manifest: RwLock<Option<Arc<capsem_assets::asset_manager::ManifestV2>>>,
    current_version: String,
    /// In-memory asset reconciliation progress. Service startup and explicit
    /// /assets/ensure share this single rail with status so status can
    /// explain both.
    asset_reconcile: Mutex<AssetReconcileState>,
    asset_reconcile_inflight: AtomicBool,
    asset_status_path: PathBuf,
    /// Route-owned MCP tool cache loaded once at service startup and refreshed
    /// by explicit MCP discovery routes. Hot MCP list routes must not read the
    /// tool cache JSON from disk.
    mcp_tool_cache: Mutex<Vec<ToolCacheEntry>>,
    /// Logger-owned DB handle for the host ledger (`sessions/host.db`): host
    /// events and policy mutations. Routes call `write`; they must never open
    /// SQLite directly or hold a side `DbWriter`.
    host_ledger: Arc<capsem_logger::DbHandle>,
    /// The `/stats` fold of the host ledger, see `host_ledger.rs`.
    host_stats: Mutex<host_ledger::HostStats>,
    /// Last wall-clock millisecond when preserved boot logs were scanned for
    /// defunct persistent VMs. Hot list/status/info polls must not rescan the
    /// filesystem on every request.
    last_defunct_reconcile_ms: AtomicU64,
    /// Session-ledger route bytes (security/detection latest, security status,
    /// history processes/counts) keyed by VM id, route and paging, pinned to
    /// the logger read-cache epoch, released when the VM's handle unregisters.
    stats_detail_response_cache: Mutex<HashMap<String, CachedLedgerResponse>>,
    /// Container workloads being set up or running, by VM id.
    containers: container_setup::ContainerSetups,
    /// Session storage diagnostics cached by session directory. These values
    /// describe the rootfs image path/size and host filesystem for status/info
    /// routes; repeated polling must not stat the filesystem on every sample.
    storage_diagnostics_cache: Mutex<HashMap<PathBuf, api::StorageDiagnostics>>,
    /// Derived persistent VM resume state keyed by stable VM id. The
    /// fingerprint covers registry fields that affect status so route polling
    /// does not revalidate asset pins every sample.
    persistent_resume_state_cache: Mutex<HashMap<String, CachedPersistentResumeState>>,
    /// The installed manifest's status, rebuilt only when manifest.json or
    /// its metadata changes; `/status` polls it.
    asset_manifest_cache: Mutex<Option<asset_routes::CachedManifestStatus>>,
    /// `/vms/list` lifecycle rows keyed by the in-memory lifecycle snapshot
    /// (uptime seconds included). Activity totals are read per poll from
    /// each ledger's cached counter snapshot, never cached here.
    list_response_cache: Mutex<Option<CachedListResponse>>,
    /// Coordinates launch admission and Apple VZ lifecycle edges. Cold starts
    /// and teardown take a VZ read guard; save/restore take a write guard.
    /// Blocking launch workers also retain admission through registration,
    /// excluding restart without serializing independent cold boots.
    /// See web/docs/src/content/docs/gotchas/concurrent-suspend-resume.mdx.
    lifecycle: capsem_service::lifecycle::VmLifecycle,
    /// Serializes VM teardown (delete / stop / purge per-VM / handle_run)
    /// across all VMs managed by this service. N concurrent shutdowns starve
    /// each other of the resources each capsem-process needs to (a) let VZ
    /// tear down the guest, (b) run the DbWriter's WAL checkpoint on Drop,
    /// and (c) clean up the session UDS files. Under that contention a
    /// single teardown can exceed `wait_for_process_exit`'s 1s fast-path
    /// budget -- at which point the service SIGKILLs capsem-process mid-
    /// checkpoint, leaving a non-empty WAL and (in the worst case) orphaned
    /// sockets. Same serialization pattern as `lifecycle.vz`: one
    /// critical-section operation in flight at a time, in-process only,
    /// sufficient because production runs exactly one capsem-service per
    /// user-host.
    shutdown_lock: tokio::sync::Mutex<()>,
    /// Serializes every explicit and automatic update command. The update
    /// transaction owns binaries, assets, and the selected manifest
    /// together, so the service must never launch split or overlapping
    /// mutations.
    update_lock: tokio::sync::Mutex<()>,
    /// Serializes every policy mutation and reload, load through VM acknowledgement.
    policy_mutation: policy_mutation::PolicyMutationLock,
    /// Requests a managed service shutdown after a package update selects a
    /// different binary. LaunchAgent/systemd then starts the newly installed
    /// service instead of leaving the old process attached to the new graph.
    update_restart: tokio::sync::Notify,
    /// Keeps the unique filesystem root alive for helpers that return only a
    /// service state. Test constructors that return their TempDir separately
    /// leave this empty.
    #[cfg(test)]
    _test_tempdir: Option<tempfile::TempDir>,
}

/// Serialized ledger route bytes and the logger generation they were read at.
#[derive(Clone)]
struct CachedLedgerResponse {
    db_epoch: u64,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct CachedPersistentResumeState {
    fingerprint: String,
    state: (VmLifecycleState, bool, Option<String>),
}

#[derive(Clone)]
struct CachedListResponse {
    fingerprint: String,
    listed: (ListResponse, Vec<PathBuf>),
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

pub struct ProvisionOptions<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub ram_mb: u64,
    pub cpus: u32,
    pub scratch_disk_size_gb: u32,
    pub version_override: Option<String>,
    pub persistent: bool,
    pub env: Option<std::collections::HashMap<String, String>>,
    pub from: Option<CloneFrom>,
    pub description: Option<String>,
}

/// The persistent session a new one is cloned from (`--from`).
pub struct CloneFrom {
    pub source: String,
    /// The new session names its own image: the source's staged image is
    /// not launched, while its workspace and image volumes carry over.
    pub replace_image: bool,
}

impl CloneFrom {
    /// A clone that boots its source's own staged image.
    pub fn keeping_image(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            replace_image: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedVmResources {
    ram_mb: u64,
    cpus: u32,
    scratch_disk_size_gb: u32,
}

/// What a VM gets when its create request names no size.
const DEFAULT_VM_CPUS: u32 = 4;
const DEFAULT_VM_RAM_MB: u64 = 12 * 1024;
/// Every new VM's scratch disk; the create API has no size for it.
const DEFAULT_SCRATCH_DISK_GB: u32 = 64;

fn resolve_vm_resources(requested_ram_mb: Option<u64>, requested_cpus: Option<u32>) -> ResolvedVmResources {
    ResolvedVmResources {
        ram_mb: requested_ram_mb.unwrap_or(DEFAULT_VM_RAM_MB),
        cpus: requested_cpus.unwrap_or(DEFAULT_VM_CPUS),
        scratch_disk_size_gb: DEFAULT_SCRATCH_DISK_GB,
    }
}

fn prewarm_system_overlay_template(run_dir: &StdPath) {
    let size_gb = DEFAULT_SCRATCH_DISK_GB;
    let template_path = capsem_core::system_overlay_template_path(run_dir, size_gb);
    match capsem_core::ensure_preformatted_system_overlay_template(&template_path, size_gb) {
        Ok(true) => info!(
            path = %template_path.display(),
            size_gb,
            "prewarmed system overlay template"
        ),
        Ok(false) => info!(
            path = %template_path.display(),
            size_gb,
            "system overlay template ready"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => warn!(
            path = %template_path.display(),
            size_gb,
            error = %error,
            "mke2fs unavailable; guest will format system overlay at first boot"
        ),
        Err(error) => warn!(
            path = %template_path.display(),
            size_gb,
            error = %error,
            "failed to prewarm system overlay template; launch will retry"
        ),
    }
}

fn prewarm_vm_asset_hash_cache(
    assets_base_dir: &StdPath,
    manifest: Option<&capsem_assets::asset_manager::ManifestV2>,
    current_version: &str,
) {
    let Some(manifest) = manifest else {
        return;
    };
    let arch = capsem_assets::asset_manager::host_manifest_arch();
    let resolved = match manifest.resolve(current_version, arch, assets_base_dir) {
        Ok(resolved) => resolved,
        Err(error) => {
            warn!(error = %error, arch, "failed to resolve VM assets for hash cache prewarm");
            return;
        }
    };
    let Some(expected) = manifest.expected_hashes(&resolved.asset_version, arch) else {
        warn!(
            arch,
            "failed to resolve expected VM asset hashes for hash cache prewarm"
        );
        return;
    };
    for (kind, path, hash) in [
        ("kernel", resolved.kernel.as_path(), expected.kernel.as_str()),
        ("initrd", resolved.initrd.as_path(), expected.initrd.as_str()),
        ("rootfs", resolved.rootfs.as_path(), expected.rootfs.as_str()),
    ] {
        match capsem_core::VmConfig::verify_hash(path, hash) {
            Ok(()) => info!(
                kind,
                path = %path.display(),
                "prewarmed VM asset hash cache"
            ),
            Err(error) => warn!(
                kind,
                path = %path.display(),
                error = %error,
                "failed to prewarm VM asset hash cache; launch will retry"
            ),
        }
    }
}

/// Maximum number of `-failed-*` session dirs preserved across crashes,
/// wait_for_vm_ready timeouts, and dead-process cleanup. The preserved dirs
/// hold the host-side post-mortem signal for genuinely failed sessions
/// (process.log, mcp-aggregator.stderr.log, serial.log, and session.db).
/// Clean DELETE is deliberately excluded: its public contract is to destroy
/// all retained state.
const MAX_FAILED_SESSIONS: usize = 32;

/// Remove a session tree after its process has exited.
///
/// Linux may return `ENOTEMPTY` from `remove_dir_all` when a late SQLite or
/// filesystem cleanup creates an entry between traversal and the final
/// `rmdir`. Retry only that transient race for a bounded number of sightings;
/// permissions, path safety, and every other filesystem error remain
/// fail-closed. Counting sightings rather than wall time keeps host load from
/// consuming the retry budget.
const SESSION_DELETE_MAX_ATTEMPTS: usize = 8;

fn remove_quiesced_session_dir(path: &StdPath) -> std::io::Result<()> {
    remove_quiesced_session_dir_with(
        path,
        |path| std::fs::remove_dir_all(path),
        || std::thread::sleep(std::time::Duration::from_millis(20)),
    )
}

fn remove_quiesced_session_dir_with(
    path: &StdPath,
    mut remove: impl FnMut(&StdPath) -> std::io::Result<()>,
    mut wait: impl FnMut(),
) -> std::io::Result<()> {
    for attempt in 1..=SESSION_DELETE_MAX_ATTEMPTS {
        match remove(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error)
                if error.kind() == std::io::ErrorKind::DirectoryNotEmpty && attempt < SESSION_DELETE_MAX_ATTEMPTS =>
            {
                wait();
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the final removal attempt always returns")
}

impl ServiceState {
    /// Build the Unix socket path for a VM instance.
    ///
    /// Delegates to `capsem_foundation::uds::instance_socket_path`, the single
    /// source of truth for the macOS `SUN_LEN` workaround. Logs when the
    /// fallback path is used so clients can correlate.
    fn instance_socket_path(&self, id: &str) -> Result<PathBuf> {
        let path = capsem_foundation::uds::instance_socket_path(&self.run_dir, id)
            .with_context(|| format!("resolve instance socket path for {id}"))?;
        if !path.starts_with(&self.run_dir) {
            let preferred = self.run_dir.join("instances").join(format!("{id}.sock"));
            tracing::info!(%id, original = %preferred.display(), short = %path.display(),
                           "socket path too long, using the private fallback dir");
        }
        Ok(path)
    }

    fn storage_diagnostics_cached(&self, session_dir: &StdPath) -> Option<api::StorageDiagnostics> {
        if let Some(cached) = self.storage_diagnostics_cache.lock().unwrap().get(session_dir).cloned() {
            return Some(cached);
        }

        let diagnostics = storage_diagnostics(session_dir)?;
        self.storage_diagnostics_cache
            .lock()
            .unwrap()
            .insert(session_dir.to_path_buf(), diagnostics.clone());
        Some(diagnostics)
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
        let mut probe = process_control::ProcessProbe::new("drain-dead-instances");
        let dead = instances
            .extract_if(|_, info| probe.is_gone(info.pid))
            .map(|(id, info)| {
                tracing::warn!(id, "drain_dead_instances removing instance");
                (id, info)
            })
            .collect();
        drop(instances);
        dead
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
        self.unregister_session_db_handle(id);
        if info.persistent {
            info!(id, "persistent VM process died, preserving session dir");
        } else {
            info!(id, "ephemeral VM process died, preserving session dir for post-mortem");
            let _ = self.preserve_failed_session_dir(&info.session_dir, id);
        }
        let _ = std::fs::remove_file(&info.uds_path);
        service_runtime::remove_instance_sentinels(&info.uds_path);
    }

    fn cleanup_stale_instances(&self) {
        for (id, info) in self.drain_dead_instances() {
            info!(id, "removing stale instance record");
            self.scrub_evicted_instance(&id, &info);
        }
    }

    fn has_existing_resume_checkpoint(&self, id: &str) -> bool {
        find_persistent_entry_by_route_id(self, id).is_some_and(|entry| {
            entry.suspended
                && entry.checkpoint_path.as_ref().is_some_and(|cp| {
                    let checkpoint = entry.session_dir.join(cp);
                    checkpoint.exists() && checkpoint_complete_path(&checkpoint).exists()
                })
        })
    }

    fn archive_failed_restore_checkpoint(&self, id: &str) -> Option<PathBuf> {
        let entry = find_persistent_entry_by_route_id(self, id)?;
        let name = entry.name;
        let checkpoint_name = entry.checkpoint_path?;
        let session_dir = entry.session_dir;

        let checkpoint_path = session_dir.join(&checkpoint_name);
        if !checkpoint_path.exists() {
            return None;
        }
        let complete_path = checkpoint_complete_path(&checkpoint_path);

        let epoch_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let archived_path = session_dir.join(format!("{checkpoint_name}.failed-restore-{epoch_ms}"));

        match std::fs::rename(&checkpoint_path, &archived_path) {
            Ok(()) => {
                if complete_path.exists() {
                    let archived_complete_path = checkpoint_complete_path(&archived_path);
                    if let Err(e) = std::fs::rename(&complete_path, &archived_complete_path) {
                        warn!(
                            name,
                            complete = %complete_path.display(),
                            archived = %archived_complete_path.display(),
                            "failed to archive restore checkpoint completion marker: {e}"
                        );
                    }
                }
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
        let key = registry
            .list()
            .find(|entry| persistent_entry_vm_id(entry) == id)
            .map(|entry| entry.name.clone());
        if let Some(entry) = key.and_then(|key| registry.get_mut(&key)) {
            if let Some(checkpoint_name) = entry.checkpoint_path.as_ref() {
                let checkpoint_path = entry.session_dir.join(checkpoint_name);
                let _ = std::fs::remove_file(checkpoint_complete_path(&checkpoint_path));
            }
            entry.suspended = false;
            entry.checkpoint_path = None;
            entry.defunct = false;
            entry.last_error = None;
            if let Err(e) = registry.save() {
                error!(id, error = %e, "failed to save persistent registry after resume");
            }
        }
    }

    /// Whether a persistent VM may boot as recorded: its registry entry is in
    /// the current shape and its pinned images are not revoked.
    fn validate_persistent_entry(&self, entry: &PersistentVmEntry) -> Result<()> {
        if let Some(profile) = &entry.legacy_profile_id {
            return Err(anyhow!(
                "VM '{}' was created from the '{profile}' profile, and profiles no longer exist; \
                 delete it and create a new VM",
                entry.name
            ));
        }
        reject_revoked_persistent_pins(&self.assets_dir, entry)
    }

    fn resolve_pinned_asset_paths(&self, pins: &BootAssetPins) -> Result<capsem_assets::asset_manager::ResolvedAssets> {
        let arch = capsem_assets::asset_manager::host_manifest_arch();
        Ok(capsem_assets::asset_manager::ResolvedAssets {
            kernel: boot_asset_pin_path(&self.assets_dir, arch, &pins.kernel),
            initrd: boot_asset_pin_path(&self.assets_dir, arch, &pins.initrd),
            rootfs: boot_asset_pin_path(&self.assets_dir, arch, &pins.rootfs),
            asset_version: "persistent-pins".to_string(),
        })
    }

    fn validate_pinned_asset_files(
        &self,
        resolved: &capsem_assets::asset_manager::ResolvedAssets,
        pins: &BootAssetPins,
    ) -> Result<()> {
        validate_asset_file_pin("kernel", &resolved.kernel, &pins.kernel)?;
        validate_asset_file_pin("initrd", &resolved.initrd, &pins.initrd)?;
        validate_asset_file_pin("rootfs", &resolved.rootfs, &pins.rootfs)
    }

    fn persistent_entry_resume_state(&self, entry: &PersistentVmEntry) -> (VmLifecycleState, bool, Option<String>) {
        if entry.defunct {
            return (VmLifecycleState::Defunct, false, entry.last_error.clone());
        }

        if let Err(err) = self.validate_persistent_entry(entry) {
            return (VmLifecycleState::Incompatible, false, Some(err.to_string()));
        }
        if let Err(err) = session_rootfs_size_gb(entry) {
            return (VmLifecycleState::Incompatible, false, Some(err.to_string()));
        }

        let status = if entry.suspended {
            VmLifecycleState::Suspended
        } else {
            VmLifecycleState::Stopped
        };

        let resolved = match self.resolve_pinned_asset_paths(&entry.asset_pins) {
            Ok(resolved) => resolved,
            Err(err) => return (status, false, Some(err.to_string())),
        };
        match self.validate_pinned_asset_files(&resolved, &entry.asset_pins) {
            Ok(()) => (status, true, None),
            Err(err) => (status, false, Some(err.to_string())),
        }
    }

    fn persistent_entry_resume_state_cached(
        &self,
        entry: &PersistentVmEntry,
    ) -> (VmLifecycleState, bool, Option<String>) {
        let vm_id = persistent_entry_vm_id(entry);
        let fingerprint = persistent_resume_state_fingerprint(self, entry);
        if let Some(cached) = self.persistent_resume_state_cache.lock().unwrap().get(&vm_id).cloned() {
            if cached.fingerprint == fingerprint {
                return cached.state;
            }
        }

        let state = self.persistent_entry_resume_state(entry);
        self.persistent_resume_state_cache.lock().unwrap().insert(
            vm_id,
            CachedPersistentResumeState {
                fingerprint,
                state: state.clone(),
            },
        );
        state
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    run_service().await
}

#[cfg(test)]
mod performance_contract_tests;
#[cfg(test)]
mod tests;
