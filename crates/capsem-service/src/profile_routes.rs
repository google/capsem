use super::*;

pub(super) async fn handle_reload_config(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    handle_reload_config_for_profile(state, None).await
}

pub(super) async fn handle_reload_config_for_profile(
    state: Arc<ServiceState>,
    profile_filter: Option<&str>,
) -> Result<Json<serde_json::Value>, AppError> {
    state
        .refresh_active_profiles(profile_filter)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state
        .refresh_profile_rule_cache(profile_filter)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state
        .refresh_profile_plugin_policy_cache(profile_filter)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Collect paths to broadcast to.
    let uds_paths = {
        let instances = state.instances.lock().unwrap();
        instances
            .iter()
            .filter(|(_, info)| {
                profile_filter
                    .map(|profile_id| info.profile_id == profile_id)
                    .unwrap_or(true)
            })
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
            format!("failed to reload config in some instances: {}", failures.join(", ")),
        ))
    }
}

pub(super) async fn handle_profile_reload(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let profile_id = validate_profile_route_id(profile_id)?;
    handle_reload_config_for_profile(state, Some(&profile_id)).await
}

// ---------------------------------------------------------------------------
// Settings endpoints
// ---------------------------------------------------------------------------

/// GET /settings/info -- unified settings tree + issues.
pub(super) async fn handle_get_settings() -> Json<serde_json::Value> {
    let resp = capsem_core::net::policy_config::load_settings_response();
    Json(serde_json::to_value(resp).unwrap_or_default())
}

/// PATCH /settings/edit -- batch-update settings and return the refreshed tree.
pub(super) async fn handle_save_settings(
    Json(raw): Json<HashMap<String, serde_json::Value>>,
) -> Result<Json<serde_json::Value>, AppError> {
    capsem_core::net::policy_config::batch_update_settings_json(&raw)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
    let resp = capsem_core::net::policy_config::load_settings_response();
    Ok(Json(serde_json::to_value(resp).unwrap_or_default()))
}

#[cfg(test)]
pub(super) fn profile_asset_status_value(state: &ServiceState, profile: &ProfileConfigFile) -> serde_json::Value {
    let reconcile = state.asset_reconcile.lock().map(|s| s.clone()).unwrap_or_default();
    let current_arch = capsem_core::net::policy_config::current_profile_arch();
    let Some(arch_assets) = profile.assets.current_arch_assets() else {
        let mut value = json!({
            "profile_id": profile.id,
            "revision": profile.revision,
            "profile_payload_hash": profile_payload_hash(profile).ok(),
            "manifest": asset_manifest_status_value(state),
            "ready": false,
            "downloading": reconcile.in_progress,
            "current_arch": current_arch,
            "error": format!("profile {} has no assets for architecture {current_arch}", profile.id),
            "assets": [],
        });
        append_asset_reconcile_status(&mut value, &reconcile);
        return value;
    };

    let assets = [
        ("kernel", &arch_assets.kernel),
        ("initrd", &arch_assets.initrd),
        ("rootfs", &arch_assets.rootfs),
    ]
    .into_iter()
    .map(|(kind, asset)| {
        let (path, materialization_error) = match profile_asset_descriptor_path(&state.assets_dir, current_arch, asset)
        {
            Ok(path) => (path, None),
            Err(error) => (state.assets_dir.join(current_arch).join(&asset.name), Some(error)),
        };
        let resolved_name = path.file_name().and_then(|name| name.to_str()).unwrap_or(&asset.name);
        let error = materialization_error.map(|error| error.to_string());
        let status = if error.is_some() {
            "error"
        } else if path.exists() {
            "present"
        } else {
            "missing"
        };
        json!({
            "kind": kind,
            "name": asset.name,
            "logical_name": asset.name,
            "resolved_name": resolved_name,
            "path": path.display().to_string(),
            "status": status,
            "hash": asset.hash,
            "size": asset.size,
            "url": asset.url,
            "error": error,
        })
    })
    .collect::<Vec<_>>();
    let all_ready = assets.iter().all(|asset| asset["status"] == "present");
    let mut value = json!({
        "profile_id": profile.id,
        "revision": profile.revision,
        "profile_payload_hash": profile_payload_hash(profile).ok(),
        "manifest": asset_manifest_status_value(state),
        "ready": all_ready,
        "downloading": reconcile.in_progress,
        "current_arch": current_arch,
        "assets": assets,
    });
    append_asset_reconcile_status(&mut value, &reconcile);
    value
}

pub(super) fn profile_update_semantics() -> api::ProfileUpdateSemantics {
    api::ProfileUpdateSemantics {
        new_sessions: api::ProfileNewSessionUpdateSemantics::UseCurrentProfileCatalog,
        existing_vms: api::ProfileExistingVmUpdateSemantics::PinnedUntilRecreate,
        upgrade_action: api::ProfileUpgradeAction::RecreateVm,
    }
}

pub(super) fn profile_update_semantics_value() -> serde_json::Value {
    serde_json::to_value(profile_update_semantics()).unwrap_or_else(|_| json!({}))
}

pub(super) fn profile_status_value(state: &ServiceState, profile: &Profile) -> serde_json::Value {
    let reconcile = state.asset_reconcile.lock().map(|s| s.clone()).unwrap_or_default();
    let current_arch = capsem_core::net::policy_config::current_profile_arch();
    let status = profile.readiness_status(&state.assets_dir, current_arch);
    let config = profile.config();
    let assets = status
        .assets
        .iter()
        .map(|asset| {
            json!({
                "arch": asset.arch,
                "kind": asset.kind,
                "name": asset.path.file_name().and_then(|name| name.to_str()).unwrap_or("asset"),
                "path": asset.path.display().to_string(),
                "status": if !asset.present { "missing" } else if !asset.valid { "invalid" } else { "present" },
                "present": asset.present,
                "valid": asset.valid,
                "expected_hash": asset.expected_hash,
                "expected_size": asset.expected_size,
                "actual_hash": asset.actual_hash,
                "actual_size": asset.actual_size,
            })
        })
        .collect::<Vec<_>>();
    let files = status
        .files
        .iter()
        .map(|file| {
            json!({
                "kind": file.kind,
                "path": file.path.display().to_string(),
                "status": if !file.present { "missing" } else if !file.valid { "invalid" } else { "present" },
                "present": file.present,
                "valid": file.valid,
                "expected_hash": file.expected_hash,
                "expected_size": file.expected_size,
                "actual_hash": file.actual_hash,
                "actual_size": file.actual_size,
            })
        })
        .collect::<Vec<_>>();
    let missing_assets = status
        .assets
        .iter()
        .filter(|asset| !asset.present)
        .map(|asset| json!({ "kind": asset.kind, "path": asset.path.display().to_string(), "valid": asset.valid }))
        .collect::<Vec<_>>();
    let invalid_assets = status
        .assets
        .iter()
        .filter(|asset| !asset.valid)
        .map(|asset| json!({ "kind": asset.kind, "path": asset.path.display().to_string(), "present": asset.present, "valid": asset.valid }))
        .collect::<Vec<_>>();
    let invalid_files = status
        .files
        .iter()
        .filter(|file| !file.valid)
        .map(|file| json!({ "kind": file.kind, "path": file.path.display().to_string(), "present": file.present, "valid": file.valid }))
        .collect::<Vec<_>>();
    let mut value = json!({
        "profile_id": config.id,
        "revision": config.revision,
        "profile_payload_hash": profile_payload_hash(config).ok(),
        "update_semantics": profile_update_semantics_value(),
        "manifest": asset_manifest_status_value(state),
        "ready": status.ready,
        "downloading": reconcile.in_progress,
        "current_arch": current_arch,
        "files": files,
        "invalid_files": invalid_files,
        "assets": assets,
        "missing_assets": missing_assets,
        "invalid_assets": invalid_assets,
        "errors": status.errors,
    });
    append_asset_reconcile_status(&mut value, &reconcile);
    value
}

