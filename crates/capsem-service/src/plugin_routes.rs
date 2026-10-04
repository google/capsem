//! Security plugins: their catalog, their effective modes -- built in, then
//! the user's settings.toml, then corp -- and what they did across sessions.
use super::*;

use capsem_core::net::policy_config::merge_plugin_policy;

#[derive(Debug, Serialize)]
pub(super) struct PluginListResponse {
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
pub(super) struct BrokeredCredentialStatus {
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
    path: &'static str,
}

#[derive(Debug, Serialize)]
pub(super) struct PluginInfo {
    id: String,
    name: &'static str,
    config: SecurityPluginConfig,
    default_config: SecurityPluginConfig,
    /// settings.toml or the corp config sets this plugin.
    overridden: bool,
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
    Inherit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CredentialBrokerVmGrant {
    vm_id: String,
    enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CredentialBrokerGrantStatus {
    enabled: bool,
    vm_grants: Vec<CredentialBrokerVmGrant>,
    fork_default: CredentialBrokerForkGrantDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CredentialBrokerCorpConstraint {
    id: String,
    description: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CredentialBrokerDetailResponse {
    plugin_id: &'static str,
    store: capsem_core::credential_broker::CredentialStoreStatus,
    inventory: Vec<BrokeredCredentialStatus>,
    grants: CredentialBrokerGrantStatus,
    corp_constraints: Vec<CredentialBrokerCorpConstraint>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PluginUpdate {
    #[serde(default)]
    mode: Option<SecurityPluginMode>,
    #[serde(default)]
    detection_level: Option<DetectionLevel>,
}

pub(super) fn default_plugin_config(mode: SecurityPluginMode) -> SecurityPluginConfig {
    SecurityPluginConfig {
        mode,
        detection_level: DetectionLevel::Informational,
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PluginCatalogEntry {
    name: &'static str,
    description: &'static str,
    default_config: SecurityPluginConfig,
    stage: PluginStage,
    version: &'static str,
}

static PLUGIN_CATALOG: LazyLock<BTreeMap<String, PluginCatalogEntry>> = LazyLock::new(|| {
    BTreeMap::from([
        (
            "credential_broker".to_string(),
            PluginCatalogEntry {
                name: "Credential Broker",
                description: "captures observed credentials into brokered credential references",
                default_config: default_plugin_config(SecurityPluginMode::Rewrite),
                stage: PluginStage::Preprocess,
                version: "1",
            },
        ),
        (
            "log_sanitizer".to_string(),
            PluginCatalogEntry {
                name: "Log Sanitizer",
                description: "sanitizes credential material before durable security ledger writes",
                default_config: default_plugin_config(SecurityPluginMode::Rewrite),
                stage: PluginStage::Logging,
                version: "1",
            },
        ),
        (
            "dummy_pre_eicar".to_string(),
            PluginCatalogEntry {
                name: "Dummy Preprocess EICAR",
                description: "debug preprocess plugin that blocks harmless EICAR test content",
                default_config: default_plugin_config(SecurityPluginMode::Disable),
                stage: PluginStage::Preprocess,
                version: "1",
            },
        ),
        (
            "dummy_post_allow".to_string(),
            PluginCatalogEntry {
                name: "Dummy Postprocess Allow",
                description: "debug postprocess plugin that requests allow to prove block is absolute",
                default_config: default_plugin_config(SecurityPluginMode::Disable),
                stage: PluginStage::Postprocess,
                version: "1",
            },
        ),
    ])
});

pub(super) fn plugin_catalog() -> &'static BTreeMap<String, PluginCatalogEntry> {
    &PLUGIN_CATALOG
}

/// Each catalogued plugin's effective configuration, and the plugins settings
/// or corp set.
type EffectivePluginPolicy = (BTreeMap<String, SecurityPluginConfig>, BTreeSet<String>);

/// The effective plugin policy from the current files.
fn effective_plugin_policy() -> Result<EffectivePluginPolicy, AppError> {
    let (settings, corp) = capsem_core::net::policy_config::load_policy_files()
        .map_err(|error| AppError(StatusCode::BAD_REQUEST, error))?;
    let mut policy: BTreeMap<_, _> = plugin_catalog()
        .iter()
        .map(|(id, entry)| (id.clone(), entry.default_config))
        .collect();
    policy.extend(merge_plugin_policy(&settings, &corp));
    let overridden = settings.plugins.keys().chain(corp.plugins.keys()).cloned().collect();
    Ok((policy, overridden))
}

async fn plugin_info_for(
    state: &Arc<ServiceState>,
    plugin_id: &str,
    include_runtime: bool,
) -> Result<PluginInfo, AppError> {
    let policy = state.off_worker(|_| effective_plugin_policy()).await??;
    plugin_info_from(state, &policy, plugin_id, include_runtime).await
}

/// One plugin's info against a policy the caller already read, so a route
/// answering for every plugin reads the policy files once.
async fn plugin_info_from(
    state: &Arc<ServiceState>,
    (policy, overridden): &EffectivePluginPolicy,
    plugin_id: &str,
    include_runtime: bool,
) -> Result<PluginInfo, AppError> {
    let Some(catalog_entry) = plugin_catalog().get(plugin_id).copied() else {
        return Err(AppError(StatusCode::NOT_FOUND, format!("unknown plugin: {plugin_id}")));
    };
    let config = policy.get(plugin_id).copied().unwrap_or(catalog_entry.default_config);
    let runtime = if include_runtime {
        plugin_runtime_status(state, plugin_id, config).await
    } else {
        plugin_runtime_config_status(config)
    };
    Ok(PluginInfo {
        id: plugin_id.to_string(),
        name: catalog_entry.name,
        config,
        default_config: catalog_entry.default_config,
        overridden: overridden.contains(plugin_id),
        description: catalog_entry.description,
        stage: catalog_entry.stage,
        version: catalog_entry.version,
        capabilities: plugin_capabilities(plugin_id),
        runtime,
        detail_routes: plugin_detail_routes(plugin_id),
    })
}

fn plugin_capabilities(plugin_id: &str) -> PluginCapabilities {
    match plugin_id {
        "credential_broker" => PluginCapabilities {
            event_families: vec!["http", "file", "mcp"],
            credential_providers: capsem_core::credential_broker::CredentialProvider::all()
                .iter()
                .map(|provider| provider.as_str())
                .collect(),
            credential_sources: vec![
                "http.authorization",
                "http.body.oauth_token",
                "file.env",
                "mcp.auth_reference",
            ],
        },
        "dummy_pre_eicar" | "dummy_post_allow" => PluginCapabilities {
            event_families: vec!["http", "model", "file", "mcp"],
            credential_providers: Vec::new(),
            credential_sources: Vec::new(),
        },
        "log_sanitizer" => PluginCapabilities {
            event_families: vec!["http", "model", "file", "mcp"],
            credential_providers: Vec::new(),
            credential_sources: vec!["security_event.credential_observations"],
        },
        _ => PluginCapabilities {
            event_families: Vec::new(),
            credential_providers: Vec::new(),
            credential_sources: Vec::new(),
        },
    }
}

fn plugin_detail_routes(plugin_id: &str) -> Vec<PluginDetailRoute> {
    match plugin_id {
        "credential_broker" => vec![
            PluginDetailRoute {
                id: "credential_broker_credentials",
                label: "Credential Broker",
                kind: PluginDetailRouteKind::CredentialBroker,
                path: "/plugins/credential_broker/credentials/info",
            },
            PluginDetailRoute {
                id: "credential_broker_credentials_reload",
                label: "Retry Credential Store",
                kind: PluginDetailRouteKind::CredentialBroker,
                path: "/plugins/credential_broker/credentials/reload",
            },
        ],
        _ => Vec::new(),
    }
}

async fn plugin_runtime_status(
    state: &ServiceState,
    plugin_id: &str,
    config: SecurityPluginConfig,
) -> PluginRuntimeStatus {
    let mut status = plugin_runtime_config_status(config);
    let snapshots = match activity::session_counters(state).await {
        Ok(snapshots) => snapshots,
        Err(error) => {
            status.last_error = Some(format!("failed to read security ledger: {}", error.1));
            return status;
        }
    };
    hydrate_plugin_execution_runtime(&snapshots, plugin_id, &mut status);
    if plugin_id == "credential_broker" {
        hydrate_credential_broker_runtime(&snapshots, &mut status);
    }
    status
}

fn plugin_runtime_config_status(config: SecurityPluginConfig) -> PluginRuntimeStatus {
    PluginRuntimeStatus {
        enabled: config.mode != SecurityPluginMode::Disable,
        event_count: 0,
        execution_count: 0,
        applied_count: 0,
        skipped_count: 0,
        total_duration_us: 0,
        max_duration_us: 0,
        detection_count: 0,
        block_count: 0,
        rewrite_count: 0,
        last_error: None,
        brokered_credentials: Vec::new(),
    }
}

/// A plugin's runtime across every session, from the sessions' counter
/// snapshots.
///
/// This used to re-parse the archived payloads of each session's latest 2000
/// rule matches, so every count stopped growing -- and started drifting --
/// once a session passed 2000 matches.
fn hydrate_plugin_execution_runtime(
    snapshots: &[Arc<capsem_logger::counters::LedgerCounters>],
    plugin_id: &str,
    status: &mut PluginRuntimeStatus,
) {
    for plugin in snapshots.iter().filter_map(|counters| counters.plugins.get(plugin_id)) {
        status.execution_count += plugin.executions;
        status.applied_count += plugin.applied;
        status.skipped_count += plugin.skipped;
        status.total_duration_us = status.total_duration_us.saturating_add(plugin.total_duration_us);
        status.max_duration_us = status.max_duration_us.max(plugin.max_duration_us);
        status.detection_count += plugin.detections;
    }
}

pub(super) fn merge_brokered_credential_status(
    credentials: &mut BTreeMap<(Option<String>, String), BrokeredCredentialStatus>,
    provider: Option<String>,
    credential_ref: String,
    observed_delta: u64,
    injected_delta: u64,
    last_seen: Option<String>,
) {
    let key = (provider.clone(), credential_ref.clone());
    let replay_available =
        capsem_core::credential_broker::broker_reference_replay_available(provider.as_deref(), &credential_ref);
    credentials
        .entry(key)
        .and_modify(|existing| {
            existing.observed_count = existing.observed_count.saturating_add(observed_delta);
            existing.injected_count = existing.injected_count.saturating_add(injected_delta);
            existing.replay_available |= replay_available;
            if last_seen.as_deref() > existing.last_seen.as_deref() {
                existing.last_seen = last_seen.clone();
            }
        })
        .or_insert(BrokeredCredentialStatus {
            provider,
            credential_ref,
            observed_count: observed_delta,
            injected_count: injected_delta,
            replay_available,
            last_seen,
        });
}

/// Brokered credential activity across every session: the substitutions the
/// broker recorded and the observations and injections the security payloads
/// carried, from the sessions' counter snapshots.
///
/// `last_seen` is one RFC 3339 spelling for both sources. It used to be a
/// ledger timestamp for one and bare epoch milliseconds for the other,
/// compared as strings.
fn hydrate_credential_broker_runtime(
    snapshots: &[Arc<capsem_logger::counters::LedgerCounters>],
    status: &mut PluginRuntimeStatus,
) {
    let mut credentials: BTreeMap<(Option<String>, String), BrokeredCredentialStatus> = BTreeMap::new();
    for (credential_ref, counts) in snapshots.iter().flat_map(|counters| &counters.credentials) {
        let observed = counts.substitutions.saturating_add(counts.observations);
        let injected = counts.injected_substitutions.saturating_add(counts.injections);
        status.event_count = status.event_count.saturating_add(observed.saturating_add(injected));
        status.rewrite_count = status.rewrite_count.saturating_add(injected);
        let last_seen = (counts.last_seen_unix_ms > 0).then(|| {
            capsem_logger::format_ledger_timestamp(
                std::time::UNIX_EPOCH + std::time::Duration::from_millis(counts.last_seen_unix_ms as u64),
            )
        });
        merge_brokered_credential_status(
            &mut credentials,
            counts.provider.clone(),
            credential_ref.clone(),
            observed,
            injected,
            last_seen,
        );
    }
    let mut values: Vec<_> = credentials.into_values().collect();
    values.sort_by(|left, right| right.last_seen.cmp(&left.last_seen));
    status.brokered_credentials = values;
}

/// GET /plugins/list -- every catalogued plugin with its effective mode.
pub(super) async fn handle_plugins(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<PluginListResponse>, AppError> {
    let policy = state.off_worker(|_| effective_plugin_policy()).await??;
    let mut plugins = Vec::new();
    for plugin_id in plugin_catalog().keys() {
        plugins.push(plugin_info_from(&state, &policy, plugin_id, false).await?);
    }
    Ok(Json(PluginListResponse { plugins }))
}

/// GET /plugins/credential_broker/credentials/info -- the broker's store,
/// grants and the credentials it brokered across sessions.
pub(super) async fn handle_credential_broker_credentials_info(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<CredentialBrokerDetailResponse>, AppError> {
    let info = plugin_info_for(&state, "credential_broker", true).await?;
    Ok(Json(CredentialBrokerDetailResponse {
        plugin_id: "credential_broker",
        store: capsem_core::credential_broker::credential_store_status(),
        inventory: info.runtime.brokered_credentials,
        grants: CredentialBrokerGrantStatus {
            enabled: info.config.mode != SecurityPluginMode::Disable,
            vm_grants: Vec::new(),
            fork_default: CredentialBrokerForkGrantDefault::Inherit,
        },
        corp_constraints: Vec::new(),
    }))
}

/// GET /plugins/{plugin_id}/info -- one plugin with its runtime activity,
/// read from the running sessions' security ledgers.
pub(super) async fn handle_plugin_info(
    State(state): State<Arc<ServiceState>>,
    Path(plugin_id): Path<String>,
) -> Result<Json<PluginInfo>, AppError> {
    Ok(Json(plugin_info_for(&state, &plugin_id, true).await?))
}

/// POST /plugins/credential_broker/credentials/reload -- retry the durable
/// credential store and re-read every session's counters.
pub(super) async fn handle_credential_broker_credentials_reload(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<CredentialBrokerDetailResponse>, AppError> {
    match capsem_core::credential_broker::hydrate_credential_runtime_cache_from_durable_store() {
        Ok(count) => info!(
            component = "credential_store",
            loaded_count = count,
            status = "ready",
            "credential store retry hydrated runtime cache"
        ),
        Err(error) => warn!(
            component = "credential_store",
            error = %error,
            status = "degraded",
            "credential store retry failed"
        ),
    }
    for (vm_id, _) in service_session_dirs(&state) {
        state.unregister_session_db_handle(&vm_id);
    }
    handle_credential_broker_credentials_info(State(state)).await
}

/// PATCH /plugins/{plugin_id}/edit -- set one plugin's mode or detection level
/// in settings.toml and put it in force in every running VM.
pub(super) async fn handle_plugin_update(
    State(state): State<Arc<ServiceState>>,
    Path(plugin_id): Path<String>,
    Json(update): Json<PluginUpdate>,
) -> Result<Json<PluginInfo>, AppError> {
    let Some(catalog_entry) = plugin_catalog().get(&plugin_id).copied() else {
        return Err(AppError(StatusCode::NOT_FOUND, format!("unknown plugin: {plugin_id}")));
    };
    let route = MutationRoute {
        name: "plugin_edit",
        target_kind: "plugin",
        target_key: &plugin_id,
        operation: "edit",
    };
    let id = plugin_id.clone();
    // The current config is read under the boundary: it is the "read" of a
    // read-modify-write, and a concurrent edit must not be overwritten.
    apply_policy_mutation(&state, route, move |edit, corp| {
        let mut config = edit.plugin_config(&id).unwrap_or(catalog_entry.default_config);
        if let Some(mode) = update.mode {
            config.mode = mode;
        }
        if let Some(detection_level) = update.detection_level {
            config.detection_level = detection_level;
        }
        edit.set_plugin_config(corp, &id, config, "service-api")
    })
    .await?;
    Ok(Json(plugin_info_for(&state, &plugin_id, true).await?))
}
