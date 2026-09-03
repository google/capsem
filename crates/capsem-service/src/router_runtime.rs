use super::*;

pub(super) fn build_service_router(state: Arc<ServiceState>) -> Router {
    Router::new()
        .route("/status", get(handle_service_status))
        .route("/update/status", get(handle_update_status))
        .route("/system/status", get(handle_system_status))
        .route("/update/check", post(handle_update_check))
        .route("/update/apply", post(handle_update_apply))
        .route(
            "/version",
            get(|| async { Json(serde_json::json!({ "version": env!("CARGO_PKG_VERSION") })) }),
        )
        .route("/vms/create", post(handle_provision))
        .route("/vms/list", get(handle_list))
        .route("/vms/{id}/info", get(handle_info))
        .route("/vms/{id}/status", get(handle_vm_status))
        .route("/vms/{id}/snapshots/status", get(handle_vm_snapshots_status))
        .route("/vms/{id}/snapshots/list", get(handle_vm_snapshots_list))
        .route("/vms/{id}/logs", get(handle_logs))
        .route("/vms/{id}/exec", post(handle_exec))
        .route("/vms/{id}/files/write", post(handle_write_file))
        .route("/vms/{id}/files/read", post(handle_read_file))
        .route("/vms/{id}/stop", post(handle_stop))
        .route("/vms/{id}/pause", post(handle_suspend))
        .route("/vms/{id}/delete", delete(handle_delete))
        .route("/vms/{id}/preserve-failure", post(handle_preserve_failure))
        .route("/vms/{id}/start", post(handle_resume))
        .route("/vms/{id}/resume", post(handle_resume))
        .route("/vms/{id}/save", post(handle_persist))
        .route("/vms/{id}/save/status", get(handle_vm_save_status))
        .route("/vms/{id}/fork/status", get(handle_vm_fork_status))
        .route("/purge", post(handle_purge))
        .route("/run", post(handle_run))
        .route("/stats", get(handle_stats))
        .route("/vms/{id}/stats/summary", get(handle_stats_summary))
        .route("/vms/{id}/stats/detail", get(handle_stats_detail))
        .route("/service-logs", get(handle_service_logs))
        .route("/triage", get(handle_triage))
        .route("/panics", get(handle_panics))
        .route("/host-logs/{name}", get(handle_host_logs))
        .route("/vms/{id}/timeline", get(handle_timeline))
        .route("/vms/{id}/security/latest", get(handle_security_latest))
        .route("/vms/{id}/security/status", get(handle_security_info))
        .route("/vms/{id}/detection/latest", get(handle_detection_latest))
        .route("/vms/{id}/detection/status", get(handle_security_info))
        .route("/vms/{id}/enforcement/latest", get(handle_security_latest))
        .route("/vms/{id}/enforcement/status", get(handle_security_info))
        .route("/security/latest", get(handle_service_security_latest))
        .route("/security/status", get(handle_service_security_status))
        .route("/enforcement/latest", get(handle_service_security_latest))
        .route("/enforcement/status", get(handle_service_security_status))
        .route("/detection/latest", get(handle_service_detection_latest))
        .route("/detection/status", get(handle_service_detection_status))
        .route("/profiles/list", get(handle_profiles_list))
        .route("/profiles/status", get(handle_profiles_status))
        .route("/profiles/reload", post(handle_profiles_reload))
        .route("/profiles/{profile_id}/info", get(handle_profile_info))
        .route("/profiles/{profile_id}/obom", get(handle_profile_obom))
        .route("/profiles/{profile_id}/validate", post(handle_profile_validate))
        .route(
            "/profiles/{profile_id}/enforcement/evaluate",
            post(handle_enforcement_evaluate),
        )
        .route("/profiles/{profile_id}/enforcement/info", get(handle_enforcement_info))
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
            "/profiles/{profile_id}/enforcement/rules/list",
            get(handle_enforcement_rules_list),
        )
        .route(
            "/profiles/{profile_id}/detection/evaluate",
            post(handle_detection_evaluate),
        )
        .route("/profiles/{profile_id}/detection/info", get(handle_detection_info))
        .route(
            "/profiles/{profile_id}/detection/rules/{rule_id}/edit",
            put(handle_detection_rule_upsert),
        )
        .route(
            "/profiles/{profile_id}/detection/rules/{rule_id}/delete",
            delete(handle_detection_rule_delete),
        )
        .route("/profiles/{profile_id}/detection/reload", post(handle_detection_reload))
        .route(
            "/profiles/{profile_id}/detection/rules/list",
            get(handle_detection_rules_list),
        )
        .route("/profiles/{profile_id}/plugins/list", get(handle_profile_plugins))
        .route("/profiles/{profile_id}/plugins/info", get(handle_profile_plugins_info))
        .route(
            "/profiles/{profile_id}/plugins/credential_broker/credentials/info",
            get(handle_profile_credential_broker_credentials_info),
        )
        .route(
            "/profiles/{profile_id}/plugins/credential_broker/credentials/reload",
            post(handle_profile_credential_broker_credentials_reload),
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
        .route("/vms/{id}/fork", post(handle_fork))
        .route("/settings/info", get(handle_get_settings))
        .route("/settings/edit", patch(handle_save_settings))
        .route(
            "/profiles/{profile_id}/assets/status",
            get(handle_profile_assets_status),
        )
        .route("/profiles/{profile_id}/assets/info", get(handle_profile_assets_info))
        .route(
            "/profiles/{profile_id}/assets/ensure",
            post(handle_profile_assets_ensure),
        )
        .route("/profiles/{profile_id}/skills/info", get(handle_profile_skills_info))
        .route("/profiles/{profile_id}/skills/list", get(handle_profile_skills_list))
        .route("/profiles/{profile_id}/skills/add", post(handle_profile_skill_add))
        .route(
            "/profiles/{profile_id}/skills/{skill_id}/edit",
            patch(handle_profile_skill_edit),
        )
        .route(
            "/profiles/{profile_id}/skills/{skill_id}/delete",
            delete(handle_profile_skill_delete),
        )
        .route("/corp/info", get(handle_corp_info))
        .route("/corp/edit", put(handle_corp_config))
        .route("/corp/validate", post(handle_corp_validate))
        .route("/corp/reload", post(handle_corp_reload))
        .route(
            "/profiles/{profile_id}/mcp/servers/list",
            get(handle_profile_mcp_servers),
        )
        .route("/profiles/{profile_id}/mcp/info", get(handle_profile_mcp_info))
        .route(
            "/profiles/{profile_id}/mcp/default/info",
            get(handle_profile_mcp_default_info),
        )
        .route(
            "/profiles/{profile_id}/mcp/default/edit",
            patch(handle_profile_mcp_default_edit),
        )
        .route(
            "/profiles/{profile_id}/mcp/servers/{server_id}/edit",
            put(handle_profile_mcp_server_edit),
        )
        .route(
            "/profiles/{profile_id}/mcp/servers/{server_id}/delete",
            delete(handle_profile_mcp_server_delete),
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
        .route("/vms/{id}/history", get(handle_history))
        .route("/vms/{id}/history/processes", get(handle_history_processes))
        .route("/vms/{id}/history/counts", get(handle_history_counts))
        .route("/vms/{id}/history/transcript", get(handle_history_transcript))
        .route("/vms/{id}/files/list", get(handle_list_files))
        .route(
            "/vms/{id}/files/content",
            get(handle_download_file).post(handle_upload_file),
        )
        .layer(TraceLayer::new_for_http().on_request(()).on_response(()))
        .with_state(state)
}

pub(super) async fn handle_update_status(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<api::UpdateStatusResponse>, AppError> {
    state.off_worker(|state| update_status_response(&state)).await.map(Json)
}

pub(super) async fn handle_system_status(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<api::SystemStatusResponse>, AppError> {
    let (manifest, manifest_metadata, updates) = state
        .off_worker(|state| {
            let manifest = read_installed_status_document(&state.assets_dir.join("manifest.json"))?;
            let metadata = read_manifest_metadata_status_document(&state.assets_dir.join("manifest-metadata.json"))?;
            Ok::<_, AppError>((manifest, metadata, update_status_response(&state)))
        })
        .await??;
    capsem_assets::asset_manager::ManifestV2::from_json(&serde_json::to_string(&manifest).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize installed manifest for validation: {error}"),
        )
    })?)
    .map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("installed manifest is invalid: {error:#}"),
        )
    })?;
    let profiles = if asset_reconcile_has_route_fields(&state) {
        refresh_reconcile_fields(&state, profile_status_cache(&state)?.catalog.clone())
    } else {
        profile_status_cache(&state)?.catalog.clone()
    };
    Ok(Json(api::SystemStatusResponse {
        version: state.current_version.clone(),
        service: "running".to_string(),
        manifest,
        manifest_metadata,
        profiles,
        corp: corp_info_value()?,
        updates,
    }))
}