pub(super) fn asset_manifest_status_value(state: &ServiceState) -> serde_json::Value {
    let path = state.assets_dir.join("manifest.json");
    let metadata_path = state.assets_dir.join("manifest-metadata.json");
    let manifest_metadata = std::fs::read_to_string(&metadata_path)
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok());
    let refreshed_at = std::fs::metadata(&path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .map(format_system_time_rfc3339);
    let blake3 = if path.is_file() {
        capsem_assets::asset_manager::hash_file(&path).ok()
    } else {
        None
    };
    let manifest_validation = validate_asset_manifest_file(&path);
    let origin = if let Some(origin) = manifest_metadata
        .as_ref()
        .and_then(|value| value.get("origin"))
        .and_then(|value| value.as_str())
    {
        origin
    } else if path.is_file() {
        "installed"
    } else {
        "missing"
    };
    let mut value = json!({
        "origin": origin,
        "path": path.display().to_string(),
        "blake3": blake3,
        "validation_status": manifest_validation.status,
    });
    if let Some(refreshed_at) = refreshed_at {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("refreshed_at".to_string(), json!(refreshed_at));
        }
    }
    if let Some(error) = manifest_validation.error.as_ref() {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("validation_error".to_string(), json!(error));
        }
    }
    if let (Some(metadata), Some(obj)) = (&manifest_metadata, value.as_object_mut()) {
        obj.insert("origin_path".to_string(), json!(metadata_path.display().to_string()));
        if let Some(source) = metadata.get("manifest_url").and_then(|value| value.as_str()) {
            obj.insert("origin_source".to_string(), json!(source));
        }
        if let Some(packaged_at) = metadata.get("packaged_at").and_then(|value| value.as_str()) {
            obj.insert("packaged_at".to_string(), json!(packaged_at));
        }
    }
    let installed_manifest = state.manifest.read().unwrap();
    let manifest = manifest_validation.manifest.as_ref().or_else(|| {
        if manifest_validation.status == "missing" {
            installed_manifest.as_deref()
        } else {
            None
        }
    });
    if let (Some(manifest), Some(obj)) = (manifest, value.as_object_mut()) {
        obj.insert("format".to_string(), json!(manifest.format));
        obj.insert("refresh_policy".to_string(), json!(manifest.refresh_policy));
        obj.insert("assets_current".to_string(), json!(manifest.assets.current));
        obj.insert("binaries_current".to_string(), json!(manifest.binaries.current));
    }
    value
}

pub(super) fn update_status_response(state: &ServiceState) -> api::UpdateStatusResponse {
    update_status_response_from_paths(
        &state.current_version,
        &state.assets_dir,
        &state.assets_dir.join("manifest-metadata.json"),
        unix_now_secs(),
    )
}

pub(super) fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) struct AssetManifestValidation {
    status: &'static str,
    manifest: Option<capsem_assets::asset_manager::ManifestV2>,
    error: Option<String>,
}

pub(super) fn validate_asset_manifest_file(path: &std::path::Path) -> AssetManifestValidation {
    if !path.is_file() {
        return AssetManifestValidation {
            status: "missing",
            manifest: None,
            error: None,
        };
    }
    match std::fs::read_to_string(path) {
        Ok(content) => match capsem_assets::asset_manager::ManifestV2::from_json(&content) {
            Ok(manifest) => AssetManifestValidation {
                status: "valid",
                manifest: Some(manifest),
                error: None,
            },
            Err(error) => AssetManifestValidation {
                status: "invalid",
                manifest: None,
                error: Some(error.to_string()),
            },
        },
        Err(error) => AssetManifestValidation {
            status: "invalid",
            manifest: None,
            error: Some(error.to_string()),
        },
    }
}

pub(super) fn format_system_time_rfc3339(time: std::time::SystemTime) -> String {
    humantime::format_rfc3339_seconds(time).to_string()
}

