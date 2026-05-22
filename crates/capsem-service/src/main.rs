use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use capsem_core::poll::{poll_until, PollOpts};
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use capsem_proto::metrics::VmMetricsSnapshot;
use capsem_security_engine as seceng;
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::UnixListener;
use tokio_unix_ipc::{channel_from_std, Receiver, Sender};
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};

mod startup;

use capsem_service::api;
use capsem_service::api::*;
use capsem_service::asset_supervisor::{
    host_asset_arch, AssetRequirement, AssetSupervisor, ProfileAssetRequirement,
};
use capsem_service::debug_report;
use capsem_service::naming::{generate_tmp_name, validate_vm_name};
use capsem_service::registry::{
    PersistentRegistry, PersistentVmEntry, SavedVmBaseAssets, SavedVmProfilePin,
};
use capsem_service::saved_vm_assets;
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
    // Tunable: bounded MITM MCP endpoint in-flight handler cap.
    "CAPSEM_MCP_INFLIGHT",
    // Tunable: pool size for the local builtin MCP server (rmcp stdio funnel).
    "CAPSEM_MCP_BUILTIN_POOL",
    // Read by capsem-process when constructing the framed MCP endpoint.
    "CAPSEM_MCP_DEFAULT_TIMEOUT_SECS",
    "CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS",
    "CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS",
    // E2E-only: lets capsem-process dial a local fixture while preserving
    // the guest-visible upstream host for MITM policy/provider detection.
    "CAPSEM_TEST_UPSTREAM_OVERRIDES",
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
    asset_locations: capsem_core::settings_profiles::ResolvedServiceAssetLocations,
    service_settings: capsem_core::settings_profiles::ServiceSettings,
    run_dir: PathBuf,
    job_counter: AtomicU64,
    /// Service-owned asset state machine and background reconciler.
    asset_supervisor: Arc<AssetSupervisor>,
    /// Runtime CEL enforcement rules installed through the service API.
    enforcement_registry: Arc<Mutex<seceng::RuntimeRuleRegistry>>,
    /// Runtime CEL/Sigma-lowered detection rules installed through the service API.
    detection_registry: Arc<Mutex<seceng::RuntimeRuleRegistry>>,
    current_version: String,
    /// Magika file-type detection session (thread-safe, shared)
    magika: Mutex<magika::Session>,
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

fn startup_asset_requirement(
    service_settings: &capsem_core::settings_profiles::ServiceSettings,
    arch: &str,
    allow_dev_logical_assets: bool,
) -> Result<AssetRequirement> {
    profile_asset_requirement_for_selection(
        service_settings,
        None,
        None,
        arch,
        allow_dev_logical_assets,
    )
}

fn profile_asset_requirement_for_selection(
    service_settings: &capsem_core::settings_profiles::ServiceSettings,
    profile_id: Option<&str>,
    profile_revision: Option<&str>,
    arch: &str,
    allow_dev_logical_assets: bool,
) -> Result<AssetRequirement> {
    let (effective, _) = capsem_core::settings_profiles::resolve_effective_vm_settings_with_corp(
        service_settings,
        profile_id,
    )
    .with_context(|| {
        format!(
            "resolve {}profile for VM assets",
            profile_id.unwrap_or("default ")
        )
    })?;
    match ProfileAssetRequirement::from_effective(&effective, arch) {
        Ok(required) => {
            let selected_profile_requires_catalog = profile_id.is_some() || profile_revision.is_some();
            let installed_revision = if selected_profile_requires_catalog {
                capsem_core::settings_profiles::load_complete_installed_profile_revision(
                    &service_settings.profiles,
                    &effective.profile_id,
                )
                .context("load complete installed profile revision for asset provenance")?
                .map(|record| (record.revision, record.payload_hash))
            } else {
                capsem_core::settings_profiles::load_installed_profile_revision(
                    &service_settings.profiles,
                    &effective.profile_id,
                )
                .context("load installed profile revision for asset provenance")?
                .map(|record| (record.revision, record.payload_hash))
            };
            let required = match installed_revision {
                Some((revision, payload_hash)) => {
                    if let Some(requested) = profile_revision {
                        if revision != requested {
                            anyhow::bail!(
                                "profile '{}' installed revision '{}' does not match requested revision '{}'",
                                effective.profile_id,
                                revision,
                                requested
                            );
                        }
                    }
                    required.with_installed_revision(Some(revision), Some(payload_hash))
                }
                None if selected_profile_requires_catalog => {
                    anyhow::bail!(
                        "profile '{}' has no installed signed catalog revision; install it before creating a VM",
                        effective.profile_id
                    );
                }
                None => required,
            };
            Ok(AssetRequirement::Profile(Box::new(required)))
        }
        Err(err) if allow_dev_logical_assets => {
            warn!(
                error = %err,
                arch,
                profile_id = %effective.profile_id,
                "profile has no VM asset declarations; using explicit development assets"
            );
            Ok(AssetRequirement::DevLogical {
                arch: arch.to_string(),
            })
        }
        Err(err) => Err(err).context(
            "release startup requires profile VM assets; old asset manifests are not runtime authority",
        ),
    }
}

#[derive(Clone)]
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
    /// Exact boot-asset identity this VM's root overlay depends on.
    base_assets: Option<SavedVmBaseAssets>,
    /// Exact profile/package/asset identity this VM was created with.
    profile_pin: Option<SavedVmProfilePin>,
}

pub struct ProvisionOptions<'a> {
    pub id: &'a str,
    pub ram_mb: u64,
    pub cpus: u32,
    pub version_override: Option<String>,
    pub persistent: bool,
    pub env: Option<std::collections::HashMap<String, String>>,
    pub from: Option<String>,
    pub profile_id: Option<String>,
    pub profile_revision: Option<String>,
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

const DEFAULT_MAX_CONCURRENT_VMS: usize = 10;

#[derive(Debug, Clone, Copy)]
struct VmRuntimeDefaults {
    ram_mb: u64,
    cpus: u32,
    max_concurrent_vms: usize,
}

/// Result of [`ServiceState::preserve_failed_session_dir_outcome`].
///
/// AB-008: pulled out so callers can distinguish "already preserved by an
/// earlier pass" (idempotent no-op) from real failures that should warn.
#[derive(Debug)]
pub(crate) enum PreserveOutcome {
    /// Renamed to a `-failed-*` sibling.
    Preserved(PathBuf),
    /// The session dir was already gone (handled by a prior call, or never
    /// there). Idempotent no-op.
    AlreadyAbsent,
    /// Rename failed for a real reason; the fallback `remove_dir_all`
    /// reclaimed disk.
    FailedAndRemoved { rename_error: std::io::Error },
    /// Rename failed AND remove failed (other than `NotFound`); the dir is
    /// orphaned on disk.
    FailedAndOrphaned {
        rename_error: std::io::Error,
        remove_error: std::io::Error,
    },
}

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

    /// Ensure a session directory has coherent Profile V2 effective-settings
    /// and resolver-trace attachments. Existing readable pairs are preserved
    /// for fork/resume provenance; missing or corrupt pairs are regenerated.
    fn ensure_vm_effective_settings(&self, session_dir: &FsPath) -> Result<()> {
        let effective_path =
            capsem_core::settings_profiles::vm_effective_settings_path(session_dir);
        let trace_path = capsem_core::settings_profiles::vm_effective_trace_path(session_dir);

        let settings_ok = effective_path.is_file()
            && match capsem_core::settings_profiles::load_vm_effective_settings(session_dir) {
                Ok(_) => true,
                Err(error) => {
                    warn!(
                        path = %effective_path.display(),
                        error = %error,
                        "existing vm-effective settings unreadable, regenerating"
                    );
                    false
                }
            };
        let trace_ok = trace_path.is_file()
            && match capsem_core::settings_profiles::load_vm_effective_trace(session_dir) {
                Ok(_) => true,
                Err(error) => {
                    warn!(
                        path = %trace_path.display(),
                        error = %error,
                        "existing vm-effective trace unreadable, regenerating"
                    );
                    false
                }
            };

        if settings_ok && trace_ok {
            return Ok(());
        }

        self.refresh_vm_effective_settings_for_profile(session_dir, None)
    }

    fn current_service_settings(&self) -> capsem_core::settings_profiles::ServiceSettings {
        let settings_path = service_settings_path();
        if !settings_path.exists() {
            return self.service_settings.clone();
        }
        capsem_core::settings_profiles::load_service_settings(&settings_path).unwrap_or_else(
            |error| {
                warn!(
                    error = %error,
                    "failed to reload service settings from disk, using startup snapshot"
                );
                self.service_settings.clone()
            },
        )
    }

    fn refresh_vm_effective_settings_for_profile(
        &self,
        session_dir: &FsPath,
        profile_id: Option<&str>,
    ) -> Result<()> {
        let settings = self.current_service_settings();
        let (effective, trace) =
            capsem_core::settings_profiles::resolve_effective_vm_settings_with_corp(
                &settings, profile_id,
            )?;
        capsem_core::settings_profiles::write_vm_effective_settings(session_dir, &effective)
            .context("persist vm-effective settings")?;
        capsem_core::settings_profiles::write_vm_effective_trace(session_dir, &trace)
            .context("persist vm-effective trace")?;
        Ok(())
    }

    fn refresh_vm_effective_settings(&self, session_dir: &FsPath) -> Result<()> {
        self.refresh_vm_effective_settings_for_profile(session_dir, None)
    }

    fn telemetry_identity_env(
        &self,
        vm_id: &str,
        session_dir: &FsPath,
    ) -> Result<Vec<(String, String)>> {
        let settings = self.current_service_settings();
        let effective = capsem_core::settings_profiles::load_vm_effective_settings(session_dir)
            .context("load vm-effective settings for telemetry identity")?;
        let profile_revision = capsem_core::settings_profiles::load_installed_profile_revision(
            &settings.profiles,
            &effective.profile_id,
        )
        .context("load installed profile revision for telemetry identity")?
        .map(|record| record.revision);
        Ok(capsem_core::telemetry::child_identity_env_with_revision(
            vm_id,
            &effective.profile_id,
            profile_revision.as_deref(),
            &capsem_core::telemetry::host_user_id(),
        ))
    }

    fn vm_profile_pin(
        &self,
        session_dir: &FsPath,
        profile_revision: Option<String>,
        profile_payload_hash: Option<String>,
        base_assets: Option<SavedVmBaseAssets>,
    ) -> Result<SavedVmProfilePin> {
        let effective = capsem_core::settings_profiles::load_vm_effective_settings(session_dir)
            .context("load vm-effective settings for profile pin")?;
        let package_json = serde_json::to_vec(&effective.packages.value)
            .context("serialize package contract for profile pin")?;
        let settings = self.current_service_settings();
        let mut installed_revision =
            capsem_core::settings_profiles::load_complete_installed_profile_revision(
                &settings.profiles,
                &effective.profile_id,
            )
            .context("load complete installed profile revision for profile pin")?;
        if installed_revision.is_none() && settings.profiles != self.service_settings.profiles {
            installed_revision =
                capsem_core::settings_profiles::load_complete_installed_profile_revision(
                    &self.service_settings.profiles,
                    &effective.profile_id,
                )
                .context("load startup installed profile revision for profile pin")?;
        }
        let (profile_revision, profile_payload_hash) = installed_revision
            .map(|record| (Some(record.revision), Some(record.payload_hash)))
            .unwrap_or((profile_revision, profile_payload_hash));
        let profile_revision = profile_revision
            .filter(|revision| !revision.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "VM profile pin requires a signed profile catalog revision; reconcile the profile catalog before creating VMs"
                )
            })?;
        let profile_payload_hash = profile_payload_hash
            .filter(|hash| !hash.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "VM profile pin requires a signed profile payload hash; reconcile the profile catalog before creating VMs"
                )
            })?;
        let base_assets = base_assets.ok_or_else(|| {
            anyhow!("VM profile pin requires pinned asset identity from the signed profile catalog")
        })?;
        Ok(SavedVmProfilePin {
            profile_id: effective.profile_id,
            profile_revision: Some(profile_revision),
            profile_payload_hash: Some(profile_payload_hash),
            package_contract_hash: format!("blake3:{}", blake3::hash(&package_json).to_hex()),
            base_assets: Some(base_assets),
        })
    }

    fn resolve_vm_runtime_defaults(&self) -> VmRuntimeDefaults {
        self.resolve_vm_runtime_defaults_for(None)
    }

    fn resolve_vm_runtime_defaults_for(&self, profile_id: Option<&str>) -> VmRuntimeDefaults {
        let fallback_vm = capsem_core::settings_profiles::VmProfileSettings::default();
        let settings = self.current_service_settings();
        match capsem_core::settings_profiles::resolve_effective_vm_settings_with_corp(
            &settings, profile_id,
        ) {
            Ok((effective, _trace)) => VmRuntimeDefaults {
                ram_mb: effective.vm.value.memory_mib as u64,
                cpus: effective.vm.value.cpus as u32,
                max_concurrent_vms: DEFAULT_MAX_CONCURRENT_VMS,
            },
            Err(error) => {
                warn!(
                    error = %error,
                    profile_id,
                    "failed to resolve vm-effective defaults, using built-in profile defaults"
                );
                VmRuntimeDefaults {
                    ram_mb: fallback_vm.memory_mib as u64,
                    cpus: fallback_vm.cpus as u32,
                    max_concurrent_vms: DEFAULT_MAX_CONCURRENT_VMS,
                }
            }
        }
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
        match self.preserve_failed_session_dir_outcome(session_dir, id) {
            PreserveOutcome::Preserved(failed_dir) => {
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
            // AB-008: idempotent. An earlier preservation pass already
            // renamed or removed this dir, or the source was never there.
            // No log -- the previous code emitted two scary WARN lines
            // ("logs lost" + "orphaned on disk") that misrepresented an
            // already-handled case as a fresh failure. Multiple cleanup
            // paths (scrub_dead_process, the spawn-completion handler,
            // handle_run cleanup) can race for the same session dir.
            PreserveOutcome::AlreadyAbsent => {}
            PreserveOutcome::FailedAndRemoved { rename_error } => {
                warn!(
                    id,
                    from = %session_dir.display(),
                    error = %rename_error,
                    "failed to preserve session dir for post-mortem -- logs lost; removed to reclaim disk"
                );
            }
            PreserveOutcome::FailedAndOrphaned {
                rename_error,
                remove_error,
            } => {
                warn!(
                    id,
                    from = %session_dir.display(),
                    rename_error = %rename_error,
                    error = %remove_error,
                    "failed to preserve and failed to remove session dir -- orphaned on disk"
                );
            }
        }
    }

    /// Pure FS-effect classifier for [`Self::preserve_failed_session_dir`].
    ///
    /// Returns the outcome so tests can assert on it without capturing
    /// tracing output. Maps `ErrorKind::NotFound` from both the rename and
    /// the fallback `remove_dir_all` to [`PreserveOutcome::AlreadyAbsent`]
    /// so duplicate calls are idempotent. AB-008.
    pub(crate) fn preserve_failed_session_dir_outcome(
        &self,
        session_dir: &std::path::Path,
        id: &str,
    ) -> PreserveOutcome {
        let failed_id = format!(
            "{}-failed-{}",
            id,
            capsem_core::session::generate_session_id(),
        );
        let failed_dir = self.run_dir.join("sessions").join(&failed_id);
        match std::fs::rename(session_dir, &failed_dir) {
            Ok(()) => PreserveOutcome::Preserved(failed_dir),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PreserveOutcome::AlreadyAbsent,
            Err(rename_error) => match std::fs::remove_dir_all(session_dir) {
                Ok(()) => PreserveOutcome::FailedAndRemoved { rename_error },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    PreserveOutcome::AlreadyAbsent
                }
                Err(remove_error) => PreserveOutcome::FailedAndOrphaned {
                    rename_error,
                    remove_error,
                },
            },
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
            profile_id,
            profile_revision,
            description,
        } = options;

        let vm_defaults = self.resolve_vm_runtime_defaults();
        let max_concurrent_vms = vm_defaults.max_concurrent_vms;

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
        if from.is_some() && (profile_id.is_some() || profile_revision.is_some()) {
            return Err(anyhow!(
                "profile selection is only valid for fresh VM create; source clones inherit the source VM profile pin"
            ));
        }
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
        if let Some(ref entry) = source_entry {
            ensure_required_vm_profile_pin(
                entry.profile_pin.as_ref(),
                &format!("source VM \"{}\"", entry.name),
            )?;
        }
        let source_base_assets = source_entry
            .as_ref()
            .map(source_vm_base_assets)
            .transpose()?;

        // If cloning from a source sandbox, inherit its base_version.
        let version = if let Some(ref entry) = source_entry {
            entry.base_version.clone()
        } else {
            version_override.unwrap_or_else(|| self.current_version.clone())
        };
        let base_assets = if let Some(source_base_assets) = source_base_assets.clone() {
            Some(source_base_assets)
        } else if profile_id.is_some() || profile_revision.is_some() {
            let settings = self.current_service_settings();
            match profile_asset_requirement_for_selection(
                &settings,
                profile_id.as_deref(),
                profile_revision.as_deref(),
                host_asset_arch(),
                false,
            )? {
                AssetRequirement::Profile(required) => Some(required.base_assets()),
                AssetRequirement::DevLogical { .. } => None,
            }
        } else {
            self.current_base_assets()?
        };
        let inherited_profile_revision = source_entry
            .as_ref()
            .and_then(|entry| entry.profile_pin.as_ref())
            .and_then(|pin| pin.profile_revision.clone());
        let inherited_profile_payload_hash = source_entry
            .as_ref()
            .and_then(|entry| entry.profile_pin.as_ref())
            .and_then(|pin| pin.profile_payload_hash.clone());

        let resolved = if let (Some(entry), Some(base_assets)) =
            (source_entry.as_ref(), source_base_assets.as_ref())
        {
            saved_vm_assets::ensure_saved_base_assets_available(
                &entry.name,
                &self.assets_dir,
                base_assets,
            )?
        } else if profile_id.is_some() || profile_revision.is_some() {
            let settings = self.current_service_settings();
            match profile_asset_requirement_for_selection(
                &settings,
                profile_id.as_deref(),
                profile_revision.as_deref(),
                host_asset_arch(),
                false,
            )? {
                AssetRequirement::Profile(required) => {
                    let resolved = required.resolved_assets(&self.assets_dir);
                    let missing = [
                        ("vmlinuz", &resolved.kernel),
                        ("initrd.img", &resolved.initrd),
                        ("rootfs.squashfs", &resolved.rootfs),
                    ]
                    .into_iter()
                    .filter_map(|(name, path)| (!path.exists()).then_some(name))
                    .collect::<Vec<_>>();
                    if !missing.is_empty() {
                        return Err(anyhow!(
                            "selected profile VM assets are not ready (profile={}, revision={:?}, missing={missing:?})",
                            profile_id.as_deref().unwrap_or("default"),
                            profile_revision
                        ));
                    }
                    resolved
                }
                AssetRequirement::DevLogical { .. } => {
                    return Err(anyhow!(
                        "selected profile VM assets must come from a signed profile catalog"
                    ));
                }
            }
        } else {
            let health = self.asset_supervisor.snapshot();
            if !health.ready {
                return Err(anyhow!(
                    "VM assets are not ready (state={}, missing={:?}, error={})",
                    health.state.as_str(),
                    health.missing,
                    health.error.unwrap_or_else(|| "none".to_string())
                ));
            }
            self.resolve_asset_paths()?
        };
        for (name, path) in [
            ("vmlinuz", &resolved.kernel),
            ("initrd.img", &resolved.initrd),
            ("rootfs.squashfs", &resolved.rootfs),
        ] {
            if !path.exists() {
                error!(asset = name, path = %path.display(), "asset NOT FOUND after ready check");
                return Err(anyhow!(
                    "{} not found at {}; service asset state is stale",
                    name,
                    path.display()
                ));
            }
        }

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
        self.refresh_vm_effective_settings_for_profile(&session_dir, profile_id.as_deref())
            .context("attach vm-effective settings to session")?;
        let profile_pin = self
            .vm_profile_pin(
                &session_dir,
                inherited_profile_revision,
                inherited_profile_payload_hash,
                base_assets.clone(),
            )
            .context("pin VM profile/package/assets")?;
        if let Some(expected_profile_id) = profile_id.as_deref() {
            if profile_pin.profile_id != expected_profile_id {
                return Err(anyhow!(
                    "selected profile '{}' resolved to pinned profile '{}'",
                    expected_profile_id,
                    profile_pin.profile_id
                ));
            }
        }
        if let Some(expected_revision) = profile_revision.as_deref() {
            if profile_pin.profile_revision.as_deref() != Some(expected_revision) {
                return Err(anyhow!(
                    "selected profile revision '{}' resolved to pinned revision {:?}",
                    expected_revision,
                    profile_pin.profile_revision
                ));
            }
        }
        let telemetry_env = self
            .telemetry_identity_env(id, &session_dir)
            .context("derive process telemetry identity")?;

        info!(process_binary = %self.process_binary.display(), exists = self.process_binary.exists(), "checking process_binary");

        info!(id, version, asset_version = %resolved.asset_version, "spawning capsem-process");

        let mut child_cmd = tokio::process::Command::new(&self.process_binary);
        if !self.process_binary.exists() {
            info!(
                "process_binary does not exist at absolute path, trying target/debug/capsem-process"
            );
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
        // Profile V2 effective settings are attached to the session; no
        // host config file is forwarded into the VM process.
        child_cmd.env_clear();
        for key in PROCESS_ENV_ALLOWLIST {
            if let Ok(val) = std::env::var(key) {
                child_cmd.env(key, val);
            }
        }
        // W4/S07a: propagate trace context plus VM/profile/user identity.
        for (k, v) in telemetry_env {
            child_cmd.env(k, v);
        }

        if let Some(expected) = self.asset_supervisor.expected_hashes() {
            child_cmd
                .arg("--expected-kernel-hash")
                .arg(expected.kernel)
                .arg("--expected-initrd-hash")
                .arg(expected.initrd)
                .arg("--expected-rootfs-hash")
                .arg(expected.rootfs);
        }

        let mut child = child_cmd
            .env(
                "RUST_LOG",
                std::env::var("RUST_LOG")
                    .unwrap_or_else(|_| capsem_core::telemetry::with_subsys_targets("capsem=info")),
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
            .context("failed to spawn capsem-process")?;

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
                    if !info.persistent {
                        let session_dir = info.session_dir.clone();
                        let cleanup_path = session_dir.clone();
                        let cleanup = tokio::task::spawn_blocking(move || {
                            std::fs::remove_dir_all(&cleanup_path)
                        })
                        .await;
                        if let Err(e) = cleanup.unwrap_or_else(|join_err| {
                            Err(std::io::Error::other(format!(
                                "cleanup task failed: {join_err}"
                            )))
                        }) {
                            tracing::warn!(
                                id_clone,
                                path = %session_dir.display(),
                                error = %e,
                                "failed to remove clean ephemeral session dir"
                            );
                        }
                    }
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
                base_assets: base_assets.clone(),
                profile_pin: Some(profile_pin.clone()),
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
                base_assets,
                profile_pin: Some(profile_pin),
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
        if entry.profile_pin.is_none() {
            return Err(anyhow!(
                "persistent VM \"{name}\" is missing required profile pin; recreate the VM from a signed profile"
            ));
        }
        ensure_required_vm_profile_pin(
            entry.profile_pin.as_ref(),
            &format!("persistent VM \"{name}\""),
        )?;
        if entry.base_assets.is_none() {
            return Err(anyhow!(
                "persistent VM \"{name}\" is missing required pinned asset identity; recreate the VM from a signed profile"
            ));
        }

        let ram_mb = ram_mb_override.unwrap_or(entry.ram_mb);
        let cpus = cpus_override.unwrap_or(entry.cpus);
        let version = entry.base_version.clone();
        let base_assets = entry.base_assets.clone();
        let profile_pin = entry.profile_pin.clone();

        info!(name, version, "resume_sandbox: re-spawning process");

        let uds_path = self.instance_socket_path(name);
        let _ = std::fs::create_dir_all(uds_path.parent().unwrap());

        // Clear stale UDS + ready sentinel from the prior boot. Without this,
        // wait_for_vm_ready returns instantly against the old .ready file and
        // callers race ahead before the resumed agent has reconnected.
        let _ = std::fs::remove_file(&uds_path);
        let _ = std::fs::remove_file(uds_path.with_extension("ready"));

        let resolved = if let Some(ref base_assets) = entry.base_assets {
            saved_vm_assets::ensure_saved_base_assets_available(
                name,
                &self.assets_dir,
                base_assets,
            )?
        } else {
            let health = self.asset_supervisor.snapshot();
            if !health.ready {
                return Err(anyhow!(
                    "VM assets are not ready (state={}, missing={:?}, error={})",
                    health.state.as_str(),
                    health.missing,
                    health.error.unwrap_or_else(|| "none".to_string())
                ));
            }
            self.resolve_asset_paths()?
        };
        if !resolved.rootfs.exists() {
            return Err(anyhow!("rootfs not found at {}", resolved.rootfs.display()));
        }
        self.ensure_vm_effective_settings(&entry.session_dir)
            .context("attach vm-effective settings to resumed session")?;
        let telemetry_env = self
            .telemetry_identity_env(name, &entry.session_dir)
            .context("derive resumed process telemetry identity")?;

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
        // Profile V2 effective settings are attached to the session; no
        // host config file is forwarded into the VM process.
        child_cmd.env_clear();
        for key in PROCESS_ENV_ALLOWLIST {
            if let Ok(val) = std::env::var(key) {
                child_cmd.env(key, val);
            }
        }
        // W4/S07a: propagate trace context plus VM/profile/user identity.
        for (k, v) in telemetry_env {
            child_cmd.env(k, v);
        }

        if let Some(expected) = self.asset_supervisor.expected_hashes() {
            child_cmd
                .arg("--expected-kernel-hash")
                .arg(expected.kernel)
                .arg("--expected-initrd-hash")
                .arg(expected.initrd)
                .arg("--expected-rootfs-hash")
                .arg(expected.rootfs);
        }

        let mut child = child_cmd
            .env(
                "RUST_LOG",
                std::env::var("RUST_LOG")
                    .unwrap_or_else(|_| capsem_core::telemetry::with_subsys_targets("capsem=info")),
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
            .context("failed to spawn capsem-process")?;

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
                base_assets,
                profile_pin,
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
        self.asset_supervisor.resolve_asset_paths()
    }

    fn current_base_assets(&self) -> Result<Option<SavedVmBaseAssets>> {
        Ok(self.asset_supervisor.current_base_assets())
    }

    async fn ensure_current_profile_assets_ready(&self) -> Result<AssetHealth> {
        self.asset_supervisor.ensure_assets_once().await;
        let health = self.asset_supervisor.snapshot();
        if !health.ready {
            return Err(anyhow!(
                "VM assets are not ready (state={}, missing={:?}, error={})",
                health.state.as_str(),
                health.missing,
                health.error.unwrap_or_else(|| "none".to_string())
            ));
        }
        Ok(health)
    }

    async fn ensure_selected_profile_assets_ready(
        &self,
        profile_id: Option<&str>,
        profile_revision: Option<&str>,
    ) -> Result<AssetHealth> {
        if profile_id.is_none() && profile_revision.is_none() {
            return self.ensure_current_profile_assets_ready().await;
        }
        let settings = self.current_service_settings();
        let requirement = profile_asset_requirement_for_selection(
            &settings,
            profile_id,
            profile_revision,
            host_asset_arch(),
            false,
        )?;
        let supervisor = AssetSupervisor::new(
            self.assets_dir.clone(),
            requirement,
            std::time::Duration::from_secs(60),
        );
        supervisor.ensure_assets_once().await;
        let health = supervisor.snapshot();
        if !health.ready {
            return Err(anyhow!(
                "selected profile VM assets are not ready after reconcile (profile={:?}, revision={:?}, state={}, missing={:?}, error={})",
                health.profile_id,
                health.profile_revision,
                health.state.as_str(),
                health.missing,
                health.error.unwrap_or_else(|| "none".to_string())
            ));
        }
        Ok(health)
    }

    fn asset_health_snapshot(&self) -> AssetHealth {
        let mut health = self.asset_supervisor.snapshot();
        health.saved_vm_dependencies = {
            let registry = self.persistent_registry.lock().unwrap();
            saved_vm_assets::saved_vm_dependency_issues(&registry, &self.assets_dir)
        };
        health
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
    let (session_dir, ram_mb, cpus, base_version, base_assets, source_profile_pin, uds_path) = {
        let instances = state.instances.lock().unwrap();
        if let Some(i) = instances.get(&id) {
            (
                i.session_dir.clone(),
                i.ram_mb,
                i.cpus,
                i.base_version.clone(),
                i.base_assets.clone(),
                i.profile_pin.clone(),
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
                    p.base_assets.clone(),
                    p.profile_pin.clone(),
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
    ensure_required_vm_profile_pin(source_profile_pin.as_ref(), &format!("source VM \"{id}\""))
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;
    let base_assets =
        source_pin_base_assets(&id, source_profile_pin.as_ref(), base_assets.as_ref())
            .map(Some)
            .map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;

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

    state
        .ensure_vm_effective_settings(&new_session_dir)
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("fork: failed to attach vm-effective settings: {e:#}"),
            )
        })?;
    let profile_pin = state
        .vm_profile_pin(
            &new_session_dir,
            source_profile_pin
                .as_ref()
                .and_then(|pin| pin.profile_revision.clone()),
            source_profile_pin
                .as_ref()
                .and_then(|pin| pin.profile_payload_hash.clone()),
            base_assets.clone(),
        )
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("fork: failed to pin profile: {e:#}"),
            )
        })?;
    ensure_fork_profile_pin_matches_source(
        &profile_pin,
        source_profile_pin
            .as_ref()
            .expect("source pin was validated above"),
        &id,
    )
    .map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;

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
                base_assets,
                profile_pin: Some(profile_pin),
            })
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(Json(ForkResponse {
        name: name.clone(),
        size_bytes,
    }))
}