pub(super) fn read_installed_status_document(path: &StdPath) -> Result<serde_json::Value, AppError> {
    let content = std::fs::read_to_string(path).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("read installed status document {}: {error}", path.display()),
        )
    })?;
    serde_json::from_str(&content).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("parse installed status document {}: {error}", path.display()),
        )
    })
}

pub(super) fn read_manifest_metadata_status_document(path: &StdPath) -> Result<serde_json::Value, AppError> {
    let value = read_installed_status_document(path)?;
    if value.get("schema").and_then(serde_json::Value::as_str) != Some("capsem.manifest_metadata.v1") {
        return Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "installed manifest metadata must use schema capsem.manifest_metadata.v1".to_string(),
        ));
    }
    Ok(value)
}

pub(super) async fn handle_update_check(
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<api::UpdateCheckRequest>,
) -> Result<Json<api::UpdateActionResponse>, AppError> {
    let plan = update_command_plan(UpdateCommandKind::Check);
    if request.dry_run {
        return Ok(Json(planned_update_response(plan)));
    }
    execute_update_command(&state, plan).await.map(Json)
}

pub(super) async fn handle_update_apply(
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<api::UpdateApplyRequest>,
) -> Result<Json<api::UpdateActionResponse>, AppError> {
    let plan = update_command_plan(UpdateCommandKind::Apply);
    if request.dry_run {
        return Ok(Json(planned_update_response(plan)));
    }
    if !request.confirmed {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "update apply requires confirmed=true or dry_run=true".to_string(),
        ));
    }
    execute_update_apply(&state, plan).await.map(Json)
}