pub(super) fn append_asset_reconcile_status(value: &mut serde_json::Value, reconcile: &AssetReconcileState) {
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

pub(super) fn vm_asset_block_reason(state: &ServiceState, profile_id: &str) -> Option<String> {
    let profile = match state.profile_config(profile_id) {
        Ok(profile) => profile,
        Err(error) => return Some(format!("VM assets are not ready: {error}")),
    };
    let resolved = match state.resolve_profile_asset_paths(&profile) {
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

pub(super) fn asset_status_path_for_run_dir(run_dir: &StdPath) -> PathBuf {
    run_dir.parent().unwrap_or(run_dir).join("asset-status.json")
}

pub(super) fn load_asset_reconcile_state(path: &StdPath) -> AssetReconcileState {
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

pub(super) fn persist_asset_reconcile_state(path: &StdPath, status: &AssetReconcileState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json =
        serde_json::to_vec_pretty(status).map_err(|e| format!("serialize asset status {}: {e}", path.display()))?;
    std::fs::write(&tmp, json).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))?;
    Ok(())
}

pub(super) fn update_asset_reconcile_state<F>(state: &ServiceState, update: F) -> Result<AssetReconcileState, String>
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

#[cfg(test)]
pub(super) async fn ensure_assets_for_state(state: Arc<ServiceState>) -> Result<usize, String> {
    claim_asset_reconcile(&state)?;
    let result = ensure_assets_after_claim(Arc::clone(&state)).await;
    state.asset_reconcile_inflight.store(false, Ordering::Release);
    result
}

pub(super) async fn ensure_assets_after_claim(state: Arc<ServiceState>) -> Result<usize, String> {
    let result: Result<usize, String> = async {
        let Some(manifest) = state.manifest.read().unwrap().as_ref().cloned() else {
            return Ok(0);
        };
        update_asset_reconcile_state(&state, |status| {
            *status = AssetReconcileState {
                in_progress: true,
                ..Default::default()
            };
        })?;
        let arch = capsem_assets::asset_manager::host_manifest_arch();
        let downloaded = capsem_assets::asset_manager::download_missing_assets(
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
                        let snapshot = state.asset_reconcile.lock().map(|status| status.clone()).ok();
                        if let Some(snapshot) = snapshot {
                            if let Err(error) = persist_asset_reconcile_state(&state.asset_status_path, &snapshot) {
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
    result
}

#[cfg(test)]
pub(super) async fn ensure_profile_assets_for_state(
    state: Arc<ServiceState>,
    profile: &ProfileConfigFile,
) -> Result<usize, String> {
    claim_asset_reconcile(&state)?;
    let result = ensure_profile_assets_after_claim(Arc::clone(&state), profile).await;
    state.asset_reconcile_inflight.store(false, Ordering::Release);
    result
}

pub(super) fn claim_asset_reconcile(state: &ServiceState) -> Result<(), String> {
    if state
        .asset_reconcile_inflight
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("asset reconciliation already in progress".to_string());
    }
    Ok(())
}

pub(super) async fn ensure_profile_assets_after_claim(
    state: Arc<ServiceState>,
    profile: &ProfileConfigFile,
) -> Result<usize, String> {
    let result: Result<usize, String> = async {
        let arch = capsem_core::net::policy_config::current_profile_arch();
        let arch_assets = profile
            .assets
            .current_arch_assets()
            .ok_or_else(|| format!("profile {} has no assets for architecture {arch}", profile.id))?;
        let assets = [&arch_assets.kernel, &arch_assets.initrd, &arch_assets.rootfs];
        update_asset_reconcile_state(&state, |status| {
            *status = AssetReconcileState {
                in_progress: true,
                ..Default::default()
            };
        })?;

        let mut downloaded = 0usize;
        for asset in assets {
            let resolved = profile_asset_descriptor_path(&state.assets_dir, arch, asset).map_err(|e| e.to_string())?;
            let expected_hash = profile_asset_hash_hex(asset).map_err(|e| e.to_string())?.to_string();
            let expected_size = required_profile_asset_size(asset).map_err(|e| e.to_string())?;
            if resolved.exists() {
                match capsem_assets::asset_manager::hash_file(&resolved) {
                    Ok(hash) if hash == expected_hash => {
                        update_asset_reconcile_state(&state, |status| {
                            status.in_progress = true;
                            status.current_asset = Some(asset.name.clone());
                            status.bytes_done = expected_size;
                            status.bytes_total = Some(expected_size);
                        })?;
                        continue;
                    }
                    Ok(_) | Err(_) => {
                        let target =
                            profile_asset_download_target(&state.assets_dir, arch, asset).map_err(|e| e.to_string())?;
                        if resolved == target {
                            let _ = std::fs::remove_file(&resolved);
                        }
                    }
                }
            }

            let target = profile_asset_download_target(&state.assets_dir, arch, asset).map_err(|e| e.to_string())?;
            download_profile_asset(asset, &target, {
                let state = Arc::clone(&state);
                move |bytes_done, bytes_total, done| {
                    if let Ok(mut status) = state.asset_reconcile.lock() {
                        status.in_progress = true;
                        status.current_asset = Some(asset.name.clone());
                        status.bytes_done = bytes_done;
                        status.bytes_total = bytes_total;
                    }
                    if done {
                        let snapshot = state.asset_reconcile.lock().map(|status| status.clone()).ok();
                        if let Some(snapshot) = snapshot {
                            if let Err(error) = persist_asset_reconcile_state(&state.asset_status_path, &snapshot) {
                                warn!(error = %error, "failed to persist profile asset progress");
                            }
                        }
                    }
                }
            })
            .await
            .map_err(|e| e.to_string())?;
            downloaded += 1;
        }
        Ok(downloaded)
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
        warn!(error = %error, "failed to persist final profile asset status");
    }
    result
}

pub(super) async fn download_profile_asset<F>(
    asset: &ProfileAssetDescriptor,
    target: &StdPath,
    mut on_progress: F,
) -> Result<()>
where
    F: FnMut(u64, Option<u64>, bool),
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = target.with_file_name(format!(
        "{}.tmp",
        target.file_name().and_then(|name| name.to_str()).unwrap_or("asset")
    ));
    let _ = std::fs::remove_file(&tmp);
    let mut output = tokio::fs::File::create(&tmp)
        .await
        .with_context(|| format!("create {}", tmp.display()))?;
    let mut bytes_done = 0u64;
    let expected_hash = profile_asset_hash_hex(asset)?.to_string();
    let total = Some(required_profile_asset_size(asset)?);

    if let Some(path) = asset.url.strip_prefix("file://") {
        let mut input = tokio::fs::File::open(path)
            .await
            .with_context(|| format!("open profile asset source {path}"))?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = input
                .read(&mut buf)
                .await
                .with_context(|| format!("read profile asset source {path}"))?;
            if n == 0 {
                break;
            }
            output
                .write_all(&buf[..n])
                .await
                .with_context(|| format!("write {}", tmp.display()))?;
            bytes_done += n as u64;
            on_progress(bytes_done, total, false);
        }
    } else {
        use futures::StreamExt;
        let client = reqwest::Client::builder()
            .user_agent(concat!("capsem/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("build reqwest client")?;
        let resp = client
            .get(&asset.url)
            .send()
            .await
            .with_context(|| format!("GET {}", asset.url))?;
        if !resp.status().is_success() {
            anyhow::bail!("GET {} returned {}", asset.url, resp.status());
        }
        let total = resp.content_length().or(total);
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.with_context(|| format!("stream {}", asset.url))?;
            output
                .write_all(&chunk)
                .await
                .with_context(|| format!("write {}", tmp.display()))?;
            bytes_done += chunk.len() as u64;
            on_progress(bytes_done, total, false);
        }
    }

    output
        .flush()
        .await
        .with_context(|| format!("flush {}", tmp.display()))?;
    drop(output);

    let actual = capsem_assets::asset_manager::hash_file(&tmp)?;
    if actual != expected_hash {
        let _ = std::fs::remove_file(&tmp);
        anyhow::bail!(
            "{}: hash mismatch (expected {}, got {})",
            asset.name,
            expected_hash,
            actual
        );
    }
    std::fs::rename(&tmp, target).with_context(|| format!("rename {} -> {}", tmp.display(), target.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o444));
    }
    on_progress(bytes_done, total, true);
    Ok(())
}

/// GET /profiles/{profile_id}/assets/status -- query profile VM asset readiness.
pub(super) async fn handle_profile_assets_status(
    Path(profile_id): Path<String>,
    State(state): State<Arc<ServiceState>>,
) -> Result<axum::response::Response, AppError> {
    let profile_id = validate_profile_route_id_from_state(&state, profile_id)?;
    if !asset_reconcile_has_route_fields(&state) {
        if let Some(body) = cached_profile_status_body_for_route(&state, &profile_id)? {
            return Ok(json_bytes_response(body));
        }
    }
    let status = cached_profile_status_for_route(&state, &profile_id)?;
    Ok(Json(refresh_reconcile_fields(&state, status)).into_response())
}

/// POST /profiles/{profile_id}/assets/ensure -- download missing/corrupt
/// profile assets when a manifest is available, then return the refreshed
/// status shape.
pub(super) async fn handle_profile_assets_ensure(
    Path(profile_id): Path<String>,
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let profile_id = validate_profile_route_id(profile_id)?;
    let profile = profile_for_route(profile_id)?;
    let started = asset_background::start_profile_ensure(&state, profile.config())?;
    let cache = profile_status_cache(&state)?;
    let mut status = cache
        .profiles
        .get(profile.config().id.as_str())
        .cloned()
        .unwrap_or_else(|| profile_status_value(&state, &profile));
    if let Some(obj) = status.as_object_mut() {
        obj.insert("started".to_string(), json!(started));
        obj.insert("ensured".to_string(), json!(true));
        obj.insert("downloaded".to_string(), json!(0));
    }
    Ok(Json(refresh_reconcile_fields(&state, status)))
}

pub(super) async fn handle_profile_assets_info(
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let manifest = profile_manifest_for_route(profile_id)?;
    let current_arch = capsem_core::net::policy_config::current_profile_arch();
    let current_assets = manifest.assets.current_arch_assets();
    Ok(Json(json!({
        "profile_id": manifest.id,
        "format": manifest.assets.format,
        "refresh_policy": manifest.assets.refresh_policy,
        "current_arch": current_arch,
        "current_arch_ready": current_assets.is_some(),
        "current_assets": current_assets,
        "arch": manifest.assets.arch,
    })))
}

/// PUT /corp/edit -- apply corporate config from URL or inline TOML.
pub(super) async fn handle_corp_config(
    Json(payload): Json<CorpConfigRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    use capsem_core::net::policy_config::corp_provision;

    let capsem_dir = capsem_foundation::paths::capsem_home_opt()
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
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "provide either 'source' (URL) or 'toml' (inline content)".into(),
        ));
    }

    Ok(Json(json!({ "success": true })))
}

/// GET /corp/info -- summarize the installed corporate overlay without exposing TOML.
pub(super) async fn handle_corp_info() -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(corp_info_value()?))
}

pub(super) fn corp_info_value() -> Result<serde_json::Value, AppError> {
    use capsem_core::net::policy_config::{corp_config_paths, corp_provision};

    let capsem_dir = capsem_foundation::paths::capsem_home_opt()
        .ok_or(AppError(StatusCode::INTERNAL_SERVER_ERROR, "HOME not set".into()))?;
    let paths: Vec<_> = corp_config_paths()
        .into_iter()
        .map(|path| {
            json!({
                "path": path.display().to_string(),
                "exists": path.exists(),
            })
        })
        .collect();
    let source = corp_provision::read_corp_source(&capsem_dir);
    Ok(json!({
        "installed": paths.iter().any(|path| path["exists"].as_bool().unwrap_or(false)),
        "paths": paths,
        "source": source,
    }))
}

/// POST /corp/validate -- validate corporate config from URL or inline TOML without installing it.
pub(super) async fn handle_corp_validate(
    Json(payload): Json<CorpConfigRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    use capsem_core::net::policy_config::corp_provision;

    if let Some(source) = &payload.source {
        let client = reqwest::Client::new();
        corp_provision::fetch_corp_config(&client, source)
            .await
            .map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;
    } else if let Some(toml_content) = &payload.toml {
        corp_provision::validate_corp_toml(toml_content)
            .map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;
    } else {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "provide either 'source' (URL) or 'toml' (inline content)".into(),
        ));
    }

    Ok(Json(json!({ "success": true })))
}

/// POST /corp/reload -- refresh/re-read corp overlay and notify running VMs.
pub(super) async fn handle_corp_reload(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    use capsem_core::net::policy_config::corp_provision;

    let capsem_dir = capsem_foundation::paths::capsem_home_opt()
        .ok_or(AppError(StatusCode::INTERNAL_SERVER_ERROR, "HOME not set".into()))?;
    corp_provision::refresh_corp_config_if_stale(capsem_dir).await;
    handle_reload_config(State(state)).await
}

// ---------------------------------------------------------------------------
// MCP API Handlers
// ---------------------------------------------------------------------------

pub(super) fn load_profile_catalog_for_service() -> Result<ProfileCatalog, AppError> {
    #[cfg(test)]
    {
        if let Some(path) = test_profile_dir_override() {
            return ProfileCatalog::load_from_dir(&path).map_err(|error| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to load profile catalog: {error}"),
                )
            });
        }
        Ok(ProfileCatalog::builtin())
    }
    #[cfg(not(test))]
    {
        ProfileCatalog::load_default().map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load profile catalog: {error}"),
            )
        })
    }
}