fn ensure_required_vm_profile_pin(pin: Option<&SavedVmProfilePin>, subject: &str) -> Result<()> {
    let Some(pin) = pin else {
        return Err(anyhow!(
            "{subject} is missing required profile pin; required profile revision pin must come from a signed profile"
        ));
    };
    if pin
        .profile_revision
        .as_deref()
        .is_none_or(|revision| revision.trim().is_empty())
    {
        return Err(anyhow!(
            "{subject} is missing required profile revision pin; recreate the VM from a signed profile"
        ));
    }
    if pin
        .profile_payload_hash
        .as_deref()
        .is_none_or(|hash| hash.trim().is_empty())
    {
        return Err(anyhow!(
            "{subject} is missing required profile payload hash; recreate the VM from a signed profile"
        ));
    }
    if pin.base_assets.is_none() {
        return Err(anyhow!(
            "{subject} is missing required pinned asset identity; recreate the VM from a signed profile"
        ));
    }
    Ok(())
}

fn source_pin_base_assets(
    source_id: &str,
    pin: Option<&SavedVmProfilePin>,
    stored_assets: Option<&SavedVmBaseAssets>,
) -> Result<SavedVmBaseAssets> {
    let pin = pin.ok_or_else(|| {
        anyhow!(
            "source VM \"{source_id}\" is missing required profile pin; required profile revision pin must come from a signed profile"
        )
    })?;
    let pinned_assets = pin.base_assets.as_ref().ok_or_else(|| {
        anyhow!(
            "source VM \"{source_id}\" is missing required pinned asset identity; recreate the VM from a signed profile"
        )
    })?;
    if let Some(stored_assets) = stored_assets {
        if stored_assets != pinned_assets {
            return Err(anyhow!(
                "source VM \"{source_id}\" has conflicting pinned asset identity; profile pin and VM registry base assets must match"
            ));
        }
    }
    Ok(pinned_assets.clone())
}

fn source_vm_base_assets(entry: &PersistentVmEntry) -> Result<SavedVmBaseAssets> {
    source_pin_base_assets(
        &entry.name,
        entry.profile_pin.as_ref(),
        entry.base_assets.as_ref(),
    )
}

fn ensure_fork_profile_pin_matches_source(
    fork_pin: &SavedVmProfilePin,
    source_pin: &SavedVmProfilePin,
    source_id: &str,
) -> Result<()> {
    if fork_pin.profile_id != source_pin.profile_id {
        return Err(anyhow!(
            "profile drift detected while forking source VM \"{source_id}\": cloned profile id '{}' does not match pinned profile id '{}'",
            fork_pin.profile_id,
            source_pin.profile_id
        ));
    }
    if fork_pin.profile_revision != source_pin.profile_revision {
        return Err(anyhow!(
            "profile drift detected while forking source VM \"{source_id}\": cloned profile revision {:?} does not match pinned profile revision {:?}",
            fork_pin.profile_revision,
            source_pin.profile_revision
        ));
    }
    if fork_pin.profile_payload_hash != source_pin.profile_payload_hash {
        return Err(anyhow!(
            "profile drift detected while forking source VM \"{source_id}\": cloned profile payload hash does not match pinned profile payload hash"
        ));
    }
    if fork_pin.package_contract_hash != source_pin.package_contract_hash {
        return Err(anyhow!(
            "profile drift detected while forking source VM \"{source_id}\": cloned package contract does not match pinned package contract"
        ));
    }
    if fork_pin.base_assets != source_pin.base_assets {
        return Err(anyhow!(
            "profile drift detected while forking source VM \"{source_id}\": cloned asset identity does not match pinned asset identity"
        ));
    }
    Ok(())
}

/// Outcome of a single provision attempt inside `handle_provision`.
/// `LaunchdTransient` is the recoverable case: VZ rejected the fresh
/// VM with the misleading entitlement string while launchd's
/// PETRIFIED-cleanup queue was draining. The poll_until loop retries
/// on this; everything else (incl. `Other`) bubbles up unchanged.
#[derive(Debug)]
enum ProvisionAttemptOutcome {
    Ready {
        uds_path: PathBuf,
        asset_health: AssetHealth,
    },
    StillBootingTimedOut {
        uds_path: PathBuf,
        asset_health: AssetHealth,
    }, // 5s envelope hit; treat as success per pre-existing contract
    LaunchdTransient,
    BootCrash {
        tail: String,
    },
    ProvisionError(anyhow::Error),
}

/// Decision the retry loop takes after observing one provision attempt.
/// Pure function of the outcome -- no side effects -- so the
/// retry-routing can be unit-tested without spawning a real VM.
#[derive(Debug)]
enum AttemptDecision {
    Succeed {
        uds_path: PathBuf,
        asset_health: Box<AssetHealth>,
    },
    BailWithError(AppError),
    RetryAfterCleanup,
}

/// Map a single attempt's outcome to the retry loop's next move.
/// The `LaunchdTransient` variant is the only one that triggers retry;
/// `BootCrash` and `ProvisionError` bail with structured errors that
/// match the pre-refactor handle_provision response shape.
fn classify_attempt_decision(outcome: ProvisionAttemptOutcome, id: &str) -> AttemptDecision {
    match outcome {
        ProvisionAttemptOutcome::Ready {
            uds_path,
            asset_health,
        }
        | ProvisionAttemptOutcome::StillBootingTimedOut {
            uds_path,
            asset_health,
        } => AttemptDecision::Succeed {
            uds_path,
            asset_health: Box::new(asset_health),
        },
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
    let id = payload.name.clone().unwrap_or_else(|| {
        let existing: Vec<String> = state.instances.lock().unwrap().keys().cloned().collect();
        generate_tmp_name(existing.iter().map(|s| s.as_str()))
    });

    // Missing ram_mb/cpus fall back to the selected profile VM settings.
    let vm_defaults = state.resolve_vm_runtime_defaults_for(payload.profile_id.as_deref());
    let ram_mb = payload.ram_mb.unwrap_or(vm_defaults.ram_mb);
    let cpus = payload.cpus.unwrap_or(vm_defaults.cpus);

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
        let payload_profile_id = payload.profile_id.clone();
        let payload_profile_revision = payload.profile_revision.clone();
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
                payload_profile_id,
                payload_profile_revision,
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
                AttemptDecision::Succeed {
                    uds_path,
                    asset_health,
                } => Some(Ok((uds_path, *asset_health))),
                AttemptDecision::RetryAfterCleanup => None, // poll_until retries
                AttemptDecision::BailWithError(err) => Some(Err(err)),
            }
        }
    })
    .await;

    match result {
        Ok(Ok((uds_path, asset_health))) => Ok(Json(provision_response_for_instance(
            &state,
            id,
            uds_path,
            Some(asset_health),
        ))),
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

fn provision_response_for_instance(
    state: &Arc<ServiceState>,
    id: String,
    uds_path: PathBuf,
    asset_health: Option<AssetHealth>,
) -> ProvisionResponse {
    let profile_pin = {
        let instances = state.instances.lock().unwrap();
        instances
            .get(&id)
            .and_then(|instance| instance.profile_pin.clone())
    };
    let profile_id = profile_pin.as_ref().map(|pin| pin.profile_id.clone());
    let profile_revision = profile_pin
        .as_ref()
        .and_then(|pin| pin.profile_revision.clone());
    let profile_status = {
        let settings = state.current_service_settings();
        let catalog = load_vm_profile_catalog_snapshot(&settings);
        Some(vm_profile_status(profile_pin.as_ref(), &catalog))
    };

    ProvisionResponse {
        id,
        uds_path: Some(uds_path),
        profile_id,
        profile_revision,
        profile_status,
        profile_pin,
        asset_health: asset_health.or_else(|| Some(state.asset_health_snapshot())),
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
    profile_id: Option<String>,
    profile_revision: Option<String>,
) -> ProvisionAttemptOutcome {
    let asset_health = if from.is_none() {
        match state
            .ensure_selected_profile_assets_ready(
                profile_id.as_deref(),
                profile_revision.as_deref(),
            )
            .await
        {
            Ok(health) => health,
            Err(e) => return ProvisionAttemptOutcome::ProvisionError(e),
        }
    } else {
        state.asset_health_snapshot()
    };
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
            profile_id,
            profile_revision,
            description: None,
        })
    })
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return ProvisionAttemptOutcome::ProvisionError(anyhow::anyhow!("provision task: {e}"));
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
            return ProvisionAttemptOutcome::Ready {
                uds_path,
                asset_health,
            };
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
                warn!(
                    id,
                    "provision: detected launchd-cleanup transient (misleading 'entitlement' error)"
                );
                ProvisionAttemptOutcome::LaunchdTransient
            } else {
                ProvisionAttemptOutcome::BootCrash { tail }
            };
        }
        if tokio::time::Instant::now() >= deadline {
            return ProvisionAttemptOutcome::StillBootingTimedOut {
                uds_path,
                asset_health,
            };
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Attach durable telemetry from session.db to a SandboxInfo.
///
/// Used by single-VM detail paths only. `/list` is a hot status path and must
/// not scan per-VM SQLite files; live counters belong in capsem-process and
/// should arrive through typed IPC snapshots.
fn enrich_telemetry_from_session_db(info: &mut SandboxInfo, session_dir: &std::path::Path) {
    let db_path = session_dir.join("session.db");
    if let Ok(reader) = capsem_logger::DbReader::open(&db_path) {
        if let Ok(Some(identity)) = reader.session_identity() {
            info.vm_id = Some(identity.vm_id);
            info.profile_id = Some(identity.profile_id);
            info.user_id = Some(identity.user_id);
        }
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

fn attach_metrics_snapshot(info: &mut SandboxInfo, snapshot: &VmMetricsSnapshot) {
    info.total_requests = Some(snapshot.http.http_requests_total);
    info.allowed_requests = Some(snapshot.http.http_requests_allowed_total);
    info.denied_requests = Some(snapshot.http.http_requests_denied_total);
    info.total_dns_queries = Some(snapshot.dns.dns_queries_total);
    info.denied_dns_queries = Some(snapshot.dns.dns_queries_denied_total);
    info.total_input_tokens = Some(snapshot.model.model_input_tokens_total);
    info.total_output_tokens = Some(snapshot.model.model_output_tokens_total);
    info.total_estimated_cost =
        Some(snapshot.model.model_estimated_cost_micros_total as f64 / 1_000_000.0);
    info.model_call_count = Some(snapshot.model.model_requests_total);
    info.total_mcp_calls = Some(snapshot.mcp.mcp_tool_invocations_total);
    info.total_file_events = Some(
        snapshot.filesystem.fs_reads_total
            + snapshot.filesystem.fs_writes_total
            + snapshot.filesystem.fs_creates_total
            + snapshot.filesystem.fs_deletes_total
            + snapshot.filesystem.fs_restores_total,
    );
    info.process_event_count = Some(snapshot.process.process_events_total);
    info.process_exec_count = Some(snapshot.process.process_exec_total);
    info.security_events_total = Some(snapshot.security.security_events_total);
    info.enforcement_decisions_total = Some(snapshot.security.enforcement_decisions_total);
    info.detection_findings_total = Some(snapshot.security.detection_findings_total);
    info.blocks_total = Some(snapshot.security.blocks_total);
    info.latest_block_event_id = snapshot.security.latest_block_event_id.clone();
    info.latest_block_rule_id = snapshot.security.latest_block_rule_id.clone();
    info.latest_block_reason = snapshot.security.latest_block_reason.clone();
    info.latest_detection_event_id = snapshot.security.latest_detection_event_id.clone();
    info.latest_detection_rule_id = snapshot.security.latest_detection_rule_id.clone();
    info.latest_detection_title = snapshot.security.latest_detection_title.clone();
    info.latest_detection_severity = snapshot.security.latest_detection_severity.clone();
}

async fn live_metrics_snapshot_for_vm(
    state: &Arc<ServiceState>,
    id: &str,
    uds_path: &std::path::Path,
) -> Option<VmMetricsSnapshot> {
    let request_id = state.next_job_id();
    match send_ipc_command(
        uds_path,
        ServiceToProcess::GetMetricsSnapshot { id: request_id },
        Some(2),
    )
    .await
    {
        Ok(ProcessToService::MetricsSnapshot {
            id: snapshot_id,
            snapshot,
        }) if snapshot_id == request_id => Some(*snapshot),
        Ok(ProcessToService::MetricsSnapshot {
            id: snapshot_id, ..
        }) => {
            warn!(
                vm_id = %id,
                expected = request_id,
                got = snapshot_id,
                "metrics snapshot id mismatch"
            );
            None
        }
        Ok(other) => {
            warn!(vm_id = %id, response = ?other, "unexpected metrics snapshot response");
            None
        }
        Err(error) => {
            warn!(vm_id = %id, error = %error, "failed to collect live VM metrics snapshot");
            None
        }
    }
}

struct VmProfileCatalogSnapshot {
    roots: capsem_core::settings_profiles::ProfileRootSettings,
    manifest: Option<capsem_core::profile_manifest::ProfileManifest>,
}

fn profile_catalog_manifest_path(
    settings: &capsem_core::settings_profiles::ServiceSettings,
) -> Option<PathBuf> {
    settings
        .profiles
        .corp_dirs
        .first()
        .map(|corp_dir| corp_dir.join(".catalog").join("profile-manifest.json"))
}

fn load_vm_profile_catalog_snapshot(
    settings: &capsem_core::settings_profiles::ServiceSettings,
) -> VmProfileCatalogSnapshot {
    let manifest = profile_catalog_manifest_path(settings)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|content| {
            capsem_core::profile_manifest::ProfileManifest::from_json(&content).ok()
        });
    VmProfileCatalogSnapshot {
        roots: settings.profiles.clone(),
        manifest,
    }
}

fn persist_profile_catalog_manifest(
    settings: &capsem_core::settings_profiles::ServiceSettings,
    manifest_json: &str,
) -> Result<(), AppError> {
    let path = profile_catalog_manifest_path(settings).ok_or_else(|| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "no corp profile directory is configured".into(),
        )
    })?;
    let parent = path.parent().ok_or_else(|| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "profile catalog manifest path has no parent: {}",
                path.display()
            ),
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create profile catalog manifest directory: {error}"),
        )
    })?;
    std::fs::write(&path, manifest_json).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write profile catalog manifest {}: {error}", path.display()),
        )
    })
}

fn vm_profile_status(
    pin: Option<&SavedVmProfilePin>,
    catalog: &VmProfileCatalogSnapshot,
) -> VmProfileStatus {
    let Some(pin) = pin else {
        return VmProfileStatus::Corrupted;
    };
    let Some(revision) = pin.profile_revision.as_deref() else {
        return VmProfileStatus::Corrupted;
    };

    if let Some(manifest) = &catalog.manifest {
        let Ok(record) = manifest.revision(&pin.profile_id, revision) else {
            return VmProfileStatus::Corrupted;
        };
        return match record.record.status {
            capsem_core::profile_manifest::ProfileRevisionStatus::Deprecated => {
                VmProfileStatus::Deprecated
            }
            capsem_core::profile_manifest::ProfileRevisionStatus::Revoked => {
                VmProfileStatus::Revoked
            }
            capsem_core::profile_manifest::ProfileRevisionStatus::Active => {
                match manifest.current_revision(&pin.profile_id) {
                    Ok(current) if current.revision == revision => VmProfileStatus::Current,
                    Ok(_) => VmProfileStatus::NeedsUpdate,
                    Err(_) => VmProfileStatus::Corrupted,
                }
            }
        };
    }

    match capsem_core::settings_profiles::load_installed_profile_revision(
        &catalog.roots,
        &pin.profile_id,
    ) {
        Ok(Some(installed)) if installed.revision == revision => VmProfileStatus::Current,
        Ok(Some(_)) => VmProfileStatus::NeedsUpdate,
        Ok(None) => VmProfileStatus::Unknown,
        Err(_) => VmProfileStatus::Unknown,
    }
}

fn attach_vm_profile_status(
    info: &mut SandboxInfo,
    pin: Option<&SavedVmProfilePin>,
    catalog: &VmProfileCatalogSnapshot,
) {
    info.profile_status = Some(vm_profile_status(pin, catalog));
    if let Some(pin) = pin {
        info.profile_id = Some(pin.profile_id.clone());
        info.profile_revision = pin.profile_revision.clone();
    }
}

async fn handle_list(State(state): State<Arc<ServiceState>>) -> Json<ListResponse> {
    let mut sandboxes: Vec<SandboxInfo> = Vec::new();
    let profile_catalog = load_vm_profile_catalog_snapshot(&state.service_settings);

    // Running instances. Keep this path in-memory only; durable session.db
    // telemetry is intentionally reserved for single-VM/detail paths.
    {
        let running: Vec<InstanceInfo> =
            state.instances.lock().unwrap().values().cloned().collect();
        for i in running {
            let mut info = SandboxInfo::new(i.id.clone(), i.pid, "Running".into(), i.persistent);
            info.name = if i.persistent {
                Some(i.id.clone())
            } else {
                None
            };
            info.ram_mb = Some(i.ram_mb);
            info.cpus = Some(i.cpus);
            info.version = Some(i.base_version.clone());
            info.base_assets = i.base_assets.clone();
            info.profile_pin = i.profile_pin.clone();
            attach_vm_profile_status(&mut info, i.profile_pin.as_ref(), &profile_catalog);
            info.forked_from = i.forked_from.clone();
            info.uptime_secs = Some(i.start_time.elapsed().as_secs());
            if let Some(snapshot) = live_metrics_snapshot_for_vm(&state, &i.id, &i.uds_path).await {
                attach_metrics_snapshot(&mut info, &snapshot);
            }
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
                info.base_assets = entry.base_assets.clone();
                info.profile_pin = entry.profile_pin.clone();
                attach_vm_profile_status(&mut info, entry.profile_pin.as_ref(), &profile_catalog);
                info.forked_from = entry.forked_from.clone();
                info.description = entry.description.clone();
                if entry.defunct {
                    info.last_error = entry.last_error.clone();
                }
                sandboxes.push(info);
            }
        }
    }

    let asset_health = Some(state.asset_health_snapshot());

    Json(ListResponse {
        sandboxes,
        asset_health,
    })
}

async fn handle_debug_report(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<debug_report::DebugReport>, AppError> {
    let (running_vm_count, total_vm_count, defunct_sessions) = {
        let instances = state.instances.lock().unwrap();
        let running_ids: HashSet<String> = instances.keys().cloned().collect();
        let running = running_ids.len();
        drop(instances);

        let registry = state.persistent_registry.lock().unwrap();
        let stopped_or_suspended = registry
            .list()
            .filter(|entry| !running_ids.contains(&entry.name))
            .count();
        let defunct_sessions: Vec<debug_report::DefunctSessionReport> = registry
            .list()
            .filter(|entry| entry.defunct)
            .map(|entry| debug_report::DefunctSessionReport {
                name: entry.name.clone(),
                last_error: entry.last_error.clone(),
            })
            .collect();
        (running, running + stopped_or_suspended, defunct_sessions)
    };
    let resolved_assets = state
        .resolve_asset_paths()
        .map(|resolved| debug_report::StatusResolvedAssets {
            kernel: resolved.kernel,
            initrd: resolved.initrd,
            rootfs: resolved.rootfs,
        })
        .map_err(|e| e.to_string());
    let status_issues = debug_report::status_issues(debug_report::StatusIssuesInput {
        gateway_port_file_exists: state.run_dir.join("gateway.port").exists(),
        gateway_token_file_exists: state.run_dir.join("gateway.token").exists(),
        assets_dir_exists: state.assets_dir.exists(),
        resolved_assets,
        defunct_session_count: defunct_sessions.len(),
    });

    let generated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| secs_to_rfc3339(d.as_secs()))
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into());
    let install = debug_report::default_install_report_input();
    let current_exe = install
        .as_ref()
        .map(|input| input.current_exe.clone())
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("capsem-service"));
    let process_pids = debug_report::default_process_report_inputs(&state.run_dir, &current_exe);
    let capsem_home = capsem_core::paths::capsem_home();
    let settings_profiles = build_settings_profiles_debug_snapshot(&capsem_home);

    let report = debug_report::build_debug_report(debug_report::DebugReportInput {
        generated_at,
        version: state.current_version.clone(),
        build_hash: option_env!("CAPSEM_BUILD_HASH")
            .unwrap_or("dev")
            .to_string(),
        build_ts: option_env!("CAPSEM_BUILD_TS").unwrap_or("dev").to_string(),
        platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
        capsem_home,
        run_dir: state.run_dir.clone(),
        assets_dir: state.assets_dir.clone(),
        asset_locations: Some(state.asset_locations.clone()),
        asset_health: Some(state.asset_health_snapshot()),
        running_vm_count,
        total_vm_count,
        status_issues,
        defunct_sessions,
        install,
        process_pids,
        settings_profiles: Some(settings_profiles),
    })
    .map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to build debug report: {e:#}"),
        )
    })?;

    Ok(Json(report))
}

fn build_settings_profiles_debug_snapshot(
    capsem_home: &FsPath,
) -> capsem_core::settings_profiles::SettingsProfilesDebugSnapshot {
    let service_settings_path = capsem_home.join("service.toml");
    let result = (|| {
        let settings = capsem_core::settings_profiles::load_service_settings_or_default(
            &service_settings_path,
        )?;
        let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)?;
        let (effective, trace) =
            capsem_core::settings_profiles::resolve_effective_vm_settings_with_corp(
                &settings, None,
            )?;
        Ok::<_, capsem_core::settings_profiles::SettingsProfilesError>(
            capsem_core::settings_profiles::SettingsProfilesDebugSnapshot::from_parts_with_trace(
                &settings,
                &catalog,
                Some(&effective),
                Some(&trace),
            ),
        )
    })();

    result.unwrap_or_else(|error| {
        capsem_core::settings_profiles::SettingsProfilesDebugSnapshot::from_error(error.to_string())
    })
}

async fn handle_info(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<SandboxInfo>, AppError> {
    let profile_catalog = load_vm_profile_catalog_snapshot(&state.service_settings);
    // Check running instances first
    {
        let (instance_data, session_dir, uds_path) = {
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
                    info.base_assets = i.base_assets.clone();
                    info.profile_pin = i.profile_pin.clone();
                    attach_vm_profile_status(&mut info, i.profile_pin.as_ref(), &profile_catalog);
                    info.forked_from = i.forked_from.clone();
                    info.uptime_secs = Some(i.start_time.elapsed().as_secs());
                    (
                        Some(info),
                        Some(i.session_dir.clone()),
                        Some(i.uds_path.clone()),
                    )
                }
                None => (None, None, None),
            }
        };
        if let (Some(mut info), Some(dir)) = (instance_data, session_dir) {
            enrich_telemetry_from_session_db(&mut info, &dir);
            if let Some(uds_path) = uds_path {
                if let Some(snapshot) = live_metrics_snapshot_for_vm(&state, &id, &uds_path).await {
                    attach_metrics_snapshot(&mut info, &snapshot);
                }
            }
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
            info.base_assets = entry.base_assets.clone();
            info.profile_pin = entry.profile_pin.clone();
            attach_vm_profile_status(&mut info, entry.profile_pin.as_ref(), &profile_catalog);
            info.forked_from = entry.forked_from.clone();
            info.description = entry.description.clone();
            if entry.defunct {
                info.last_error = entry.last_error.clone();
            }
            info.size_bytes =
                capsem_core::auto_snapshot::sandbox_disk_usage(&entry.session_dir).ok();
            enrich_telemetry_from_session_db(&mut info, &entry.session_dir);
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
                            ));
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

    // F6/T6: when `id` is set, query session.db for session-scoped error
    // signals. Best-effort -- a missing or vacuumed DB just leaves the
    // session block empty, the host-side triage still returns. Persistent
    // stopped sessions are supported through the registry resolver.
    let session_block = if let Some(ref vm_id) = params.id {
        if let Ok(session_dir) = resolve_session_dir(&state, vm_id) {
            let path = session_dir.join("session.db");
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
        "SELECT timestamp, domain, decision, status_code, duration_ms, \
                policy_mode, policy_action, policy_rule, policy_reason, trace_id \
         FROM net_events WHERE decision = 'denied' OR status_code >= 500 \
         ORDER BY timestamp DESC LIMIT {limit}"
    );
    let mcp_errors_sql = format!(
        "SELECT timestamp, server_name, method, decision, policy_mode, policy_action, \
                policy_rule, policy_reason, error_message, duration_ms, trace_id \
         FROM mcp_calls WHERE decision IN ('denied','error') OR error_message IS NOT NULL \
         ORDER BY timestamp DESC LIMIT {limit}"
    );
    let exec_failures_sql = format!(
        "SELECT timestamp, exec_id, command, exit_code, duration_ms, trace_id \
         FROM exec_events WHERE exit_code IS NOT NULL AND exit_code != 0 \
         ORDER BY timestamp DESC LIMIT {limit}"
    );
    let dns_issues_sql = format!(
        "SELECT timestamp, qname, rcode, decision, matched_rule, policy_mode, \
                policy_action, policy_rule, policy_reason, trace_id \
         FROM dns_events WHERE decision != 'allowed' OR rcode != 0 \
         ORDER BY timestamp DESC LIMIT {limit}"
    );
    let audit_failures_sql = format!(
        "SELECT a.timestamp, a.pid, a.ppid, a.uid, a.exe, a.comm, a.argv, \
                COALESCE(a.exit_code, e.exit_code) AS exit_code, a.audit_id, \
                a.exec_event_id, a.trace_id \
         FROM audit_events a \
         LEFT JOIN exec_events e ON a.exec_event_id = e.exec_id \
         WHERE COALESCE(a.exit_code, e.exit_code) IS NOT NULL \
           AND COALESCE(a.exit_code, e.exit_code) != 0 \
         ORDER BY a.timestamp DESC LIMIT {limit}"
    );
    let security_decisions_sql = format!(
        "SELECT se.timestamp, se.event_id, se.event_type, se.final_action, \
                se.finding_count, se.trace_id, steps.kind, steps.status, \
                steps.rule_id, steps.pack_id, steps.message \
         FROM security_events se \
         LEFT JOIN security_event_steps steps ON steps.event_id = se.event_id \
         WHERE se.final_action != 'continue' \
            OR se.finding_count > 0 \
            OR steps.status = 'error' \
         ORDER BY se.timestamp DESC, steps.step_index ASC LIMIT {limit}"
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
    let dns_issues = reader
        .query_raw(&dns_issues_sql)
        .unwrap_or_else(|_| "[]".into());
    let audit_failures = reader
        .query_raw(&audit_failures_sql)
        .unwrap_or_else(|_| "[]".into());
    let security_decisions = reader
        .query_raw(&security_decisions_sql)
        .unwrap_or_else(|_| "[]".into());

    let denied_net_v: serde_json::Value = serde_json::from_str(&denied_net).unwrap_or_default();
    let mcp_errors_v: serde_json::Value = serde_json::from_str(&mcp_errors).unwrap_or_default();
    let exec_failures_v: serde_json::Value =
        serde_json::from_str(&exec_failures).unwrap_or_default();
    let dns_issues_v: serde_json::Value = serde_json::from_str(&dns_issues).unwrap_or_default();
    let audit_failures_v: serde_json::Value =
        serde_json::from_str(&audit_failures).unwrap_or_default();
    let security_decisions_v: serde_json::Value =
        serde_json::from_str(&security_decisions).unwrap_or_default();

    Ok(serde_json::json!({
        "denied_net": denied_net_v,
        "dns_issues": dns_issues_v,
        "mcp_errors": mcp_errors_v,
        "exec_failures": exec_failures_v,
        "audit_failures": audit_failures_v,
        "security_decisions": security_decisions_v,
    }))
}

#[derive(Deserialize, Debug, Default)]
struct TriageQuery {
    /// Lookback window. Default "30m". Accepts "5m", "1h", "24h", or
    /// RFC3339 ("2026-05-02T17:30:00Z").
    since: Option<String>,
    /// Max items per category. Default 20, capped at 200.
    limit: Option<usize>,
    /// Optional session id for session.db cross-reference.
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
                if matches!(
                    cmd,
                    ServiceToProcess::Ping | ServiceToProcess::ReloadConfig { .. }
                ) {
                    return Ok(ProcessToService::Pong);
                }
                continue;
            }
            ProcessToService::ReloadConfigResult { success, error } => {
                if matches!(cmd, ServiceToProcess::ReloadConfig { .. }) {
                    return Ok(ProcessToService::ReloadConfigResult { success, error });
                }
                continue;
            }
            ProcessToService::RuntimeRuleMatches { id, matches } => {
                if matches!(cmd, ServiceToProcess::DrainRuntimeRuleMatches { .. }) {
                    return Ok(ProcessToService::RuntimeRuleMatches { id, matches });
                }
                continue;
            }
            ProcessToService::MetricsSnapshot { id, snapshot } => {
                if matches!(cmd, ServiceToProcess::GetMetricsSnapshot { .. }) {
                    return Ok(ProcessToService::MetricsSnapshot { id, snapshot });
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
    .await;
    if died.load(std::sync::atomic::Ordering::Acquire) {
        return Err("capsem-process exited before signalling ready".into());
    }
    res.map_err(|e| format!("{e}"))
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

async fn handle_reload_config(
    State(state): State<Arc<ServiceState>>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let runtime_rules = runtime_security_rules_snapshot_from_registries(&state)?;
    // Collect paths to broadcast to.
    let reload_targets = {
        let instances = state.instances.lock().unwrap();
        instances
            .iter()
            .map(|(id, info)| (id.clone(), info.uds_path.clone(), info.session_dir.clone()))
            .collect::<Vec<_>>()
    };

    let results =
        futures::future::join_all(reload_targets.iter().map(|(id, uds_path, session_dir)| {
            let id = id.clone();
            let session_dir = session_dir.clone();
            let state = state.clone();
            let runtime_rules = runtime_rules.clone();
            async move {
                if let Err(error) = state.refresh_vm_effective_settings(&session_dir) {
                    return Some(ReloadConfigFailure {
                        session_id: id,
                        message: format!("refresh vm-effective settings: {error}"),
                    });
                }
                match send_ipc_command(
                    uds_path,
                    ServiceToProcess::ReloadConfig {
                        runtime_rules: Some(runtime_rules),
                    },
                    Some(5),
                )
                .await
                {
                    Ok(ProcessToService::ReloadConfigResult {
                        success: true,
                        error: _,
                    }) => None,
                    Ok(ProcessToService::ReloadConfigResult {
                        success: false,
                        error,
                    }) => Some(ReloadConfigFailure {
                        session_id: id,
                        message: error.unwrap_or_else(|| "reload failed".to_string()),
                    }),
                    Ok(ProcessToService::Pong) => None,
                    Ok(_) => Some(ReloadConfigFailure {
                        session_id: id,
                        message: "unexpected response".to_string(),
                    }),
                    Err(e) => Some(ReloadConfigFailure {
                        session_id: id,
                        message: e,
                    }),
                }
            }
        }))
        .await;
    let failures: Vec<ReloadConfigFailure> = results.into_iter().flatten().collect();
    let failed_session_ids: Vec<String> = failures
        .iter()
        .map(|failure| failure.session_id.clone())
        .collect();
    let reloaded = reload_targets.len().saturating_sub(failures.len());

    if failures.is_empty() {
        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "reloaded": reload_targets.len(),
                "failed_session_count": 0,
                "failed_session_ids": [],
                "failures": [],
                "message": null,
            })),
        ))
    } else {
        let message = format!(
            "failed to reload config in {} running session{}",
            failures.len(),
            if failures.len() == 1 { "" } else { "s" }
        );
        Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "reloaded": reloaded,
                "failed_session_count": failures.len(),
                "failed_session_ids": failed_session_ids,
                "failures": failures,
                "message": message,
            })),
        ))
    }
}