pub(super) fn planned_update_response(plan: api::UpdateCommandPlan) -> api::UpdateActionResponse {
    api::UpdateActionResponse {
        status: "planned".to_string(),
        command: plan,
        exit_code: None,
        stdout: None,
        stderr: None,
    }
}

pub(super) async fn execute_update_command(
    state: &ServiceState,
    plan: api::UpdateCommandPlan,
) -> Result<api::UpdateActionResponse, AppError> {
    let _update_guard = state.update_lock.lock().await;
    execute_update_command_unlocked(plan).await
}

pub(super) async fn execute_update_apply(
    state: &ServiceState,
    plan: api::UpdateCommandPlan,
) -> Result<api::UpdateActionResponse, AppError> {
    let _update_guard = state.update_lock.lock().await;
    let response = execute_update_command_unlocked(plan).await?;
    if response.status == "succeeded" {
        reload_activated_update_runtime(state)?;
    }
    Ok(response)
}

pub(super) async fn execute_update_command_unlocked(
    plan: api::UpdateCommandPlan,
) -> Result<api::UpdateActionResponse, AppError> {
    let output = Command::new(&plan.program)
        .args(&plan.args)
        .output()
        .await
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to start update command: {error}"),
            )
        })?;
    let status = if output.status.success() { "succeeded" } else { "failed" };
    Ok(api::UpdateActionResponse {
        status: status.to_string(),
        command: plan,
        exit_code: output.status.code(),
        stdout: Some(String::from_utf8_lossy(&output.stdout).to_string()),
        stderr: Some(String::from_utf8_lossy(&output.stderr).to_string()),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdateRuntimeDisposition {
    Reloaded,
    RestartRequested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AutomaticUpdateOutcome {
    Disabled,
    Busy,
    Succeeded(UpdateRuntimeDisposition),
    Failed(String),
}

pub(super) fn automatic_updates_enabled() -> bool {
    let (user, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
    let resolved = capsem_core::net::policy_config::resolve_settings(&user, &corp);
    automatic_updates_enabled_from_resolved(&resolved)
}

pub(super) fn automatic_updates_enabled_from_resolved(
    settings: &[capsem_core::net::policy_config::ResolvedSetting],
) -> bool {
    settings
        .iter()
        .find(|setting| setting.id == "app.auto_update")
        .and_then(|setting| setting.effective_value.as_bool())
        .unwrap_or(true)
}

pub(super) fn automatic_update_failure_backoff(consecutive_failures: u32) -> std::time::Duration {
    let exponent = consecutive_failures.saturating_sub(1).min(8);
    let seconds = AUTOMATIC_UPDATE_POLL_SECS
        .saturating_mul(1_u64 << exponent)
        .min(AUTOMATIC_UPDATE_MAX_BACKOFF_SECS);
    std::time::Duration::from_secs(seconds)
}

pub(super) fn automatic_update_delay_from_value(
    value: Option<&std::ffi::OsStr>,
    default_seconds: u64,
) -> std::time::Duration {
    let seconds = value
        .and_then(std::ffi::OsStr::to_str)
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .unwrap_or(default_seconds);
    std::time::Duration::from_secs(seconds)
}

pub(super) fn automatic_update_delay(environment_name: &str, default_seconds: u64) -> std::time::Duration {
    let value = std::env::var_os(environment_name);
    automatic_update_delay_from_value(value.as_deref(), default_seconds)
}

pub(super) async fn run_automatic_update_once(state: &ServiceState) -> AutomaticUpdateOutcome {
    if !automatic_updates_enabled() {
        return AutomaticUpdateOutcome::Disabled;
    }
    let Ok(_update_guard) = state.update_lock.try_lock() else {
        return AutomaticUpdateOutcome::Busy;
    };
    let plan = update_command_plan(UpdateCommandKind::Apply);
    let response = match execute_update_command_unlocked(plan).await {
        Ok(response) => response,
        Err(error) => return AutomaticUpdateOutcome::Failed(error.1),
    };
    if response.status != "succeeded" {
        let detail = response
            .stderr
            .as_deref()
            .map(str::trim)
            .filter(|detail| !detail.is_empty())
            .unwrap_or("update command returned a non-zero exit status");
        return AutomaticUpdateOutcome::Failed(format!("exit {:?}: {detail}", response.exit_code));
    }
    match reload_activated_update_runtime(state) {
        Ok(disposition) => AutomaticUpdateOutcome::Succeeded(disposition),
        Err(error) => AutomaticUpdateOutcome::Failed(error.1),
    }
}

pub(super) async fn run_automatic_update_loop(state: Arc<ServiceState>) {
    let mut delay = automatic_update_delay(AUTOMATIC_UPDATE_INITIAL_DELAY_ENV, AUTOMATIC_UPDATE_INITIAL_DELAY_SECS);
    let poll_delay = automatic_update_delay(AUTOMATIC_UPDATE_POLL_ENV, AUTOMATIC_UPDATE_POLL_SECS);
    let mut consecutive_failures = 0_u32;
    loop {
        tokio::time::sleep(delay).await;
        match run_automatic_update_once(&state).await {
            AutomaticUpdateOutcome::Disabled => {
                consecutive_failures = 0;
                delay = poll_delay;
                info!("automatic release polling is disabled by app.auto_update");
            }
            AutomaticUpdateOutcome::Busy => {
                delay = std::time::Duration::from_secs(AUTOMATIC_UPDATE_BUSY_RETRY_SECS);
                info!("automatic release polling skipped because an explicit update owns the lock");
            }
            AutomaticUpdateOutcome::Succeeded(disposition) => {
                consecutive_failures = 0;
                delay = poll_delay;
                info!(
                    ?disposition,
                    next_poll_secs = poll_delay.as_secs(),
                    "automatic release update completed"
                );
            }
            AutomaticUpdateOutcome::Failed(error) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                delay = automatic_update_failure_backoff(consecutive_failures);
                warn!(
                    error = %error,
                    consecutive_failures,
                    retry_secs = delay.as_secs(),
                    "automatic release update failed"
                );
            }
        }
    }
}

pub(super) fn should_start_automatic_update_loop(parent_pid: Option<u32>) -> bool {
    // `--parent-pid` is the explicit bounded-test harness rail. Production
    // services never receive it, while long VM suites must not gain an
    // unrelated network/package mutation one minute into a test process.
    parent_pid.is_none()
}

pub(super) fn reload_activated_update_runtime(state: &ServiceState) -> Result<UpdateRuntimeDisposition, AppError> {
    let path = state.assets_dir.join("manifest.json");
    let content = std::fs::read_to_string(&path).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("read activated update manifest {}: {error}", path.display()),
        )
    })?;
    let manifest = capsem_assets::asset_manager::ManifestV2::from_json(&content).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("validate activated update manifest {}: {error:#}", path.display()),
        )
    })?;
    let selected_binary = manifest.binaries.current.clone();
    let previous_manifest = {
        let mut installed = state.manifest.write().map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("installed manifest lock poisoned: {error}"),
            )
        })?;
        installed.replace(Arc::new(manifest))
    };

    if let Err(error) = refresh_profile_route_caches(state) {
        if let Ok(mut installed) = state.manifest.write() {
            *installed = previous_manifest;
        }
        return Err(error);
    }

    if selected_binary != state.current_version {
        info!(
            running = %state.current_version,
            selected = %selected_binary,
            "activated binary differs from the running service; requesting managed restart"
        );
        state.update_restart.notify_one();
        Ok(UpdateRuntimeDisposition::RestartRequested)
    } else {
        info!(
            selected = %selected_binary,
            "reloaded activated manifest and profile caches in the running service"
        );
        Ok(UpdateRuntimeDisposition::Reloaded)
    }
}

pub(super) async fn handle_service_status(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let credential_store = capsem_core::credential_broker::credential_store_status();
    let ready = credential_store.ready;
    Ok(Json(serde_json::json!({
        "service": "capsem-service",
        "version": state.current_version,
        "ready": ready,
        "components": {
            "credential_store": {
                "ready": credential_store.ready,
                "status": credential_store.status,
                "last_error": credential_store.last_error,
            },
        },
    })))
}