pub(super) fn profile_catalog_source_label(source: &ProfileCatalogSource) -> String {
    match source {
        ProfileCatalogSource::BuiltIn => "built_in".to_string(),
        ProfileCatalogSource::Directory(_) => "profile".to_string(),
    }
}

pub(super) fn builtin_profile_config_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../config")
        .components()
        .collect()
}

pub(super) fn profile_from_catalog_entry(
    profile: &ProfileConfigFile,
    source: &ProfileCatalogSource,
) -> Result<Profile, AppError> {
    let (config_root, profile_dir) = match source {
        ProfileCatalogSource::BuiltIn => {
            let config_root = builtin_profile_config_root();
            let profile_dir = config_root.join("profiles").join(&profile.id);
            (config_root, profile_dir)
        }
        ProfileCatalogSource::Directory(profiles_dir) => {
            let config_root = profiles_dir.parent().ok_or_else(|| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "profile directory {} must be under a config root",
                        profiles_dir.display()
                    ),
                )
            })?;
            (config_root.to_path_buf(), profiles_dir.join(&profile.id))
        }
    };
    Profile::from_config(config_root, profile_dir, profile.clone()).map_err(|error| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("invalid profile {}: {error}", profile.id),
        )
    })
}

pub(super) fn profile_for_route(profile_id: String) -> Result<Profile, AppError> {
    let profile_id = validate_profile_route_id(profile_id)?;
    let catalog = load_profile_catalog_for_service()?;
    let profile = catalog
        .get(&profile_id)
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("profile not found: {profile_id}")))?;
    profile_from_catalog_entry(profile, catalog.source())
}

#[cfg(test)]
pub(super) fn profile_catalog_status_value(state: &ServiceState, catalog: &ProfileCatalog) -> serde_json::Value {
    build_profile_status_cache(state, catalog, profile_status_inputs(state)).catalog
}

pub(super) fn build_profile_status_cache(
    state: &ServiceState,
    catalog: &ProfileCatalog,
    inputs: ProfileStatusInputs,
) -> ProfileStatusCache {
    let mut profile_statuses = BTreeMap::new();
    let mut profile_bodies = BTreeMap::new();
    let profiles = catalog
        .profiles()
        .map(|profile| {
            let status = profile_from_catalog_entry(profile, catalog.source())
                .map(|profile| profile_status_value(state, &profile))
                .unwrap_or_else(|error| {
                    json!({
                        "ready": false,
                        "current_arch": capsem_core::net::policy_config::current_profile_arch(),
                        "assets": [],
                        "missing_assets": [],
                        "invalid_assets": [],
                        "invalid_files": [],
                        "errors": [error.1],
                    })
                });
            profile_statuses.insert(profile.id.clone(), status.clone());
            profile_bodies.insert(
                profile.id.clone(),
                Bytes::from(serde_json::to_vec(&status).unwrap_or_default()),
            );
            let missing = status["missing_assets"].clone();
            json!({
                "id": profile.id,
                "name": profile.name,
                "description": profile.description,
                "revision": profile.revision,
                "profile_payload_hash": profile_payload_hash(profile).ok(),
                "update_semantics": profile_update_semantics_value(),
                "ready": status["ready"].as_bool().unwrap_or(false),
                "current_arch": status["current_arch"].clone(),
                "missing_assets": missing,
                "invalid_assets": status["invalid_assets"].clone(),
                "invalid_files": status["invalid_files"].clone(),
                "errors": status["errors"].clone(),
                "asset_count": status["assets"].as_array().map_or(0, Vec::len),
            })
        })
        .collect::<Vec<_>>();
    let ready_count = profiles
        .iter()
        .filter(|profile| profile["ready"].as_bool().unwrap_or(false))
        .count();
    let catalog_status = json!({
        "source": profile_catalog_source_label(catalog.source()),
        "asset_manifest": asset_manifest_status_value(state),
        "profile_count": profiles.len(),
        "ready_count": ready_count,
        "profiles": profiles,
    });
    let catalog_body = Bytes::from(serde_json::to_vec(&catalog_status).unwrap_or_default());
    ProfileStatusCache {
        inputs,
        catalog: catalog_status,
        catalog_body,
        profiles: profile_statuses,
        profile_bodies,
    }
}

pub(super) fn asset_reconcile_has_route_fields(state: &ServiceState) -> bool {
    state.asset_reconcile_inflight.load(Ordering::Acquire)
        || state
            .asset_reconcile
            .lock()
            .map(|reconcile| reconcile.in_progress)
            .unwrap_or(true)
}

pub(super) fn json_bytes_response(body: Bytes) -> axum::response::Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

pub(super) fn refresh_reconcile_fields(state: &ServiceState, mut value: serde_json::Value) -> serde_json::Value {
    let reconcile = state.asset_reconcile.lock().map(|s| s.clone()).unwrap_or_default();
    if let Some(obj) = value.as_object_mut() {
        if obj.contains_key("downloading") {
            let active = reconcile.in_progress || state.asset_reconcile_inflight.load(Ordering::Acquire);
            obj.insert("downloading".to_string(), json!(active));
        }
    }
    append_asset_reconcile_status(&mut value, &reconcile);
    value
}

pub(super) fn cached_profile_status_for_route(
    state: &ServiceState,
    profile_id: &str,
) -> Result<serde_json::Value, AppError> {
    let cached = state
        .profile_status_cache
        .lock()
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("profile status cache lock poisoned: {error}"),
            )
        })?
        .as_ref()
        .and_then(|cache| cache.profiles.get(profile_id).cloned());
    if let Some(status) = cached {
        return Ok(status);
    }

    let cache = rebuild_profile_status_cache(state)?;
    cache
        .profiles
        .get(profile_id)
        .cloned()
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("profile not found: {profile_id}")))
}

pub(super) fn cached_profile_status_body_for_route(
    state: &ServiceState,
    profile_id: &str,
) -> Result<Option<Bytes>, AppError> {
    let cache = profile_status_cache(state)?;
    if let Some(body) = cache.profile_bodies.get(profile_id).cloned() {
        return Ok(Some(body));
    }
    if cache.profiles.contains_key(profile_id) {
        return Ok(None);
    }
    Err(AppError(
        StatusCode::NOT_FOUND,
        format!("profile not found: {profile_id}"),
    ))
}

pub(super) fn validate_profile_route_id(profile_id: String) -> Result<String, AppError> {
    if profile_id.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "profile id must not be empty".to_string(),
        ));
    }
    let catalog = load_profile_catalog_for_service()?;
    if catalog.get(&profile_id).is_none() {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            format!("profile not found: {profile_id}"),
        ));
    }
    Ok(profile_id)
}

pub(super) fn build_profile_summary(
    manifest: &ProfileConfigFile,
    source: &ProfileCatalogSource,
    _user: &SettingsFile,
    corp: &SettingsFile,
    plugin_count: usize,
) -> Result<api::ProfileSummary, AppError> {
    let profile = profile_from_catalog_entry(manifest, source)?;
    let mut rules = Vec::new();
    append_compiled_rules(
        &mut rules,
        SecurityRuleSource::BuiltinDefault,
        ProviderRuleProfile::builtin_security_defaults(),
    )?;
    append_compiled_rules(
        &mut rules,
        SecurityRuleSource::User,
        profile
            .config()
            .security_rule_profile_from_files(profile.config_root())
            .map_err(|error| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    format!("invalid profile rule files for {}: {error}", manifest.id),
                )
            })?,
    )?;
    append_compiled_rules(
        &mut rules,
        SecurityRuleSource::Corp,
        SecurityRuleProfile {
            corp: corp.corp.clone(),
            profiles: corp.profiles.clone(),
            ai: corp.ai.clone(),
            ..SecurityRuleProfile::default()
        },
    )?;
    let default_rule_count = rules.iter().filter(|rule| rule.default_rule).count();
    let profile_rule_count = rules.len();
    let mcp_server_count = manifest.mcp.as_ref().map_or(0, |mcp| {
        mcp.servers.len() + usize::from(mcp.server_enabled.get("local").copied().unwrap_or(false))
    });

    Ok(api::ProfileSummary {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        icon_svg: manifest.icon_svg.clone(),
        availability: api::ProfileAvailabilitySummary {
            web: manifest.availability.web,
            shell: manifest.availability.shell,
            mobile: manifest.availability.mobile,
        },
        source: profile_catalog_source_label(source),
        rule_count: profile_rule_count,
        default_rule_count,
        plugin_count,
        mcp_server_count,
        update_semantics: profile_update_semantics(),
    })
}