#[derive(Debug, Clone, Serialize)]
struct ReloadConfigFailure {
    session_id: String,
    message: String,
}

// ---------------------------------------------------------------------------
// Settings endpoints
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct SettingsIssue {
    path: String,
    severity: String,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyRuleUpdate {
    #[serde(rename = "on")]
    callback: String,
    #[serde(rename = "if")]
    condition: String,
    decision: capsem_core::settings_profiles::RuleDecision,
    #[serde(default = "default_profile_rule_priority")]
    priority: i32,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    rewrite_target: Option<String>,
    #[serde(default)]
    rewrite_value: Option<String>,
    #[serde(default)]
    strip_request_headers: Vec<String>,
    #[serde(default)]
    strip_response_headers: Vec<String>,
}

fn default_profile_rule_priority() -> i32 {
    1
}

fn service_settings_path() -> PathBuf {
    capsem_core::paths::capsem_home().join("service.toml")
}

fn load_service_profiles_state() -> Result<
    (
        capsem_core::settings_profiles::ServiceSettings,
        capsem_core::settings_profiles::ProfileCatalog,
        capsem_core::settings_profiles::EffectiveVmSettings,
        capsem_core::settings_profiles::ResolverTrace,
    ),
    String,
> {
    let settings_path = service_settings_path();
    let settings = capsem_core::settings_profiles::load_service_settings_or_default(&settings_path)
        .map_err(|e| format!("load {}: {e}", settings_path.display()))?;
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| format!("discover profiles: {e}"))?;
    let (effective, trace) =
        capsem_core::settings_profiles::resolve_effective_vm_settings_with_corp(
            &settings,
            Some(&settings.profiles.default_profile),
        )
        .map_err(|e| {
            format!(
                "resolve effective profile '{}': {e}",
                settings.profiles.default_profile
            )
        })?;
    Ok((settings, catalog, effective, trace))
}

fn rule_type_from_callback(callback: &str) -> Option<&'static str> {
    match callback {
        "mcp.request" | "mcp.response" => Some("mcp"),
        "http.request" | "http.read" | "http.write" | "http.response" => Some("http"),
        "dns.request" | "dns.response" => Some("dns"),
        "model.request" | "model.response" | "model.tool_call" | "model.tool_response" => {
            Some("model")
        }
        "hook.decision" => Some("hook"),
        _ => None,
    }
}

fn split_policy_key(key: &str) -> Result<(String, String), String> {
    let mut parts = key.split('.');
    let prefix = parts.next();
    let rule_type = parts.next();
    let rule_name = parts.next();
    if prefix != Some("policy")
        || rule_type.is_none()
        || rule_name.is_none()
        || parts.next().is_some()
    {
        return Err(format!(
            "unsupported settings key '{key}'; only policy.<type>.<rule_name> is accepted"
        ));
    }
    let rule_type = rule_type.unwrap_or_default();
    if !matches!(rule_type, "mcp" | "http" | "dns" | "model" | "hook") {
        return Err(format!("unsupported policy rule type in key '{key}'"));
    }
    let rule_name = rule_name.unwrap_or_default();
    if rule_name.is_empty()
        || !rule_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err(format!("invalid policy rule name in key '{key}'"));
    }
    Ok((rule_type.to_string(), rule_name.to_string()))
}

fn profile_rule_from_update(
    update: PolicyRuleUpdate,
) -> capsem_core::settings_profiles::ProfileRule {
    capsem_core::settings_profiles::ProfileRule {
        callback: update.callback,
        condition: update.condition,
        decision: update.decision,
        priority: update.priority,
        reason: update.reason,
        rewrite_target: update.rewrite_target,
        rewrite_value: update.rewrite_value,
        strip_request_headers: normalize_header_names(update.strip_request_headers),
        strip_response_headers: normalize_header_names(update.strip_response_headers),
    }
}

fn normalize_header_names(headers: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for header in headers {
        let trimmed = header.trim();
        let Ok(name) = axum::http::header::HeaderName::from_bytes(trimmed.as_bytes()) else {
            continue;
        };
        let name = name.as_str().to_string();
        if seen.insert(name.clone()) {
            normalized.push(name);
        }
    }
    normalized
}

fn validate_policy_rule_update(
    rule_type: &str,
    rule_name: &str,
    update: &PolicyRuleUpdate,
) -> Result<(), String> {
    let Some(callback_type) = rule_type_from_callback(&update.callback) else {
        return Err(format!("unsupported policy callback '{}'", update.callback));
    };
    if callback_type != rule_type {
        return Err(format!(
            "policy rule 'policy.{rule_type}.{rule_name}' uses callback for a different policy type"
        ));
    }
    if update.condition.trim().is_empty() {
        return Err(format!(
            "invalid policy rule policy.{rule_type}.{rule_name}: condition cannot be empty"
        ));
    }
    validate_policy_condition_terms(rule_type, rule_name, &update.condition)?;
    Ok(())
}

fn validate_policy_condition_terms(
    rule_type: &str,
    rule_name: &str,
    condition: &str,
) -> Result<(), String> {
    if condition.contains(".match(") {
        return Err(format!(
            "invalid policy rule policy.{rule_type}.{rule_name}: unsupported CEL condition term '.match('; use '.matches(' for regular-expression predicates"
        ));
    }
    Ok(())
}

fn upsert_profile_rule(
    profile: &mut capsem_core::settings_profiles::Profile,
    rule_type: &str,
    rule_name: String,
    rule: capsem_core::settings_profiles::ProfileRule,
) {
    match rule_type {
        "mcp" => {
            profile.security.rules.mcp.insert(rule_name, rule);
        }
        "http" => {
            profile.security.rules.http.insert(rule_name, rule);
        }
        "dns" => {
            profile.security.rules.dns.insert(rule_name, rule);
        }
        "model" => {
            profile.security.rules.model.insert(rule_name, rule);
        }
        "hook" => {
            profile.security.rules.hook.insert(rule_name, rule);
        }
        _ => {}
    }
}

fn remove_profile_rule(
    profile: &mut capsem_core::settings_profiles::Profile,
    rule_type: &str,
    rule_name: &str,
) {
    match rule_type {
        "mcp" => {
            profile.security.rules.mcp.remove(rule_name);
        }
        "http" => {
            profile.security.rules.http.remove(rule_name);
        }
        "dns" => {
            profile.security.rules.dns.remove(rule_name);
        }
        "model" => {
            profile.security.rules.model.remove(rule_name);
        }
        "hook" => {
            profile.security.rules.hook.remove(rule_name);
        }
        _ => {}
    }
}

fn policy_json_from_effective(
    effective: &capsem_core::settings_profiles::EffectiveVmSettings,
) -> serde_json::Value {
    let mut policy = serde_json::Map::new();
    for rule in &effective.rules {
        if rule.derived {
            continue;
        }
        let Some(rule_type) = rule_type_from_callback(&rule.callback) else {
            continue;
        };
        let rule_name = rule
            .id
            .split_once('.')
            .map(|(_, name)| name)
            .filter(|name| !name.is_empty())
            .unwrap_or(rule.id.as_str())
            .to_string();

        let rule_json = json!({
            "on": rule.callback,
            "if": rule.condition,
            "decision": rule.decision,
            "priority": rule.priority,
            "reason": rule.reason,
            "rewrite_target": rule.rewrite_target,
            "rewrite_value": rule.rewrite_value,
            "strip_request_headers": rule.strip_request_headers,
            "strip_response_headers": rule.strip_response_headers,
        });
        let entry = policy
            .entry(rule_type.to_string())
            .or_insert_with(|| json!({}));
        if let Some(map) = entry.as_object_mut() {
            map.insert(rule_name, rule_json);
        }
    }
    serde_json::Value::Object(policy)
}

fn profile_presets_json(
    catalog: &capsem_core::settings_profiles::ProfileCatalog,
) -> serde_json::Value {
    let mut presets = catalog
        .list()
        .map(|record| {
            json!({
                "id": record.profile.id,
                "name": record.profile.name,
                "description": record.profile.description,
                "settings": {
                    "profiles.default_profile": record.profile.id,
                },
            })
        })
        .collect::<Vec<_>>();
    presets.sort_by(|left, right| {
        left["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["name"].as_str().unwrap_or_default())
    });
    serde_json::Value::Array(presets)
}

fn profile_record_json(
    record: &capsem_core::settings_profiles::ProfileRecord,
) -> serde_json::Value {
    json!({
        "profile": record.profile,
        "source": record.source.as_str(),
        "path": record.path.as_ref().map(|path| path.display().to_string()),
        "locked": record.locked,
    })
}

#[derive(Debug, Deserialize)]
struct ProfileForkRequest {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileCatalogReconcileRequest {
    manifest_json: String,
    profile_payload_pubkey: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileRevisionActionRequest {
    #[serde(default)]
    revision: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RulesQuery {
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    callback: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RulesMutationQuery {
    #[serde(default)]
    profile: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleCreateRequest {
    #[serde(default, alias = "profile_id")]
    profile: Option<String>,
    id: String,
    #[serde(flatten)]
    update: PolicyRuleUpdate,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeEnforcementRuleRequest {
    id: String,
    #[serde(default)]
    pack_id: Option<String>,
    condition: String,
    decision: seceng::SecurityDecisionAction,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeDetectionRuleRequest {
    id: String,
    pack_id: String,
    #[serde(default)]
    sigma_id: Option<String>,
    title: String,
    condition: String,
    severity: seceng::Severity,
    confidence: seceng::Confidence,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeBacktestEvent {
    #[serde(default)]
    event_ref: Option<seceng::BacktestEventRef>,
    event: seceng::SecurityEvent,
    #[serde(default)]
    expected: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeEnforcementBacktestRequest {
    rule: RuntimeEnforcementRuleRequest,
    events: Vec<RuntimeBacktestEvent>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeDetectionBacktestRequest {
    rule: RuntimeDetectionRuleRequest,
    events: Vec<RuntimeBacktestEvent>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeDetectionHuntRequest {
    rules: Vec<RuntimeDetectionRuleRequest>,
    events: Vec<RuntimeBacktestEvent>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeSessionDetectionHuntRequest {
    rules: Vec<RuntimeDetectionRuleRequest>,
    #[serde(default)]
    limit: Option<usize>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SkillKind {
    Group,
    #[default]
    Enabled,
    Disabled,
}

impl SkillKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Group => "group",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Deserialize)]
struct SkillsQuery {
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    kind: Option<SkillKind>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillMutationRequest {
    #[serde(default, alias = "profile_id")]
    profile: Option<String>,
    id: String,
    #[serde(default)]
    kind: SkillKind,
}

fn load_service_settings_for_profiles(
) -> Result<capsem_core::settings_profiles::ServiceSettings, AppError> {
    let settings_path = service_settings_path();
    capsem_core::settings_profiles::load_service_settings_or_default(&settings_path).map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("load {}: {e}", settings_path.display()),
        )
    })
}

/// GET /profiles -- list typed Profile V2 profile records.
async fn handle_list_profiles() -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;

    let mut profiles = catalog.list().map(profile_record_json).collect::<Vec<_>>();
    profiles.sort_by(|left, right| {
        left["profile"]["id"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["profile"]["id"].as_str().unwrap_or_default())
    });

    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "default_profile": settings.profiles.default_profile,
        "profiles": profiles,
    })))
}

/// GET /profiles/catalog -- show signed catalog and installed revision state.
async fn handle_profile_catalog() -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    Ok(Json(profile_catalog_status_json(&settings)?))
}

fn load_persisted_profile_manifest(
    settings: &capsem_core::settings_profiles::ServiceSettings,
) -> Result<
    (
        Option<PathBuf>,
        Option<capsem_core::profile_manifest::ProfileManifest>,
    ),
    AppError,
> {
    let manifest_path = profile_catalog_manifest_path(settings);
    let manifest_json = match manifest_path.as_ref() {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("read profile catalog manifest {}: {error}", path.display()),
                ));
            }
        },
        None => None,
    };
    let manifest = match manifest_json.as_deref() {
        Some(content) => Some(
            capsem_core::profile_manifest::ProfileManifest::from_json(content).map_err(
                |error| {
                    AppError(
                        StatusCode::BAD_REQUEST,
                        format!("parse persisted profile catalog manifest: {error}"),
                    )
                },
            )?,
        ),
        None => None,
    };

    Ok((manifest_path, manifest))
}

fn profile_revision_records_json(
    profile: &capsem_core::profile_manifest::ManifestProfile,
    installed: Option<&capsem_core::settings_profiles::InstalledProfileRevisionRecord>,
) -> Vec<serde_json::Value> {
    let mut revisions = profile
        .revisions
        .iter()
        .map(|(revision, record)| {
            json!({
                "revision": revision,
                "status": record.status.as_str(),
                "current": revision == &profile.current_revision,
                "installed": installed
                    .is_some_and(|installed| installed.revision == *revision),
                "profile_hash": record.profile_hash,
                "min_binary": record.min_binary,
            })
        })
        .collect::<Vec<_>>();
    revisions.sort_by(|left, right| {
        left["revision"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["revision"].as_str().unwrap_or_default())
    });
    revisions
}

fn profile_catalog_status_json(
    settings: &capsem_core::settings_profiles::ServiceSettings,
) -> Result<serde_json::Value, AppError> {
    let (manifest_path, manifest) = load_persisted_profile_manifest(settings)?;
    let mut profiles = Vec::new();
    if let Some(manifest) = &manifest {
        for (profile_id, profile) in &manifest.profiles {
            let installed = capsem_core::settings_profiles::load_installed_profile_revision(
                &settings.profiles,
                profile_id,
            )
            .map_err(|error| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("load installed profile revision '{profile_id}': {error}"),
                )
            })?;
            profiles.push(json!({
                "profile_id": profile_id,
                "current_revision": profile.current_revision,
                "installed_revision": installed.as_ref().map(|installed| installed.revision.clone()),
                "installed_payload_hash": installed.as_ref().map(|installed| installed.payload_hash.clone()),
                "revisions": profile_revision_records_json(profile, installed.as_ref()),
            }));
        }
    }
    profiles.sort_by(|left, right| {
        left["profile_id"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["profile_id"].as_str().unwrap_or_default())
    });

    Ok(json!({
        "mode": "settings_profiles_v2",
        "configured": settings.profile_catalog.is_configured(),
        "manifest_url": settings.profile_catalog.manifest_url.clone(),
        "check_interval_secs": settings.profile_catalog.check_interval_secs,
        "manifest_path": manifest_path.map(|path| path.display().to_string()),
        "manifest_present": manifest.is_some(),
        "profiles": profiles,
    }))
}

/// GET /profiles/{id}/revisions -- show signed catalog revisions for one profile.
async fn handle_profile_revisions(
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let (_, manifest) = load_persisted_profile_manifest(&settings)?;
    let manifest = manifest.ok_or_else(|| {
        AppError(
            StatusCode::NOT_FOUND,
            "profile catalog manifest is not present".into(),
        )
    })?;
    let profile = manifest.profiles.get(&profile_id).ok_or_else(|| {
        AppError(
            StatusCode::NOT_FOUND,
            format!("profile catalog entry '{profile_id}' not found"),
        )
    })?;
    let installed = capsem_core::settings_profiles::load_installed_profile_revision(
        &settings.profiles,
        &profile_id,
    )
    .map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("load installed profile revision '{profile_id}': {error}"),
        )
    })?;

    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "profile_id": profile_id,
        "current_revision": profile.current_revision,
        "installed_revision": installed.as_ref().map(|installed| installed.revision.clone()),
        "installed_payload_hash": installed.as_ref().map(|installed| installed.payload_hash.clone()),
        "revisions": profile_revision_records_json(profile, installed.as_ref()),
    })))
}

/// POST /profiles/{id}/revisions/install -- install an active signed catalog revision.
async fn handle_install_profile_revision(
    Path(profile_id): Path<String>,
    Json(body): Json<ProfileRevisionActionRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    Ok(Json(
        reconcile_selected_profile_revision(&settings, &profile_id, body.revision.as_deref(), true)
            .await?,
    ))
}

/// POST /profiles/{id}/revisions/update -- reconcile one signed catalog revision.
async fn handle_update_profile_revision_lifecycle(
    Path(profile_id): Path<String>,
    Json(body): Json<ProfileRevisionActionRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    Ok(Json(
        reconcile_selected_profile_revision(
            &settings,
            &profile_id,
            body.revision.as_deref(),
            false,
        )
        .await?,
    ))
}

/// POST /profiles/{id}/revisions/remove -- remove local launchable state for one revision.
async fn handle_remove_profile_revision(
    Path(profile_id): Path<String>,
    Json(body): Json<ProfileRevisionActionRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let selected_revision = match body.revision.as_deref() {
        Some(revision) => revision.to_string(),
        None => capsem_core::settings_profiles::load_installed_profile_revision(
            &settings.profiles,
            &profile_id,
        )
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("load installed profile revision '{profile_id}': {error}"),
            )
        })?
        .map(|installed| installed.revision)
        .ok_or_else(|| {
            AppError(
                StatusCode::NOT_FOUND,
                format!("profile '{profile_id}' has no installed revision to remove"),
            )
        })?,
    };
    let removed = capsem_core::settings_profiles::remove_installed_profile_revision(
        &settings.profiles,
        &profile_id,
        Some(&selected_revision),
    )
    .map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "remove installed profile revision '{profile_id}@{selected_revision}': {error}"
            ),
        )
    })?;

    let outcome = match removed {
        Some(record) => json!({
            "profile_id": record.profile_id,
            "revision": record.revision,
            "payload_hash": record.payload_hash,
            "outcome": "removed",
        }),
        None => json!({
            "profile_id": profile_id,
            "revision": selected_revision,
            "outcome": "not_installed",
        }),
    };

    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "action": "remove",
        "profile_id": outcome["profile_id"],
        "selected_revision": outcome["revision"],
        "outcome": outcome,
    })))
}

async fn reconcile_selected_profile_revision(
    settings: &capsem_core::settings_profiles::ServiceSettings,
    profile_id: &str,
    requested_revision: Option<&str>,
    install_only: bool,
) -> Result<serde_json::Value, AppError> {
    let (_, manifest) = load_persisted_profile_manifest(settings)?;
    let manifest = manifest.ok_or_else(|| {
        AppError(
            StatusCode::NOT_FOUND,
            "profile catalog manifest is not present".into(),
        )
    })?;
    let revision = match requested_revision {
        Some(revision) => manifest.revision(profile_id, revision).map_err(|error| {
            AppError(
                StatusCode::NOT_FOUND,
                format!("resolve profile revision '{profile_id}@{revision}': {error}"),
            )
        })?,
        None => manifest.current_revision(profile_id).map_err(|error| {
            AppError(
                StatusCode::NOT_FOUND,
                format!("resolve current profile revision '{profile_id}': {error}"),
            )
        })?,
    };
    if install_only
        && revision.record.status != capsem_core::profile_manifest::ProfileRevisionStatus::Active
    {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            format!(
                "profile revision '{}@{}' has status {}; only active revisions can be installed",
                revision.profile_id,
                revision.revision,
                revision.record.status.as_str()
            ),
        ));
    }
    let profile_payload_pubkey = settings
        .profile_catalog
        .profile_payload_pubkey
        .as_deref()
        .ok_or_else(|| {
            AppError(
                StatusCode::BAD_REQUEST,
                "profile catalog profile_payload_pubkey is not configured".into(),
            )
        })?;
    let selected_profile_id = revision.profile_id.to_string();
    let selected_revision = revision.revision.to_string();
    let action = if install_only { "install" } else { "update" };
    let mut summary = ProfileCatalogReconcileSummary::default();
    let outcome = capsem_core::settings_profiles::reconcile_profile_revision_from_manifest(
        &settings.profiles,
        revision,
        profile_payload_pubkey,
    )
    .await
    .map_err(|error| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!(
                "reconcile profile revision '{selected_profile_id}@{selected_revision}': {error:#}"
            ),
        )
    })
    .map(|outcome| profile_reconcile_outcome_json(outcome, &mut summary))?;

    Ok(json!({
        "mode": "settings_profiles_v2",
        "action": action,
        "profile_id": selected_profile_id,
        "selected_revision": selected_revision,
        "requested_revision": requested_revision,
        "summary": summary,
        "outcome": outcome,
    }))
}

/// GET /profiles/{id} -- fetch one typed Profile V2 profile record.
async fn handle_get_profile(Path(id): Path<String>) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    let record = catalog
        .get(&id)
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("profile '{id}' not found")))?;

    Ok(Json(profile_record_json(record)))
}

/// POST /profiles -- create a user-owned Profile V2 profile.
async fn handle_create_profile(
    Json(profile): Json<capsem_core::settings_profiles::Profile>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    if let Some(existing) = catalog.get(&profile.id) {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            format!(
                "profile '{}' already exists ({})",
                profile.id,
                existing.source.as_str()
            ),
        ));
    }
    let record = capsem_core::settings_profiles::create_user_profile(&settings.profiles, profile)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("create profile: {e}")))?;
    Ok(Json(profile_record_json(&record)))
}

/// POST /profiles/{id}/fork -- fork an existing profile into a user profile.
async fn handle_fork_profile(
    Path(source_id): Path<String>,
    Json(body): Json<ProfileForkRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let record = capsem_core::settings_profiles::fork_user_profile(
        &settings.profiles,
        &source_id,
        &body.id,
        &body.name,
    )
    .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("fork profile: {e}")))?;
    Ok(Json(profile_record_json(&record)))
}

/// PUT /profiles/{id} -- update an existing user-owned Profile V2 profile.
async fn handle_update_profile(
    Path(id): Path<String>,
    Json(profile): Json<capsem_core::settings_profiles::Profile>,
) -> Result<Json<serde_json::Value>, AppError> {
    if profile.id != id {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            format!(
                "profile body id '{}' does not match route id '{id}'",
                profile.id
            ),
        ));
    }
    let settings = load_service_settings_for_profiles()?;
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    if let Some(record) = catalog.get(&id) {
        if record.locked {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                format!("profile '{id}' is locked ({})", record.source.as_str()),
            ));
        }
        ensure_locked_profile_sections_unchanged(&record.profile, &profile)?;
    }
    let record = capsem_core::settings_profiles::update_user_profile(&settings.profiles, profile)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("update profile: {e}")))?;
    Ok(Json(profile_record_json(&record)))
}

/// DELETE /profiles/{id} -- delete an existing user-owned Profile V2 profile.
async fn handle_delete_profile(
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    if let Some(record) = catalog.get(&id) {
        if record.locked {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                format!("profile '{id}' is locked ({})", record.source.as_str()),
            ));
        }
    }
    capsem_core::settings_profiles::delete_user_profile(&settings.profiles, &id)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("delete profile: {e}")))?;
    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "deleted": id,
    })))
}

/// GET /profiles/{id}/effective -- resolve one profile to VM-effective settings.
async fn handle_resolve_profile(
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let (effective, trace) =
        capsem_core::settings_profiles::resolve_effective_vm_settings_with_corp(
            &settings,
            Some(&id),
        )
        .map_err(|e| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("resolve effective profile '{id}': {e}"),
            )
        })?;

    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "profile_id": effective.profile_id,
        "effective": effective,
        "resolver_trace": trace,
    })))
}

/// POST /profiles/catalog/reconcile -- apply signed profile catalog lifecycle state.
async fn handle_reconcile_profile_catalog(
    Json(body): Json<ProfileCatalogReconcileRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let result = reconcile_profile_catalog_manifest(
        &settings,
        &body.manifest_json,
        &body.profile_payload_pubkey,
    )
    .await?;
    Ok(Json(result))
}

async fn reconcile_configured_profile_catalog(
    settings: &capsem_core::settings_profiles::ServiceSettings,
) -> Result<serde_json::Value, AppError> {
    let manifest_url = settings
        .profile_catalog
        .manifest_url
        .as_deref()
        .ok_or_else(|| {
            AppError(
                StatusCode::BAD_REQUEST,
                "profile catalog manifest_url is not configured".into(),
            )
        })?;
    let profile_payload_pubkey = settings
        .profile_catalog
        .profile_payload_pubkey
        .as_deref()
        .ok_or_else(|| {
            AppError(
                StatusCode::BAD_REQUEST,
                "profile catalog profile_payload_pubkey is not configured".into(),
            )
        })?;
    let url = capsem_core::profile_manifest::parse_profile_catalog_manifest_url(manifest_url)
        .map_err(|error| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("parse configured profile catalog manifest URL: {error}"),
            )
        })?;
    let manifest_json = capsem_core::profile_manifest::fetch_profile_catalog_manifest_url(url)
        .await
        .map_err(|error| {
            AppError(
                StatusCode::BAD_GATEWAY,
                format!("fetch configured profile catalog manifest: {error:#}"),
            )
        })?;
    reconcile_profile_catalog_manifest(settings, &manifest_json, profile_payload_pubkey).await
}

fn spawn_profile_catalog_reconcile_task(
    settings: capsem_core::settings_profiles::ServiceSettings,
) -> Option<tokio::task::JoinHandle<()>> {
    if !settings.profile_catalog.is_configured() {
        return None;
    }
    let check_interval =
        std::time::Duration::from_secs(settings.profile_catalog.check_interval_secs);
    Some(tokio::spawn(async move {
        loop {
            match reconcile_configured_profile_catalog(&settings).await {
                Ok(result) => {
                    let summary = &result["summary"];
                    info!(
                        installed = summary["installed"].as_u64().unwrap_or_default(),
                        unchanged = summary["unchanged"].as_u64().unwrap_or_default(),
                        deprecated_kept = summary["deprecated_kept"].as_u64().unwrap_or_default(),
                        revoked_removed = summary["revoked_removed"].as_u64().unwrap_or_default(),
                        absent_removed = summary["absent_removed"].as_u64().unwrap_or_default(),
                        errors = summary["errors"].as_u64().unwrap_or_default(),
                        "profile catalog scheduled reconcile completed"
                    );
                }
                Err(error) => {
                    warn!(
                        status = error.0.as_u16(),
                        error = %error.1,
                        "profile catalog scheduled reconcile failed"
                    );
                }
            }
            tokio::time::sleep(check_interval).await;
        }
    }))
}

