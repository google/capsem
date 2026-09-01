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
    mcp::{
        policy::{McpManualServer, McpProfileConfig},
        ToolCacheEntry,
    },
    net::policy_config::{
        skill_id_for_path, ActiveProfileFile, CompiledSecurityRule, DetectionLevel, Profile, ProfileAssetDescriptor,
        ProfileCatalog, ProfileCatalogSource, ProfileConfigFile, ProviderRuleProfile, SecurityPluginConfig,
        SecurityPluginMode, SecurityRule, SecurityRuleAction, SecurityRuleGroup, SecurityRuleProfile, SecurityRuleSet,
        SecurityRuleSource, SettingsFile,
    },
    security_engine::{
        DnsSecurityEvent, FileSecurityEvent, HttpSecurityEvent, IpSecurityEvent, McpSecurityEvent, ModelSecurityEvent,
        ProcessSecurityEvent, RuntimeSecurityEventType, SecurityActionRegistry, SecurityEmitError, SecurityEvent,
        SecurityEventEmitter, SecurityEventEngine, SerializableSecurityEvent, TcpSecurityEvent, UdpSecurityEvent,
    },
};
use capsem_foundation::poll::{poll_until, PollOpts};
use capsem_proto::ipc::{FileBoundaryAction, ProcessToService, ServiceToProcess};
use capsem_service::errors::AppError;
use clap::Parser;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path as StdPath, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use tokio::net::UnixListener;
use tokio::process::Command;
use tokio_unix_ipc::{channel_from_std, Receiver, Sender};
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn, Instrument};

mod asset_background;
mod instance_reaper;
mod proctable;
mod profile_status_cache;
mod session_cleanup;
use session_cleanup::{finalize_one_shot_session, handle_preserve_failure, preserve_failed_run_shutdown_result};
mod ledger_routes;
mod profile_routes;
mod router_runtime;
mod service_runtime;
mod shutdown_policy;
mod startup;
mod suspend_confirmation;
mod update_command;
mod update_status;
mod vm_files;
mod vm_lifecycle;

use ledger_routes::*;
use profile_routes::*;
use router_runtime::*;
use service_runtime::*;
use vm_files::*;
use vm_lifecycle::*;

use profile_status_cache::*;
use shutdown_policy::*;
use suspend_confirmation::{observe_suspend_message, suspend_channel_closed, suspend_failure, SuspendConfirmation};
use update_command::{update_command_plan, UpdateCommandKind};
use update_status::update_status_response_from_paths;

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