pub(super) fn build_profile_summary_cache() -> Result<Vec<api::ProfileSummary>, AppError> {
    let catalog = load_profile_catalog_for_service()?;
    let (user, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
    catalog
        .profiles()
        .map(|profile| build_profile_summary(profile, catalog.source(), &user, &corp, 0))
        .collect::<Result<Vec<_>, AppError>>()
}

pub(super) fn build_profile_cache() -> Result<BTreeMap<String, Profile>, AppError> {
    let catalog = load_profile_catalog_for_service()?;
    let mut profiles = BTreeMap::new();
    for manifest in catalog.profiles() {
        profiles.insert(
            manifest.id.clone(),
            profile_from_catalog_entry(manifest, catalog.source())?,
        );
    }
    Ok(profiles)
}

pub(super) fn cached_profile_for_route(state: &ServiceState, profile_id: String) -> Result<Profile, AppError> {
    if profile_id.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "profile id must not be empty".to_string(),
        ));
    }
    state
        .profile_cache
        .lock()
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("profile cache lock poisoned: {error}"),
            )
        })?
        .get(&profile_id)
        .cloned()
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("profile not found: {profile_id}")))
}

pub(super) fn refresh_profile_route_caches(state: &ServiceState) -> Result<(), AppError> {
    let profile_summary_cache = build_profile_summary_cache()?;
    let profile_cache = build_profile_cache()?;
    let profile_rule_cache = build_profile_rule_cache(None)?;
    let profile_mcp_default_cache = build_profile_mcp_default_cache(None)?;
    let profile_plugin_policy_cache = build_profile_plugin_policy_cache(None)?;
    let status_cache = build_stable_profile_status_cache(state)?;

    *state.profile_summary_cache.lock().map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("profile summary cache lock poisoned: {error}"),
        )
    })? = profile_summary_cache;
    *state.profile_cache.lock().map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("profile cache lock poisoned: {error}"),
        )
    })? = profile_cache;
    *state.profile_rule_cache.lock().map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("profile rule cache lock poisoned: {error}"),
        )
    })? = profile_rule_cache;
    *state.profile_mcp_default_cache.lock().map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("profile MCP default cache lock poisoned: {error}"),
        )
    })? = profile_mcp_default_cache;
    *state.profile_plugin_policy_cache.lock().map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("profile plugin cache lock poisoned: {error}"),
        )
    })? = profile_plugin_policy_cache;
    *state.profile_status_cache.lock().map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("profile status cache lock poisoned: {error}"),
        )
    })? = Some(status_cache);
    state.persistent_resume_state_cache.lock().unwrap().clear();
    state.profile_rule_response_cache.lock().unwrap().clear();
    state.profile_plugin_response_cache.lock().unwrap().clear();
    state.evaluate_response_cache.lock().unwrap().clear();
    *state.evaluate_last_response_cache.lock().unwrap() = None;
    Ok(())
}

pub(super) fn build_profile_rule_cache(
    profile_filter: Option<&str>,
) -> Result<BTreeMap<String, Vec<api::EnforcementRuleInfo>>, AppError> {
    let catalog = load_profile_catalog_for_service()?;
    let (_, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
    let mut output = BTreeMap::new();
    for manifest in catalog.profiles() {
        if profile_filter
            .map(|profile_id| profile_id != manifest.id)
            .unwrap_or(false)
        {
            continue;
        }
        let profile = profile_from_catalog_entry(manifest, catalog.source())?;
        output.insert(
            manifest.id.clone(),
            list_enforcement_rules_for_profile_config(&profile, &corp)?,
        );
    }
    Ok(output)
}

pub(super) fn build_profile_mcp_default_cache(
    profile_filter: Option<&str>,
) -> Result<BTreeMap<String, Result<api::McpDefaultPermissionResponse, String>>, AppError> {
    let catalog = load_profile_catalog_for_service()?;
    let mut output = BTreeMap::new();
    for manifest in catalog.profiles() {
        if profile_filter
            .map(|profile_id| profile_id != manifest.id)
            .unwrap_or(false)
        {
            continue;
        }
        let profile = match profile_from_catalog_entry(manifest, catalog.source()) {
            Ok(profile) => profile,
            Err(AppError(_, error)) => {
                output.insert(manifest.id.clone(), Err(error));
                continue;
            }
        };
        let permission = match profile.mcp_default_permission() {
            Ok(permission) => permission,
            Err(error) => {
                output.insert(
                    manifest.id.clone(),
                    Err(format!(
                        "resolve MCP default permission for profile {}: {error}",
                        manifest.id
                    )),
                );
                continue;
            }
        };
        output.insert(
            manifest.id.clone(),
            Ok(api::McpDefaultPermissionResponse {
                action: permission.action,
                source: permission.source,
                rule_id: permission.rule_id,
            }),
        );
    }
    Ok(output)
}

pub(super) fn build_profile_plugin_policy_cache(
    profile_filter: Option<&str>,
) -> Result<BTreeMap<String, BTreeMap<String, SecurityPluginConfig>>, AppError> {
    let catalog = load_profile_catalog_for_service()?;
    let mut output = BTreeMap::new();
    for manifest in catalog.profiles() {
        if profile_filter
            .map(|profile_id| profile_id != manifest.id)
            .unwrap_or(false)
        {
            continue;
        }
        output.insert(manifest.id.clone(), manifest.plugins.clone());
    }
    Ok(output)
}

pub(super) fn profile_summary_with_live_plugin_count(
    state: &ServiceState,
    summary: &api::ProfileSummary,
) -> api::ProfileSummary {
    let mut summary = summary.clone();
    summary.plugin_count = effective_plugin_policy(state, &summary.id).len();
    summary
}

pub(super) async fn handle_profiles_list(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<api::ProfilesListResponse>, AppError> {
    let profiles = state
        .profile_summary_cache
        .lock()
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("profile summary cache lock poisoned: {error}"),
            )
        })?
        .iter()
        .map(|summary| profile_summary_with_live_plugin_count(&state, summary))
        .collect();
    Ok(Json(api::ProfilesListResponse { profiles }))
}

pub(super) async fn handle_profiles_status(
    State(state): State<Arc<ServiceState>>,
) -> Result<axum::response::Response, AppError> {
    if !asset_reconcile_has_route_fields(&state) {
        return Ok(json_bytes_response(profile_status_catalog_body(&state)?));
    }
    let cache = profile_status_cache(&state)?;
    let value = refresh_reconcile_fields(&state, cache.catalog.clone());
    let body = serde_json::to_vec(&value).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize profiles status response: {error}"),
        )
    })?;
    Ok(json_bytes_response(Bytes::from(body)))
}

pub(super) async fn handle_profiles_reload(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let cache = rebuild_profile_status_cache(&state)?;
    Ok(Json(json!({
        "reloaded": true,
        "catalog": refresh_reconcile_fields(&state, cache.catalog.clone()),
    })))
}

pub(super) async fn handle_profile_info(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<api::ProfileInfoResponse>, AppError> {
    let catalog = load_profile_catalog_for_service()?;
    let manifest = catalog
        .get(&profile_id)
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("profile not found: {profile_id}")))?;
    let summary = state
        .profile_summary_cache
        .lock()
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("profile summary cache lock poisoned: {error}"),
            )
        })?
        .iter()
        .find(|summary| summary.id == manifest.id)
        .map(|summary| profile_summary_with_live_plugin_count(&state, summary))
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("profile not found: {profile_id}")))?;
    Ok(Json(api::ProfileInfoResponse {
        profile: summary,
        obom: profile_obom_info(manifest),
    }))
}

pub(super) fn profile_obom_info(profile: &ProfileConfigFile) -> Option<api::ProfileObomInfo> {
    let obom = profile.obom.as_ref()?;
    let current_arch = capsem_core::net::policy_config::current_profile_arch().to_string();
    let descriptor = obom.current_arch_obom()?;
    let rootfs_hash = profile
        .assets
        .current_arch_assets()
        .and_then(|assets| assets.rootfs.hash.clone())?;
    Some(api::ProfileObomInfo {
        profile_id: profile.id.clone(),
        current_arch,
        scope: "base_image".to_string(),
        format: obom.format.clone(),
        name: descriptor.name.clone(),
        url: descriptor.url.clone(),
        hash: descriptor.hash.clone(),
        size: descriptor.size,
        generator: descriptor.generator.clone(),
        generator_version: descriptor.generator_version.clone(),
        rootfs_hash,
        route: format!("/profiles/{}/obom", profile.id),
    })
}