async fn reconcile_profile_catalog_manifest(
    settings: &capsem_core::settings_profiles::ServiceSettings,
    manifest_json: &str,
    profile_payload_pubkey: &str,
) -> Result<serde_json::Value, AppError> {
    let manifest = match capsem_core::profile_manifest::ProfileManifest::from_json(manifest_json) {
        Ok(manifest) => manifest,
        Err(error) => {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                format!("parse profile catalog manifest: {error}"),
            ));
        }
    };
    persist_profile_catalog_manifest(settings, manifest_json)?;
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    for profile_id in manifest.profiles.keys() {
        let current = manifest.current_revision(profile_id).map_err(|e| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("resolve current profile revision: {e}"),
            )
        })?;
        if seen.insert((current.profile_id.to_string(), current.revision.to_string())) {
            targets.push((current.profile_id.to_string(), current.revision.to_string()));
        }
        let Some(profile) = manifest.profiles.get(profile_id) else {
            continue;
        };
        for (revision, record) in &profile.revisions {
            if record.status == capsem_core::profile_manifest::ProfileRevisionStatus::Active {
                continue;
            }
            if seen.insert((profile_id.clone(), revision.clone())) {
                targets.push((profile_id.clone(), revision.clone()));
            }
        }
    }
    targets.sort();

    let mut summary = ProfileCatalogReconcileSummary::default();
    let mut outcomes = Vec::new();
    for (profile_id, revision_id) in targets {
        let revision = manifest.revision(&profile_id, &revision_id).map_err(|e| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("resolve profile revision '{profile_id}@{revision_id}': {e}"),
            )
        })?;
        match capsem_core::settings_profiles::reconcile_profile_revision_from_manifest(
            &settings.profiles,
            revision,
            profile_payload_pubkey,
        )
        .await
        {
            Ok(outcome) => outcomes.push(profile_reconcile_outcome_json(outcome, &mut summary)),
            Err(error) => {
                summary.errors += 1;
                outcomes.push(json!({
                    "profile_id": profile_id,
                    "revision": revision_id,
                    "outcome": "error",
                    "error": format!("{error:#}"),
                }));
            }
        }
    }
    let absent_outcomes =
        capsem_core::settings_profiles::reconcile_absent_installed_profiles_from_manifest(
            &settings.profiles,
            &manifest,
        )
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("reconcile absent profile catalog entries: {error}"),
            )
        })?;
    for outcome in absent_outcomes {
        outcomes.push(profile_reconcile_outcome_json(outcome, &mut summary));
    }

    Ok(json!({
        "mode": "settings_profiles_v2",
        "summary": summary,
        "outcomes": outcomes,
    }))
}

#[derive(Debug, Default, Serialize)]
struct ProfileCatalogReconcileSummary {
    installed: usize,
    unchanged: usize,
    deprecated_kept: usize,
    deprecated_not_installed: usize,
    revoked_removed: usize,
    revoked_not_installed: usize,
    absent_removed: usize,
    errors: usize,
}

fn profile_reconcile_outcome_json(
    outcome: capsem_core::settings_profiles::ProfileRevisionReconcileOutcome,
    summary: &mut ProfileCatalogReconcileSummary,
) -> serde_json::Value {
    match outcome {
        capsem_core::settings_profiles::ProfileRevisionReconcileOutcome::Installed(installed) => {
            summary.installed += 1;
            json!({
                "profile_id": installed.profile_id,
                "revision": installed.revision,
                "payload_hash": installed.payload_hash,
                "outcome": "installed",
                "runtime_profile_path": installed.runtime_profile_path.display().to_string(),
                "payload_path": installed.payload_path.display().to_string(),
                "current_record_path": installed.current_record_path.display().to_string(),
            })
        }
        capsem_core::settings_profiles::ProfileRevisionReconcileOutcome::Unchanged(record) => {
            summary.unchanged += 1;
            json!({
                "profile_id": record.profile_id,
                "revision": record.revision,
                "payload_hash": record.payload_hash,
                "outcome": "unchanged",
            })
        }
        capsem_core::settings_profiles::ProfileRevisionReconcileOutcome::DeprecatedKept(
            record,
        ) => {
            summary.deprecated_kept += 1;
            json!({
                "profile_id": record.profile_id,
                "revision": record.revision,
                "payload_hash": record.payload_hash,
                "outcome": "deprecated_kept",
            })
        }
        capsem_core::settings_profiles::ProfileRevisionReconcileOutcome::DeprecatedNotInstalled {
            profile_id,
            revision,
        } => {
            summary.deprecated_not_installed += 1;
            json!({
                "profile_id": profile_id,
                "revision": revision,
                "outcome": "deprecated_not_installed",
            })
        }
        capsem_core::settings_profiles::ProfileRevisionReconcileOutcome::RevokedRemoved {
            profile_id,
            revision,
        } => {
            summary.revoked_removed += 1;
            json!({
                "profile_id": profile_id,
                "revision": revision,
                "outcome": "revoked_removed",
            })
        }
        capsem_core::settings_profiles::ProfileRevisionReconcileOutcome::RevokedNotInstalled {
            profile_id,
            revision,
        } => {
            summary.revoked_not_installed += 1;
            json!({
                "profile_id": profile_id,
                "revision": revision,
                "outcome": "revoked_not_installed",
            })
        }
        capsem_core::settings_profiles::ProfileRevisionReconcileOutcome::AbsentRemoved {
            profile_id,
            revision,
        } => {
            summary.absent_removed += 1;
            json!({
                "profile_id": profile_id,
                "revision": revision,
                "outcome": "absent_removed",
            })
        }
    }
}

fn canonical_rule_id(rule: &capsem_core::settings_profiles::EffectiveRule) -> String {
    if rule.id.starts_with("security.rules.") {
        return rule.id.clone();
    }
    let Some((rule_type, name)) = rule.id.split_once('.') else {
        return format!("security.rules.{}", rule.id);
    };
    if matches!(rule_type, "mcp" | "http" | "dns" | "model" | "hook") && !name.is_empty() {
        format!("security.rules.{rule_type}.{name}")
    } else {
        format!("security.rules.{}", rule.id)
    }
}

fn rule_type_and_name_from_effective_id(id: &str) -> Option<(&str, &str)> {
    let (rule_type, name) = id.split_once('.')?;
    if matches!(rule_type, "mcp" | "http" | "dns" | "model" | "hook") && !name.is_empty() {
        Some((rule_type, name))
    } else {
        None
    }
}

fn rule_json_from_effective(
    rule: &capsem_core::settings_profiles::EffectiveRule,
) -> serde_json::Value {
    let rule_type = rule_type_and_name_from_effective_id(&rule.id)
        .map(|(rule_type, _)| rule_type.to_string())
        .or_else(|| rule_type_from_callback(&rule.callback).map(ToOwned::to_owned));
    json!({
        "id": canonical_rule_id(rule),
        "effective_id": rule.id,
        "rule_type": rule_type,
        "source_profile": rule.provenance.profile_id,
        "callback": rule.callback,
        "condition": rule.condition,
        "decision": rule.decision,
        "priority": rule.priority,
        "derived": rule.derived,
        "editable": rule.editable,
        "owner_setting_path": rule.owner_setting_path,
        "owner_setting_label": rule.owner_setting_label,
        "provenance": rule.provenance,
        "rule": {
            "on": rule.callback,
            "if": rule.condition,
            "decision": rule.decision,
            "priority": rule.priority,
            "reason": rule.reason,
            "rewrite_target": rule.rewrite_target,
            "rewrite_value": rule.rewrite_value,
            "strip_request_headers": rule.strip_request_headers,
            "strip_response_headers": rule.strip_response_headers,
        },
    })
}

fn resolve_effective_for_rules(
    profile: Option<String>,
) -> Result<capsem_core::settings_profiles::EffectiveVmSettings, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let profile_id = profile.unwrap_or_else(|| settings.profiles.default_profile.clone());
    let (effective, _) = capsem_core::settings_profiles::resolve_effective_vm_settings_with_corp(
        &settings,
        Some(&profile_id),
    )
    .map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("resolve effective profile '{profile_id}': {e}"),
        )
    })?;
    Ok(effective)
}

fn find_effective_rule<'a>(
    effective: &'a capsem_core::settings_profiles::EffectiveVmSettings,
    rule_id: &str,
) -> Option<&'a capsem_core::settings_profiles::EffectiveRule> {
    effective.rules.iter().find(|rule| {
        rule.id == rule_id
            || canonical_rule_id(rule) == rule_id
            || rule
                .id
                .strip_prefix("security.rules.")
                .is_some_and(|stripped| stripped == rule_id)
    })
}

fn parse_rule_resource_id(rule_id: &str) -> Result<(String, String), String> {
    let stripped = rule_id.strip_prefix("security.rules.").unwrap_or(rule_id);
    let mut parts = stripped.split('.');
    let rule_type = parts.next();
    let rule_name = parts.next();
    if rule_type.is_none() || rule_name.is_none() || parts.next().is_some() {
        return Err(format!(
            "invalid rule id '{rule_id}'; expected security.rules.<type>.<name>"
        ));
    }
    let rule_type = rule_type.unwrap_or_default();
    if !matches!(rule_type, "mcp" | "http" | "dns" | "model" | "hook") {
        return Err(format!(
            "unsupported policy rule type in rule id '{rule_id}'"
        ));
    }
    let rule_name = rule_name.unwrap_or_default();
    if rule_name.is_empty()
        || !rule_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err(format!("invalid policy rule name in rule id '{rule_id}'"));
    }
    Ok((rule_type.to_string(), rule_name.to_string()))
}

fn profile_has_rule(
    profile: &capsem_core::settings_profiles::Profile,
    rule_type: &str,
    rule_name: &str,
) -> bool {
    match rule_type {
        "mcp" => profile.security.rules.mcp.contains_key(rule_name),
        "http" => profile.security.rules.http.contains_key(rule_name),
        "dns" => profile.security.rules.dns.contains_key(rule_name),
        "model" => profile.security.rules.model.contains_key(rule_name),
        "hook" => profile.security.rules.hook.contains_key(rule_name),
        _ => false,
    }
}

fn skill_list(profile: &capsem_core::settings_profiles::Profile, kind: SkillKind) -> &[String] {
    match kind {
        SkillKind::Group => &profile.skills.groups,
        SkillKind::Enabled => &profile.skills.enabled,
        SkillKind::Disabled => &profile.skills.disabled,
    }
}

fn skill_list_mut(
    profile: &mut capsem_core::settings_profiles::Profile,
    kind: SkillKind,
) -> &mut Vec<String> {
    match kind {
        SkillKind::Group => &mut profile.skills.groups,
        SkillKind::Enabled => &mut profile.skills.enabled,
        SkillKind::Disabled => &mut profile.skills.disabled,
    }
}

fn remove_skill_from(
    profile: &mut capsem_core::settings_profiles::Profile,
    kind: SkillKind,
    id: &str,
) {
    skill_list_mut(profile, kind).retain(|candidate| candidate != id);
}

fn profile_has_skill(
    profile: &capsem_core::settings_profiles::Profile,
    kind: SkillKind,
    id: &str,
) -> bool {
    skill_list(profile, kind)
        .iter()
        .any(|candidate| candidate == id)
}

fn skill_owner<'a>(
    catalog: &'a capsem_core::settings_profiles::ProfileCatalog,
    profile_id: &str,
    kind: SkillKind,
    id: &str,
) -> Result<Option<&'a capsem_core::settings_profiles::ProfileRecord>, AppError> {
    let chain = capsem_core::settings_profiles::resolve_ancestor_chain(catalog, profile_id)
        .map_err(|e| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("resolve profile chain: {e}"),
            )
        })?;
    Ok(chain
        .into_iter()
        .rfind(|record| profile_has_skill(&record.profile, kind, id)))
}

fn skill_json(
    id: &str,
    kind: SkillKind,
    owner: Option<&capsem_core::settings_profiles::ProfileRecord>,
    selected_profile_id: &str,
) -> serde_json::Value {
    let source_profile = owner.map(|record| record.profile.id.as_str());
    let source = owner.map(|record| record.source.as_str());
    let direct = source_profile == Some(selected_profile_id);
    let editable = direct
        && owner
            .map(|record| record.source == capsem_core::settings_profiles::ProfileSource::User)
            .unwrap_or(false);
    json!({
        "id": id,
        "kind": kind,
        "source_profile": source_profile,
        "source": source,
        "direct": direct,
        "editable": editable,
    })
}

fn save_mutated_profile(
    settings: &capsem_core::settings_profiles::ServiceSettings,
    source: capsem_core::settings_profiles::ProfileSource,
    profile: capsem_core::settings_profiles::Profile,
) -> Result<(), AppError> {
    match source {
        capsem_core::settings_profiles::ProfileSource::User => {
            capsem_core::settings_profiles::update_user_profile(&settings.profiles, profile)
                .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("update profile: {e}")))?;
        }
        capsem_core::settings_profiles::ProfileSource::BuiltIn => {
            capsem_core::settings_profiles::create_user_profile(&settings.profiles, profile)
                .map_err(|e| {
                    AppError(
                        StatusCode::BAD_REQUEST,
                        format!("create profile override: {e}"),
                    )
                })?;
        }
        capsem_core::settings_profiles::ProfileSource::Base
        | capsem_core::settings_profiles::ProfileSource::Corp => {
            return Err(AppError(
                StatusCode::CONFLICT,
                format!(
                    "profile '{}' is locked ({source:?}); switch to a user-editable profile first",
                    profile.id
                ),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum ProfileEditableSection {
    General,
    Appearance,
    Ai,
    McpServers,
    Skills,
    Packages,
    Tools,
    Vm,
    SecurityCapabilities,
    SecurityRules,
}

impl ProfileEditableSection {
    fn path(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Ai => "ai",
            Self::McpServers => "mcpServers",
            Self::Skills => "skills",
            Self::Packages => "packages",
            Self::Tools => "tools",
            Self::Vm => "vm",
            Self::SecurityCapabilities => "security.capabilities",
            Self::SecurityRules => "security.rules",
        }
    }

    fn is_editable(self, profile: &capsem_core::settings_profiles::Profile) -> bool {
        match self {
            Self::General => profile.editable.general,
            Self::Appearance => profile.editable.appearance,
            Self::Ai => profile.editable.ai,
            Self::McpServers => profile.editable.mcp_servers,
            Self::Skills => profile.editable.skills,
            Self::Packages => profile.editable.packages,
            Self::Tools => profile.editable.tools,
            Self::Vm => profile.editable.vm,
            Self::SecurityCapabilities => profile.editable.security_capabilities,
            Self::SecurityRules => profile.editable.security_rules,
        }
    }
}

fn ensure_profile_section_editable(
    profile: &capsem_core::settings_profiles::Profile,
    section: ProfileEditableSection,
) -> Result<(), AppError> {
    if section.is_editable(profile) {
        return Ok(());
    }
    Err(AppError(
        StatusCode::CONFLICT,
        format!(
            "profile_section_locked: profile '{}' section '{}' is not editable",
            profile.id,
            section.path()
        ),
    ))
}

fn ensure_locked_profile_sections_unchanged(
    previous: &capsem_core::settings_profiles::Profile,
    updated: &capsem_core::settings_profiles::Profile,
) -> Result<(), AppError> {
    if previous.editable != updated.editable {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!(
                "profile_section_locked: profile '{}' section 'editable' is not editable",
                previous.id
            ),
        ));
    }

    let checks = [
        (
            ProfileEditableSection::General,
            previous.general == updated.general,
        ),
        (
            ProfileEditableSection::Appearance,
            previous.appearance == updated.appearance,
        ),
        (ProfileEditableSection::Ai, previous.ai == updated.ai),
        (
            ProfileEditableSection::McpServers,
            previous.mcp == updated.mcp,
        ),
        (
            ProfileEditableSection::Skills,
            previous.skills == updated.skills,
        ),
        (
            ProfileEditableSection::Packages,
            previous.packages == updated.packages,
        ),
        (
            ProfileEditableSection::Tools,
            previous.tools == updated.tools,
        ),
        (ProfileEditableSection::Vm, previous.vm == updated.vm),
        (
            ProfileEditableSection::SecurityCapabilities,
            previous.security.capabilities == updated.security.capabilities,
        ),
        (
            ProfileEditableSection::SecurityRules,
            previous.security.rules == updated.security.rules,
        ),
    ];
    for (section, unchanged) in checks {
        if !unchanged {
            ensure_profile_section_editable(previous, section)?;
        }
    }
    Ok(())
}

/// GET /rules -- list resolved Profile V2 rules for a profile.
async fn handle_list_rules(
    Query(query): Query<RulesQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    if let Some(callback) = query.callback.as_deref() {
        if rule_type_from_callback(callback).is_none() {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                format!("unsupported policy callback '{callback}'"),
            ));
        }
    }
    let effective = resolve_effective_for_rules(query.profile)?;
    let mut rules = effective
        .rules
        .iter()
        .filter(|rule| {
            query
                .callback
                .as_deref()
                .map(|callback| rule.callback == callback)
                .unwrap_or(true)
        })
        .map(rule_json_from_effective)
        .collect::<Vec<_>>();
    rules.sort_by(|left, right| {
        left["id"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["id"].as_str().unwrap_or_default())
    });

    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "profile_id": effective.profile_id,
        "rules": rules,
    })))
}

/// GET /rules/{rule_id} -- fetch one resolved rule with provenance.
async fn handle_get_rule(Path(rule_id): Path<String>) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    let mut profile_ids = vec![settings.profiles.default_profile.clone()];
    let mut remaining = catalog
        .list()
        .map(|record| record.profile.id.clone())
        .filter(|id| id != &settings.profiles.default_profile)
        .collect::<Vec<_>>();
    remaining.sort();
    profile_ids.extend(remaining);

    for profile_id in profile_ids {
        let (effective, _) =
            capsem_core::settings_profiles::resolve_effective_vm_settings_with_corp(
                &settings,
                Some(&profile_id),
            )
            .map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    format!("resolve effective profile '{profile_id}': {e}"),
                )
            })?;
        if let Some(rule) = find_effective_rule(&effective, &rule_id) {
            return Ok(Json(rule_json_from_effective(rule)));
        }
    }

    Err(AppError(
        StatusCode::NOT_FOUND,
        format!("rule '{rule_id}' not found"),
    ))
}

/// POST /rules -- create a user-editable Profile V2 rule.
async fn handle_create_rule(
    Json(request): Json<RuleCreateRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (rule_type, rule_name) =
        parse_rule_resource_id(&request.id).map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
    validate_policy_rule_update(&rule_type, &rule_name, &request.update)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;

    let settings = load_service_settings_for_profiles()?;
    let target_profile_id = request
        .profile
        .clone()
        .unwrap_or_else(|| settings.profiles.default_profile.clone());
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    let selected = catalog.get(&target_profile_id).ok_or_else(|| {
        AppError(
            StatusCode::NOT_FOUND,
            format!("profile '{target_profile_id}' not found"),
        )
    })?;
    ensure_profile_section_editable(&selected.profile, ProfileEditableSection::SecurityRules)?;
    let mut profile = selected.profile.clone();
    if profile_has_rule(&profile, &rule_type, &rule_name) {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!("rule_exists: security.rules.{rule_type}.{rule_name}"),
        ));
    }
    upsert_profile_rule(
        &mut profile,
        &rule_type,
        rule_name.clone(),
        profile_rule_from_update(request.update),
    );
    profile.validate().map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("profile validation failed: {e}"),
        )
    })?;
    save_mutated_profile(&settings, selected.source, profile)?;

    let effective = resolve_effective_for_rules(Some(target_profile_id.clone()))?;
    let canonical = format!("security.rules.{rule_type}.{rule_name}");
    let rule = find_effective_rule(&effective, &canonical).ok_or_else(|| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("created rule '{canonical}' was not visible after profile save"),
        )
    })?;
    Ok(Json(rule_json_from_effective(rule)))
}

/// DELETE /rules/{rule_id} -- remove a user-authored Profile V2 rule.
async fn handle_delete_rule(
    Path(rule_id): Path<String>,
    Query(query): Query<RulesMutationQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (rule_type, rule_name) =
        parse_rule_resource_id(&rule_id).map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
    let settings = load_service_settings_for_profiles()?;
    let target_profile_id = query
        .profile
        .clone()
        .unwrap_or_else(|| settings.profiles.default_profile.clone());
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    let selected = catalog.get(&target_profile_id).ok_or_else(|| {
        AppError(
            StatusCode::NOT_FOUND,
            format!("profile '{target_profile_id}' not found"),
        )
    })?;
    ensure_profile_section_editable(&selected.profile, ProfileEditableSection::SecurityRules)?;
    if selected.source != capsem_core::settings_profiles::ProfileSource::User {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!(
                "rule_is_builtin: profile '{}' is locked ({:?})",
                selected.profile.id, selected.source
            ),
        ));
    }

    let effective = resolve_effective_for_rules(Some(target_profile_id.clone()))?;
    let effective_rule = find_effective_rule(&effective, &rule_id)
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("rule '{rule_id}' not found")))?;
    if effective_rule.provenance.profile_id != target_profile_id
        || !profile_has_rule(&selected.profile, &rule_type, &rule_name)
    {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!(
                "rule_is_builtin: rule '{}' is inherited from profile '{}'",
                canonical_rule_id(effective_rule),
                effective_rule.provenance.profile_id
            ),
        ));
    }
    capsem_core::settings_profiles::ensure_rule_editable(effective_rule)
        .map_err(|e| AppError(StatusCode::CONFLICT, format!("rule_is_builtin: {e}")))?;

    let mut profile = selected.profile.clone();
    remove_profile_rule(&mut profile, &rule_type, &rule_name);
    profile.validate().map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("profile validation failed: {e}"),
        )
    })?;
    capsem_core::settings_profiles::update_user_profile(&settings.profiles, profile)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("update profile: {e}")))?;

    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "profile_id": target_profile_id,
        "rule_id": format!("security.rules.{rule_type}.{rule_name}"),
        "removed": true,
    })))
}

fn validate_runtime_rule_id(id: &str) -> Result<(), AppError> {
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
    {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            format!("invalid runtime rule id '{id}'"),
        ));
    }
    Ok(())
}

fn runtime_rule_plan_id(condition: &str) -> String {
    format!("cel:{}", blake3::hash(condition.as_bytes()).to_hex())
}

fn compile_runtime_enforcement_rule(
    request: &RuntimeEnforcementRuleRequest,
) -> Result<String, seceng::SecurityEngineError> {
    seceng::CelEnforcementEvaluator::compile(vec![seceng::CelEnforcementRule {
        id: request.id.clone(),
        pack_id: request.pack_id.clone(),
        condition: request.condition.clone(),
        decision: request.decision,
        reason: request.reason.clone(),
    }])?;
    Ok(runtime_rule_plan_id(&request.condition))
}

fn compile_runtime_detection_rule(
    request: &RuntimeDetectionRuleRequest,
) -> Result<String, seceng::SecurityEngineError> {
    seceng::CelDetectionEvaluator::compile(vec![seceng::CelDetectionRule {
        id: request.id.clone(),
        pack_id: request.pack_id.clone(),
        sigma_id: request.sigma_id.clone(),
        title: request.title.clone(),
        condition: request.condition.clone(),
        severity: request.severity,
        confidence: request.confidence,
        tags: request.tags.clone(),
    }])?;
    Ok(runtime_rule_plan_id(&request.condition))
}

fn runtime_enforcement_record(
    request: &RuntimeEnforcementRuleRequest,
) -> seceng::RuntimeRuleRecord {
    seceng::RuntimeRuleRecord {
        metadata: seceng::RuntimeRuleMetadata {
            id: request.id.clone(),
            pack_id: request.pack_id.clone(),
            scope: seceng::RuleScope::Runtime,
            origin: seceng::RuleOrigin::Runtime,
        },
        definition: seceng::RuntimeRuleDefinition::Enforcement {
            decision: request.decision,
            reason: request.reason.clone(),
        },
        source: request.condition.clone(),
        enabled: request.enabled,
    }
}

fn runtime_detection_record(request: &RuntimeDetectionRuleRequest) -> seceng::RuntimeRuleRecord {
    seceng::RuntimeRuleRecord {
        metadata: seceng::RuntimeRuleMetadata {
            id: request.id.clone(),
            pack_id: Some(request.pack_id.clone()),
            scope: seceng::RuleScope::Runtime,
            origin: seceng::RuleOrigin::Runtime,
        },
        definition: seceng::RuntimeRuleDefinition::Detection {
            sigma_id: request.sigma_id.clone(),
            title: request.title.clone(),
            severity: request.severity,
            confidence: request.confidence,
            tags: request.tags.clone(),
        },
        source: request.condition.clone(),
        enabled: request.enabled,
    }
}

fn runtime_rule_entry_json(entry: &seceng::RuntimeRuleEntry) -> serde_json::Value {
    let compiled = matches!(&entry.compile_status, seceng::CompileStatus::Compiled);
    json!({
        "id": &entry.metadata.id,
        "pack_id": &entry.metadata.pack_id,
        "scope": entry.metadata.scope,
        "origin": entry.metadata.origin,
        "definition": &entry.definition,
        "enabled": entry.enabled,
        "compiled": compiled,
        "compile_status": &entry.compile_status,
        "generation": entry.generation,
        "condition": &entry.source,
        "compiled_plan": &entry.compiled_plan,
        "match_count": entry.stats.match_count,
        "last_matched_event": &entry.stats.last_matched_event,
        "last_matched_unix_ms": entry.stats.last_matched_unix_ms,
    })
}

fn runtime_registry_rules_json(
    registry: &Arc<Mutex<seceng::RuntimeRuleRegistry>>,
) -> Result<Vec<serde_json::Value>, AppError> {
    let registry = registry.lock().map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("runtime rule registry lock poisoned: {error}"),
        )
    })?;
    Ok(registry
        .list()
        .into_iter()
        .map(runtime_rule_entry_json)
        .collect())
}

struct RuntimeSecurityMatchRecorder {
    enforcement_registry: Arc<Mutex<seceng::RuntimeRuleRegistry>>,
    detection_registry: Arc<Mutex<seceng::RuntimeRuleRegistry>>,
}

impl seceng::RuleMatchRecorder for RuntimeSecurityMatchRecorder {
    fn record_rule_match(
        &mut self,
        rule_id: &str,
        event_id: &str,
        timestamp_unix_ms: u64,
    ) -> Result<(), seceng::SecurityEngineError> {
        let mut recorded = false;
        record_runtime_rule_match_if_present(
            &self.enforcement_registry,
            rule_id,
            event_id,
            timestamp_unix_ms,
            &mut recorded,
        )?;
        record_runtime_rule_match_if_present(
            &self.detection_registry,
            rule_id,
            event_id,
            timestamp_unix_ms,
            &mut recorded,
        )?;
        if recorded {
            Ok(())
        } else {
            Err(seceng::SecurityEngineError::PhaseFailed {
                phase: seceng::SecurityEnginePhase::Detection,
                message: format!("runtime rule not found while recording match: {rule_id}"),
            })
        }
    }
}

fn record_runtime_rule_match_if_present(
    registry: &Arc<Mutex<seceng::RuntimeRuleRegistry>>,
    rule_id: &str,
    event_id: &str,
    timestamp_unix_ms: u64,
    recorded: &mut bool,
) -> Result<(), seceng::SecurityEngineError> {
    let mut registry =
        registry
            .lock()
            .map_err(|error| seceng::SecurityEngineError::PhaseFailed {
                phase: seceng::SecurityEnginePhase::Detection,
                message: format!("runtime rule registry lock poisoned: {error}"),
            })?;
    match registry.record_match(rule_id, event_id, timestamp_unix_ms) {
        Ok(()) => {
            *recorded = true;
            Ok(())
        }
        Err(seceng::RuleRegistryError::NotFound(_)) => Ok(()),
        Err(error) => Err(seceng::SecurityEngineError::PhaseFailed {
            phase: seceng::SecurityEnginePhase::Detection,
            message: error.to_string(),
        }),
    }
}

fn record_runtime_rule_match_count_if_present(
    registry: &Arc<Mutex<seceng::RuntimeRuleRegistry>>,
    rule_id: &str,
    event_id: &str,
    timestamp_unix_ms: u64,
    count: u64,
    recorded: &mut bool,
) -> Result<(), seceng::SecurityEngineError> {
    for _ in 0..count {
        record_runtime_rule_match_if_present(
            registry,
            rule_id,
            event_id,
            timestamp_unix_ms,
            recorded,
        )?;
    }
    Ok(())
}