#[cfg(test)]
thread_local! {
    static TEST_PROFILE_DIR_OVERRIDE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn test_profile_dir_override() -> Option<PathBuf> {
    TEST_PROFILE_DIR_OVERRIDE.with(|cell| {
        let path = cell.borrow().clone();
        if path.as_ref().is_some_and(|path| !path.exists()) {
            cell.replace(None);
            None
        } else {
            path
        }
    })
}

#[cfg(test)]
fn set_test_profile_dir_override(path: Option<PathBuf>) -> Option<PathBuf> {
    TEST_PROFILE_DIR_OVERRIDE.with(|cell| cell.replace(path))
}

use capsem_service::api;
use capsem_service::api::*;
use capsem_service::naming::{generate_profile_session_name, validate_vm_name};
use capsem_service::registry::{
    new_persistent_vm_id, BootAssetPin, BootAssetPins, PersistentRegistry, PersistentVmEntry,
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

const ACTIVE_PROFILE_DIR: &str = "vm";
const ACTIVE_PROFILE_FILE: &str = "active_profile.toml";

// Service state

struct ServiceState {
    /// Map of instance ID to Process Info
    instances: Mutex<HashMap<String, InstanceInfo>>,
    /// Logger-owned DB handles keyed by session/VM id. Logged-data routes
    /// resolve a handle here and call `ready/query`; they do not open SQLite
    /// readers or create per-route projection caches.
    session_db_handles: Mutex<HashMap<String, Arc<capsem_logger::DbHandle>>>,
    /// Registry of persistent (named) VMs
    persistent_registry: Mutex<PersistentRegistry>,
    process_binary: PathBuf,
    assets_dir: PathBuf,
    run_dir: PathBuf,
    job_counter: AtomicU64,
    /// v2 manifest (None in dev mode where assets use logical names)
    manifest: RwLock<Option<Arc<capsem_assets::asset_manager::ManifestV2>>>,
    current_version: String,
    /// In-memory asset reconciliation progress. Service startup and explicit
    /// /profiles/{profile_id}/assets/ensure shares this single rail with
    /// status so status can explain both.
    asset_reconcile: Mutex<AssetReconcileState>,
    asset_reconcile_inflight: AtomicBool,
    asset_status_path: PathBuf,
    /// Magika file-type detection session (thread-safe, shared)
    magika: Mutex<magika::Session>,
    /// Profile-owned plugin policy overrides. Effective policy is built-in
    /// plugin defaults plus overrides for the profile executing the VM.
    plugin_policy_by_profile: Mutex<HashMap<String, BTreeMap<String, SecurityPluginConfig>>>,
    /// Route-owned profile summaries loaded once at service startup. Hot
    /// profile routes must not re-read profile files or recompile rules.
    profile_summary_cache: Mutex<Vec<api::ProfileSummary>>,
    /// Route-owned full profile objects loaded once at service startup.
    /// Hot profile/MCP routes must not reload profile files from disk.
    profile_cache: Mutex<BTreeMap<String, Profile>>,
    /// Route-owned profile readiness snapshot loaded once at service startup
    /// and refreshed by explicit profile/asset mutation routes. Hot status
    /// routes must not re-read profile TOML, re-hash assets, or validate the
    /// manifest on every UI/TUI poll.
    profile_status_cache: Mutex<Option<Arc<ProfileStatusCache>>>,
    /// Route-owned compiled rule DTOs loaded once at service startup and
    /// refreshed by profile/corp mutation routes. Polling UI/TUI routes must
    /// not parse profile TOML or compile CEL on every request.
    profile_rule_cache: Mutex<BTreeMap<String, Vec<api::EnforcementRuleInfo>>>,
    /// Route-owned default MCP permission readbacks loaded with the profile
    /// rule cache. Hot MCP routes must not reload and verify enforcement files.
    profile_mcp_default_cache: Mutex<BTreeMap<String, Result<api::McpDefaultPermissionResponse, String>>>,
    /// Route-owned profile plugin configs loaded once at service startup and
    /// refreshed by profile/corp mutation routes. Hot plugin/profile routes
    /// must not re-read profile TOML just to list effective plugin modes.
    profile_plugin_policy_cache: Mutex<BTreeMap<String, BTreeMap<String, SecurityPluginConfig>>>,
    /// Route-owned MCP tool cache loaded once at service startup and refreshed
    /// by explicit MCP discovery routes. Hot MCP list routes must not read the
    /// tool cache JSON from disk.
    mcp_tool_cache: Mutex<Vec<ToolCacheEntry>>,
    /// Logger-owned DB handle for the profile mutation ledger in main.db.
    /// Profile/MCP/rule/plugin edit routes call `write`; they must never open
    /// SQLite directly or hold a side `DbWriter`.
    profile_mutation_db: Arc<capsem_logger::DbHandle>,
    /// Last wall-clock millisecond when preserved boot logs were scanned for
    /// defunct persistent VMs. Hot list/status/info polls must not rescan the
    /// filesystem on every request.
    last_defunct_reconcile_ms: AtomicU64,
    /// Final `/stats` HTTP response bytes derived from the logger-owned
    /// `main.db` query. The epoch ties this cache to the DB handle's own
    /// invalidation generation so writes cannot leave stale route bytes behind.
    stats_response_cache: Mutex<Option<CachedStatsResponse>>,
    /// Final stats/detail bytes for inactive sessions. Running sessions keep
    /// reading live DB state; stopped/seeded sessions can reuse bytes until
    /// their session.db metadata changes.
    stats_detail_response_cache: Mutex<HashMap<String, CachedStatsDetailResponse>>,
    /// Session storage diagnostics cached by session directory. These values
    /// describe the rootfs image path/size and host filesystem for status/info
    /// routes; repeated polling must not stat the filesystem on every sample.
    storage_diagnostics_cache: Mutex<HashMap<PathBuf, api::StorageDiagnostics>>,
    /// Derived persistent VM resume state keyed by stable VM id. The
    /// fingerprint covers registry fields that affect status so route polling
    /// does not reload profile files and revalidate asset pins every sample.
    persistent_resume_state_cache: Mutex<HashMap<String, CachedPersistentResumeState>>,
    /// User-supplied evaluate-route rule snippets compiled by exact TOML
    /// content. Evaluation remains per request; parsing/compilation does not.
    evaluate_rule_cache: Mutex<HashMap<String, SecurityRuleSet>>,
    /// Final JSON bytes for profile rule inventory routes, cleared whenever
    /// the route-owned rule cache is refreshed.
    profile_rule_response_cache: Mutex<HashMap<String, Bytes>>,
    /// Final JSON bytes for profile plugin inventory routes, cleared whenever
    /// profile/plugin policy caches are refreshed.
    profile_plugin_response_cache: Mutex<HashMap<String, Bytes>>,
    /// Final evaluate-route JSON bytes keyed by profile id and exact request
    /// body. Plugin policy refresh clears this cache so repeated UI/TUI probes
    /// do not re-run plugin simulation or serialize identical payloads.
    evaluate_response_cache: Mutex<HashMap<Vec<u8>, Bytes>>,
    /// Final `/vms/list` JSON bytes keyed by the in-memory lifecycle snapshot.
    /// The fingerprint includes running VM uptime seconds, so repeated polling
    /// reuses bytes within the same visible state without freezing the counter.
    list_response_cache: Mutex<Option<CachedListResponse>>,
    /// One-entry hot evaluate cache for repeated probes with the same exact
    /// body. Checked before allocating the multi-entry cache key.
    evaluate_last_response_cache: Mutex<Option<CachedEvaluateResponse>>,
    /// Guards Apple VZ lifecycle edges across all VMs managed by this
    /// service. Cold starts and teardown take a read guard; save/restore take
    /// a write guard. That keeps checkpoint edges exclusive without
    /// serializing independent cold boots and breaking the boot latency gate.
    /// See web/docs/src/content/docs/gotchas/concurrent-suspend-resume.mdx.
    save_restore_lock: tokio::sync::RwLock<()>,
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
    /// Serializes every explicit and automatic update command. The update
    /// transaction owns binaries, profiles, assets, and the selected manifest
    /// together, so the service must never launch split or overlapping
    /// mutations.
    update_lock: tokio::sync::Mutex<()>,
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

#[derive(Clone)]
struct CachedStatsResponse {
    db_epoch: u64,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct CachedStatsDetailResponse {
    db_fingerprint: String,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct CachedPersistentResumeState {
    fingerprint: String,
    state: (VmLifecycleState, bool, Option<String>),
}

#[derive(Clone)]
struct CachedEvaluateResponse {
    profile_id: String,
    request_body: Bytes,
    response_body: Bytes,
}

#[derive(Clone)]
struct CachedListResponse {
    fingerprint: String,
    bytes: Bytes,
}

fn session_db_path_for_session_dir(session_dir: &StdPath) -> PathBuf {
    session_dir.join("session.db")
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
    name: String,
    profile_id: String,
    profile_revision: String,
    profile_payload_hash: String,
    asset_pins: BootAssetPins,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PluginStage {
    Preprocess,
    Postprocess,
    Logging,
}

#[derive(Debug, Clone, Serialize)]
struct PluginRuntimeStatus {
    enabled: bool,
    event_count: u64,
    execution_count: u64,
    applied_count: u64,
    skipped_count: u64,
    total_duration_us: u64,
    max_duration_us: u64,
    detection_count: u64,
    block_count: u64,
    rewrite_count: u64,
    last_error: Option<String>,
    brokered_credentials: Vec<BrokeredCredentialStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PluginCapabilities {
    event_families: Vec<&'static str>,
    credential_providers: Vec<&'static str>,
    credential_sources: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
struct BrokeredCredentialStatus {
    provider: Option<String>,
    credential_ref: String,
    observed_count: u64,
    injected_count: u64,
    replay_available: bool,
    last_seen: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PluginDetailRouteKind {
    CredentialBroker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PluginDetailRoute {
    id: &'static str,
    label: &'static str,
    kind: PluginDetailRouteKind,
    path: String,
}

#[derive(Debug, Serialize)]
struct PluginInfo {
    id: String,
    name: &'static str,
    config: SecurityPluginConfig,
    default_config: SecurityPluginConfig,
    overridden: bool,
    scope: PluginScope,
    description: &'static str,
    stage: PluginStage,
    version: &'static str,
    capabilities: PluginCapabilities,
    runtime: PluginRuntimeStatus,
    detail_routes: Vec<PluginDetailRoute>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CredentialBrokerForkGrantDefault {
    InheritProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CredentialBrokerVmGrant {
    vm_id: String,
    enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CredentialBrokerGrantStatus {
    profile_enabled: bool,
    vm_grants: Vec<CredentialBrokerVmGrant>,
    fork_default: CredentialBrokerForkGrantDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CredentialBrokerCorpConstraint {
    id: String,
    description: String,
}

#[derive(Debug, Clone, Serialize)]
struct CredentialBrokerDetailResponse {
    scope: PluginScope,
    plugin_id: &'static str,
    store: capsem_core::credential_broker::CredentialStoreStatus,
    inventory: Vec<BrokeredCredentialStatus>,
    grants: CredentialBrokerGrantStatus,
    corp_constraints: Vec<CredentialBrokerCorpConstraint>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginUpdate {
    #[serde(default)]
    mode: Option<SecurityPluginMode>,
    #[serde(default)]
    detection_level: Option<DetectionLevel>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpToolEditRequest {
    pub action: SecurityRuleAction,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpServerEditRequest {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileSkillAddRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileSkillEditRequest {
    path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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
action = "allow"
detection_level = "high"
match = 'file.import.content.contains("EICAR")'
"#
            .to_string(),
            event: EnforcementEventInput {
                event_type: "file.import".to_string(),
                file_import_content: Some(capsem_core::security_engine::DUMMY_EICAR_TEST_STRING.to_string()),
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct EnforcementEventInput {
    event_type: String,
    #[serde(default)]
    file_import_content: Option<String>,
    #[serde(default)]
    http_host: Option<String>,
    #[serde(default)]
    http_method: Option<String>,
    #[serde(default)]
    http_path: Option<String>,
    #[serde(default)]
    http_query: Option<String>,
    #[serde(default)]
    http_status: Option<String>,
    #[serde(default)]
    http_body: Option<String>,
    #[serde(default)]
    dns_qname: Option<String>,
    #[serde(default)]
    dns_qtype: Option<String>,
    #[serde(default)]
    mcp_method: Option<String>,
    #[serde(default)]
    mcp_server_name: Option<String>,
    #[serde(default)]
    mcp_tool_call_name: Option<String>,
    #[serde(default)]
    mcp_tool_list: Option<String>,
    #[serde(default)]
    mcp_request_preview: Option<String>,
    #[serde(default)]
    mcp_response_preview: Option<String>,
    #[serde(default)]
    model_provider: Option<String>,
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    model_request_body: Option<String>,
    #[serde(default)]
    model_response_body: Option<String>,
    #[serde(default)]
    model_tool_calls: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    file_ext: Option<String>,
    #[serde(default)]
    file_mime_type: Option<String>,
    #[serde(default)]
    file_content: Option<String>,
    #[serde(default)]
    process_exec_id: Option<String>,
    #[serde(default)]
    process_exec_path: Option<String>,
    #[serde(default)]
    process_command: Option<String>,
    #[serde(default)]
    process_exit_code: Option<String>,
    #[serde(default)]
    process_stdout: Option<String>,
    #[serde(default)]
    process_stderr: Option<String>,
    #[serde(default)]
    ip_value: Option<String>,
    #[serde(default)]
    ip_version: Option<String>,
    #[serde(default)]
    tcp_port: Option<String>,
    #[serde(default)]
    udp_port: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
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
    pub name: &'a str,
    pub profile_id: String,
    pub ram_mb: u64,
    pub cpus: u32,
    pub scratch_disk_size_gb: u32,
    pub version_override: Option<String>,
    pub persistent: bool,
    pub env: Option<std::collections::HashMap<String, String>>,
    pub from: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedVmResources {
    ram_mb: u64,
    cpus: u32,
    scratch_disk_size_gb: u32,
}

fn resolve_profile_vm_resources(
    profile: &ProfileConfigFile,
    requested_ram_mb: Option<u64>,
    requested_cpus: Option<u32>,
) -> ResolvedVmResources {
    ResolvedVmResources {
        ram_mb: requested_ram_mb.unwrap_or(u64::from(profile.vm.ram_gb) * 1024),
        cpus: requested_cpus.unwrap_or(profile.vm.cpu_count),
        scratch_disk_size_gb: profile.vm.scratch_disk_size_gb,
    }
}

fn prewarm_system_overlay_templates(run_dir: &StdPath, profiles: &BTreeMap<String, Profile>) {
    let sizes: HashSet<u32> = profiles
        .values()
        .map(|profile| profile.config().vm.scratch_disk_size_gb)
        .chain(std::iter::once(16))
        .collect();
    for size_gb in sizes {
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
    let Some(expected) = manifest.expected_hashes_current(arch) else {
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
    fn instance_socket_path(&self, id: &str) -> PathBuf {
        let path = capsem_foundation::uds::instance_socket_path(&self.run_dir, id);
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
        main_db_path_for_run_dir(&self.run_dir)
    }

    fn open_profile_mutation_db_handle(run_dir: &StdPath) -> anyhow::Result<Arc<capsem_logger::DbHandle>> {
        let db_path = main_db_path_for_run_dir(run_dir);
        capsem_logger::ensure_session_index_schema(&db_path)
            .with_context(|| format!("failed to initialize session index in main.db: {}", db_path.display()))?;
        let started = std::time::Instant::now();
        let handle = Arc::new(
            capsem_logger::DbHandle::open(&db_path)
                .with_context(|| format!("failed to open profile mutation DB handle: {}", db_path.display()))?,
        );
        info!(
            db_path = %db_path.display(),
            operation = "open_profile_mutation_db_handle",
            duration_ms = started.elapsed().as_millis(),
            "opened profile mutation DB handle"
        );
        Ok(handle)
    }

    #[allow(clippy::too_many_arguments)]
    fn record_session_index_start(
        &self,
        id: &str,
        persistent: bool,
        scratch_disk_size_gb: u32,
        ram_mb: u64,
        rootfs_hash: Option<&str>,
        rootfs_version: Option<&str>,
        forked_from: Option<&str>,
    ) -> anyhow::Result<()> {
        let record = capsem_core::session::SessionRecord {
            id: id.to_string(),
            mode: if persistent { "persistent" } else { "ephemeral" }.to_string(),
            command: None,
            status: "running".to_string(),
            created_at: capsem_core::session::now_iso(),
            stopped_at: None,
            scratch_disk_size_gb,
            ram_bytes: ram_mb.saturating_mul(1024 * 1024),
            total_requests: 0,
            allowed_requests: 0,
            denied_requests: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_estimated_cost: 0.0,
            total_tool_calls: 0,
            total_file_events: 0,
            compressed_size_bytes: None,
            vacuumed_at: None,
            storage_mode: "virtiofs".to_string(),
            rootfs_hash: rootfs_hash.map(ToOwned::to_owned),
            rootfs_version: rootfs_version.map(ToOwned::to_owned),
            forked_from: forked_from.map(ToOwned::to_owned),
            persistent,
            exec_count: 0,
            audit_event_count: 0,
        };

        capsem_logger::record_session_start(&self.main_db_path(), &record)
            .map(|()| self.invalidate_main_db_route_caches())
            .context("create or mark running main.db session row")
    }

    fn record_session_index_stop(
        &self,
        id: &str,
        status: &str,
        session_dir_for_rollup: Option<&StdPath>,
    ) -> anyhow::Result<()> {
        let stopped_at = capsem_core::session::now_iso();
        let session_db_path = session_dir_for_rollup.map(session_db_path_for_session_dir);
        let session_db_path = session_db_path.filter(|path| path.exists());
        capsem_logger::record_session_stop(
            &self.main_db_path(),
            id,
            status,
            Some(&stopped_at),
            session_db_path.as_deref(),
        )
        .map(|()| self.invalidate_main_db_route_caches())
        .with_context(|| {
            if let Some(session_db_path) = session_db_path.as_ref() {
                format!(
                    "roll up session.db into main.db for {id}: {}",
                    session_db_path.display()
                )
            } else {
                format!("mark main.db session row {status} for {id} without session.db")
            }
        })?;
        Ok(())
    }

    fn invalidate_main_db_route_caches(&self) {
        self.profile_mutation_db.invalidate_read_cache();
        *self.stats_response_cache.lock().unwrap() = None;
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

    fn register_session_db_handle(
        &self,
        vm_id: &str,
        session_dir: &StdPath,
    ) -> anyhow::Result<Arc<capsem_logger::DbHandle>> {
        let db_path = session_db_path_for_session_dir(session_dir);
        let started = std::time::Instant::now();
        let handles = self.session_db_handles.lock().unwrap();
        if let Some(handle) = handles.get(vm_id) {
            if handle.path() == db_path.as_path() {
                tracing::debug!(
                    vm_id,
                    db_path = %db_path.display(),
                    operation = "register_session_db_handle",
                    duration_ms = started.elapsed().as_millis(),
                    "reused existing session DB handle"
                );
                return Ok(Arc::clone(handle));
            }
            warn!(
                vm_id,
                cached_db_path = %handle.path().display(),
                db_path = %db_path.display(),
                operation = "register_session_db_handle",
                "replacing session DB handle for rebound session path"
            );
        }
        drop(handles);
        let handle = match capsem_logger::DbHandle::open_external_reader(&db_path) {
            Ok(handle) => Arc::new(handle),
            Err(error) => {
                error!(
                    vm_id,
                    db_path = %db_path.display(),
                    operation = "register_session_db_handle",
                    duration_ms = started.elapsed().as_millis(),
                    error = %error,
                    "failed to register session DB handle"
                );
                return Err(anyhow!(
                    "failed to open session DB handle for {vm_id}: {}: {error}",
                    db_path.display()
                ));
            }
        };
        let mut handles = self.session_db_handles.lock().unwrap();
        handles.insert(vm_id.to_string(), Arc::clone(&handle));
        drop(handles);
        info!(
            vm_id,
            db_path = %db_path.display(),
            operation = "register_session_db_handle",
            duration_ms = started.elapsed().as_millis(),
            "registered session DB handle"
        );
        Ok(handle)
    }

    fn unregister_session_db_handle(&self, vm_id: &str) {
        let removed = self.session_db_handles.lock().unwrap().remove(vm_id);
        if removed.is_some() {
            info!(
                vm_id,
                operation = "unregister_session_db_handle",
                "unregistered session DB handle"
            );
        }
    }

    #[cfg(test)]
    fn rename_session_db_handle(&self, old_vm_id: &str, new_vm_id: &str) {
        let mut handles = self.session_db_handles.lock().unwrap();
        if let Some(handle) = handles.remove(old_vm_id) {
            handles.insert(new_vm_id.to_string(), handle);
            drop(handles);
            info!(
                old_vm_id,
                new_vm_id,
                operation = "rename_session_db_handle",
                "renamed session DB handle"
            );
        }
    }

    fn session_db_handle(&self, vm_id: &str) -> Option<Arc<capsem_logger::DbHandle>> {
        self.session_db_handles.lock().unwrap().get(vm_id).cloned()
    }

    fn hydrate_session_db_handles(&self) {
        let mut candidates: Vec<(String, PathBuf)> = {
            let instances = self.instances.lock().unwrap();
            instances
                .values()
                .map(|info| (info.id.clone(), info.session_dir.clone()))
                .collect()
        };
        {
            let registry = self.persistent_registry.lock().unwrap();
            candidates.extend(
                registry
                    .data
                    .vms
                    .values()
                    .map(|entry| (entry.name.clone(), entry.session_dir.clone())),
            );
        }

        let mut hydrated = 0usize;
        for (vm_id, session_dir) in candidates {
            let db_path = session_db_path_for_session_dir(&session_dir);
            if !db_path.exists() {
                info!(
                    vm_id,
                    operation = "hydrate_session_db_handle",
                    db_path = %db_path.display(),
                    "session DB absent during startup handle hydration"
                );
                continue;
            }
            match self.register_session_db_handle(&vm_id, &session_dir) {
                Ok(_) => hydrated += 1,
                Err(error) => {
                    warn!(
                        vm_id,
                        operation = "hydrate_session_db_handle",
                        db_path = %db_path.display(),
                        error = %error,
                        "failed to hydrate session DB handle"
                    );
                }
            }
        }
        info!(
            operation = "hydrate_session_db_handles",
            hydrated, "startup session DB handle hydration complete"
        );
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
        let dead = instances
            .extract_if(|_, info| unsafe { nix::libc::kill(info.pid as i32, 0) } != 0)
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
        let _ = std::fs::remove_file(info.uds_path.with_extension("ready"));
    }

    fn cleanup_stale_instances(&self) {
        for (id, info) in self.drain_dead_instances() {
            info!(id, "removing stale instance record");
            self.scrub_evicted_instance(&id, &info);
        }
    }

    fn reconcile_persistent_defunct_from_logs(&self) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or_default();
        let last_ms = self.last_defunct_reconcile_ms.load(Ordering::Acquire);
        if now_ms.saturating_sub(last_ms) < 1_000 {
            return;
        }
        if self
            .last_defunct_reconcile_ms
            .compare_exchange(last_ms, now_ms, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let candidates: Vec<(String, PathBuf)> = {
            let registry = self.persistent_registry.lock().unwrap();
            let instances = self.instances.lock().unwrap();
            registry
                .list()
                .filter(|entry| !entry.defunct)
                .filter(|entry| !instances.contains_key(&persistent_entry_vm_id(entry)))
                .map(|entry| (entry.name.clone(), entry.session_dir.clone()))
                .collect()
        };

        let updates: Vec<(String, String)> = candidates
            .into_iter()
            .filter_map(|(name, session_dir)| read_boot_failure_tail(&session_dir).map(|tail| (name, tail)))
            .collect();
        if updates.is_empty() {
            return;
        }

        let mut registry = self.persistent_registry.lock().unwrap();
        let instances = self.instances.lock().unwrap();
        let mut changed = false;
        for (name, tail) in updates {
            if instances.contains_key(&name) {
                continue;
            }
            if let Some(entry) = registry.get_mut(&name) {
                if !entry.defunct {
                    warn!(
                        name,
                        cause = capsem_core::session::boot_failure_summary(&tail),
                        "marking persistent VM defunct from preserved boot logs"
                    );
                    entry.defunct = true;
                    entry.last_error = Some(tail);
                    entry.suspended = false;
                    entry.checkpoint_path = None;
                    changed = true;
                }
            }
        }
        drop(instances);
        if changed {
            if let Err(error) = registry.save() {
                error!(error = %error, "failed to save persistent registry after defunct reconciliation");
            }
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
    fn preserve_failed_session_dir(&self, session_dir: &std::path::Path, id: &str) -> Option<PathBuf> {
        let failed_id = format!("{}-failed-{}", id, capsem_core::session::generate_session_id(),);
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
                Some(failed_dir)
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
                None
            }
        }
    }

    fn cull_failed_sessions(&self) -> Result<()> {
        let sessions_dir = self.run_dir.join("sessions");
        if !sessions_dir.exists() {
            return Ok(());
        }
        let mut failed_dirs: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        let entries =
            std::fs::read_dir(&sessions_dir).with_context(|| format!("read_dir({})", sessions_dir.display()))?;
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

    /// Permanently remove one service-owned session directory.
    ///
    /// Persistent registry data is user-writable state, so never pass its
    /// `session_dir` directly to `remove_dir_all`. Restrict deletion to a
    /// real, direct child of this service's sessions/ or persistent/ roots
    /// and reject symlinks before performing the recursive removal.
    fn delete_session_dir(&self, session_dir: &StdPath) -> Result<()> {
        let parent = session_dir.parent().ok_or_else(|| {
            anyhow!(
                "refusing to delete session path without a parent: {}",
                session_dir.display()
            )
        })?;
        let allowed_parents = [self.run_dir.join("sessions"), self.run_dir.join("persistent")];

        let canonical_run_dir = self.run_dir.canonicalize().with_context(|| {
            format!(
                "canonicalize service run directory before delete: {}",
                self.run_dir.display()
            )
        })?;
        let canonical_requested_parent = parent.canonicalize().with_context(|| {
            format!(
                "canonicalize requested session root before delete: {}",
                parent.display()
            )
        })?;
        let mut canonical_parent = None;
        for allowed_parent in &allowed_parents {
            let parent_metadata = match std::fs::symlink_metadata(allowed_parent) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "inspect service session root before delete: {}",
                            allowed_parent.display()
                        )
                    });
                }
            };
            if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
                if parent == allowed_parent.as_path() {
                    return Err(anyhow!(
                        "refusing to delete through non-directory service session root: {}",
                        allowed_parent.display()
                    ));
                }
                continue;
            }

            let candidate = allowed_parent.canonicalize().with_context(|| {
                format!(
                    "canonicalize service session root before delete: {}",
                    allowed_parent.display()
                )
            })?;
            if candidate.parent() != Some(canonical_run_dir.as_path()) {
                if canonical_requested_parent == candidate {
                    return Err(anyhow!(
                        "refusing to delete through session root outside canonical run directory: {}",
                        allowed_parent.display()
                    ));
                }
                continue;
            }
            if canonical_requested_parent == candidate {
                canonical_parent = Some(candidate);
                break;
            }
        }
        let canonical_parent = canonical_parent.ok_or_else(|| {
            anyhow!(
                "refusing to delete session path outside service roots: {}",
                session_dir.display()
            )
        })?;

        let metadata = match std::fs::symlink_metadata(session_dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect session path before delete: {}", session_dir.display()));
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(anyhow!(
                "refusing to recursively delete non-directory session path: {}",
                session_dir.display()
            ));
        }

        let canonical_session = session_dir.canonicalize().with_context(|| {
            format!(
                "canonicalize service session path before delete: {}",
                session_dir.display()
            )
        })?;
        if canonical_session.parent() != Some(canonical_parent.as_path()) {
            return Err(anyhow!(
                "refusing to delete session path outside canonical service root: {}",
                session_dir.display()
            ));
        }

        // Remove the already verified canonical child, not a registry-provided
        // alias. This keeps a legitimate macOS /var -> /private/var spelling
        // difference working without giving a mutable alias another path
        // resolution opportunity at the destructive operation.
        remove_quiesced_session_dir(&canonical_session)
            .with_context(|| format!("delete canonical session directory: {}", canonical_session.display()))
    }

    fn provision_sandbox(self: &Arc<Self>, options: ProvisionOptions) -> Result<()> {
        let ProvisionOptions {
            id,
            name,
            profile_id,
            ram_mb,
            cpus,
            scratch_disk_size_gb,
            version_override,
            persistent,
            env,
            from,
            description,
        } = options;
        validate_profile_route_id(profile_id.clone()).map_err(|error| anyhow!("invalid profile_id: {}", error.1))?;

        let vm_settings = capsem_core::net::policy_config::load_merged_vm_settings();
        let max_concurrent_vms = vm_settings.max_concurrent_vms.unwrap_or(10) as usize;

        if !(1..=8).contains(&cpus) {
            return Err(anyhow!("cpus must be between 1 and 8"));
        }
        if !(256..=16384).contains(&ram_mb) {
            return Err(anyhow!("ram_mb must be between 256 and 16384"));
        }

        // Persistent VMs: validate the human display name and reject duplicate names.
        if persistent {
            validate_vm_name(name)?;
            let registry = self.persistent_registry.lock().unwrap();
            if registry.contains(name) {
                return Err(anyhow!(
                    "persistent VM \"{}\" already exists. Use `capsem resume {}` to reconnect.",
                    name,
                    name
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
            drop(registry);
            if entry.profile_id != profile_id {
                return Err(anyhow!(
                    "source sandbox '{}' uses profile '{}', not '{}'",
                    from_name,
                    entry.profile_id,
                    profile_id
                ));
            }
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

        let runtime_profile = self.cached_profile_for_runtime(&profile_id)?;
        let active_profile_path = self.materialize_active_profile(&runtime_profile, &session_dir)?;
        let profile = runtime_profile.config();
        let profile_revision = profile.revision.clone();
        let profile_payload_hash = profile_payload_hash(profile)?;
        let asset_pins = profile_asset_pins(profile)?;
        self.validate_profile_pins(profile, &profile_revision, &profile_payload_hash, &asset_pins)?;
        let resolved = self.resolve_profile_asset_paths(profile)?;

        info!(process_binary = %self.process_binary.display(), exists = self.process_binary.exists(), "checking process_binary");

        info!(id, version, asset_version = %resolved.asset_version, "spawning capsem-process");

        let mut child_cmd = tokio::process::Command::new(&self.process_binary);
        if !self.process_binary.exists() {
            info!("process_binary does not exist at absolute path, trying cache/target/cargo/debug/capsem-process");
            child_cmd = tokio::process::Command::new("cache/target/cargo/debug/capsem-process");
        }

        let process_log_path = session_dir.join("process.log");
        let process_log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&process_log_path)
            .context("failed to open process.log")?;

        // Inject VM identity so the guest knows its own name/ID. Anonymous
        // sessions have a generated display name for UI lists, but the guest
        // hostname must remain the opaque route id.
        let guest_name = if persistent { name } else { id };
        child_cmd.arg("--env").arg(format!("CAPSEM_VM_ID={}", id));
        child_cmd.arg("--env").arg(format!("CAPSEM_VM_NAME={}", guest_name));

        // Add --env KEY=VALUE args for each user-specified env var
        if let Some(ref env_vars) = env {
            for (k, v) in env_vars {
                child_cmd.arg("--env").arg(format!("{}={}", k, v));
            }
        }

        // Clear inherited env to prevent API key/token leakage, then
        // re-add only the minimal set needed for the process to function.
        // CAPSEM_HOME and CAPSEM_CORP_CONFIG are forwarded so the child loads
        // the same settings/corp contract as the service.
        child_cmd.env_clear();
        for key in PROCESS_ENV_ALLOWLIST {
            if let Ok(val) = std::env::var(key) {
                child_cmd.env(key, val);
            }
        }
        // W4: propagate trace context to the child process.
        // CAPSEM_VM_ID, CAPSEM_TRACE_ID, TRACEPARENT, TRACESTATE.
        for (k, v) in capsem_foundation::telemetry::child_trace_env(id) {
            child_cmd.env(k, v);
        }

        let process_spawn_span = tracing::debug_span!(
            target: "capsem.launch",
            capsem_foundation::telemetry::LAUNCH_PROCESS_SPAWN_SPAN,
            boot_mode = "provision",
            status = tracing::field::Empty,
        );
        let mut child = match process_spawn_span.in_scope(|| {
            child_cmd
                .env(
                    "RUST_LOG",
                    std::env::var("RUST_LOG")
                        .unwrap_or_else(|_| capsem_foundation::telemetry::with_subsys_targets("capsem=info")),
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
                // The profile's own pins. Boot verifies against these, never
                // against a channel-wide pointer that can only name one profile.
                .arg("--expected-kernel-hash")
                .arg(&asset_pins.kernel.hash)
                .arg("--expected-initrd-hash")
                .arg(&asset_pins.initrd.hash)
                .arg("--expected-rootfs-hash")
                .arg(&asset_pins.rootfs.hash)
                .arg("--session-dir")
                .arg(&session_dir)
                .arg("--active-profile")
                .arg(&active_profile_path)
                .arg("--cpus")
                .arg(cpus.to_string())
                .arg("--ram-mb")
                .arg(ram_mb.to_string())
                .arg("--scratch-disk-size-gb")
                .arg(scratch_disk_size_gb.to_string())
                .arg("--uds-path")
                .arg(&uds_path)
                // Explicitly, because `uds_path` may have been shortened out
                // of the run tree and cannot be walked back up.
                .arg("--run-dir")
                .arg(&self.run_dir)
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

        if let Err(error) = self.record_session_index_start(
            id,
            persistent,
            scratch_disk_size_gb,
            ram_mb,
            Some(&asset_pins.rootfs.hash),
            Some(&version),
            from.as_deref(),
        ) {
            let _ = child.start_kill();
            return Err(error.context("failed to record main.db session start"));
        }

        if persistent {
            let registration = self.persistent_registry.lock().unwrap().register(PersistentVmEntry {
                id: id.to_string(),
                name: name.to_string(),
                profile_id: profile_id.clone(),
                profile_revision: profile_revision.clone(),
                profile_payload_hash: profile_payload_hash.clone(),
                asset_pins: asset_pins.clone(),
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
                description,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: env.clone(),
            });
            if let Err(error) = registration {
                instance_reaper::kill_and_reap(child);
                return Err(error);
            }
        }

        if session_db_path_for_session_dir(&session_dir).exists() {
            if let Err(error) = self.register_session_db_handle(id, &session_dir) {
                instance_reaper::kill_and_reap(child);
                return Err(error);
            }
        } else {
            info!(
                vm_id = id,
                operation = "defer_session_db_handle_registration",
                session_dir = %session_dir.display(),
                "session DB not present yet; route will register the external reader lazily"
            );
        }

        let mut instances = self.instances.lock().unwrap();
        instances.insert(
            id.to_string(),
            InstanceInfo {
                id: id.to_string(),
                name: name.to_string(),
                profile_id,
                profile_revision,
                profile_payload_hash,
                asset_pins,
                pid,
                uds_path: uds_path.clone(),
                session_dir: session_dir.clone(),
                ram_mb,
                cpus,
                start_time: std::time::Instant::now(),
                base_version: version,
                persistent,
                env,
                forked_from: from,
            },
        );
        drop(instances);
        instance_reaper::spawn_provision(
            child,
            id.to_string(),
            name.to_string(),
            Arc::clone(self),
            uds_path,
            session_dir,
        );

        Ok(())
    }

    /// Resume a stopped persistent VM by re-spawning capsem-process against its
    /// existing session directory.
    fn resume_sandbox(
        self: &Arc<Self>,
        id: &str,
        ram_mb_override: Option<u64>,
        cpus_override: Option<u32>,
    ) -> Result<String> {
        self.cleanup_stale_instances();
        self.reconcile_persistent_defunct_from_logs();

        let entry = find_persistent_entry_by_route_id(self.as_ref(), id)
            .ok_or_else(|| anyhow!("no persistent VM with id \"{}\"", id))?;
        let vm_id = persistent_entry_vm_id(&entry);
        if vm_id != id {
            return Err(anyhow!(
                "route id mismatch: requested \"{}\", registry has \"{}\"",
                id,
                vm_id
            ));
        }
        let name = entry.name.clone();

        // Check if already running
        {
            let instances = self.instances.lock().unwrap();
            if instances.contains_key(&vm_id) {
                return Ok(vm_id);
            }
        }

        if !entry.session_dir.exists() {
            return Err(anyhow!("session directory for \"{}\" is missing", name));
        }
        if entry.defunct {
            let reason = entry
                .last_error
                .as_deref()
                .unwrap_or("previous boot failed before the VM reached ready");
            return Err(anyhow!("persistent VM \"{}\" is defunct: {}", name, reason));
        }

        let ram_mb = ram_mb_override.unwrap_or(entry.ram_mb);
        let cpus = cpus_override.unwrap_or(entry.cpus);
        let version = entry.base_version.clone();

        info!(name, version, "resume_sandbox: re-spawning process");

        let uds_path = self.instance_socket_path(&vm_id);
        let _ = std::fs::create_dir_all(uds_path.parent().unwrap());

        // Clear stale UDS + ready sentinel from the prior boot. Without this,
        // wait_for_vm_ready returns instantly against the old .ready file and
        // callers race ahead before the resumed agent has reconnected.
        let _ = std::fs::remove_file(&uds_path);
        let _ = std::fs::remove_file(uds_path.with_extension("ready"));

        self.validate_persistent_profile_authority(&entry)?;
        let active_profile_path = self.persistent_active_profile_path(&entry)?;
        let scratch_disk_size_gb = self.persistent_scratch_disk_size_gb(&entry)?;
        let resolved = self.resolve_pinned_asset_paths(&entry.asset_pins)?;
        self.validate_pinned_asset_files(&resolved, &entry.asset_pins)?;

        let process_log_path = entry.session_dir.join("process.log");
        let process_log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&process_log_path)
            .context("failed to open process.log")?;

        let mut child_cmd = tokio::process::Command::new(&self.process_binary);
        if !self.process_binary.exists() {
            child_cmd = tokio::process::Command::new("cache/target/cargo/debug/capsem-process");
        }

        // Inject VM identity so the guest knows its own name/ID.
        child_cmd.arg("--env").arg(format!("CAPSEM_VM_ID={}", vm_id));
        child_cmd.arg("--env").arg(format!("CAPSEM_VM_NAME={}", name));

        // Replay user-provided env vars so they survive stop/resume cycles.
        if let Some(ref env_vars) = entry.env {
            for (k, v) in env_vars {
                child_cmd.arg("--env").arg(format!("{}={}", k, v));
            }
        }

        // Pass checkpoint path for warm restore from suspended state only
        // when the completion marker proves save_state + fsync finished.
        if entry.suspended {
            if let Some(ref cp) = entry.checkpoint_path {
                let full_checkpoint = entry.session_dir.join(cp);
                let complete = checkpoint_complete_path(&full_checkpoint);
                if full_checkpoint.exists() && complete.exists() {
                    child_cmd.arg("--checkpoint-path").arg(&full_checkpoint);
                    info!(name, checkpoint = %full_checkpoint.display(), "warm restore from checkpoint");
                } else {
                    tracing::warn!(name, checkpoint = %full_checkpoint.display(), complete = %complete.display(), "checkpoint incomplete, cold booting");
                }
            }
        }

        // Clear inherited env to prevent API key/token leakage, then
        // re-add only the minimal set needed for the process to function.
        // CAPSEM_HOME and CAPSEM_CORP_CONFIG are forwarded so the child loads
        // the same settings/corp contract as the service.
        child_cmd.env_clear();
        for key in PROCESS_ENV_ALLOWLIST {
            if let Ok(val) = std::env::var(key) {
                child_cmd.env(key, val);
            }
        }
        // W4: propagate trace context (resume path).
        for (k, v) in capsem_foundation::telemetry::child_trace_env(&vm_id) {
            child_cmd.env(k, v);
        }

        let process_spawn_span = tracing::debug_span!(
            target: "capsem.launch",
            capsem_foundation::telemetry::LAUNCH_PROCESS_SPAWN_SPAN,
            boot_mode = "resume",
            status = tracing::field::Empty,
        );
        let child = match process_spawn_span.in_scope(|| {
            child_cmd
                .env(
                    "RUST_LOG",
                    std::env::var("RUST_LOG")
                        .unwrap_or_else(|_| capsem_foundation::telemetry::with_subsys_targets("capsem=info")),
                )
                .arg("--id")
                .arg(&vm_id)
                .arg("--assets-dir")
                .arg(&self.assets_dir)
                .arg("--rootfs")
                .arg(&resolved.rootfs)
                .arg("--kernel")
                .arg(&resolved.kernel)
                .arg("--initrd")
                .arg(&resolved.initrd)
                // The profile's own pins. Boot verifies against these, never
                // against a channel-wide pointer that can only name one profile.
                .arg("--expected-kernel-hash")
                .arg(&entry.asset_pins.kernel.hash)
                .arg("--expected-initrd-hash")
                .arg(&entry.asset_pins.initrd.hash)
                .arg("--expected-rootfs-hash")
                .arg(&entry.asset_pins.rootfs.hash)
                .arg("--session-dir")
                .arg(&entry.session_dir)
                .arg("--active-profile")
                .arg(&active_profile_path)
                .arg("--cpus")
                .arg(cpus.to_string())
                .arg("--ram-mb")
                .arg(ram_mb.to_string())
                .arg("--scratch-disk-size-gb")
                .arg(scratch_disk_size_gb.to_string())
                .arg("--uds-path")
                .arg(&uds_path)
                // Explicitly, because `uds_path` may have been shortened out
                // of the run tree and cannot be walked back up.
                .arg("--run-dir")
                .arg(&self.run_dir)
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

        if session_db_path_for_session_dir(&entry.session_dir).exists() {
            if let Err(error) = self.register_session_db_handle(&vm_id, &entry.session_dir) {
                instance_reaper::kill_and_reap(child);
                return Err(error);
            }
        } else {
            info!(
                vm_id = vm_id,
                operation = "defer_session_db_handle_registration",
                session_dir = %entry.session_dir.display(),
                "session DB not present yet; route will register the external reader lazily"
            );
        }

        let mut instances = self.instances.lock().unwrap();
        instances.insert(
            vm_id.clone(),
            InstanceInfo {
                id: vm_id.clone(),
                name: entry.name.clone(),
                profile_id: entry.profile_id.clone(),
                profile_revision: entry.profile_revision.clone(),
                profile_payload_hash: entry.profile_payload_hash.clone(),
                asset_pins: entry.asset_pins.clone(),
                pid,
                uds_path: uds_path.clone(),
                session_dir: entry.session_dir.clone(),
                ram_mb,
                cpus,
                start_time: std::time::Instant::now(),
                base_version: version,
                persistent: true,
                env: None,
                forked_from: entry.forked_from,
            },
        );
        drop(instances);
        instance_reaper::spawn_resume(child, vm_id.clone(), Arc::clone(self), uds_path);

        Ok(vm_id)
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

    /// Resolve asset file paths for a VM.
    ///
    /// In v2 mode (manifest present): resolves hash-based filenames from manifest.
    /// In dev mode (no manifest): finds assets by logical name in arch subdirs.
    #[cfg(test)]
    fn resolve_asset_paths(&self) -> Result<capsem_assets::asset_manager::ResolvedAssets> {
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x86_64"
        };

        // Resolve from v2 manifest (works for both dev and installed --
        // dev creates hash-named symlinks, installed has hash-named files)
        if let Some(manifest) = self.manifest.read().unwrap().as_ref().cloned() {
            return manifest.resolve(&self.current_version, arch, &self.assets_dir);
        }

        // No manifest: use logical EROFS names so callers report missing
        // assets rather than accepting an obsolete rootfs format.
        let base = if self.assets_dir.join(arch).join("rootfs.erofs").exists() {
            self.assets_dir.join(arch)
        } else {
            self.assets_dir.clone()
        };
        let rootfs = base.join("rootfs.erofs");
        Ok(capsem_assets::asset_manager::ResolvedAssets {
            kernel: base.join("vmlinuz"),
            initrd: base.join("initrd.img"),
            rootfs,
            asset_version: "dev".to_string(),
        })
    }

    fn profile_config(&self, profile_id: &str) -> Result<ProfileConfigFile> {
        #[cfg(test)]
        let catalog = if let Some(path) = test_profile_dir_override() {
            ProfileCatalog::load_from_dir(&path).map_err(|e| anyhow!("load profile catalog: {e}"))?
        } else {
            ProfileCatalog::builtin()
        };
        #[cfg(not(test))]
        let catalog = ProfileCatalog::load_default().map_err(|e| anyhow!("load profile catalog: {e}"))?;
        catalog
            .get(profile_id)
            .cloned()
            .ok_or_else(|| anyhow!("profile not found: {profile_id}"))
    }

    fn cached_profile_for_runtime(&self, profile_id: &str) -> Result<Profile> {
        self.profile_cache
            .lock()
            .map_err(|error| anyhow!("profile cache lock poisoned: {error}"))?
            .get(profile_id)
            .cloned()
            .ok_or_else(|| anyhow!("profile not found: {profile_id}"))
    }

    fn cached_profile_config(&self, profile_id: &str) -> Result<ProfileConfigFile> {
        Ok(self.cached_profile_for_runtime(profile_id)?.config().clone())
    }

    fn profile_for_runtime(&self, profile_id: &str) -> Result<Profile> {
        #[cfg(test)]
        let catalog = if let Some(path) = test_profile_dir_override() {
            ProfileCatalog::load_from_dir(&path).map_err(|e| anyhow!("load profile catalog: {e}"))?
        } else {
            ProfileCatalog::builtin()
        };
        #[cfg(not(test))]
        let catalog = ProfileCatalog::load_default().map_err(|e| anyhow!("load profile catalog: {e}"))?;
        let profile = catalog
            .get(profile_id)
            .cloned()
            .ok_or_else(|| anyhow!("profile not found: {profile_id}"))?;
        match catalog.source() {
            ProfileCatalogSource::BuiltIn => {
                let config_root = builtin_profile_config_root();
                let profile_dir = config_root.join("profiles").join(&profile.id);
                Profile::from_config(config_root, profile_dir, profile)
                    .map_err(|e| anyhow!("load builtin profile {profile_id}: {e}"))
            }
            ProfileCatalogSource::Directory(profiles_dir) => {
                let config_root = profiles_dir.parent().ok_or_else(|| {
                    anyhow!(
                        "profile directory {} must be under a config root",
                        profiles_dir.display()
                    )
                })?;
                Profile::from_config(config_root.to_path_buf(), profiles_dir.join(profile_id), profile)
                    .map_err(|e| anyhow!("load profile {profile_id}: {e}"))
            }
        }
    }

    fn materialize_active_profile(&self, profile: &Profile, session_dir: &StdPath) -> Result<PathBuf> {
        let config = profile.config();
        let (_, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
        let plugins = self
            .plugin_policy_by_profile
            .lock()
            .unwrap()
            .get(&config.id)
            .cloned()
            .unwrap_or_default();
        let active_profile = ActiveProfileFile::from_profile_and_corp(profile, &corp, plugins)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("build active profile for {}", config.id))?;
        let active_profile_dir = session_dir.join(ACTIVE_PROFILE_DIR);
        std::fs::create_dir_all(&active_profile_dir)
            .with_context(|| format!("create {}", active_profile_dir.display()))?;
        let active_profile_path = active_profile_dir.join(ACTIVE_PROFILE_FILE);
        std::fs::write(
            &active_profile_path,
            toml::to_string_pretty(&active_profile).context("serialize active profile")?,
        )
        .with_context(|| format!("write {}", active_profile_path.display()))?;

        let stale_runtime_config = session_dir.join("runtime-config");
        if stale_runtime_config.exists() {
            std::fs::remove_dir_all(&stale_runtime_config)
                .with_context(|| format!("remove stale {}", stale_runtime_config.display()))?;
        }

        Ok(active_profile_path)
    }

    fn refresh_active_profiles(&self, profile_filter: Option<&str>) -> Result<usize> {
        let targets = {
            let instances = self.instances.lock().unwrap();
            instances
                .iter()
                .filter(|(_, info)| {
                    profile_filter
                        .map(|profile_id| info.profile_id == profile_id)
                        .unwrap_or(true)
                })
                .map(|(id, info)| (id.clone(), info.profile_id.clone(), info.session_dir.clone()))
                .collect::<Vec<_>>()
        };

        for (id, profile_id, session_dir) in &targets {
            let runtime_profile = self
                .profile_for_runtime(profile_id)
                .with_context(|| format!("load runtime profile {profile_id} for {id}"))?;
            self.materialize_active_profile(&runtime_profile, session_dir)
                .with_context(|| {
                    format!(
                        "refresh active profile config for {id} ({profile_id}) in {}",
                        session_dir.display()
                    )
                })?;
        }

        Ok(targets.len())
    }

    fn refresh_profile_rule_cache(&self, profile_filter: Option<&str>) -> Result<()> {
        let updates = build_profile_rule_cache(profile_filter)
            .map_err(|error| anyhow!("refresh profile rule cache: {}", error.1))?;
        let mcp_default_updates = build_profile_mcp_default_cache(profile_filter)
            .map_err(|error| anyhow!("refresh profile MCP default cache: {}", error.1))?;
        {
            let mut cache = self.profile_rule_cache.lock().unwrap();
            if profile_filter.is_none() {
                *cache = updates;
            } else {
                for (profile_id, rules) in updates {
                    cache.insert(profile_id, rules);
                }
            }
        }
        {
            let mut cache = self.profile_mcp_default_cache.lock().unwrap();
            if profile_filter.is_none() {
                *cache = mcp_default_updates;
            } else {
                for (profile_id, permission) in mcp_default_updates {
                    cache.insert(profile_id, permission);
                }
            }
        }
        self.profile_rule_response_cache.lock().unwrap().clear();
        Ok(())
    }

    fn refresh_profile_plugin_policy_cache(&self, profile_filter: Option<&str>) -> Result<()> {
        let updates = build_profile_plugin_policy_cache(profile_filter)
            .map_err(|error| anyhow!("refresh profile plugin cache: {}", error.1))?;
        let mut cache = self.profile_plugin_policy_cache.lock().unwrap();
        if profile_filter.is_none() {
            *cache = updates;
        } else {
            for (profile_id, plugins) in updates {
                cache.insert(profile_id, plugins);
            }
        }
        drop(cache);
        self.profile_plugin_response_cache.lock().unwrap().clear();
        self.evaluate_response_cache.lock().unwrap().clear();
        *self.evaluate_last_response_cache.lock().unwrap() = None;
        Ok(())
    }

    fn resolve_profile_asset_paths(
        &self,
        profile: &ProfileConfigFile,
    ) -> Result<capsem_assets::asset_manager::ResolvedAssets> {
        let arch = capsem_core::net::policy_config::current_profile_arch();
        let arch_assets = profile
            .assets
            .current_arch_assets()
            .ok_or_else(|| anyhow!("profile {} has no assets for architecture {arch}", profile.id))?;

        Ok(capsem_assets::asset_manager::ResolvedAssets {
            kernel: profile_asset_descriptor_path(&self.assets_dir, arch, &arch_assets.kernel)?,
            initrd: profile_asset_descriptor_path(&self.assets_dir, arch, &arch_assets.initrd)?,
            rootfs: profile_asset_descriptor_path(&self.assets_dir, arch, &arch_assets.rootfs)?,
            asset_version: format!("profile:{}@{}", profile.id, profile.revision),
        })
    }

    fn validate_profile_pins(
        &self,
        profile: &ProfileConfigFile,
        profile_revision: &str,
        pinned_profile_payload_hash: &str,
        pins: &BootAssetPins,
    ) -> Result<()> {
        self.validate_profile_identity_and_pins(profile, profile_revision, pinned_profile_payload_hash, pins)?;
        self.validate_profile_asset_files(profile, pins)
    }

    fn validate_profile_identity_and_pins(
        &self,
        profile: &ProfileConfigFile,
        profile_revision: &str,
        pinned_profile_payload_hash: &str,
        pins: &BootAssetPins,
    ) -> Result<()> {
        if profile.revision != profile_revision {
            return Err(anyhow!(
                "profile '{}' revision mismatch: VM pinned '{}', current '{}'",
                profile.id,
                profile_revision,
                profile.revision
            ));
        }
        let current_payload_hash = profile_payload_hash(profile)?;
        if current_payload_hash != pinned_profile_payload_hash {
            return Err(anyhow!(
                "profile '{}' payload hash mismatch: VM pinned '{}', current '{}'",
                profile.id,
                pinned_profile_payload_hash,
                current_payload_hash
            ));
        }
        let current = profile_asset_pins(profile)?;
        if &current != pins {
            return Err(anyhow!(
                "profile '{}' asset pins changed: VM pinned {:?}, current {:?}",
                profile.id,
                pins,
                current
            ));
        }
        Ok(())
    }

    fn validate_profile_asset_files(&self, profile: &ProfileConfigFile, pins: &BootAssetPins) -> Result<()> {
        let resolved = self.resolve_profile_asset_paths(profile)?;
        validate_asset_file_pin("kernel", &resolved.kernel, &pins.kernel)?;
        validate_asset_file_pin("initrd", &resolved.initrd, &pins.initrd)?;
        validate_asset_file_pin("rootfs", &resolved.rootfs, &pins.rootfs)?;
        Ok(())
    }

    fn persistent_active_profile_path(&self, entry: &PersistentVmEntry) -> Result<PathBuf> {
        let active_profile_path = entry.session_dir.join(ACTIVE_PROFILE_DIR).join(ACTIVE_PROFILE_FILE);
        if active_profile_path.exists() {
            validate_saved_active_profile(&active_profile_path, entry)?;
            return Ok(active_profile_path);
        }

        let current = self.profile_for_runtime(&entry.profile_id)?;
        self.validate_profile_identity_and_pins(
            current.config(),
            &entry.profile_revision,
            &entry.profile_payload_hash,
            &entry.asset_pins,
        )?;
        self.materialize_active_profile(&current, &entry.session_dir)
    }

    fn validate_persistent_profile_authority(&self, entry: &PersistentVmEntry) -> Result<()> {
        reject_revoked_persistent_pins(&self.assets_dir, entry)?;
        let active_profile_path = entry.session_dir.join(ACTIVE_PROFILE_DIR).join(ACTIVE_PROFILE_FILE);
        if active_profile_path.exists() {
            validate_saved_active_profile(&active_profile_path, entry)?;
            return Ok(());
        }

        let current = self.profile_config(&entry.profile_id)?;
        self.validate_profile_identity_and_pins(
            &current,
            &entry.profile_revision,
            &entry.profile_payload_hash,
            &entry.asset_pins,
        )
    }

    fn persistent_scratch_disk_size_gb(&self, entry: &PersistentVmEntry) -> Result<u32> {
        let actual = session_rootfs_size_gb(entry)?;
        let Ok(current) = self.profile_config(&entry.profile_id) else {
            return Ok(actual);
        };
        if self
            .validate_profile_identity_and_pins(
                &current,
                &entry.profile_revision,
                &entry.profile_payload_hash,
                &entry.asset_pins,
            )
            .is_ok()
            && actual != current.vm.scratch_disk_size_gb
        {
            return Err(anyhow!(
                "VM '{}' rootfs.img logical size mismatch: current {} GiB, pinned profile '{}' requires {} GiB",
                entry.name,
                actual,
                current.id,
                current.vm.scratch_disk_size_gb
            ));
        }
        Ok(actual)
    }

    fn resolve_pinned_asset_paths(&self, pins: &BootAssetPins) -> Result<capsem_assets::asset_manager::ResolvedAssets> {
        let arch = capsem_core::net::policy_config::current_profile_arch();
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

        if let Err(err) = self.validate_persistent_profile_authority(entry) {
            return (VmLifecycleState::Incompatible, false, Some(err.to_string()));
        }
        if let Err(err) = self.persistent_scratch_disk_size_gb(entry) {
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