pub(super) async fn handle_profile_obom(
    Path(profile_id): Path<String>,
) -> Result<Json<api::ProfileObomResponse>, AppError> {
    let profile = profile_manifest_for_route(profile_id)?;
    let obom = profile_obom_info(&profile).ok_or_else(|| {
        AppError(
            StatusCode::NOT_FOUND,
            format!("profile {} has no OBOM for current architecture", profile.id),
        )
    })?;
    let document = if let Some(path) = obom.url.strip_prefix("file://") {
        Some(read_local_profile_obom(StdPath::new(path), &obom)?)
    } else {
        None
    };
    Ok(Json(api::ProfileObomResponse {
        profile_id: profile.id,
        current_arch: obom.current_arch.clone(),
        obom,
        document,
    }))
}

pub(super) fn read_local_profile_obom(
    path: &StdPath,
    info: &api::ProfileObomInfo,
) -> Result<serde_json::Value, AppError> {
    let bytes = std::fs::read(path).map_err(|error| {
        AppError(
            StatusCode::NOT_FOUND,
            format!("read profile OBOM {}: {error}", path.display()),
        )
    })?;
    if bytes.len() as u64 != info.size {
        return Err(AppError(
            StatusCode::PRECONDITION_FAILED,
            format!(
                "profile OBOM size mismatch for {}: expected {}, got {}",
                path.display(),
                info.size,
                bytes.len()
            ),
        ));
    }
    let actual_hash = blake3::hash(&bytes).to_hex().to_string();
    let expected_hash = info.hash.strip_prefix("blake3:").ok_or_else(|| {
        AppError(
            StatusCode::PRECONDITION_FAILED,
            format!("profile OBOM hash must use blake3:<hex>, got {}", info.hash),
        )
    })?;
    if actual_hash != expected_hash {
        return Err(AppError(
            StatusCode::PRECONDITION_FAILED,
            format!(
                "profile OBOM hash mismatch for {}: expected {}, got {}",
                path.display(),
                expected_hash,
                actual_hash
            ),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        AppError(
            StatusCode::PRECONDITION_FAILED,
            format!("parse profile OBOM {}: {error}", path.display()),
        )
    })
}

pub(super) fn profile_manifest_for_route(profile_id: String) -> Result<ProfileConfigFile, AppError> {
    let profile_id = validate_profile_route_id(profile_id)?;
    let catalog = load_profile_catalog_for_service()?;
    catalog
        .get(&profile_id)
        .cloned()
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("profile not found: {profile_id}")))
}

pub(super) async fn handle_profile_validate(
    Path(profile_id): Path<String>,
    Json(request): Json<api::ProfileValidateRequest>,
) -> Result<Json<api::ProfileValidateResponse>, AppError> {
    let route_profile_id = validate_profile_route_id(profile_id)?;
    let profile = if let Some(toml) = request.toml {
        toml::from_str::<ProfileConfigFile>(&toml)
            .map_err(|error| AppError(StatusCode::BAD_REQUEST, format!("invalid profile TOML: {error}")))?
    } else if let Some(profile) = request.profile {
        profile
    } else {
        profile_manifest_for_route(route_profile_id.clone())?
    };
    profile
        .validate()
        .map_err(|error| AppError(StatusCode::BAD_REQUEST, format!("invalid profile: {error}")))?;
    if profile.id != route_profile_id {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            format!(
                "profile id mismatch: route has {route_profile_id}, payload has {}",
                profile.id
            ),
        ));
    }
    Ok(Json(api::ProfileValidateResponse {
        valid: true,
        profile_id: profile.id,
    }))
}

pub(super) async fn handle_profile_skills_info(
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let manifest = profile_manifest_for_route(profile_id)?;
    Ok(Json(json!({
        "profile_id": manifest.id,
        "skill_count": manifest.skills.paths.len(),
        "paths": manifest.skills.paths,
    })))
}

pub(super) async fn handle_profile_skills_list(
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let manifest = profile_manifest_for_route(profile_id)?;
    Ok(Json(json!({
        "profile_id": manifest.id,
        "skills": manifest.skills.paths.into_iter().map(|path| {
            let id = skill_id_for_path(&path).unwrap_or_else(|_| path.clone());
            json!({ "id": id, "path": path })
        }).collect::<Vec<_>>(),
    })))
}

pub(super) async fn handle_profile_skill_add(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
    Json(request): Json<ProfileSkillAddRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    log_profile_mutation_route_request("profile_skill_add", &profile_id, "skill", &request.path, "add");
    let mut profile = profile_for_route(profile_id.clone()).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "profile_skill_add",
            &profile_id,
            "skill",
            &request.path,
            "add",
            &error.1,
        );
    })?;
    let summary = profile.add_skill_path(&request.path, "service-api").map_err(|error| {
        log_profile_mutation_route_rejected("profile_skill_add", &profile_id, "skill", &request.path, "add", &error);
        AppError(StatusCode::BAD_REQUEST, error)
    })?;
    let event = write_profile_mutation_event(&state, summary, &profile).await?;
    log_profile_mutation_applied("profile_skill_add", &event);
    Ok(Json(json!({
        "profile_id": event.profile_id,
        "skill_id": event.target_key,
        "path": request.path,
        "mutation": event,
    })))
}

pub(super) async fn handle_profile_skill_edit(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, _skill_id)): Path<(String, String)>,
    Json(request): Json<ProfileSkillEditRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    log_profile_mutation_route_request("profile_skill_edit", &profile_id, "skill", &_skill_id, "edit");
    let mut profile = profile_for_route(profile_id.clone()).inspect_err(|error| {
        log_profile_mutation_route_rejected("profile_skill_edit", &profile_id, "skill", &_skill_id, "edit", &error.1);
    })?;
    let summary = profile
        .edit_skill_path(&_skill_id, &request.path, "service-api")
        .map_err(|error| {
            log_profile_mutation_route_rejected("profile_skill_edit", &profile_id, "skill", &_skill_id, "edit", &error);
            AppError(StatusCode::BAD_REQUEST, error)
        })?;
    let event = write_profile_mutation_event(&state, summary, &profile).await?;
    log_profile_mutation_applied("profile_skill_edit", &event);
    Ok(Json(json!({
        "profile_id": event.profile_id,
        "skill_id": event.target_key,
        "path": request.path,
        "mutation": event,
    })))
}

pub(super) async fn handle_profile_skill_delete(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, _skill_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    log_profile_mutation_route_request("profile_skill_delete", &profile_id, "skill", &_skill_id, "delete");
    let mut profile = profile_for_route(profile_id.clone()).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "profile_skill_delete",
            &profile_id,
            "skill",
            &_skill_id,
            "delete",
            &error.1,
        );
    })?;
    let summary = profile.delete_skill(&_skill_id, "service-api").map_err(|error| {
        log_profile_mutation_route_rejected(
            "profile_skill_delete",
            &profile_id,
            "skill",
            &_skill_id,
            "delete",
            &error,
        );
        AppError(StatusCode::BAD_REQUEST, error)
    })?;
    let event = write_profile_mutation_event(&state, summary, &profile).await?;
    log_profile_mutation_applied("profile_skill_delete", &event);
    Ok(Json(json!({
        "profile_id": event.profile_id,
        "skill_id": event.target_key,
        "mutation": event,
    })))
}

pub(super) fn resolve_mcp_tool_id(server_id: &str, tool_id: &str) -> Result<String, AppError> {
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
pub(super) async fn handle_profile_mcp_info(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let profile = cached_profile_for_route(&state, profile_id)?;
    let profile = profile.config();
    let mcp = profile.mcp.as_ref();
    let builtin_local_enabled = mcp
        .and_then(|mcp| mcp.server_enabled.get("local").copied())
        .unwrap_or(false);
    let manual_server_count = mcp.map_or(0, |mcp| mcp.servers.len());
    Ok(Json(json!({
        "profile_id": profile.id,
        "server_count": manual_server_count + usize::from(builtin_local_enabled),
        "manual_server_count": manual_server_count,
        "builtin_local_enabled": builtin_local_enabled,
    })))
}

pub(super) fn profile_mcp_server_configured(profile: &ProfileConfigFile, server_id: &str) -> bool {
    let Some(mcp) = profile.mcp.as_ref() else {
        return false;
    };
    if server_id == "local" {
        return mcp.server_enabled.get("local").copied().unwrap_or(false);
    }
    mcp.servers.iter().any(|server| server.name == server_id)
}

pub(super) fn ensure_profile_mcp_server(profile_id: String, server_id: &str) -> Result<ProfileConfigFile, AppError> {
    let profile = profile_manifest_for_route(profile_id)?;
    if profile_mcp_server_configured(&profile, server_id) {
        Ok(profile)
    } else {
        Err(AppError(
            StatusCode::NOT_FOUND,
            format!("MCP server not found in profile {}: {server_id}", profile.id),
        ))
    }
}

pub(super) fn validate_mcp_server_id(server_id: &str) -> Result<(), AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "MCP server id must not be empty".to_string(),
        ));
    }
    if server_id.contains(capsem_proto::mcp_contracts::NS_SEP) {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            format!(
                "MCP server id must not contain namespace separator {}",
                capsem_proto::mcp_contracts::NS_SEP
            ),
        ));
    }
    Ok(())
}