fn runtime_security_engine_from_registries(
    state: &Arc<ServiceState>,
) -> Result<seceng::SecurityEngine, AppError> {
    let enforcement_rules = {
        let registry = state.enforcement_registry.lock().map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("runtime enforcement registry lock poisoned: {error}"),
            )
        })?;
        registry.enabled_enforcement_rules()
    };
    let detection_rules = {
        let registry = state.detection_registry.lock().map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("runtime detection registry lock poisoned: {error}"),
            )
        })?;
        registry.enabled_detection_rules()
    };

    let mut engine = seceng::SecurityEngine::default();
    if !enforcement_rules.is_empty() {
        engine.set_enforcement(Box::new(
            seceng::CelEnforcementEvaluator::compile(enforcement_rules).map_err(|error| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("compile installed enforcement rules: {error}"),
                )
            })?,
        ));
    }
    if !detection_rules.is_empty() {
        engine.set_detection(Box::new(
            seceng::CelDetectionEvaluator::compile(detection_rules).map_err(|error| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("compile installed detection rules: {error}"),
                )
            })?,
        ));
    }
    engine.set_match_recorder(Box::new(RuntimeSecurityMatchRecorder {
        enforcement_registry: state.enforcement_registry.clone(),
        detection_registry: state.detection_registry.clone(),
    }));
    Ok(engine)
}

fn runtime_security_rules_snapshot_from_registries(
    state: &Arc<ServiceState>,
) -> Result<capsem_proto::ipc::RuntimeSecurityRulesSnapshot, AppError> {
    let enforcement = {
        let registry = state.enforcement_registry.lock().map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("runtime enforcement registry lock poisoned: {error}"),
            )
        })?;
        registry
            .enabled_enforcement_rules()
            .into_iter()
            .map(runtime_enforcement_rule_snapshot)
            .collect()
    };
    let detection = {
        let registry = state.detection_registry.lock().map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("runtime detection registry lock poisoned: {error}"),
            )
        })?;
        registry
            .enabled_detection_rules()
            .into_iter()
            .map(runtime_detection_rule_snapshot)
            .collect()
    };

    Ok(capsem_proto::ipc::RuntimeSecurityRulesSnapshot {
        enforcement,
        detection,
    })
}

fn runtime_enforcement_rule_snapshot(
    rule: seceng::CelEnforcementRule,
) -> capsem_proto::ipc::RuntimeEnforcementRuleSnapshot {
    capsem_proto::ipc::RuntimeEnforcementRuleSnapshot {
        id: rule.id,
        pack_id: rule.pack_id,
        condition: rule.condition,
        decision: runtime_decision_action_snapshot(rule.decision),
        reason: rule.reason,
    }
}

fn runtime_detection_rule_snapshot(
    rule: seceng::CelDetectionRule,
) -> capsem_proto::ipc::RuntimeDetectionRuleSnapshot {
    capsem_proto::ipc::RuntimeDetectionRuleSnapshot {
        id: rule.id,
        pack_id: rule.pack_id,
        sigma_id: rule.sigma_id,
        title: rule.title,
        condition: rule.condition,
        severity: runtime_detection_severity_snapshot(rule.severity),
        confidence: runtime_detection_confidence_snapshot(rule.confidence),
        tags: rule.tags,
    }
}

fn runtime_decision_action_snapshot(
    action: seceng::SecurityDecisionAction,
) -> capsem_proto::ipc::RuntimeSecurityDecisionAction {
    match action {
        seceng::SecurityDecisionAction::Allow => {
            capsem_proto::ipc::RuntimeSecurityDecisionAction::Allow
        }
        seceng::SecurityDecisionAction::Ask => {
            capsem_proto::ipc::RuntimeSecurityDecisionAction::Ask
        }
        seceng::SecurityDecisionAction::Block => {
            capsem_proto::ipc::RuntimeSecurityDecisionAction::Block
        }
        seceng::SecurityDecisionAction::Rewrite => {
            capsem_proto::ipc::RuntimeSecurityDecisionAction::Rewrite
        }
        seceng::SecurityDecisionAction::Throttle => {
            capsem_proto::ipc::RuntimeSecurityDecisionAction::Throttle
        }
    }
}

fn runtime_detection_severity_snapshot(
    severity: seceng::Severity,
) -> capsem_proto::ipc::RuntimeDetectionSeverity {
    match severity {
        seceng::Severity::Info => capsem_proto::ipc::RuntimeDetectionSeverity::Info,
        seceng::Severity::Low => capsem_proto::ipc::RuntimeDetectionSeverity::Low,
        seceng::Severity::Medium => capsem_proto::ipc::RuntimeDetectionSeverity::Medium,
        seceng::Severity::High => capsem_proto::ipc::RuntimeDetectionSeverity::High,
        seceng::Severity::Critical => capsem_proto::ipc::RuntimeDetectionSeverity::Critical,
    }
}

fn runtime_detection_confidence_snapshot(
    confidence: seceng::Confidence,
) -> capsem_proto::ipc::RuntimeDetectionConfidence {
    match confidence {
        seceng::Confidence::Low => capsem_proto::ipc::RuntimeDetectionConfidence::Low,
        seceng::Confidence::Medium => capsem_proto::ipc::RuntimeDetectionConfidence::Medium,
        seceng::Confidence::High => capsem_proto::ipc::RuntimeDetectionConfidence::High,
    }
}

#[derive(Debug, Clone)]
struct RuntimeRulePropagationSummary {
    target_count: usize,
    failed_session_ids: Vec<String>,
    failures: Vec<ReloadConfigFailure>,
}

impl RuntimeRulePropagationSummary {
    fn json(&self) -> serde_json::Value {
        json!({
            "target_count": self.target_count,
            "failed_session_count": self.failures.len(),
            "failed_session_ids": self.failed_session_ids,
            "failures": self.failures,
        })
    }
}

async fn broadcast_runtime_security_rules(
    state: &Arc<ServiceState>,
) -> Result<RuntimeRulePropagationSummary, AppError> {
    let runtime_rules = runtime_security_rules_snapshot_from_registries(state)?;
    let targets = {
        let instances = state.instances.lock().unwrap();
        instances
            .iter()
            .map(|(id, info)| (id.clone(), info.uds_path.clone()))
            .collect::<Vec<_>>()
    };

    let results = futures::future::join_all(targets.iter().map(|(id, uds_path)| {
        let id = id.clone();
        let runtime_rules = runtime_rules.clone();
        async move {
            match send_ipc_command(
                uds_path,
                ServiceToProcess::ReloadConfig {
                    runtime_rules: Some(runtime_rules),
                },
                Some(5),
            )
            .await
            {
                Ok(ProcessToService::ReloadConfigResult {
                    success: true,
                    error: _,
                }) => None,
                Ok(ProcessToService::ReloadConfigResult {
                    success: false,
                    error,
                }) => Some(ReloadConfigFailure {
                    session_id: id,
                    message: error.unwrap_or_else(|| "runtime rule propagation failed".to_string()),
                }),
                Ok(ProcessToService::Pong) => None,
                Ok(_) => Some(ReloadConfigFailure {
                    session_id: id,
                    message: "unexpected response".to_string(),
                }),
                Err(error) => Some(ReloadConfigFailure {
                    session_id: id,
                    message: error,
                }),
            }
        }
    }))
    .await;
    let failures: Vec<ReloadConfigFailure> = results.into_iter().flatten().collect();
    let failed_session_ids = failures
        .iter()
        .map(|failure| failure.session_id.clone())
        .collect();
    Ok(RuntimeRulePropagationSummary {
        target_count: targets.len(),
        failed_session_ids,
        failures,
    })
}

async fn drain_runtime_rule_matches_from_processes(
    state: &Arc<ServiceState>,
) -> Result<RuntimeRulePropagationSummary, AppError> {
    let targets = {
        let instances = state.instances.lock().unwrap();
        instances
            .iter()
            .map(|(id, info)| (id.clone(), info.uds_path.clone()))
            .collect::<Vec<_>>()
    };
    let results = futures::future::join_all(targets.iter().map(|(session_id, uds_path)| {
        let session_id = session_id.clone();
        let uds_path = uds_path.clone();
        let state = state.clone();
        async move {
            let drain_id = state.next_job_id();
            match send_ipc_command(
                &uds_path,
                ServiceToProcess::DrainRuntimeRuleMatches { id: drain_id },
                Some(5),
            )
            .await
            {
                Ok(ProcessToService::RuntimeRuleMatches { id, matches }) if id == drain_id => {
                    for rule_match in matches {
                        let mut recorded_any = false;
                        let event_id = rule_match
                            .last_matched_event
                            .as_deref()
                            .unwrap_or("unknown");
                        let timestamp_unix_ms = rule_match.last_matched_unix_ms.unwrap_or_default();
                        if let Err(error) = record_runtime_rule_match_count_if_present(
                            &state.enforcement_registry,
                            &rule_match.rule_id,
                            event_id,
                            timestamp_unix_ms,
                            rule_match.match_count,
                            &mut recorded_any,
                        ) {
                            return Some(ReloadConfigFailure {
                                session_id,
                                message: format!("record enforcement runtime match: {error}"),
                            });
                        }
                        if let Err(error) = record_runtime_rule_match_count_if_present(
                            &state.detection_registry,
                            &rule_match.rule_id,
                            event_id,
                            timestamp_unix_ms,
                            rule_match.match_count,
                            &mut recorded_any,
                        ) {
                            return Some(ReloadConfigFailure {
                                session_id,
                                message: format!("record detection runtime match: {error}"),
                            });
                        }
                        if !recorded_any && rule_match.match_count > 0 {
                            tracing::debug!(
                                rule_id = %rule_match.rule_id,
                                "process reported runtime rule match for a rule no longer in the service registry"
                            );
                        }
                    }
                    None
                }
                Ok(ProcessToService::RuntimeRuleMatches { id, .. }) => Some(ReloadConfigFailure {
                    session_id,
                    message: format!(
                        "runtime rule match drain id mismatch: expected {drain_id}, got {id}"
                    ),
                }),
                Ok(_) => Some(ReloadConfigFailure {
                    session_id,
                    message: "unexpected response".to_string(),
                }),
                Err(error) => Some(ReloadConfigFailure {
                    session_id,
                    message: error,
                }),
            }
        }
    }))
    .await;
    let failures: Vec<ReloadConfigFailure> = results.into_iter().flatten().collect();
    let failed_session_ids = failures
        .iter()
        .map(|failure| failure.session_id.clone())
        .collect();
    Ok(RuntimeRulePropagationSummary {
        target_count: targets.len(),
        failed_session_ids,
        failures,
    })
}

fn runtime_backtest_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(seceng::DEFAULT_BACKTEST_MATCH_LIMIT)
}

fn inline_backtest_event_ref(input: &RuntimeBacktestEvent) -> seceng::BacktestEventRef {
    input
        .event_ref
        .clone()
        .unwrap_or_else(|| seceng::BacktestEventRef {
            corpus: "inline".into(),
            session_id: input.event.common.session_id.clone(),
            event_id: input.event.common.event_id.clone(),
            sequence_no: input.event.common.sequence_no,
            timestamp_unix_ms: input.event.common.timestamp_unix_ms,
        })
}

fn backtest_evidence_signature(event: &seceng::SecurityEvent) -> Result<String, AppError> {
    let evidence = serde_json::json!({
        "event_type": &event.common.event_type,
        "subject": &event.subject,
    });
    let evidence = serde_json::to_vec(&evidence).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize backtest evidence: {error}"),
        )
    })?;
    Ok(blake3::hash(&evidence).to_hex().to_string())
}

fn backtest_matched_fields(
    event: &seceng::SecurityEvent,
) -> Result<Vec<seceng::MatchedField>, AppError> {
    Ok(vec![seceng::MatchedField {
        path: "subject".into(),
        value: serde_json::to_value(&event.subject).map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize backtest matched field: {error}"),
            )
        })?,
    }])
}

fn backtest_outcome(expected: Option<&str>, actual: &str) -> seceng::BacktestOutcome {
    match expected {
        Some(expected) if expected != actual => seceng::BacktestOutcome::Mismatch {
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        },
        _ => seceng::BacktestOutcome::Matched,
    }
}

fn security_events_query_rows(
    reader: &capsem_logger::DbReader,
) -> Result<Vec<serde_json::Value>, AppError> {
    let json_str = reader
        .query_raw(
            "SELECT
                se.event_id, se.timestamp_unix_ms, se.event_family, se.event_type,
                se.source_engine, se.enforceability, se.attribution_scope,
                se.origin_kind, se.accounting_owner, se.trace_id, se.span_id,
                se.parent_event_id, se.stream_id, se.activity_id, se.sequence_no,
                se.vm_id, se.session_id, se.profile_id, se.profile_revision,
                se.user_id, se.process_id, se.parent_process_id, se.exec_id,
                se.turn_id, se.message_id, se.tool_call_id, se.mcp_call_id,
                se.redaction_state,
                n.domain, n.port, n.method, n.path, n.query, n.status_code,
                n.bytes_sent, n.bytes_received,
                d.qname,
                m.server_name, m.tool_name,
                mc.provider, mc.model, mc.input_tokens, mc.output_tokens,
                f.action, f.path, f.size,
                x.command, x.process_name,
                s.slot, s.origin, s.name,
                ami.interaction_id, ami.trace_id, ami.attribution_scope,
                ami.source_engine, ami.origin_kind, ami.accounting_owner,
                ami.profile_id, ami.vm_id, ami.session_id, ami.user_id,
                ami.provider, ami.api_family, ami.model, ami.parse_status,
                ami.evidence_status, ami.request_id, ami.request_model,
                ami.request_stream, ami.request_system_prompt_preview,
                ami.request_message_count, ami.request_tools_declared_count,
                ami.request_raw_shape_version,
                ami.request_unknown_fields_present,
                ami.response_id, ami.response_provider_response_id,
                ami.response_stop_reason, ami.response_text_preview,
                ami.response_thinking_preview, ami.response_raw_shape_version,
                ami.usage_input_tokens, ami.usage_output_tokens,
                ami.usage_estimated_cost_micros,
                ame.mcp_call_id, ame.server_id, ame.tool_name,
                ame.namespaced_tool_name, ame.transport,
                ame.request_arguments_raw, ame.request_arguments_json,
                ame.result_kind, ame.result_preview, ame.result_json,
                ame.is_error, ame.latency_ms,
                ame.linked_model_interaction_id,
                ame.linked_model_tool_call_id, ame.link_status
             FROM security_events se
             LEFT JOIN net_events n
                ON n.trace_id = se.trace_id
               AND se.event_family = 'http'
             LEFT JOIN dns_events d
                ON d.trace_id = se.trace_id
               AND se.event_family = 'dns'
             LEFT JOIN mcp_calls m
                ON m.trace_id = se.trace_id
               AND se.event_family = 'mcp'
             LEFT JOIN model_calls mc
                ON mc.trace_id = se.trace_id
               AND se.event_family = 'model'
             LEFT JOIN fs_events f
                ON f.trace_id = se.trace_id
               AND se.event_family = 'file'
             LEFT JOIN exec_events x
                ON x.trace_id = se.trace_id
               AND se.event_family = 'process'
             LEFT JOIN snapshot_events s
                ON s.trace_id = se.trace_id
               AND se.event_family = 'snapshot'
             LEFT JOIN ai_model_interactions ami
                ON ami.trace_id = se.trace_id
               AND se.event_family = 'model'
             LEFT JOIN ai_mcp_execution_evidence ame
                ON (ame.mcp_call_id = se.mcp_call_id
                    OR ame.mcp_call_id = m.request_id)
               AND se.event_family = 'mcp'
             ORDER BY se.timestamp_unix_ms ASC, se.id ASC
             LIMIT 10000",
        )
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("query session security events: {error}"),
            )
        })?;
    let value: serde_json::Value = serde_json::from_str(&json_str).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("parse session security events: {error}"),
        )
    })?;
    Ok(value
        .get("rows")
        .and_then(|rows| rows.as_array())
        .cloned()
        .unwrap_or_default())
}

fn session_cell<'a>(
    row: &'a serde_json::Value,
    index: usize,
) -> Result<&'a serde_json::Value, AppError> {
    row.as_array()
        .and_then(|cells| cells.get(index))
        .ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session security event row missing column {index}"),
            )
        })
}

fn session_required_string(row: &serde_json::Value, index: usize) -> Result<String, AppError> {
    session_cell(row, index)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session security event column {index} was not a string"),
            )
        })
}

fn session_optional_string(
    row: &serde_json::Value,
    index: usize,
) -> Result<Option<String>, AppError> {
    let value = session_cell(row, index)?;
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_str()
            .map(|value| Some(value.to_owned()))
            .ok_or_else(|| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("session security event column {index} was not a nullable string"),
                )
            })
    }
}

fn session_required_u64(row: &serde_json::Value, index: usize) -> Result<u64, AppError> {
    let value = session_cell(row, index)?;
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session security event column {index} was not an unsigned integer"),
            )
        })
}

fn session_optional_u64(row: &serde_json::Value, index: usize) -> Result<Option<u64>, AppError> {
    let value = session_cell(row, index)?;
    if value.is_null() {
        Ok(None)
    } else {
        session_required_u64(row, index).map(Some)
    }
}

fn session_optional_bool(row: &serde_json::Value, index: usize) -> Result<Option<bool>, AppError> {
    let value = session_cell(row, index)?;
    if value.is_null() {
        return Ok(None);
    }
    if let Some(value) = value.as_bool() {
        return Ok(Some(value));
    }
    if let Some(value) = value.as_i64() {
        return Ok(Some(value != 0));
    }
    if let Some(value) = value.as_u64() {
        return Ok(Some(value != 0));
    }
    Err(AppError(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("session security event column {index} was not a nullable boolean"),
    ))
}

fn parse_session_enum<T>(value: &str, label: &str) -> Result<T, AppError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::String(value.to_owned())).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("unsupported session {label} '{value}': {error}"),
        )
    })
}

fn parse_session_source_engine(value: &str) -> Result<seceng::SourceEngine, AppError> {
    match value {
        "network" => Ok(seceng::SourceEngine::Network),
        "file" => Ok(seceng::SourceEngine::File),
        "process" => Ok(seceng::SourceEngine::Process),
        "conversation" => Ok(seceng::SourceEngine::Conversation),
        "security" => Ok(seceng::SourceEngine::Security),
        "vm" => Ok(seceng::SourceEngine::Vm),
        "profile" => Ok(seceng::SourceEngine::Profile),
        "host_ai" => Ok(seceng::SourceEngine::HostAi),
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("unsupported session source_engine '{value}'"),
        )),
    }
}

fn parse_session_attribution_scope(value: &str) -> Result<seceng::AiAttributionScope, AppError> {
    match value {
        "host" => Ok(seceng::AiAttributionScope::Host),
        "vm" => Ok(seceng::AiAttributionScope::Vm),
        "profile" => Ok(seceng::AiAttributionScope::Profile),
        "session" => Ok(seceng::AiAttributionScope::Session),
        "unknown" => Ok(seceng::AiAttributionScope::Unknown),
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("unsupported session attribution_scope '{value}'"),
        )),
    }
}

fn parse_session_origin_kind(value: &str) -> Result<seceng::AiOriginKind, AppError> {
    match value {
        "guest_network" => Ok(seceng::AiOriginKind::GuestNetwork),
        "host_service" => Ok(seceng::AiOriginKind::HostService),
        "host_admin" => Ok(seceng::AiOriginKind::HostAdmin),
        "host_workbench" => Ok(seceng::AiOriginKind::HostWorkbench),
        "test_fixture" => Ok(seceng::AiOriginKind::TestFixture),
        "unknown" => Ok(seceng::AiOriginKind::Unknown),
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("unsupported session origin_kind '{value}'"),
        )),
    }
}

fn parse_session_enforceability(value: &str) -> Result<seceng::Enforceability, AppError> {
    match value {
        "inline_blockable" => Ok(seceng::Enforceability::InlineBlockable),
        "observe_only" => Ok(seceng::Enforceability::ObserveOnly),
        "remediation_only" => Ok(seceng::Enforceability::RemediationOnly),
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("unsupported session enforceability '{value}'"),
        )),
    }
}

fn parse_session_redaction_state(value: &str) -> Result<seceng::RedactionState, AppError> {
    match value {
        "raw" => Ok(seceng::RedactionState::Raw),
        "redacted" => Ok(seceng::RedactionState::Redacted),
        "summary-only" => Ok(seceng::RedactionState::SummaryOnly),
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("unsupported session redaction_state '{value}'"),
        )),
    }
}

const SESSION_COL_EVENT_ID: usize = 0;
const SESSION_COL_TIMESTAMP_UNIX_MS: usize = 1;
const SESSION_COL_EVENT_FAMILY: usize = 2;
const SESSION_COL_EVENT_TYPE: usize = 3;
const SESSION_COL_SOURCE_ENGINE: usize = 4;
const SESSION_COL_ENFORCEABILITY: usize = 5;
const SESSION_COL_ATTRIBUTION_SCOPE: usize = 6;
const SESSION_COL_ORIGIN_KIND: usize = 7;
const SESSION_COL_ACCOUNTING_OWNER: usize = 8;
const SESSION_COL_TRACE_ID: usize = 9;
const SESSION_COL_SPAN_ID: usize = 10;
const SESSION_COL_PARENT_EVENT_ID: usize = 11;
const SESSION_COL_STREAM_ID: usize = 12;
const SESSION_COL_ACTIVITY_ID: usize = 13;
const SESSION_COL_SEQUENCE_NO: usize = 14;
const SESSION_COL_VM_ID: usize = 15;
const SESSION_COL_SESSION_ID: usize = 16;
const SESSION_COL_PROFILE_ID: usize = 17;
const SESSION_COL_PROFILE_REVISION: usize = 18;
const SESSION_COL_USER_ID: usize = 19;
const SESSION_COL_PROCESS_ID: usize = 20;
const SESSION_COL_PARENT_PROCESS_ID: usize = 21;
const SESSION_COL_EXEC_ID: usize = 22;
const SESSION_COL_TURN_ID: usize = 23;
const SESSION_COL_MESSAGE_ID: usize = 24;
const SESSION_COL_TOOL_CALL_ID: usize = 25;
const SESSION_COL_MCP_CALL_ID: usize = 26;
const SESSION_COL_REDACTION_STATE: usize = 27;
const SESSION_COL_HTTP_HOST: usize = 28;
const SESSION_COL_HTTP_PORT: usize = 29;
const SESSION_COL_HTTP_METHOD: usize = 30;
const SESSION_COL_HTTP_PATH: usize = 31;
const SESSION_COL_HTTP_QUERY: usize = 32;
const SESSION_COL_HTTP_STATUS: usize = 33;
const SESSION_COL_HTTP_REQUEST_BYTES: usize = 34;
const SESSION_COL_HTTP_RESPONSE_BYTES: usize = 35;
const SESSION_COL_DNS_QNAME: usize = 36;
const SESSION_COL_MCP_SERVER_ID: usize = 37;
const SESSION_COL_MCP_TOOL_NAME: usize = 38;
const SESSION_COL_MODEL_PROVIDER: usize = 39;
const SESSION_COL_MODEL_NAME: usize = 40;
const SESSION_COL_MODEL_INPUT_TOKENS: usize = 41;
const SESSION_COL_MODEL_OUTPUT_TOKENS: usize = 42;
const SESSION_COL_FILE_OPERATION: usize = 43;
const SESSION_COL_FILE_PATH: usize = 44;
const SESSION_COL_FILE_BYTE_COUNT: usize = 45;
const SESSION_COL_PROCESS_COMMAND: usize = 46;
const SESSION_COL_PROCESS_NAME: usize = 47;
const SESSION_COL_SNAPSHOT_SLOT: usize = 48;
const SESSION_COL_SNAPSHOT_NAME: usize = 50;
const SESSION_COL_AI_INTERACTION_ID: usize = 51;
const SESSION_COL_AI_TRACE_ID: usize = 52;
const SESSION_COL_AI_ATTRIBUTION_SCOPE: usize = 53;
const SESSION_COL_AI_SOURCE_ENGINE: usize = 54;
const SESSION_COL_AI_ORIGIN_KIND: usize = 55;
const SESSION_COL_AI_ACCOUNTING_OWNER: usize = 56;
const SESSION_COL_AI_PROFILE_ID: usize = 57;
const SESSION_COL_AI_VM_ID: usize = 58;
const SESSION_COL_AI_SESSION_ID: usize = 59;
const SESSION_COL_AI_USER_ID: usize = 60;
const SESSION_COL_AI_PROVIDER: usize = 61;
const SESSION_COL_AI_API_FAMILY: usize = 62;
const SESSION_COL_AI_MODEL: usize = 63;
const SESSION_COL_AI_PARSE_STATUS: usize = 64;
const SESSION_COL_AI_EVIDENCE_STATUS: usize = 65;
const SESSION_COL_AI_REQUEST_ID: usize = 66;
const SESSION_COL_AI_REQUEST_MODEL: usize = 67;
const SESSION_COL_AI_REQUEST_STREAM: usize = 68;
const SESSION_COL_AI_REQUEST_SYSTEM_PROMPT: usize = 69;
const SESSION_COL_AI_REQUEST_MESSAGE_COUNT: usize = 70;
const SESSION_COL_AI_REQUEST_TOOLS_COUNT: usize = 71;
const SESSION_COL_AI_REQUEST_RAW_SHAPE: usize = 72;
const SESSION_COL_AI_REQUEST_UNKNOWN_FIELDS: usize = 73;
const SESSION_COL_AI_RESPONSE_ID: usize = 74;
const SESSION_COL_AI_RESPONSE_PROVIDER_ID: usize = 75;
const SESSION_COL_AI_RESPONSE_STOP_REASON: usize = 76;
const SESSION_COL_AI_RESPONSE_TEXT_PREVIEW: usize = 77;
const SESSION_COL_AI_RESPONSE_THINKING_PREVIEW: usize = 78;
const SESSION_COL_AI_RESPONSE_RAW_SHAPE: usize = 79;
const SESSION_COL_AI_USAGE_INPUT_TOKENS: usize = 80;
const SESSION_COL_AI_USAGE_OUTPUT_TOKENS: usize = 81;
const SESSION_COL_AI_USAGE_COST_MICROS: usize = 82;
const SESSION_COL_MCP_EVIDENCE_CALL_ID: usize = 83;
const SESSION_COL_MCP_EVIDENCE_SERVER_ID: usize = 84;
const SESSION_COL_MCP_EVIDENCE_TOOL_NAME: usize = 85;
const SESSION_COL_MCP_EVIDENCE_NAMESPACED_TOOL: usize = 86;
const SESSION_COL_MCP_EVIDENCE_TRANSPORT: usize = 87;
const SESSION_COL_MCP_EVIDENCE_REQUEST_RAW: usize = 88;
const SESSION_COL_MCP_EVIDENCE_REQUEST_JSON: usize = 89;
const SESSION_COL_MCP_EVIDENCE_RESULT_KIND: usize = 90;
const SESSION_COL_MCP_EVIDENCE_RESULT_PREVIEW: usize = 91;
const SESSION_COL_MCP_EVIDENCE_RESULT_JSON: usize = 92;
const SESSION_COL_MCP_EVIDENCE_IS_ERROR: usize = 93;
const SESSION_COL_MCP_EVIDENCE_LATENCY_MS: usize = 94;
const SESSION_COL_MCP_EVIDENCE_LINKED_INTERACTION: usize = 95;
const SESSION_COL_MCP_EVIDENCE_LINKED_TOOL_CALL: usize = 96;
const SESSION_COL_MCP_EVIDENCE_LINK_STATUS: usize = 97;

fn session_ai_usage_from_row(row: &serde_json::Value) -> Result<seceng::AiUsageEvidence, AppError> {
    Ok(seceng::AiUsageEvidence {
        input_tokens: session_optional_u64(row, SESSION_COL_AI_USAGE_INPUT_TOKENS)?,
        output_tokens: session_optional_u64(row, SESSION_COL_AI_USAGE_OUTPUT_TOKENS)?,
        estimated_cost_micros: session_optional_u64(row, SESSION_COL_AI_USAGE_COST_MICROS)?,
        details: std::collections::BTreeMap::new(),
    })
}

fn session_model_evidence_from_row(
    reader: &capsem_logger::DbReader,
    row: &serde_json::Value,
) -> Result<Option<seceng::ModelInteractionEvidence>, AppError> {
    let interaction_id = match session_optional_string(row, SESSION_COL_AI_INTERACTION_ID)? {
        Some(interaction_id) => interaction_id,
        None => return Ok(None),
    };
    let provider = parse_session_enum::<seceng::AiProvider>(
        &session_required_string(row, SESSION_COL_AI_PROVIDER)?,
        "AI provider",
    )?;
    let api_family = parse_session_enum::<seceng::AiApiFamily>(
        &session_required_string(row, SESSION_COL_AI_API_FAMILY)?,
        "AI API family",
    )?;
    let usage = session_ai_usage_from_row(row)?;
    let response = match session_optional_string(row, SESSION_COL_AI_RESPONSE_ID)? {
        Some(response_id) => Some(seceng::ModelResponseEvidence {
            response_id,
            provider_response_id: session_optional_string(
                row,
                SESSION_COL_AI_RESPONSE_PROVIDER_ID,
            )?,
            stop_reason: session_optional_string(row, SESSION_COL_AI_RESPONSE_STOP_REASON)?,
            text_preview: session_optional_string(row, SESSION_COL_AI_RESPONSE_TEXT_PREVIEW)?,
            thinking_preview: session_optional_string(
                row,
                SESSION_COL_AI_RESPONSE_THINKING_PREVIEW,
            )?,
            content_blocks: Vec::new(),
            usage: usage.clone(),
            raw_shape_version: session_optional_string(row, SESSION_COL_AI_RESPONSE_RAW_SHAPE)?
                .unwrap_or_else(|| "unknown".into()),
        }),
        None => None,
    };
    let tool_calls = session_model_tool_calls(reader, &interaction_id)?;
    let tool_results = session_model_tool_results(reader, &interaction_id)?;
    Ok(Some(seceng::ModelInteractionEvidence {
        interaction_id,
        trace_id: session_required_string(row, SESSION_COL_AI_TRACE_ID)?,
        attribution_scope: parse_session_attribution_scope(&session_required_string(
            row,
            SESSION_COL_AI_ATTRIBUTION_SCOPE,
        )?)?,
        source_engine: parse_session_source_engine(&session_required_string(
            row,
            SESSION_COL_AI_SOURCE_ENGINE,
        )?)?,
        origin_kind: parse_session_origin_kind(&session_required_string(
            row,
            SESSION_COL_AI_ORIGIN_KIND,
        )?)?,
        accounting_owner: session_optional_string(row, SESSION_COL_AI_ACCOUNTING_OWNER)?,
        profile_id: session_optional_string(row, SESSION_COL_AI_PROFILE_ID)?,
        vm_id: session_optional_string(row, SESSION_COL_AI_VM_ID)?,
        session_id: session_optional_string(row, SESSION_COL_AI_SESSION_ID)?,
        user_id: session_optional_string(row, SESSION_COL_AI_USER_ID)?,
        provider,
        api_family,
        model: session_required_string(row, SESSION_COL_AI_MODEL)?,
        request: seceng::ModelRequestEvidence {
            request_id: session_required_string(row, SESSION_COL_AI_REQUEST_ID)?,
            provider,
            api_family,
            model: session_optional_string(row, SESSION_COL_AI_REQUEST_MODEL)?,
            stream: session_optional_bool(row, SESSION_COL_AI_REQUEST_STREAM)?.unwrap_or(false),
            system_prompt_preview: session_optional_string(
                row,
                SESSION_COL_AI_REQUEST_SYSTEM_PROMPT,
            )?,
            message_count: session_optional_u64(row, SESSION_COL_AI_REQUEST_MESSAGE_COUNT)?
                .unwrap_or_default(),
            tools_declared_count: session_optional_u64(row, SESSION_COL_AI_REQUEST_TOOLS_COUNT)?
                .unwrap_or_default(),
            raw_shape_version: session_required_string(row, SESSION_COL_AI_REQUEST_RAW_SHAPE)?,
            unknown_fields_present: session_optional_bool(
                row,
                SESSION_COL_AI_REQUEST_UNKNOWN_FIELDS,
            )?
            .unwrap_or(false),
        },
        response,
        tool_calls,
        tool_results,
        mcp_executions: Vec::new(),
        usage,
        parse_status: parse_session_enum::<seceng::ParseStatus>(
            &session_required_string(row, SESSION_COL_AI_PARSE_STATUS)?,
            "AI parse status",
        )?,
        evidence_status: parse_session_enum::<seceng::EvidenceStatus>(
            &session_required_string(row, SESSION_COL_AI_EVIDENCE_STATUS)?,
            "AI evidence status",
        )?,
    }))
}

fn session_tool_call_row_string(row: &serde_json::Value, index: usize) -> Result<String, AppError> {
    row.as_array()
        .and_then(|cells| cells.get(index))
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session model tool-call row missing string column {index}"),
            )
        })
}

fn session_tool_call_row_optional_string(
    row: &serde_json::Value,
    index: usize,
) -> Result<Option<String>, AppError> {
    let value = row
        .as_array()
        .and_then(|cells| cells.get(index))
        .ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session model tool-call row missing column {index}"),
            )
        })?;
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_str()
            .map(|value| Some(value.to_owned()))
            .ok_or_else(|| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("session model tool-call column {index} was not a nullable string"),
                )
            })
    }
}

fn session_tool_call_row_u64(row: &serde_json::Value, index: usize) -> Result<u64, AppError> {
    let value = row
        .as_array()
        .and_then(|cells| cells.get(index))
        .ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session model tool-call row missing column {index}"),
            )
        })?;
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session model tool-call column {index} was not an unsigned integer"),
            )
        })
}

fn session_model_tool_calls(
    reader: &capsem_logger::DbReader,
    interaction_id: &str,
) -> Result<Vec<seceng::ModelToolCallEvidence>, AppError> {
    let json_str = reader
        .query_raw_with_params(
            "SELECT
                tc.tool_call_id, tc.call_index, tc.provider_call_id,
                tc.raw_name, tc.normalized_name, tc.arguments_raw,
                tc.arguments_json, tc.arguments_status, tc.origin,
                tc.linked_mcp_call_id, tc.status, tc.parse_confidence
             FROM ai_model_interactions ami
             JOIN ai_model_tool_calls tc ON tc.interaction_id = ami.id
             WHERE ami.interaction_id = ?
             ORDER BY tc.call_index ASC, tc.id ASC",
            &[serde_json::Value::String(interaction_id.to_owned())],
        )
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("query session model tool calls: {error}"),
            )
        })?;
    let value: serde_json::Value = serde_json::from_str(&json_str).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("parse session model tool calls: {error}"),
        )
    })?;

    let mut tool_calls = Vec::new();
    for row in value
        .get("rows")
        .and_then(|rows| rows.as_array())
        .cloned()
        .unwrap_or_default()
    {
        tool_calls.push(seceng::ModelToolCallEvidence {
            tool_call_id: session_tool_call_row_string(&row, 0)?,
            index: session_tool_call_row_u64(&row, 1)?,
            provider_call_id: session_tool_call_row_optional_string(&row, 2)?,
            raw_name: session_tool_call_row_string(&row, 3)?,
            normalized_name: session_tool_call_row_string(&row, 4)?,
            arguments_raw: session_tool_call_row_optional_string(&row, 5)?,
            arguments_json: session_tool_call_row_optional_string(&row, 6)?,
            arguments_status: parse_session_enum::<seceng::ArgumentsStatus>(
                &session_tool_call_row_string(&row, 7)?,
                "model tool-call arguments status",
            )?,
            origin: parse_session_enum::<seceng::ToolOrigin>(
                &session_tool_call_row_string(&row, 8)?,
                "model tool-call origin",
            )?,
            linked_mcp_call_id: session_tool_call_row_optional_string(&row, 9)?,
            status: parse_session_enum::<seceng::ToolCallStatus>(
                &session_tool_call_row_string(&row, 10)?,
                "model tool-call status",
            )?,
            parse_confidence: parse_session_enum::<seceng::Confidence>(
                &session_tool_call_row_string(&row, 11)?,
                "model tool-call parse confidence",
            )?,
        });
    }
    Ok(tool_calls)
}

fn session_model_tool_results(
    reader: &capsem_logger::DbReader,
    interaction_id: &str,
) -> Result<Vec<seceng::ModelToolResultEvidence>, AppError> {
    let json_str = reader
        .query_raw_with_params(
            "SELECT
                tr.tool_call_id, tr.linked_mcp_call_id, tr.content_kind,
                tr.content_preview, tr.content_json, tr.is_error,
                tr.result_status, tr.returned_to_model, tr.parse_confidence
             FROM ai_model_interactions ami
             JOIN ai_model_tool_results tr ON tr.interaction_id = ami.id
             WHERE ami.interaction_id = ?
             ORDER BY tr.id ASC",
            &[serde_json::Value::String(interaction_id.to_owned())],
        )
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("query session model tool results: {error}"),
            )
        })?;
    let value: serde_json::Value = serde_json::from_str(&json_str).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("parse session model tool results: {error}"),
        )
    })?;

    let mut tool_results = Vec::new();
    for row in value
        .get("rows")
        .and_then(|rows| rows.as_array())
        .cloned()
        .unwrap_or_default()
    {
        tool_results.push(seceng::ModelToolResultEvidence {
            tool_call_id: session_tool_call_row_string(&row, 0)?,
            linked_mcp_call_id: session_tool_call_row_optional_string(&row, 1)?,
            content_kind: parse_session_enum::<seceng::AiContentKind>(
                &session_tool_call_row_string(&row, 2)?,
                "model tool-result content kind",
            )?,
            content_preview: session_tool_call_row_optional_string(&row, 3)?,
            content_json: session_tool_call_row_optional_string(&row, 4)?,
            is_error: session_optional_bool(&row, 5)?.unwrap_or(false),
            result_status: parse_session_enum::<seceng::ToolCallStatus>(
                &session_tool_call_row_string(&row, 6)?,
                "model tool-result status",
            )?,
            returned_to_model: session_optional_bool(&row, 7)?.unwrap_or(false),
            parse_confidence: parse_session_enum::<seceng::Confidence>(
                &session_tool_call_row_string(&row, 8)?,
                "model tool-result parse confidence",
            )?,
        });
    }
    Ok(tool_results)
}

fn session_mcp_evidence_from_row(
    row: &serde_json::Value,
) -> Result<Option<seceng::McpToolExecutionEvidence>, AppError> {
    let mcp_call_id = match session_optional_string(row, SESSION_COL_MCP_EVIDENCE_CALL_ID)? {
        Some(mcp_call_id) => mcp_call_id,
        None => return Ok(None),
    };
    Ok(Some(seceng::McpToolExecutionEvidence {
        mcp_call_id,
        server_id: session_required_string(row, SESSION_COL_MCP_EVIDENCE_SERVER_ID)?,
        tool_name: session_required_string(row, SESSION_COL_MCP_EVIDENCE_TOOL_NAME)?,
        namespaced_tool_name: session_required_string(
            row,
            SESSION_COL_MCP_EVIDENCE_NAMESPACED_TOOL,
        )?,
        transport: session_required_string(row, SESSION_COL_MCP_EVIDENCE_TRANSPORT)?,
        request_arguments_raw: session_optional_string(row, SESSION_COL_MCP_EVIDENCE_REQUEST_RAW)?,
        request_arguments_json: session_optional_string(
            row,
            SESSION_COL_MCP_EVIDENCE_REQUEST_JSON,
        )?,
        result_kind: parse_session_enum::<seceng::AiContentKind>(
            &session_required_string(row, SESSION_COL_MCP_EVIDENCE_RESULT_KIND)?,
            "MCP evidence result kind",
        )?,
        result_preview: session_optional_string(row, SESSION_COL_MCP_EVIDENCE_RESULT_PREVIEW)?,
        result_json: session_optional_string(row, SESSION_COL_MCP_EVIDENCE_RESULT_JSON)?,
        is_error: session_optional_bool(row, SESSION_COL_MCP_EVIDENCE_IS_ERROR)?.unwrap_or(false),
        latency_ms: session_optional_u64(row, SESSION_COL_MCP_EVIDENCE_LATENCY_MS)?
            .unwrap_or_default(),
        linked_model_interaction_id: session_optional_string(
            row,
            SESSION_COL_MCP_EVIDENCE_LINKED_INTERACTION,
        )?,
        linked_model_tool_call_id: session_optional_string(
            row,
            SESSION_COL_MCP_EVIDENCE_LINKED_TOOL_CALL,
        )?,
        link_status: parse_session_enum::<seceng::LinkStatus>(
            &session_required_string(row, SESSION_COL_MCP_EVIDENCE_LINK_STATUS)?,
            "MCP evidence link status",
        )?,
    }))
}

fn session_security_event_common_from_row(
    row: &serde_json::Value,
) -> Result<seceng::SecurityEventCommon, AppError> {
    Ok(seceng::SecurityEventCommon {
        event_id: session_required_string(row, SESSION_COL_EVENT_ID)?,
        parent_event_id: session_optional_string(row, SESSION_COL_PARENT_EVENT_ID)?,
        stream_id: session_optional_string(row, SESSION_COL_STREAM_ID)?,
        activity_id: session_optional_string(row, SESSION_COL_ACTIVITY_ID)?,
        sequence_no: session_optional_u64(row, SESSION_COL_SEQUENCE_NO)?,
        source_engine: parse_session_source_engine(&session_required_string(
            row,
            SESSION_COL_SOURCE_ENGINE,
        )?)?,
        attribution_scope: parse_session_attribution_scope(&session_required_string(
            row,
            SESSION_COL_ATTRIBUTION_SCOPE,
        )?)?,
        origin_kind: parse_session_origin_kind(&session_required_string(
            row,
            SESSION_COL_ORIGIN_KIND,
        )?)?,
        accounting_owner: session_optional_string(row, SESSION_COL_ACCOUNTING_OWNER)?,
        enforceability: parse_session_enforceability(&session_required_string(
            row,
            SESSION_COL_ENFORCEABILITY,
        )?)?,
        trace_id: session_optional_string(row, SESSION_COL_TRACE_ID)?,
        span_id: session_optional_string(row, SESSION_COL_SPAN_ID)?,
        timestamp_unix_ms: session_required_u64(row, SESSION_COL_TIMESTAMP_UNIX_MS)?,
        vm_id: session_optional_string(row, SESSION_COL_VM_ID)?,
        session_id: session_optional_string(row, SESSION_COL_SESSION_ID)?,
        profile_id: session_optional_string(row, SESSION_COL_PROFILE_ID)?,
        profile_revision: session_optional_string(row, SESSION_COL_PROFILE_REVISION)?,
        profile_pack_ids: Vec::new(),
        enforcement_packs: Vec::new(),
        detection_packs: Vec::new(),
        user_id: session_optional_string(row, SESSION_COL_USER_ID)?,
        process_id: session_optional_string(row, SESSION_COL_PROCESS_ID)?,
        parent_process_id: session_optional_string(row, SESSION_COL_PARENT_PROCESS_ID)?,
        exec_id: session_optional_string(row, SESSION_COL_EXEC_ID)?,
        turn_id: session_optional_string(row, SESSION_COL_TURN_ID)?,
        message_id: session_optional_string(row, SESSION_COL_MESSAGE_ID)?,
        tool_call_id: session_optional_string(row, SESSION_COL_TOOL_CALL_ID)?,
        mcp_call_id: session_optional_string(row, SESSION_COL_MCP_CALL_ID)?,
        event_type: session_required_string(row, SESSION_COL_EVENT_TYPE)?,
        redaction_state: parse_session_redaction_state(&session_required_string(
            row,
            SESSION_COL_REDACTION_STATE,
        )?)?,
    })
}

fn session_event_operation(event_type: &str, fallback: &str) -> String {
    event_type
        .split_once('.')
        .map(|(_, operation)| operation)
        .filter(|operation| !operation.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn session_domain_class(qname: &str) -> String {
    if qname == "localhost"
        || qname.ends_with(".localhost")
        || qname.ends_with(".internal")
        || qname.contains("metadata")
    {
        "internal".into()
    } else {
        "external".into()
    }
}

fn session_file_path_class(path: &str) -> String {
    if path == "/workspace" || path.starts_with("/workspace/") {
        "workspace".into()
    } else if path == "/tmp" || path.starts_with("/tmp/") {
        "temporary".into()
    } else if path.starts_with("/var/folders/") {
        "temporary".into()
    } else {
        "unknown".into()
    }
}

fn session_security_event_from_row(
    reader: &capsem_logger::DbReader,
    row: &serde_json::Value,
) -> Result<Option<seceng::SecurityEvent>, AppError> {
    let event_family = session_required_string(row, SESSION_COL_EVENT_FAMILY)?;
    let common = session_security_event_common_from_row(row)?;
    match event_family.as_str() {
        "http" => {
            let host = match session_optional_string(row, SESSION_COL_HTTP_HOST)? {
                Some(host) => host,
                None => return Ok(None),
            };
            let method = session_optional_string(row, SESSION_COL_HTTP_METHOD)?
                .unwrap_or_else(|| "GET".into());
            let path = session_optional_string(row, SESSION_COL_HTTP_PATH)?;
            let query = session_optional_string(row, SESSION_COL_HTTP_QUERY)?;
            let port = session_optional_u64(row, SESSION_COL_HTTP_PORT)?
                .and_then(|value| u16::try_from(value).ok());
            let status = session_optional_u64(row, SESSION_COL_HTTP_STATUS)?
                .and_then(|value| u16::try_from(value).ok());
            let request_bytes =
                session_optional_u64(row, SESSION_COL_HTTP_REQUEST_BYTES)?.unwrap_or_default();
            let response_bytes = session_optional_u64(row, SESSION_COL_HTTP_RESPONSE_BYTES)?;
            let url = Some(match (&path, &query) {
                (Some(path), Some(query)) if !query.is_empty() => {
                    format!("https://{host}{path}?{query}")
                }
                (Some(path), _) => format!("https://{host}{path}"),
                _ => format!("https://{host}"),
            });
            Ok(Some(seceng::SecurityEvent::http(
                common,
                seceng::HttpSecuritySubject {
                    method,
                    scheme: Some("https".into()),
                    host,
                    port,
                    path_class: path.clone().unwrap_or_default(),
                    path,
                    query,
                    url,
                    request_bytes,
                    request_headers: Default::default(),
                    request_body: None,
                    response_status: status,
                    response_headers: Default::default(),
                    response_bytes,
                    response_body: None,
                },
            )))
        }
        "dns" => {
            let qname = match session_optional_string(row, SESSION_COL_DNS_QNAME)? {
                Some(qname) => qname,
                None => return Ok(None),
            };
            let domain_class = session_domain_class(&qname);
            Ok(Some(seceng::SecurityEvent::dns(
                common,
                seceng::DnsSecuritySubject {
                    qname,
                    domain_class,
                },
            )))
        }
        "mcp" => {
            let evidence = session_mcp_evidence_from_row(row)?;
            let server_id = evidence
                .as_ref()
                .map(|evidence| evidence.server_id.clone())
                .or_else(|| {
                    session_optional_string(row, SESSION_COL_MCP_SERVER_ID)
                        .ok()
                        .flatten()
                });
            let tool_name = evidence
                .as_ref()
                .map(|evidence| evidence.tool_name.clone())
                .or_else(|| {
                    session_optional_string(row, SESSION_COL_MCP_TOOL_NAME)
                        .ok()
                        .flatten()
                });
            let (Some(server_id), Some(tool_name)) = (server_id, tool_name) else {
                return Ok(None);
            };
            Ok(Some(seceng::SecurityEvent::mcp(
                common,
                seceng::McpSecuritySubject {
                    server_id,
                    tool_name,
                    evidence: evidence.map(Box::new),
                },
            )))
        }
        "model" => {
            if let Some(evidence) = session_model_evidence_from_row(reader, row)? {
                return Ok(Some(seceng::SecurityEvent::model(
                    common,
                    seceng::ModelSecuritySubject::from_interaction_evidence(evidence),
                )));
            }
            let provider = match session_optional_string(row, SESSION_COL_MODEL_PROVIDER)? {
                Some(provider) => provider,
                None => return Ok(None),
            };
            let model = match session_optional_string(row, SESSION_COL_MODEL_NAME)? {
                Some(model) => model,
                None => return Ok(None),
            };
            Ok(Some(seceng::SecurityEvent::model(
                common,
                seceng::ModelSecuritySubject {
                    provider,
                    model,
                    estimated_input_tokens: session_optional_u64(
                        row,
                        SESSION_COL_MODEL_INPUT_TOKENS,
                    )?,
                    estimated_output_tokens: session_optional_u64(
                        row,
                        SESSION_COL_MODEL_OUTPUT_TOKENS,
                    )?,
                    estimated_cost_micros: None,
                    evidence: None,
                },
            )))
        }
        "file" => {
            let operation = session_optional_string(row, SESSION_COL_FILE_OPERATION)?
                .unwrap_or_else(|| session_event_operation(&common.event_type, "activity"));
            let path = session_optional_string(row, SESSION_COL_FILE_PATH)?;
            let path_class = path
                .as_deref()
                .map(session_file_path_class)
                .unwrap_or_else(|| "unknown".into());
            Ok(Some(seceng::SecurityEvent::file(
                common,
                seceng::FileSecuritySubject {
                    operation,
                    path,
                    path_class,
                    byte_count: session_optional_u64(row, SESSION_COL_FILE_BYTE_COUNT)?,
                },
            )))
        }
        "process" => {
            let operation = session_event_operation(&common.event_type, "activity");
            let command = session_optional_string(row, SESSION_COL_PROCESS_COMMAND)?;
            let process_name = session_optional_string(row, SESSION_COL_PROCESS_NAME)?;
            let command_class = command
                .as_deref()
                .and_then(capsem_core::process_security_events::classify_command_class)
                .or_else(|| {
                    process_name
                        .as_deref()
                        .and_then(capsem_core::process_security_events::classify_command_class)
                })
                .map(str::to_owned);
            Ok(Some(seceng::SecurityEvent::process(
                common,
                seceng::ProcessSecuritySubject {
                    operation,
                    command_class,
                },
            )))
        }
        "snapshot" => {
            let operation = session_event_operation(&common.event_type, "activity");
            let snapshot_id = session_optional_string(row, SESSION_COL_SNAPSHOT_NAME)?
                .or_else(|| {
                    session_optional_u64(row, SESSION_COL_SNAPSHOT_SLOT)
                        .ok()
                        .flatten()
                        .map(|slot| slot.to_string())
                })
                .unwrap_or_else(|| common.event_id.clone());
            Ok(Some(seceng::SecurityEvent::snapshot(
                common,
                seceng::SnapshotSecuritySubject {
                    operation,
                    snapshot_id,
                },
            )))
        }
        "vm" => {
            let operation = session_event_operation(&common.event_type, "activity");
            Ok(Some(seceng::SecurityEvent::vm_lifecycle(
                common,
                seceng::VmLifecycleSecuritySubject { operation },
            )))
        }
        "profile" => {
            let operation = session_event_operation(&common.event_type, "activity");
            let profile_id = common.profile_id.clone().unwrap_or_default();
            let profile_revision = common.profile_revision.clone().unwrap_or_default();
            Ok(Some(seceng::SecurityEvent::profile(
                common,
                seceng::ProfileSecuritySubject {
                    operation,
                    profile_id,
                    profile_revision,
                },
            )))
        }
        "conversation" => {
            let operation = session_event_operation(&common.event_type, "activity");
            let conversation_id = common
                .activity_id
                .clone()
                .or_else(|| common.turn_id.clone());
            Ok(Some(seceng::SecurityEvent::conversation(
                common,
                seceng::ConversationSecuritySubject {
                    operation,
                    conversation_id,
                },
            )))
        }
        _ => Ok(None),
    }
}

fn session_backtest_events(
    session_id: &str,
    reader: &capsem_logger::DbReader,
) -> Result<Vec<RuntimeBacktestEvent>, AppError> {
    let mut events = Vec::new();
    for row in security_events_query_rows(reader)? {
        if let Some(event) = session_security_event_from_row(reader, &row)? {
            events.push(RuntimeBacktestEvent {
                event_ref: Some(seceng::BacktestEventRef {
                    corpus: "session_db".into(),
                    session_id: event
                        .common
                        .session_id
                        .clone()
                        .or_else(|| Some(session_id.to_owned())),
                    event_id: event.common.event_id.clone(),
                    sequence_no: event.common.sequence_no,
                    timestamp_unix_ms: event.common.timestamp_unix_ms,
                }),
                event,
                expected: None,
            });
        }
    }
    Ok(events)
}

fn security_decision_action_text(
    action: seceng::SecurityDecisionAction,
) -> Result<String, AppError> {
    serde_json::to_value(action)
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize security decision action: {error}"),
            )
        })?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "security decision action did not serialize as a string".into(),
            )
        })
}

async fn handle_compile_enforcement_rule(
    Json(request): Json<RuntimeEnforcementRuleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_runtime_rule_id(&request.id)?;
    let compiled_plan = compile_runtime_enforcement_rule(&request).map_err(|error| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("compile enforcement rule: {error}"),
        )
    })?;
    Ok(Json(json!({
        "compiled": true,
        "id": request.id,
        "compiled_plan": compiled_plan,
    })))
}

async fn handle_validate_enforcement_rule(
    Json(request): Json<RuntimeEnforcementRuleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    handle_compile_enforcement_rule(Json(request)).await
}

async fn handle_create_enforcement_rule(
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<RuntimeEnforcementRuleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_runtime_rule_id(&request.id)?;
    let compiled_plan = compile_runtime_enforcement_rule(&request).map_err(|error| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("compile enforcement rule: {error}"),
        )
    })?;
    let record = runtime_enforcement_record(&request);
    let rule = {
        let mut registry = state.enforcement_registry.lock().map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("runtime enforcement registry lock poisoned: {error}"),
            )
        })?;
        registry
            .add_or_update(record, |_| Ok(compiled_plan.clone()))
            .map_err(|error| AppError(StatusCode::BAD_REQUEST, format!("install rule: {error}")))?;
        registry
            .list()
            .into_iter()
            .find(|entry| entry.metadata.id == request.id)
            .map(runtime_rule_entry_json)
            .ok_or_else(|| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "installed enforcement rule '{}' was not readable",
                        request.id
                    ),
                )
            })?
    };
    let propagation = broadcast_runtime_security_rules(&state).await?;
    Ok(Json(json!({
        "kind": "enforcement",
        "rule": rule,
        "propagation": propagation.json(),
    })))
}

async fn handle_update_enforcement_rule(
    Path(id): Path<String>,
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<RuntimeEnforcementRuleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if request.id != id {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "path rule id must match request id".into(),
        ));
    }
    handle_create_enforcement_rule(State(state), Json(request)).await
}

async fn handle_delete_enforcement_rule(
    Path(id): Path<String>,
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_runtime_rule_id(&id)?;
    {
        let mut registry = state.enforcement_registry.lock().map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("runtime enforcement registry lock poisoned: {error}"),
            )
        })?;
        registry
            .delete(&id)
            .map_err(|error| AppError(StatusCode::NOT_FOUND, error.to_string()))?;
    }
    let propagation = broadcast_runtime_security_rules(&state).await?;
    Ok(Json(json!({
        "kind": "enforcement",
        "id": id,
        "removed": true,
        "propagation": propagation.json(),
    })))
}

async fn handle_list_enforcement_rules(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(json!({
        "kind": "enforcement",
        "rules": runtime_registry_rules_json(&state.enforcement_registry)?,
    })))
}

async fn handle_enforcement_stats(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let sync = drain_runtime_rule_matches_from_processes(&state).await?;
    Ok(Json(json!({
        "kind": "enforcement",
        "rules": runtime_registry_rules_json(&state.enforcement_registry)?,
        "sync": sync.json(),
    })))
}

async fn handle_enforcement_backtest(
    Json(request): Json<RuntimeEnforcementBacktestRequest>,
) -> Result<Json<seceng::BacktestResult>, AppError> {
    validate_runtime_rule_id(&request.rule.id)?;
    let mut evaluator =
        seceng::CelEnforcementEvaluator::compile(vec![seceng::CelEnforcementRule {
            id: request.rule.id.clone(),
            pack_id: request.rule.pack_id.clone(),
            condition: request.rule.condition.clone(),
            decision: request.rule.decision,
            reason: request.rule.reason.clone(),
        }])
        .map_err(|error| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("compile enforcement rule: {error}"),
            )
        })?;

    let mut rows = Vec::new();
    for input in &request.events {
        if let Some(decision) = seceng::EnforcementEvaluator::evaluate(&mut evaluator, &input.event)
            .map_err(|error| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    format!("backtest enforcement rule: {error}"),
                )
            })?
        {
            let actual = security_decision_action_text(decision.action)?;
            rows.push(seceng::BacktestMatchRow {
                event_ref: inline_backtest_event_ref(input),
                rule_id: decision.rule.unwrap_or_else(|| request.rule.id.clone()),
                pack_id: decision
                    .pack_id
                    .or_else(|| request.rule.pack_id.clone())
                    .unwrap_or_else(|| "runtime".into()),
                evidence_signature: backtest_evidence_signature(&input.event)?,
                matched_fields: backtest_matched_fields(&input.event)?,
                outcome: backtest_outcome(input.expected.as_deref(), &actual),
            });
        }
    }

    Ok(Json(seceng::dedupe_backtest_matches(
        rows,
        runtime_backtest_limit(request.limit),
    )))
}

async fn handle_compile_detection_rule(
    Json(request): Json<RuntimeDetectionRuleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_runtime_rule_id(&request.id)?;
    let compiled_plan = compile_runtime_detection_rule(&request).map_err(|error| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("compile detection rule: {error}"),
        )
    })?;
    Ok(Json(json!({
        "compiled": true,
        "id": request.id,
        "compiled_plan": compiled_plan,
    })))
}

async fn handle_validate_detection_rule(
    Json(request): Json<RuntimeDetectionRuleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    handle_compile_detection_rule(Json(request)).await
}