pub(super) fn validate_mcp_server_edit_request(
    server_id: &str,
    update: McpServerEditRequest,
) -> Result<McpManualServer, AppError> {
    validate_mcp_server_id(server_id)?;
    let url = update
        .url
        .ok_or_else(|| AppError(StatusCode::BAD_REQUEST, "MCP server URL is required".to_string()))?;
    if url.trim().is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "MCP server URL must not be empty".to_string(),
        ));
    }
    let server = McpManualServer {
        name: server_id.to_string(),
        url,
        headers: update.headers,
        auth: None,
        enabled: update.enabled.unwrap_or(true),
    };
    McpProfileConfig {
        servers: vec![server.clone()],
        ..McpProfileConfig::default()
    }
    .validate("profile")
    .map_err(|error| AppError(StatusCode::BAD_REQUEST, error))?;
    Ok(server)
}

pub(super) fn unix_timestamp_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

pub(super) async fn write_profile_mutation_event(
    state: &ServiceState,
    summary: capsem_core::net::policy_config::ProfileMutationSummary,
    profile: &Profile,
) -> Result<capsem_logger::ProfileMutationEvent, AppError> {
    let mutation_id = capsem_core::security_engine::SecurityEventId::new_uuid4()
        .as_str()
        .to_string();
    let event = summary.into_logger_event(
        unix_timestamp_ms(),
        mutation_id,
        capsem_logger::ProfileMutationStatus::Applied,
        None,
        None,
    );
    state
        .profile_mutation_db
        .write(capsem_logger::WriteOp::ProfileMutationEvent(event.clone()))
        .await
        .map_err(|error| {
            error!(
                target: "capsem.profile_mutation",
                mutation_id = %event.mutation_id,
                profile_id = %event.profile_id,
                operation = "profile_mutation_ledger_write",
                error = %error,
                "profile mutation ledger write failed"
            );
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("profile mutation ledger write failed: {error}"),
            )
        })?;
    profile_mutation_cache::refresh_after_profile_mutation(state, profile, &event)?;
    log_profile_mutation_applied("profile_mutation_ledger", &event);
    Ok(event)
}

pub(super) fn profile_mutation_log_fields(
    route: &'static str,
    event: &capsem_logger::ProfileMutationEvent,
) -> serde_json::Value {
    json!({
        "route": route,
        "mutation_id": event.mutation_id,
        "profile_id": event.profile_id,
        "actor": event.actor,
        "category": event.category,
        "filename": event.filename,
        "affected_path": event.affected_path,
        "target_kind": event.target_kind,
        "target_key": event.target_key,
        "operation": event.operation,
        "rule_id": event.rule_id.as_deref().unwrap_or(""),
        "old_hash": event.old_hash,
        "old_size": event.old_size,
        "new_hash": event.new_hash,
        "new_size": event.new_size,
        "status": event.status.as_str(),
        "error": event.error.as_deref().unwrap_or(""),
        "trace_id": event.trace_id.as_deref().unwrap_or(""),
    })
}

pub(super) fn log_profile_mutation_applied(route: &'static str, event: &capsem_logger::ProfileMutationEvent) {
    info!(
        target: "capsem.profile_mutation",
        route,
        mutation_id = %event.mutation_id,
        profile_id = %event.profile_id,
        actor = %event.actor,
        category = %event.category,
        filename = %event.filename,
        affected_path = %event.affected_path,
        target_kind = %event.target_kind,
        target_key = %event.target_key,
        operation = %event.operation,
        rule_id = event.rule_id.as_deref().unwrap_or(""),
        old_hash = %event.old_hash,
        old_size = event.old_size,
        new_hash = %event.new_hash,
        new_size = event.new_size,
        status = %event.status.as_str(),
        trace_id = event.trace_id.as_deref().unwrap_or(""),
        fields = %profile_mutation_log_fields(route, event),
        "profile mutation applied"
    );
}

pub(super) fn log_profile_mutation_route_request(
    route: &'static str,
    profile_id: &str,
    target_kind: &'static str,
    target_key: &str,
    operation: &'static str,
) {
    info!(
        target: "capsem.profile_mutation",
        route,
        profile_id,
        target_kind,
        target_key,
        operation,
        actor = "service-api",
        "profile mutation route requested"
    );
}

pub(super) fn log_profile_mutation_route_rejected(
    route: &'static str,
    profile_id: &str,
    target_kind: &'static str,
    target_key: &str,
    operation: &'static str,
    error: &str,
) {
    warn!(
        target: "capsem.profile_mutation",
        route,
        profile_id,
        target_kind,
        target_key,
        operation,
        actor = "service-api",
        error,
        "profile mutation route rejected"
    );
}

/// PUT /profiles/:profile_id/mcp/servers/:server_id/edit -- add or replace one MCP server.
pub(super) async fn handle_profile_mcp_server_edit(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, server_id)): Path<(String, String)>,
    Json(update): Json<McpServerEditRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    log_profile_mutation_route_request(
        "profile_mcp_server_edit",
        &profile_id,
        "mcp_server",
        &server_id,
        "upsert",
    );
    let server = validate_mcp_server_edit_request(&server_id, update).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "profile_mcp_server_edit",
            &profile_id,
            "mcp_server",
            &server_id,
            "upsert",
            &error.1,
        );
    })?;
    let mut profile = profile_for_route(profile_id.clone()).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "profile_mcp_server_edit",
            &profile_id,
            "mcp_server",
            &server_id,
            "upsert",
            &error.1,
        );
    })?;
    let summary = profile
        .upsert_mcp_server(server.clone(), "service-api")
        .map_err(|error| {
            log_profile_mutation_route_rejected(
                "profile_mcp_server_edit",
                &profile_id,
                "mcp_server",
                &server_id,
                "upsert",
                &error,
            );
            AppError(StatusCode::BAD_REQUEST, error)
        })?;
    let event = write_profile_mutation_event(&state, summary, &profile).await?;
    log_profile_mutation_applied("profile_mcp_server_edit", &event);
    Ok(Json(json!({
        "profile_id": event.profile_id,
        "server_id": server_id,
        "url": server.url,
        "enabled": server.enabled,
        "mutation": event,
    })))
}

/// DELETE /profiles/:profile_id/mcp/servers/:server_id/delete -- remove one MCP server.
pub(super) async fn handle_profile_mcp_server_delete(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, server_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    log_profile_mutation_route_request(
        "profile_mcp_server_delete",
        &profile_id,
        "mcp_server",
        &server_id,
        "delete",
    );
    validate_mcp_server_id(&server_id).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "profile_mcp_server_delete",
            &profile_id,
            "mcp_server",
            &server_id,
            "delete",
            &error.1,
        );
    })?;
    let mut profile = profile_for_route(profile_id.clone()).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "profile_mcp_server_delete",
            &profile_id,
            "mcp_server",
            &server_id,
            "delete",
            &error.1,
        );
    })?;
    let summary = profile.delete_mcp_server(&server_id, "service-api").map_err(|error| {
        log_profile_mutation_route_rejected(
            "profile_mcp_server_delete",
            &profile_id,
            "mcp_server",
            &server_id,
            "delete",
            &error,
        );
        AppError(StatusCode::BAD_REQUEST, error)
    })?;
    let event = write_profile_mutation_event(&state, summary, &profile).await?;
    log_profile_mutation_applied("profile_mcp_server_delete", &event);
    Ok(Json(json!({
        "profile_id": event.profile_id,
        "server_id": server_id,
        "mutation": event,
    })))
}