async fn handle_create_detection_rule(
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<RuntimeDetectionRuleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_runtime_rule_id(&request.id)?;
    let compiled_plan = compile_runtime_detection_rule(&request).map_err(|error| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("compile detection rule: {error}"),
        )
    })?;
    let record = runtime_detection_record(&request);
    let rule = {
        let mut registry = state.detection_registry.lock().map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("runtime detection registry lock poisoned: {error}"),
            )
        })?;
        registry
            .add_or_update(record, |_| Ok(compiled_plan.clone()))
            .map_err(|error| AppError(StatusCode::BAD_REQUEST, format!("install rule: {error}")))?;
        registry
            .list()
            .into_iter()
            .find(|entry| entry.metadata.id == request.id)
            .map(runtime_rule_entry_json)
            .ok_or_else(|| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("installed detection rule '{}' was not readable", request.id),
                )
            })?
    };
    let propagation = broadcast_runtime_security_rules(&state).await?;
    Ok(Json(json!({
        "kind": "detection",
        "rule": rule,
        "propagation": propagation.json(),
    })))
}

async fn handle_update_detection_rule(
    Path(id): Path<String>,
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<RuntimeDetectionRuleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if request.id != id {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "path rule id must match request id".into(),
        ));
    }
    handle_create_detection_rule(State(state), Json(request)).await
}

async fn handle_delete_detection_rule(
    Path(id): Path<String>,
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_runtime_rule_id(&id)?;
    {
        let mut registry = state.detection_registry.lock().map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("runtime detection registry lock poisoned: {error}"),
            )
        })?;
        registry
            .delete(&id)
            .map_err(|error| AppError(StatusCode::NOT_FOUND, error.to_string()))?;
    }
    let propagation = broadcast_runtime_security_rules(&state).await?;
    Ok(Json(json!({
        "kind": "detection",
        "id": id,
        "removed": true,
        "propagation": propagation.json(),
    })))
}

async fn handle_list_detection_rules(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(json!({
        "kind": "detection",
        "rules": runtime_registry_rules_json(&state.detection_registry)?,
    })))
}

async fn handle_detection_stats(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let sync = drain_runtime_rule_matches_from_processes(&state).await?;
    Ok(Json(json!({
        "kind": "detection",
        "rules": runtime_registry_rules_json(&state.detection_registry)?,
        "sync": sync.json(),
    })))
}

async fn handle_detection_backtest(
    Json(request): Json<RuntimeDetectionBacktestRequest>,
) -> Result<Json<seceng::BacktestResult>, AppError> {
    validate_runtime_rule_id(&request.rule.id)?;
    let mut evaluator = seceng::CelDetectionEvaluator::compile(vec![seceng::CelDetectionRule {
        id: request.rule.id.clone(),
        pack_id: request.rule.pack_id.clone(),
        sigma_id: request.rule.sigma_id.clone(),
        title: request.rule.title.clone(),
        condition: request.rule.condition.clone(),
        severity: request.rule.severity,
        confidence: request.rule.confidence,
        tags: request.rule.tags.clone(),
    }])
    .map_err(|error| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("compile detection rule: {error}"),
        )
    })?;

    let mut rows = Vec::new();
    for input in &request.events {
        let findings = seceng::DetectionEvaluator::evaluate(&mut evaluator, &input.event).map_err(
            |error| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    format!("backtest detection rule: {error}"),
                )
            },
        )?;
        for finding in findings {
            rows.push(seceng::BacktestMatchRow {
                event_ref: inline_backtest_event_ref(input),
                rule_id: finding.rule_id,
                pack_id: finding.pack_id,
                evidence_signature: backtest_evidence_signature(&input.event)?,
                matched_fields: backtest_matched_fields(&input.event)?,
                outcome: backtest_outcome(input.expected.as_deref(), "finding"),
            });
        }
    }

    Ok(Json(seceng::dedupe_backtest_matches(
        rows,
        runtime_backtest_limit(request.limit),
    )))
}

fn run_detection_hunt(
    rules: &[RuntimeDetectionRuleRequest],
    events: &[RuntimeBacktestEvent],
    limit: Option<usize>,
) -> Result<seceng::BacktestResult, AppError> {
    if rules.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "detection hunt requires at least one rule".into(),
        ));
    }

    let mut compiled_rules = Vec::with_capacity(rules.len());
    for rule in rules {
        validate_runtime_rule_id(&rule.id)?;
        compiled_rules.push(seceng::CelDetectionRule {
            id: rule.id.clone(),
            pack_id: rule.pack_id.clone(),
            sigma_id: rule.sigma_id.clone(),
            title: rule.title.clone(),
            condition: rule.condition.clone(),
            severity: rule.severity,
            confidence: rule.confidence,
            tags: rule.tags.clone(),
        });
    }

    let mut evaluator =
        seceng::CelDetectionEvaluator::compile(compiled_rules).map_err(|error| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("compile detection hunt rules: {error}"),
            )
        })?;

    let mut rows = Vec::new();
    for input in events {
        let findings = seceng::DetectionEvaluator::evaluate(&mut evaluator, &input.event).map_err(
            |error| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    format!("hunt detection rules: {error}"),
                )
            },
        )?;
        for finding in findings {
            rows.push(seceng::BacktestMatchRow {
                event_ref: inline_backtest_event_ref(input),
                rule_id: finding.rule_id,
                pack_id: finding.pack_id,
                evidence_signature: backtest_evidence_signature(&input.event)?,
                matched_fields: backtest_matched_fields(&input.event)?,
                outcome: backtest_outcome(input.expected.as_deref(), "finding"),
            });
        }
    }

    Ok(seceng::dedupe_backtest_matches(
        rows,
        runtime_backtest_limit(limit),
    ))
}

async fn handle_detection_hunt(
    Json(request): Json<RuntimeDetectionHuntRequest>,
) -> Result<Json<seceng::BacktestResult>, AppError> {
    Ok(Json(run_detection_hunt(
        &request.rules,
        &request.events,
        request.limit,
    )?))
}

async fn handle_session_detection_hunt(
    Path(id): Path<String>,
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<RuntimeSessionDetectionHuntRequest>,
) -> Result<Json<seceng::BacktestResult>, AppError> {
    let db_path = resolve_session_dir(&state, &id)?.join("session.db");
    let reader = capsem_logger::DbReader::open(&db_path).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to open session DB for detection hunt: {error}"),
        )
    })?;
    let events = session_backtest_events(&id, &reader)?;
    Ok(Json(run_detection_hunt(
        &request.rules,
        &events,
        request.limit,
    )?))
}

/// GET /confirm/pending -- list pending S15 confirmation prompts.
async fn handle_list_pending_confirms() -> Json<serde_json::Value> {
    Json(json!({
        "mode": "settings_profiles_v2",
        "pending": [],
        "pending_count": 0,
        "resolve_available": false,
        "resolve_owner": "S15-confirm-ux",
    }))
}

/// GET /skills -- list resolved Profile V2 skills for a profile.
async fn handle_list_skills(
    Query(query): Query<SkillsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let target_profile_id = query
        .profile
        .clone()
        .unwrap_or_else(|| settings.profiles.default_profile.clone());
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    let (effective, _) = capsem_core::settings_profiles::resolve_effective_vm_settings_with_corp(
        &settings,
        Some(&target_profile_id),
    )
    .map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("resolve effective profile '{target_profile_id}': {e}"),
        )
    })?;

    let mut skills = Vec::new();
    let kinds = [SkillKind::Group, SkillKind::Enabled, SkillKind::Disabled];
    for kind in kinds {
        if query.kind.is_some_and(|requested| requested != kind) {
            continue;
        }
        let ids = match kind {
            SkillKind::Group => &effective.skills.value.groups,
            SkillKind::Enabled => &effective.skills.value.enabled,
            SkillKind::Disabled => &effective.skills.value.disabled,
        };
        for id in ids {
            let owner = skill_owner(&catalog, &effective.profile_id, kind, id)?;
            skills.push(skill_json(id, kind, owner, &effective.profile_id));
        }
    }
    skills.sort_by(|left, right| {
        left["kind"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["kind"].as_str().unwrap_or_default())
            .then_with(|| {
                left["id"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["id"].as_str().unwrap_or_default())
            })
    });

    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "profile_id": effective.profile_id,
        "groups": effective.skills.value.groups,
        "enabled": effective.skills.value.enabled,
        "disabled": effective.skills.value.disabled,
        "skills": skills,
    })))
}

/// POST /skills -- add a direct Profile V2 skill entry to a user profile.
async fn handle_create_skill(
    Json(request): Json<SkillMutationRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let target_profile_id = request
        .profile
        .clone()
        .unwrap_or_else(|| settings.profiles.default_profile.clone());
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    let selected = catalog.get(&target_profile_id).ok_or_else(|| {
        AppError(
            StatusCode::NOT_FOUND,
            format!("profile '{target_profile_id}' not found"),
        )
    })?;
    ensure_profile_section_editable(&selected.profile, ProfileEditableSection::Skills)?;
    if profile_has_skill(&selected.profile, request.kind, &request.id) {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!(
                "skill_exists: skills.{}.{}",
                request.kind.as_str(),
                request.id
            ),
        ));
    }
    if let Some(owner) = skill_owner(&catalog, &target_profile_id, request.kind, &request.id)? {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!(
                "skill_exists: skills.{}.{} is inherited from profile '{}'",
                request.kind.as_str(),
                request.id,
                owner.profile.id
            ),
        ));
    }

    let mut profile = selected.profile.clone();
    if request.kind == SkillKind::Enabled {
        remove_skill_from(&mut profile, SkillKind::Disabled, &request.id);
    } else if request.kind == SkillKind::Disabled {
        remove_skill_from(&mut profile, SkillKind::Enabled, &request.id);
    }
    skill_list_mut(&mut profile, request.kind).push(request.id.clone());
    profile.validate().map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("profile validation failed: {e}"),
        )
    })?;
    save_mutated_profile(&settings, selected.source, profile)?;

    let Json(listed) = handle_list_skills(Query(SkillsQuery {
        profile: Some(target_profile_id),
        kind: Some(request.kind),
    }))
    .await?;
    let skill = listed["skills"]
        .as_array()
        .and_then(|skills| {
            skills
                .iter()
                .find(|skill| skill["id"] == serde_json::json!(request.id))
                .cloned()
        })
        .ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "created skill '{}' was not visible after profile save",
                    request.id
                ),
            )
        })?;
    Ok(Json(skill))
}

/// DELETE /skills/{id} -- remove a direct user Profile V2 skill entry.
async fn handle_delete_skill(
    Path(skill_id): Path<String>,
    Query(query): Query<SkillsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let kind = query.kind.unwrap_or_default();
    let settings = load_service_settings_for_profiles()?;
    let target_profile_id = query
        .profile
        .clone()
        .unwrap_or_else(|| settings.profiles.default_profile.clone());
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    let selected = catalog.get(&target_profile_id).ok_or_else(|| {
        AppError(
            StatusCode::NOT_FOUND,
            format!("profile '{target_profile_id}' not found"),
        )
    })?;
    ensure_profile_section_editable(&selected.profile, ProfileEditableSection::Skills)?;
    if selected.source != capsem_core::settings_profiles::ProfileSource::User {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!(
                "skill_is_locked: profile '{}' is locked ({:?})",
                selected.profile.id, selected.source
            ),
        ));
    }
    if !profile_has_skill(&selected.profile, kind, &skill_id) {
        let owner = skill_owner(&catalog, &target_profile_id, kind, &skill_id)?;
        return match owner {
            Some(owner) => Err(AppError(
                StatusCode::CONFLICT,
                format!(
                    "skill_is_locked: skill '{}' is inherited from profile '{}'",
                    skill_id, owner.profile.id
                ),
            )),
            None => Err(AppError(
                StatusCode::NOT_FOUND,
                format!("skill '{skill_id}' not found"),
            )),
        };
    }

    let mut profile = selected.profile.clone();
    remove_skill_from(&mut profile, kind, &skill_id);
    profile.validate().map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("profile validation failed: {e}"),
        )
    })?;
    capsem_core::settings_profiles::update_user_profile(&settings.profiles, profile)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("update profile: {e}")))?;

    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "profile_id": target_profile_id,
        "skill_id": skill_id,
        "kind": kind,
        "removed": true,
    })))
}

fn settings_response_json() -> serde_json::Value {
    match load_service_profiles_state() {
        Ok((settings, catalog, effective, trace)) => {
            let snapshot =
                capsem_core::settings_profiles::SettingsProfilesDebugSnapshot::from_parts_with_trace(
                    &settings,
                    &catalog,
                    Some(&effective),
                    Some(&trace),
                );
            json!({
                "profile_presets": profile_presets_json(&catalog),
                "effective_rules": policy_json_from_effective(&effective),
                "settings_profiles": snapshot,
                "mode": "settings_profiles_v2",
            })
        }
        Err(error) => json!({
            "profile_presets": [],
            "effective_rules": {},
            "settings_profiles": capsem_core::settings_profiles::SettingsProfilesDebugSnapshot::from_error(error),
            "mode": "settings_profiles_v2",
        }),
    }
}

/// GET /settings -- typed settings-profiles snapshot + rules/presets.
async fn handle_get_settings() -> Json<serde_json::Value> {
    Json(settings_response_json())
}

/// POST /settings -- batch-update policy rules and return refreshed typed state.
async fn handle_save_settings(
    Json(raw): Json<HashMap<String, serde_json::Value>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings_path = service_settings_path();
    let settings = capsem_core::settings_profiles::load_service_settings_or_default(&settings_path)
        .map_err(|e| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("load {}: {e}", settings_path.display()),
            )
        })?;
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    let selected_id = settings.profiles.default_profile.clone();
    let selected = catalog.get(&selected_id).ok_or_else(|| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("default profile '{selected_id}' not found"),
        )
    })?;
    ensure_profile_section_editable(&selected.profile, ProfileEditableSection::SecurityRules)?;

    let mut profile = selected.profile.clone();
    for (key, value) in raw {
        let (rule_type, rule_name) =
            split_policy_key(&key).map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
        if value.is_null() {
            remove_profile_rule(&mut profile, &rule_type, &rule_name);
            continue;
        }
        let update: PolicyRuleUpdate = serde_json::from_value(value).map_err(|e| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("invalid policy rule '{key}': {e}"),
            )
        })?;
        validate_policy_rule_update(&rule_type, &rule_name, &update)
            .map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
        upsert_profile_rule(
            &mut profile,
            &rule_type,
            rule_name,
            profile_rule_from_update(update),
        );
    }
    profile.validate().map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("profile validation failed: {e}"),
        )
    })?;

    match selected.source {
        capsem_core::settings_profiles::ProfileSource::User => {
            capsem_core::settings_profiles::update_user_profile(&settings.profiles, profile)
                .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("update profile: {e}")))?;
        }
        capsem_core::settings_profiles::ProfileSource::BuiltIn => {
            capsem_core::settings_profiles::create_user_profile(&settings.profiles, profile)
                .map_err(|e| {
                    AppError(
                        StatusCode::BAD_REQUEST,
                        format!("create profile override: {e}"),
                    )
                })?;
        }
        capsem_core::settings_profiles::ProfileSource::Base
        | capsem_core::settings_profiles::ProfileSource::Corp => {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                format!(
                    "default profile '{}' is locked ({:?}); switch to a user-editable profile first",
                    selected.profile.id, selected.source
                ),
            ));
        }
    }

    Ok(Json(settings_response_json()))
}

/// GET /settings/presets -- list security presets.
async fn handle_get_presets() -> Json<serde_json::Value> {
    match load_service_profiles_state() {
        Ok((_, catalog, _, _)) => Json(profile_presets_json(&catalog)),
        Err(error) => Json(json!([{
            "id": "settings-profiles-error",
            "name": "Settings Profiles Error",
            "description": error,
            "settings": {},
        }])),
    }
}

/// POST /settings/presets/{id} -- select a default profile and return refreshed typed state.
async fn handle_select_profile_preset(
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings_path = service_settings_path();
    let mut settings = capsem_core::settings_profiles::load_service_settings_or_default(
        &settings_path,
    )
    .map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("load {}: {e}", settings_path.display()),
        )
    })?;
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    if catalog.get(&id).is_none() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            format!("unknown profile preset '{id}'"),
        ));
    }
    settings.profiles.default_profile = id;
    capsem_core::settings_profiles::write_service_settings(&settings_path, &settings).map_err(
        |e| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("write {}: {e}", settings_path.display()),
            )
        },
    )?;
    Ok(Json(settings_response_json()))
}

/// POST /settings/lint -- validate config and return issues.
async fn handle_lint_config() -> Json<serde_json::Value> {
    let mut issues: Vec<SettingsIssue> = Vec::new();
    let settings_path = service_settings_path();
    match capsem_core::settings_profiles::load_service_settings_or_default(&settings_path) {
        Ok(settings) => {
            if let Err(error) =
                capsem_core::settings_profiles::discover_profiles(&settings.profiles)
            {
                issues.push(SettingsIssue {
                    path: "profiles".to_string(),
                    severity: "error".to_string(),
                    message: error.to_string(),
                });
            }
            if let Err(error) = capsem_core::settings_profiles::resolve_effective_vm_settings(
                &settings.profiles,
                Some(&settings.profiles.default_profile),
            ) {
                issues.push(SettingsIssue {
                    path: "profiles.default_profile".to_string(),
                    severity: "error".to_string(),
                    message: error.to_string(),
                });
            }
        }
        Err(error) => issues.push(SettingsIssue {
            path: settings_path.display().to_string(),
            severity: "error".to_string(),
            message: error.to_string(),
        }),
    }
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
    let summary =
        tokio::task::spawn_blocking(capsem_core::host_config::detect_and_write_to_settings)
            .await
            .unwrap_or_else(|_| {
                capsem_core::host_config::DetectedConfigSummary::from(
                    &capsem_core::host_config::HostConfig::default(),
                )
            });
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
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to spawn capsem setup: {e}"),
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let code = output.status.code().unwrap_or(-1);
        warn!(exit_code = code, stderr = %stderr, "capsem setup retry failed");
        return Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "setup exited {code}: {}",
                stderr.lines().last().unwrap_or("(no output)")
            ),
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
async fn handle_asset_status(State(state): State<Arc<ServiceState>>) -> Json<serde_json::Value> {
    let health = state.asset_supervisor.snapshot();
    match state.resolve_asset_paths() {
        Ok(resolved) => {
            let progress_name = health.progress.as_ref().map(|p| p.logical_name.as_str());
            let status_for = |name: &str, path: &std::path::Path| {
                if path.exists() {
                    "present"
                } else if health.state == AssetHealthState::Updating
                    && (progress_name == Some(name) || health.missing.iter().any(|m| m == name))
                {
                    "downloading"
                } else {
                    "missing"
                }
            };
            let assets = vec![
                json!({ "name": "vmlinuz", "path": resolved.kernel.display().to_string(), "status": status_for("vmlinuz", &resolved.kernel) }),
                json!({ "name": "initrd.img", "path": resolved.initrd.display().to_string(), "status": status_for("initrd.img", &resolved.initrd) }),
                json!({ "name": "rootfs.squashfs", "path": resolved.rootfs.display().to_string(), "status": status_for("rootfs.squashfs", &resolved.rootfs) }),
            ];
            Json(json!({
                "ready": health.ready,
                "state": health.state,
                "downloading": health.state == AssetHealthState::Updating,
                "asset_locations": asset_locations_status_json(&state.asset_locations),
                "asset_version": health.version.unwrap_or(resolved.asset_version),
                "profile_id": health.profile_id,
                "profile_revision": health.profile_revision,
                "profile_payload_hash": health.profile_payload_hash,
                "profile_assets": health.profile_assets,
                "arch": health.arch,
                "missing": health.missing,
                "progress": health.progress,
                "error": health.error,
                "retry_count": health.retry_count,
                "retryable": health.retryable,
                "assets": assets,
            }))
        }
        Err(e) => Json(json!({
            "ready": false,
            "state": "error",
            "downloading": false,
            "asset_locations": asset_locations_status_json(&state.asset_locations),
            "error": e.to_string(),
            "retryable": false,
            "retry_count": health.retry_count,
            "assets": [],
        })),
    }
}

/// POST /setup/assets/reconcile -- force a Profile V2 asset check/download now.
async fn handle_asset_reconcile(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let before = state.asset_supervisor.snapshot();
    info!(
        event = "profile_asset_check_start",
        state = before.state.as_str(),
        ready = before.ready,
        missing = ?before.missing,
        "profile asset reconcile requested"
    );

    state.asset_supervisor.ensure_assets_once().await;
    let health = state.asset_supervisor.snapshot();
    let outcome = if before.ready && health.ready {
        "already_ready"
    } else if health.ready {
        "downloaded"
    } else if health.state == AssetHealthState::Error {
        "error"
    } else {
        "checking"
    };

    info!(
        event = "profile_asset_check_finish",
        outcome,
        state = health.state.as_str(),
        ready = health.ready,
        retryable = health.retryable,
        error = health.error.as_deref().unwrap_or(""),
        missing = ?health.missing,
        "profile asset reconcile finished"
    );

    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "outcome": outcome,
        "health": health,
    })))
}

/// POST /setup/assets/cleanup -- remove unreferenced profile-era VM assets.
async fn handle_asset_cleanup(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.asset_supervisor.refresh_local_state();
    let health = state.asset_supervisor.snapshot();
    if health.state != AssetHealthState::Ready {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!(
                "asset cleanup is blocked while assets are {}; retry once assets are ready",
                health.state.as_str()
            ),
        ));
    }

    let retention = {
        let registry = state.persistent_registry.lock().unwrap();
        saved_vm_assets::cleanup_retention_asset_filenames(
            &registry,
            &state.service_settings.profiles,
        )
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("derive asset cleanup retention set: {error:#}"),
            )
        })?
    };
    let removed = capsem_core::asset_manager::cleanup_unreferenced_assets_preserving(
        &state.assets_dir,
        retention.iter(),
    )
    .map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cleanup unreferenced assets: {error:#}"),
        )
    })?;
    let removed_paths = removed
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();

    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "skipped": false,
        "asset_state": health.state,
        "retained_count": retention.len(),
        "removed_count": removed_paths.len(),
        "removed": removed_paths,
    })))
}

fn asset_locations_status_json(
    locations: &capsem_core::settings_profiles::ResolvedServiceAssetLocations,
) -> serde_json::Value {
    json!({
        "assets_dir": locations.assets_dir.display().to_string(),
        "assets_dir_origin": locations.assets_dir_origin.as_str(),
        "image_roots": locations
            .image_roots
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>(),
        "image_roots_origin": locations.image_roots_origin.as_str(),
        "download_base_url": locations.download_base_url,
    })
}

/// POST /setup/corp-config -- apply corporate config from URL or inline TOML.
async fn handle_corp_config(
    Json(payload): Json<CorpConfigRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let capsem_dir = capsem_core::paths::capsem_home_opt()
        .ok_or_else(|| AppError(StatusCode::INTERNAL_SERVER_ERROR, "HOME not set".into()))?;

    if let Some(source) = &payload.source {
        let response = reqwest::Client::new()
            .get(source)
            .header("User-Agent", "capsem")
            .send()
            .await
            .map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    format!("failed to fetch corp profile: {e}"),
                )
            })?;
        if !response.status().is_success() {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                format!(
                    "corp profile fetch failed: HTTP {} for {source}",
                    response.status()
                ),
            ));
        }
        let body = response.text().await.map_err(|e| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("failed to read corp profile body: {e}"),
            )
        })?;
        capsem_core::settings_profiles::install_corp_profile_toml(&capsem_dir, &body)
            .map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;
    } else if let Some(toml_content) = &payload.toml {
        capsem_core::settings_profiles::install_corp_profile_toml(&capsem_dir, toml_content)
            .map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;
    } else {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "provide either 'source' (URL) or 'toml' (inline content)".into(),
        ));
    }

    Ok(Json(json!({ "success": true })))
}

// ---------------------------------------------------------------------------
// Profile V2 MCP server API handlers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct McpConnectorsQuery {
    #[serde(default)]
    profile: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpConnectorMutationRequest {
    #[serde(default, alias = "profile_id")]
    profile: Option<String>,
    id: String,
    #[serde(flatten)]
    connector: capsem_core::settings_profiles::McpConnectorConfig,
}

fn validate_mcp_connector_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("MCP server id cannot be empty".to_string());
    }
    if id
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_' | '.'))
    {
        Ok(())
    } else {
        Err(
            "MCP server id may only contain lowercase letters, digits, '-', '_', and '.'"
                .to_string(),
        )
    }
}

fn profile_has_mcp_connector(
    profile: &capsem_core::settings_profiles::Profile,
    connector_id: &str,
) -> bool {
    profile.mcp.connectors.contains_key(connector_id)
}

fn mcp_connector_owner<'a>(
    catalog: &'a capsem_core::settings_profiles::ProfileCatalog,
    profile_id: &str,
    connector_id: &str,
) -> Result<Option<&'a capsem_core::settings_profiles::ProfileRecord>, AppError> {
    let chain = capsem_core::settings_profiles::resolve_ancestor_chain(catalog, profile_id)
        .map_err(|e| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("resolve profile chain: {e}"),
            )
        })?;
    Ok(chain
        .into_iter()
        .rfind(|record| profile_has_mcp_connector(&record.profile, connector_id)))
}

fn mcp_connector_json(
    id: &str,
    connector: &capsem_core::settings_profiles::McpConnectorConfig,
    owner: Option<&capsem_core::settings_profiles::ProfileRecord>,
    selected_profile_id: &str,
) -> serde_json::Value {
    let source_profile = owner.map(|record| record.profile.id.as_str());
    let source = owner.map(|record| record.source.as_str());
    let direct = source_profile == Some(selected_profile_id);
    let editable = direct
        && owner
            .map(|record| record.source == capsem_core::settings_profiles::ProfileSource::User)
            .unwrap_or(false);
    json!({
        "id": id,
        "source_profile": source_profile,
        "source": source,
        "direct": direct,
        "editable": editable,
        "server": connector,
    })
}

/// GET /mcp/connectors -- list effective Profile V2 MCP servers.
async fn handle_mcp_connectors(
    Query(query): Query<McpConnectorsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let settings = load_service_settings_for_profiles()?;
    let target_profile_id = query
        .profile
        .clone()
        .unwrap_or_else(|| settings.profiles.default_profile.clone());
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    let (effective, _) = capsem_core::settings_profiles::resolve_effective_vm_settings_with_corp(
        &settings,
        Some(&target_profile_id),
    )
    .map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("resolve effective profile '{target_profile_id}': {e}"),
        )
    })?;

    let mut servers = effective
        .mcp
        .value
        .connectors
        .iter()
        .map(|(id, connector)| {
            let owner = mcp_connector_owner(&catalog, &effective.profile_id, id)?;
            Ok(mcp_connector_json(
                id,
                connector,
                owner,
                &effective.profile_id,
            ))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    servers.sort_by(|left, right| {
        left["id"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["id"].as_str().unwrap_or_default())
    });

    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "profile_id": effective.profile_id,
        "servers": servers,
    })))
}

/// POST /mcp/connectors -- create a direct Profile V2 MCP server.
async fn handle_create_mcp_connector(
    Json(request): Json<McpConnectorMutationRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_mcp_connector_id(&request.id).map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
    let settings = load_service_settings_for_profiles()?;
    let target_profile_id = request
        .profile
        .clone()
        .unwrap_or_else(|| settings.profiles.default_profile.clone());
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    let selected = catalog.get(&target_profile_id).ok_or_else(|| {
        AppError(
            StatusCode::NOT_FOUND,
            format!("profile '{target_profile_id}' not found"),
        )
    })?;
    ensure_profile_section_editable(&selected.profile, ProfileEditableSection::McpServers)?;
    if profile_has_mcp_connector(&selected.profile, &request.id) {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!("server_exists: mcpServers.{}", request.id),
        ));
    }

    let mut profile = selected.profile.clone();
    profile
        .mcp
        .connectors
        .insert(request.id.clone(), request.connector);
    profile.validate().map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("profile validation failed: {e}"),
        )
    })?;
    save_mutated_profile(&settings, selected.source, profile)?;

    let Json(listed) = handle_mcp_connectors(Query(McpConnectorsQuery {
        profile: Some(target_profile_id),
    }))
    .await?;
    let connector = listed["servers"]
        .as_array()
        .and_then(|servers| {
            servers
                .iter()
                .find(|connector| connector["id"] == serde_json::json!(request.id))
                .cloned()
        })
        .ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "created MCP server '{}' was not visible after profile save",
                    request.id
                ),
            )
        })?;
    Ok(Json(connector))
}