pub(super) async fn handle_profile_mcp_servers(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let profile = cached_profile_for_route(&state, profile_id)?;
    use capsem_core::mcp::build_profile_server_list;

    let profile_mcp = profile.config().mcp.clone().unwrap_or_default();

    // Include the "local" builtin server if the binary exists.
    let builtin_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("capsem-mcp-builtin")));
    let servers = build_profile_server_list(&profile_mcp, builtin_bin.as_deref(), std::collections::HashMap::new());
    let cache = latest_mcp_tool_cache(&state);

    let resp: Vec<api::McpServerInfoResponse> = servers
        .iter()
        .map(|s| {
            let tool_count = cache.iter().filter(|t| t.server_name == s.name).count();
            api::McpServerInfoResponse {
                name: s.name.clone(),
                url: s.url.clone(),
                has_auth_credential: s.auth.is_some(),
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

/// GET /profiles/:profile_id/mcp/default/info -- read the profile MCP default permission.
pub(super) async fn handle_profile_mcp_default_info(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
) -> Result<Json<api::McpDefaultPermissionResponse>, AppError> {
    if profile_id.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "profile id must not be empty".to_string(),
        ));
    }
    let permission = state
        .profile_mcp_default_cache
        .lock()
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("profile MCP default cache lock poisoned: {error}"),
            )
        })?
        .get(&profile_id)
        .cloned()
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("profile not found: {profile_id}")))?
        .map_err(|error| AppError(StatusCode::BAD_REQUEST, error))?;
    Ok(Json(permission))
}

pub(super) fn latest_mcp_tool_cache(state: &ServiceState) -> Vec<ToolCacheEntry> {
    let latest = capsem_core::mcp::load_tool_cache();
    let Ok(mut cache) = state.mcp_tool_cache.lock() else {
        return latest;
    };
    if !latest.is_empty() || cache.is_empty() {
        *cache = latest;
    }
    cache.clone()
}

/// GET /profiles/:profile_id/mcp/servers/:server_id/tools/list -- list one server's tools.
pub(super) async fn handle_profile_mcp_server_tools(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, server_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    if server_id.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "MCP server id must not be empty".to_string(),
        ));
    }
    let profile = cached_profile_for_route(&state, profile_id)?;
    if !profile_mcp_server_configured(profile.config(), &server_id) {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            format!("MCP server not found in profile {}: {server_id}", profile.config().id),
        ));
    }

    let cache = latest_mcp_tool_cache(&state);
    let resp: Result<Vec<api::McpToolInfoResponse>, AppError> = cache
        .iter()
        .filter(|entry| entry.server_name == server_id)
        .map(|entry| {
            let permission = profile
                .mcp_tool_permission(&server_id, &entry.original_name)
                .map_err(|error| {
                    AppError(
                        StatusCode::BAD_REQUEST,
                        format!(
                            "resolve MCP tool permission {}/{}: {error}",
                            server_id, entry.original_name
                        ),
                    )
                })?;
            Ok(api::McpToolInfoResponse {
                namespaced_name: entry.namespaced_name.clone(),
                original_name: entry.original_name.clone(),
                description: entry.description.clone(),
                server_name: entry.server_name.clone(),
                annotations: entry.annotations.as_ref().map(|a| a.to_mcp_json()),
                pin_hash: Some(entry.pin_hash.clone()),
                pin_changed: false, // Would need live catalog comparison.
                permission_action: permission.action,
                permission_source: permission.source,
            })
        })
        .collect();
    Ok(Json(serde_json::to_value(resp?).unwrap_or_default()))
}

/// POST /profiles/:profile_id/mcp/servers/:server_id/refresh -- refresh one server's tool discovery.
pub(super) async fn handle_profile_mcp_server_refresh(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, server_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    if server_id.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "MCP server id must not be empty".to_string(),
        ));
    }
    ensure_profile_mcp_server(profile_id, &server_id)?;
    // Send McpRefreshTools to all running instances.
    let uds_paths = {
        let instances = state.instances.lock().unwrap();
        instances.values().map(|info| info.uds_path.clone()).collect::<Vec<_>>()
    };
    for uds_path in &uds_paths {
        let id = state.next_job_id();
        let _ = send_ipc_command(uds_path, ServiceToProcess::McpRefreshTools { id }, Some(30)).await;
    }
    if let Ok(mut cache) = state.mcp_tool_cache.lock() {
        *cache = capsem_core::mcp::load_tool_cache();
    }
    Ok(Json(
        serde_json::json!({"success": true, "server_id": server_id, "instances": uds_paths.len()}),
    ))
}

/// PATCH /profiles/:profile_id/mcp/default/edit -- edit the default MCP permission rule.
pub(super) async fn handle_profile_mcp_default_edit(
    State(state): State<Arc<ServiceState>>,
    Path(profile_id): Path<String>,
    Json(update): Json<McpToolEditRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    log_profile_mutation_route_request(
        "profile_mcp_default_edit",
        &profile_id,
        "mcp_default",
        "default.mcp",
        "permission",
    );
    let mut profile = profile_for_route(profile_id.clone()).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "profile_mcp_default_edit",
            &profile_id,
            "mcp_default",
            "default.mcp",
            "permission",
            &error.1,
        );
    })?;
    let summary = profile
        .set_mcp_default_permission(update.action, "service-api")
        .map_err(|error| {
            log_profile_mutation_route_rejected(
                "profile_mcp_default_edit",
                &profile_id,
                "mcp_default",
                "default.mcp",
                "permission",
                &error,
            );
            AppError(StatusCode::BAD_REQUEST, error)
        })?;
    let event = write_profile_mutation_event(&state, summary, &profile).await?;
    log_profile_mutation_applied("profile_mcp_default_edit", &event);
    Ok(Json(json!({
        "profile_id": event.profile_id,
        "action": update.action,
        "mutation": event,
    })))
}

/// PATCH /profiles/:profile_id/mcp/servers/:server_id/tools/:tool_id/edit -- edit tool mechanics.
pub(super) async fn handle_profile_mcp_tool_edit(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, server_id, tool_id)): Path<(String, String, String)>,
    Json(update): Json<McpToolEditRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let target_key = format!("{server_id}/{tool_id}");
    log_profile_mutation_route_request(
        "profile_mcp_tool_edit",
        &profile_id,
        "mcp_tool",
        &target_key,
        "permission",
    );
    let mut profile = profile_for_route(profile_id.clone()).inspect_err(|error| {
        log_profile_mutation_route_rejected(
            "profile_mcp_tool_edit",
            &profile_id,
            "mcp_tool",
            &target_key,
            "permission",
            &error.1,
        );
    })?;
    let summary = profile
        .set_mcp_tool_permission(&server_id, &tool_id, update.action, "service-api")
        .map_err(|error| {
            log_profile_mutation_route_rejected(
                "profile_mcp_tool_edit",
                &profile_id,
                "mcp_tool",
                &target_key,
                "permission",
                &error,
            );
            AppError(StatusCode::BAD_REQUEST, error)
        })?;
    let event = write_profile_mutation_event(&state, summary, &profile).await?;
    log_profile_mutation_applied("profile_mcp_tool_edit", &event);
    Ok(Json(json!({
        "profile_id": event.profile_id,
        "server_id": server_id,
        "tool_id": tool_id,
        "action": update.action,
        "mutation": event,
    })))
}

/// POST /profiles/:profile_id/mcp/servers/:server_id/tools/:tool_id/call -- call a tool via a VM aggregator.
pub(super) async fn handle_profile_mcp_tool_call(
    State(state): State<Arc<ServiceState>>,
    Path((profile_id, server_id, tool_id)): Path<(String, String, String)>,
    Json(arguments): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    ensure_profile_mcp_server(profile_id, &server_id)?;
    let namespaced_name = resolve_mcp_tool_id(&server_id, &tool_id)?;
    // Find any running instance to route the call through.
    let selected_session = {
        let instances = state.instances.lock().unwrap();
        instances
            .iter()
            .next()
            .map(|(id, info)| (id.clone(), info.uds_path.clone()))
    };
    let (_session_id, uds_path) =
        selected_session.ok_or_else(|| AppError(StatusCode::SERVICE_UNAVAILABLE, "no running sessions".into()))?;

    let arguments_json = serde_json::to_string(&arguments)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("invalid arguments: {e}")))?;
    let job_id = state.next_job_id();
    let msg = ServiceToProcess::McpCallTool {
        id: job_id,
        namespaced_name: namespaced_name.clone(),
        arguments_json,
    };
    let resp = send_ipc_command(&uds_path, msg, Some(60))
        .await
        .map_err(|e| AppError(StatusCode::BAD_GATEWAY, e))?;

    match resp {
        ProcessToService::McpCallToolResult { result_json, error, .. } => {
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