/// DELETE /mcp/connectors/{id} -- remove a direct user Profile V2 MCP server.
async fn handle_delete_mcp_connector(
    Path(connector_id): Path<String>,
    Query(query): Query<McpConnectorsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_mcp_connector_id(&connector_id).map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
    let settings = load_service_settings_for_profiles()?;
    let target_profile_id = query
        .profile
        .clone()
        .unwrap_or_else(|| settings.profiles.default_profile.clone());
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("discover profiles: {e}")))?;
    let selected = catalog.get(&target_profile_id).ok_or_else(|| {
        AppError(
            StatusCode::NOT_FOUND,
            format!("profile '{target_profile_id}' not found"),
        )
    })?;
    ensure_profile_section_editable(&selected.profile, ProfileEditableSection::McpServers)?;
    if selected.source != capsem_core::settings_profiles::ProfileSource::User {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!(
                "server_is_locked: profile '{}' is locked ({:?})",
                selected.profile.id, selected.source
            ),
        ));
    }
    if !profile_has_mcp_connector(&selected.profile, &connector_id) {
        let owner = mcp_connector_owner(&catalog, &target_profile_id, &connector_id)?;
        return match owner {
            Some(owner) => Err(AppError(
                StatusCode::CONFLICT,
                format!(
                    "server_is_locked: MCP server '{}' is inherited from profile '{}'",
                    connector_id, owner.profile.id
                ),
            )),
            None => Err(AppError(
                StatusCode::NOT_FOUND,
                format!("MCP server '{connector_id}' not found"),
            )),
        };
    }

    let mut profile = selected.profile.clone();
    profile.mcp.connectors.remove(&connector_id);
    profile.validate().map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("profile validation failed: {e}"),
        )
    })?;
    capsem_core::settings_profiles::update_user_profile(&settings.profiles, profile)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("update profile: {e}")))?;

    Ok(Json(json!({
        "mode": "settings_profiles_v2",
        "profile_id": target_profile_id,
        "server_id": connector_id,
        "removed": true,
    })))
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
/// `exec_events`, `mcp_calls`, `net_events`, `dns_events`, `security_events`,
/// `audit_events`, `snapshot_events`, `fs_events`, and `model_calls` via
/// UNION ALL. Used by the `capsem_timeline` MCP tool.
///
/// W6 added `trace_id` to every layer; this handler filters with
/// `WHERE trace_id = ? OR trace_id IS NULL` so rows that pre-date W4's
/// trace propagation still surface for the user.
const ALLOWED_TIMELINE_LAYERS: &[&str] = &[
    "exec", "mcp", "net", "dns", "security", "audit", "snapshot", "fs", "model",
];

fn timeline_existing_tables(reader: &capsem_logger::DbReader) -> Result<HashSet<String>, AppError> {
    let raw = reader
        .query_raw("SELECT name FROM sqlite_master WHERE type='table'")
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to inspect DB schema: {e}"),
            )
        })?;
    let val: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to parse DB schema: {e}"),
        )
    })?;
    let mut out = HashSet::new();
    if let Some(rows) = val.get("rows").and_then(|r| r.as_array()) {
        for row in rows {
            if let Some(name) = row
                .as_array()
                .and_then(|cells| cells.first())
                .and_then(|cell| cell.as_str())
            {
                out.insert(name.to_string());
            }
        }
    }
    Ok(out)
}

fn timeline_table_columns(
    reader: &capsem_logger::DbReader,
    table: &str,
) -> Result<HashSet<String>, AppError> {
    let raw = reader
        .query_raw(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to inspect DB columns for {table}: {e}"),
            )
        })?;
    let val: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to parse DB columns for {table}: {e}"),
        )
    })?;
    let mut out = HashSet::new();
    if let Some(rows) = val.get("rows").and_then(|r| r.as_array()) {
        for row in rows {
            if let Some(name) = row
                .as_array()
                .and_then(|cells| cells.first())
                .and_then(|cell| cell.as_str())
            {
                out.insert(name.to_string());
            }
        }
    }
    Ok(out)
}

fn timeline_existing_columns(
    reader: &capsem_logger::DbReader,
    tables: &HashSet<String>,
) -> Result<HashMap<String, HashSet<String>>, AppError> {
    let mut out = HashMap::new();
    for table in [
        "exec_events",
        "mcp_calls",
        "net_events",
        "dns_events",
        "security_events",
        "audit_events",
        "snapshot_events",
        "fs_events",
        "model_calls",
        "tool_calls",
    ] {
        if tables.contains(table) {
            out.insert(table.to_string(), timeline_table_columns(reader, table)?);
        }
    }
    Ok(out)
}

fn timeline_has_column(
    columns: &HashMap<String, HashSet<String>>,
    table: &str,
    column: &str,
) -> bool {
    columns.get(table).is_some_and(|cols| cols.contains(column))
}

fn timeline_col(
    columns: &HashMap<String, HashSet<String>>,
    table: &str,
    column: &str,
    fallback: &str,
) -> String {
    if timeline_has_column(columns, table, column) {
        column.to_string()
    } else {
        fallback.to_string()
    }
}

fn timeline_alias_col(
    columns: &HashMap<String, HashSet<String>>,
    table: &str,
    alias: &str,
    column: &str,
    fallback: &str,
) -> String {
    if timeline_has_column(columns, table, column) {
        format!("{alias}.{column}")
    } else {
        fallback.to_string()
    }
}

fn timeline_policy_suffix(
    columns: &HashMap<String, HashSet<String>>,
    table: &str,
    qualifier: Option<&str>,
) -> &'static str {
    if timeline_has_column(columns, table, "policy_action")
        && timeline_has_column(columns, table, "policy_rule")
    {
        match qualifier {
            Some("m") => "COALESCE(' policy=' || m.policy_action || '/' || m.policy_rule, '')",
            _ => "COALESCE(' policy=' || policy_action || '/' || policy_rule, '')",
        }
    } else {
        "''"
    }
}

async fn handle_timeline(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<TimelineQuery>,
) -> Result<impl IntoResponse, AppError> {
    let db_path = resolve_session_dir(&state, &id)?.join("session.db");
    let reader = capsem_logger::DbReader::open(&db_path).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to open DB: {e}"),
        )
    })?;
    let existing_tables = timeline_existing_tables(&reader)?;
    let existing_columns = timeline_existing_columns(&reader, &existing_tables)?;

    let limit = params.limit.unwrap_or(200).min(2000);
    let since_filter = params
        .since
        .as_deref()
        .and_then(triage::parse_since)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    // Layers the caller wants. Default to all current layers. C1: filter against
    // a hard allowlist BEFORE building SQL so even a future careless
    // copy-paste of this format!() can't leak attacker-supplied
    // tokens into the query string.
    let layers: Vec<&str> = params
        .layers
        .as_deref()
        .map(|s| {
            s.split(',')
                .filter(|x| !x.is_empty())
                .filter(|x| ALLOWED_TIMELINE_LAYERS.contains(x))
                .collect()
        })
        .unwrap_or_else(|| ALLOWED_TIMELINE_LAYERS.to_vec());

    let mut parts: Vec<String> = Vec::new();
    if layers.contains(&"exec") && existing_tables.contains("exec_events") {
        let status = timeline_col(&existing_columns, "exec_events", "exit_code", "NULL");
        let duration = timeline_col(&existing_columns, "exec_events", "duration_ms", "NULL");
        let trace_id = timeline_col(&existing_columns, "exec_events", "trace_id", "NULL");
        parts.push(format!(
            "SELECT timestamp, 'exec' AS layer, exec_id AS ref, command AS summary, \
             {status} AS status, {duration} AS duration_ms, {trace_id} AS trace_id FROM exec_events"
        ));
    }
    if layers.contains(&"mcp") && existing_tables.contains("mcp_calls") {
        // F7: include the originating model_call's tool_calls.call_id when
        // an mcp_call serviced a model tool_use, so the timeline shows
        // "model X tool_use Y -> mcp_call Z" inline. Best-effort LEFT JOIN
        // -- mcp_calls without a tool_calls peer just show NULL.
        let tool_summary = if timeline_has_column(&existing_columns, "mcp_calls", "tool_name") {
            "COALESCE(m.tool_name, m.method)"
        } else {
            "m.method"
        };
        let join_tool_calls = existing_tables.contains("tool_calls")
            && timeline_has_column(&existing_columns, "tool_calls", "mcp_call_id")
            && timeline_has_column(&existing_columns, "tool_calls", "call_id");
        let join_sql = if join_tool_calls {
            " LEFT JOIN tool_calls tc ON tc.mcp_call_id = m.id"
        } else {
            ""
        };
        let call_id_suffix = if join_tool_calls {
            "COALESCE(' (call_id=' || tc.call_id || ')', '')"
        } else {
            "''"
        };
        let duration =
            timeline_alias_col(&existing_columns, "mcp_calls", "m", "duration_ms", "NULL");
        let trace_id = timeline_alias_col(&existing_columns, "mcp_calls", "m", "trace_id", "NULL");
        let policy_suffix = timeline_policy_suffix(&existing_columns, "mcp_calls", Some("m"));
        parts.push(format!(
            "SELECT m.timestamp AS timestamp, 'mcp' AS layer, m.id AS ref, \
             m.server_name || '/' || {tool_summary} || {call_id_suffix} || {policy_suffix} AS summary, \
             NULL AS status, {duration} AS duration_ms, {trace_id} AS trace_id \
             FROM mcp_calls m{join_sql}"
        ));
    }
    if layers.contains(&"net") && existing_tables.contains("net_events") {
        let method = timeline_col(&existing_columns, "net_events", "method", "'GET'");
        let path = timeline_col(&existing_columns, "net_events", "path", "''");
        let status = timeline_col(&existing_columns, "net_events", "status_code", "NULL");
        let duration = timeline_col(&existing_columns, "net_events", "duration_ms", "NULL");
        let trace_id = timeline_col(&existing_columns, "net_events", "trace_id", "NULL");
        let policy_suffix = timeline_policy_suffix(&existing_columns, "net_events", None);
        parts.push(format!(
            "SELECT timestamp, 'net' AS layer, id AS ref, \
             COALESCE({method}, 'GET') || ' ' || domain || COALESCE({path}, '') || \
                {policy_suffix} AS summary, \
             {status} AS status, {duration} AS duration_ms, {trace_id} AS trace_id FROM net_events"
        ));
    }
    if layers.contains(&"dns") && existing_tables.contains("dns_events") {
        let duration = timeline_col(
            &existing_columns,
            "dns_events",
            "upstream_resolver_ms",
            "NULL",
        );
        let trace_id = timeline_col(&existing_columns, "dns_events", "trace_id", "NULL");
        let policy_suffix = timeline_policy_suffix(&existing_columns, "dns_events", None);
        parts.push(format!(
            "SELECT timestamp, 'dns' AS layer, id AS ref, \
             qname || ' rcode=' || rcode || {policy_suffix} AS summary, \
             decision AS status, {duration} AS duration_ms, {trace_id} AS trace_id FROM dns_events"
        ));
    }
    if layers.contains(&"security") && existing_tables.contains("security_events") {
        let trace_id = timeline_col(&existing_columns, "security_events", "trace_id", "NULL");
        let event_ref = timeline_col(&existing_columns, "security_events", "event_id", "id");
        let event_type = timeline_col(
            &existing_columns,
            "security_events",
            "event_type",
            "'security.event'",
        );
        let event_family = timeline_col(
            &existing_columns,
            "security_events",
            "event_family",
            "'security'",
        );
        let final_action = timeline_col(
            &existing_columns,
            "security_events",
            "final_action",
            "'continue'",
        );
        parts.push(format!(
            "SELECT timestamp, 'security' AS layer, {event_ref} AS ref, \
             {event_family} || '/' || {event_type} || ' action=' || {final_action} AS summary, \
             {final_action} AS status, NULL AS duration_ms, {trace_id} AS trace_id FROM security_events"
        ));
    }
    if layers.contains(&"audit") && existing_tables.contains("audit_events") {
        let status = timeline_col(&existing_columns, "audit_events", "exit_code", "NULL");
        let trace_id = timeline_col(&existing_columns, "audit_events", "trace_id", "NULL");
        parts.push(format!(
            "SELECT timestamp, 'audit' AS layer, id AS ref, \
             COALESCE(comm, exe) || ' ' || argv AS summary, \
             {status} AS status, NULL AS duration_ms, {trace_id} AS trace_id FROM audit_events"
        ));
    }
    if layers.contains(&"snapshot") && existing_tables.contains("snapshot_events") {
        let trace_id = timeline_col(&existing_columns, "snapshot_events", "trace_id", "NULL");
        parts.push(format!(
            "SELECT timestamp, 'snapshot' AS layer, id AS ref, \
             origin || ' cp-' || slot || COALESCE(' ' || name, '') AS summary, \
             NULL AS status, NULL AS duration_ms, {trace_id} AS trace_id FROM snapshot_events"
        ));
    }
    if layers.contains(&"fs") && existing_tables.contains("fs_events") {
        let trace_id = timeline_col(&existing_columns, "fs_events", "trace_id", "NULL");
        parts.push(format!(
            "SELECT timestamp, 'fs' AS layer, id AS ref, action || ' ' || path AS summary, \
             NULL AS status, NULL AS duration_ms, {trace_id} AS trace_id FROM fs_events"
        ));
    }
    if layers.contains(&"model") && existing_tables.contains("model_calls") {
        let model = timeline_col(&existing_columns, "model_calls", "model", "'?'");
        let status = timeline_col(&existing_columns, "model_calls", "status_code", "NULL");
        let duration = timeline_col(&existing_columns, "model_calls", "duration_ms", "NULL");
        let trace_id = timeline_col(&existing_columns, "model_calls", "trace_id", "NULL");
        parts.push(format!(
            "SELECT timestamp, 'model' AS layer, id AS ref, \
             provider || '/' || COALESCE({model}, '?') AS summary, \
             {status} AS status, {duration} AS duration_ms, {trace_id} AS trace_id FROM model_calls"
        ));
    }

    if parts.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "no selected layers found in session DB".into(),
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
                            return Ok(Json(provision_response_for_instance(
                                &state,
                                cold_id,
                                cold_uds_path,
                                None,
                            )));
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
            Ok(Json(provision_response_for_instance(
                &state, id, uds_path, None,
            )))
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
    let (old_session_dir, ram_mb, cpus, base_version, forked_from, env, base_assets, profile_pin) = {
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
            i.base_assets.clone(),
            i.profile_pin.clone(),
        )
    };
    ensure_required_vm_profile_pin(profile_pin.as_ref(), &format!("running VM \"{id}\""))
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;
    let base_assets = source_pin_base_assets(&id, profile_pin.as_ref(), base_assets.as_ref())
        .map(Some)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;

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
                base_assets: base_assets.clone(),
                profile_pin: profile_pin.clone(),
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
                    base_assets,
                    profile_pin,
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
    let id = {
        let existing: Vec<String> = state.instances.lock().unwrap().keys().cloned().collect();
        generate_tmp_name(existing.iter().map(|s| s.as_str()))
    };

    // Resolve ram/cpu from the selected profile VM settings if omitted.
    let vm_defaults = state.resolve_vm_runtime_defaults_for(payload.profile_id.as_deref());
    let ram_mb = payload.ram_mb.unwrap_or(vm_defaults.ram_mb);
    let cpus = payload.cpus.unwrap_or(vm_defaults.cpus);

    let ram_bytes = ram_mb * 1024 * 1024;
    let session_dir = state.run_dir.join("sessions").join(&id);

    // 1. Provision ephemeral VM. `provision_sandbox` is synchronous and
    // does heavy I/O (APFS clonefile, rootfs.img fsync, child spawn);
    // offload to the blocking pool, matching `handle_provision` -- the
    // tokio::process::Command::spawn inside still works because
    // spawn_blocking preserves the runtime handle via thread-locals.
    state
        .ensure_selected_profile_assets_ready(
            payload.profile_id.as_deref(),
            payload.profile_revision.as_deref(),
        )
        .await
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("provision failed: {e}"),
            )
        })?;
    let state_clone = Arc::clone(&state);
    let id_clone = id.clone();
    let version = state.current_version.clone();
    let env = payload.env.clone();
    let profile_id = payload.profile_id.clone();
    let profile_revision = payload.profile_revision.clone();
    let provision_result = tokio::task::spawn_blocking(move || {
        state_clone.provision_sandbox(ProvisionOptions {
            id: &id_clone,
            ram_mb,
            cpus,
            version_override: Some(version),
            persistent: false,
            env,
            from: None,
            profile_id,
            profile_revision,
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

    info!("capsem-service starting up");
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
    let service_settings_path = service_settings_path();
    let service_settings =
        capsem_core::settings_profiles::load_service_settings_or_default(&service_settings_path)
            .with_context(|| format!("load {}", service_settings_path.display()))?;
    let asset_locations = capsem_core::settings_profiles::resolve_service_asset_locations(
        &service_settings,
        args.assets_dir.clone(),
        Some(capsem_core::paths::capsem_assets_dir()),
        run_dir.parent().unwrap().join("assets"),
    )
    .context("resolve service asset locations")?;
    let assets_base_dir = asset_locations.assets_dir.clone();

    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let asset_requirement = startup_asset_requirement(
        &service_settings,
        host_asset_arch(),
        cfg!(debug_assertions) || args.assets_dir.is_some(),
    )
    .context("resolve startup VM asset requirement")?;

    let registry_path = run_dir.join("persistent_registry.json");
    let persistent_registry = PersistentRegistry::load(registry_path);
    info!(
        persistent_vms = persistent_registry.data.vms.len(),
        "loaded persistent VM registry"
    );

    let asset_supervisor = Arc::new(AssetSupervisor::new(
        assets_base_dir.clone(),
        asset_requirement,
        std::time::Duration::from_secs(300),
    ));
    asset_supervisor.refresh_local_state();

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
        asset_locations,
        service_settings,
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        asset_supervisor,
        enforcement_registry: Arc::new(Mutex::new(seceng::RuntimeRuleRegistry::default())),
        detection_registry: Arc::new(Mutex::new(seceng::RuntimeRuleRegistry::default())),
        current_version,
        magika: Mutex::new(magika_session),
        save_restore_lock: tokio::sync::Mutex::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
    });

    Arc::clone(&state.asset_supervisor).spawn();
    let _profile_catalog_reconcile_task =
        spawn_profile_catalog_reconcile_task(state.service_settings.clone());

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
                let state = Arc::clone(&state_for_cleanup);
                if let Err(e) =
                    tokio::task::spawn_blocking(move || state.cleanup_stale_instances()).await
                {
                    warn!(error = %e, "stale instance cleanup task failed");
                }
            }
        });
    }

    // Spawn companion processes (gateway + tray) in the background so the UDS
    // starts accepting immediately. The previous .await here delayed accept()
    // by up to 5s on every startup while polling gateway.token into existence
    // -- fatal under parallel test load. Companions are stateless and can come
    // up after the service is already serving clients.
    let companions = Arc::new(std::sync::Mutex::new(CompanionManager {
        children: Vec::new(),
        spawn_task: None,
        #[cfg(target_os = "macos")]
        run_dir: run_dir.clone(),
        #[cfg(target_os = "macos")]
        tray_bin: args.tray_binary.clone(),
    }));
    let companions_for_route = Arc::clone(&companions);

    let app = Router::new()
        .route(
            "/version",
            get(|| async { Json(serde_json::json!({ "version": env!("CARGO_PKG_VERSION") })) }),
        )
        .route(
            "/companions/tray/ensure",
            post(move || handle_ensure_tray(Arc::clone(&companions_for_route))),
        )
        .route("/provision", post(handle_provision))
        .route("/list", get(handle_list))
        .route("/info/{id}", get(handle_info))
        .route("/logs/{id}", get(handle_logs))
        .route("/inspect/{id}", post(handle_inspect))
        .route("/exec/{id}", post(handle_exec))
        .route("/stop/{id}", post(handle_stop))
        .route("/suspend/{id}", post(handle_suspend))
        .route("/delete/{id}", delete(handle_delete))
        .route("/resume/{name}", post(handle_resume))
        .route("/persist/{id}", post(handle_persist))
        .route("/purge", post(handle_purge))
        .route("/run", post(handle_run))
        .route("/stats", get(handle_stats))
        .route("/service-logs", get(handle_service_logs))
        .route("/debug/report", get(handle_debug_report))
        .route("/triage", get(handle_triage))
        .route("/panics", get(handle_panics))
        .route("/host-logs/{name}", get(handle_host_logs))
        .route("/timeline/{id}", get(handle_timeline))
        .route("/reload-config", post(handle_reload_config))
        .route("/fork/{id}", post(handle_fork))
        .route(
            "/settings",
            get(handle_get_settings).post(handle_save_settings),
        )
        .route("/settings/presets", get(handle_get_presets))
        .route("/settings/presets/{id}", post(handle_select_profile_preset))
        .route("/settings/lint", post(handle_lint_config))
        .route("/settings/validate-key", post(handle_validate_key))
        .route(
            "/profiles",
            get(handle_list_profiles).post(handle_create_profile),
        )
        .route(
            "/profiles/catalog/reconcile",
            post(handle_reconcile_profile_catalog),
        )
        .route("/profiles/catalog", get(handle_profile_catalog))
        .route(
            "/profiles/{id}/revisions/install",
            post(handle_install_profile_revision),
        )
        .route(
            "/profiles/{id}/revisions/update",
            post(handle_update_profile_revision_lifecycle),
        )
        .route(
            "/profiles/{id}/revisions/remove",
            post(handle_remove_profile_revision),
        )
        .route("/profiles/{id}/revisions", get(handle_profile_revisions))
        .route(
            "/profiles/{id}",
            get(handle_get_profile)
                .put(handle_update_profile)
                .delete(handle_delete_profile),
        )
        .route("/profiles/{id}/fork", post(handle_fork_profile))
        .route("/profiles/{id}/effective", get(handle_resolve_profile))
        .route("/rules", get(handle_list_rules).post(handle_create_rule))
        .route(
            "/rules/{rule_id}",
            get(handle_get_rule).delete(handle_delete_rule),
        )
        .route(
            "/enforcement",
            get(handle_list_enforcement_rules).post(handle_create_enforcement_rule),
        )
        .route(
            "/enforcement/validate",
            post(handle_validate_enforcement_rule),
        )
        .route(
            "/enforcement/compile",
            post(handle_compile_enforcement_rule),
        )
        .route("/enforcement/backtest", post(handle_enforcement_backtest))
        .route("/enforcement/stats", get(handle_enforcement_stats))
        .route(
            "/enforcement/{id}",
            put(handle_update_enforcement_rule).delete(handle_delete_enforcement_rule),
        )
        .route(
            "/detection",
            get(handle_list_detection_rules).post(handle_create_detection_rule),
        )
        .route("/detection/validate", post(handle_validate_detection_rule))
        .route("/detection/compile", post(handle_compile_detection_rule))
        .route("/detection/backtest", post(handle_detection_backtest))
        .route("/detection/hunt", post(handle_detection_hunt))
        .route(
            "/sessions/{id}/detection/hunt",
            post(handle_session_detection_hunt),
        )
        .route("/detection/stats", get(handle_detection_stats))
        .route(
            "/detection/{id}",
            put(handle_update_detection_rule).delete(handle_delete_detection_rule),
        )
        .route("/confirm/pending", get(handle_list_pending_confirms))
        .route("/skills", get(handle_list_skills).post(handle_create_skill))
        .route("/skills/{id}", delete(handle_delete_skill))
        .route("/setup/state", get(handle_get_setup_state))
        .route("/setup/detect", get(handle_detect_host_config))
        .route("/setup/complete", post(handle_complete_onboarding))
        .route("/setup/retry", post(handle_setup_retry))
        .route("/setup/assets", get(handle_asset_status))
        .route("/setup/assets/reconcile", post(handle_asset_reconcile))
        .route("/setup/assets/cleanup", post(handle_asset_cleanup))
        .route("/setup/corp-config", post(handle_corp_config))
        .route(
            "/mcp/connectors",
            get(handle_mcp_connectors).post(handle_create_mcp_connector),
        )
        .route("/mcp/connectors/{id}", delete(handle_delete_mcp_connector))
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

    let uds = UnixListener::bind(&service_sock).context("failed to bind UDS")?;
    // Socket is bound; release the startup lock so any peer starter still in
    // its flock wait can fast-probe us and exit 0.
    drop(startup_lock_guard);

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
            for mut companion in children {
                info!(
                    pid = companion.child.id(),
                    kind = ?companion.kind,
                    "killing companion process"
                );
                let _ = companion.child.kill().await;
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

fn companion_log_dir(run_dir: &std::path::Path) -> PathBuf {
    if std::env::var("CAPSEM_RUN_DIR").is_ok() {
        run_dir.join("logs")
    } else {
        std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join("Library/Logs/capsem"))
            .unwrap_or_else(|_| run_dir.join("logs"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompanionKind {
    Gateway,
    #[cfg(target_os = "macos")]
    Tray,
}

struct CompanionProcess {
    kind: CompanionKind,
    child: tokio::process::Child,
}

struct CompanionManager {
    children: Vec<CompanionProcess>,
    spawn_task: Option<tokio::task::JoinHandle<()>>,
    #[cfg(target_os = "macos")]
    run_dir: PathBuf,
    #[cfg(target_os = "macos")]
    tray_bin: Option<PathBuf>,
}

#[derive(Serialize)]
struct EnsureTrayResponse {
    tray: &'static str,
    pid: Option<u32>,
    reason: Option<String>,
}

#[cfg(target_os = "macos")]
fn spawn_tray_companion(
    run_dir: &std::path::Path,
    tray_bin: Option<PathBuf>,
) -> std::io::Result<CompanionProcess> {
    let tray_bin = tray_bin.unwrap_or_else(|| find_sibling_binary("capsem-tray"));
    let log_dir = companion_log_dir(run_dir);
    let _ = std::fs::create_dir_all(&log_dir);
    let (tray_out, tray_err) = companion_stdio(&log_dir.join("tray.log"));
    info!(binary = %tray_bin.display(), "spawning capsem-tray");
    tokio::process::Command::new(&tray_bin)
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .stdout(tray_out)
        .stderr(tray_err)
        .kill_on_drop(true)
        .spawn()
        .map(|child| CompanionProcess {
            kind: CompanionKind::Tray,
            child,
        })
}

fn ensure_tray_running(manager: &mut CompanionManager) -> (StatusCode, EnsureTrayResponse) {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = manager;
        return (
            StatusCode::OK,
            EnsureTrayResponse {
                tray: "unsupported",
                pid: None,
                reason: Some("capsem-tray is only supported on macOS".into()),
            },
        );
    }

    #[cfg(target_os = "macos")]
    {
        manager.children.retain_mut(|companion| {
            if companion.kind != CompanionKind::Tray {
                return true;
            }
            match companion.child.try_wait() {
                Ok(Some(status)) => {
                    info!(
                        pid = companion.child.id(),
                        ?status,
                        "dropping exited capsem-tray child"
                    );
                    false
                }
                Ok(None) => true,
                Err(e) => {
                    warn!(
                        pid = companion.child.id(),
                        error = %e,
                        "dropping unreadable capsem-tray child handle"
                    );
                    false
                }
            }
        });

        if let Some(companion) = manager
            .children
            .iter()
            .find(|companion| companion.kind == CompanionKind::Tray)
        {
            return (
                StatusCode::OK,
                EnsureTrayResponse {
                    tray: "running",
                    pid: companion.child.id(),
                    reason: None,
                },
            );
        }

        if !manager.run_dir.join("gateway.token").exists() {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                EnsureTrayResponse {
                    tray: "unavailable",
                    pid: None,
                    reason: Some("gateway token is not ready yet".into()),
                },
            );
        }

        match spawn_tray_companion(&manager.run_dir, manager.tray_bin.clone()) {
            Ok(companion) => {
                let pid = companion.child.id();
                info!(pid, "capsem-tray spawned by ensure request");
                manager.children.push(companion);
                (
                    StatusCode::OK,
                    EnsureTrayResponse {
                        tray: "spawned",
                        pid,
                        reason: None,
                    },
                )
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                EnsureTrayResponse {
                    tray: "error",
                    pid: None,
                    reason: Some(e.to_string()),
                },
            ),
        }
    }
}

async fn handle_ensure_tray(
    companions: Arc<std::sync::Mutex<CompanionManager>>,
) -> impl IntoResponse {
    let (status, response) = {
        let mut manager = companions.lock().unwrap();
        ensure_tray_running(&mut manager)
    };
    (status, Json(response))
}

/// Spawn the gateway and tray as child processes of the service.
async fn spawn_companions(
    service_sock: &std::path::Path,
    run_dir: &std::path::Path,
    gateway_bin: Option<PathBuf>,
    gateway_port: Option<u16>,
    tray_bin: Option<PathBuf>,
) -> Vec<CompanionProcess> {
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
    let log_dir = companion_log_dir(run_dir);
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
    match gw_cmd
        .stdout(gw_out)
        .stderr(gw_err)
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => {
            info!(pid = child.id(), "capsem-gateway spawned");
            children.push(CompanionProcess {
                kind: CompanionKind::Gateway,
                child,
            });

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
                .await;
            }

            // 2. Spawn capsem-tray (menu bar) -- only on macOS, only after gateway ready
            #[cfg(target_os = "macos")]
            if token_path.exists() {
                match spawn_tray_companion(run_dir, tray_bin) {
                    Ok(companion) => {
                        info!(pid = companion.child.id(), "capsem-tray spawned");
                        children.push(companion);
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
mod tests;
