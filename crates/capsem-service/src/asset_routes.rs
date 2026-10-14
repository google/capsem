//! The VM boot assets: which ones a new VM boots, whether they are on disk,
//! and the reconciliation that downloads the missing ones.
//!
//! There is one runtime asset set: kernel, initrd and rootfs of the installed
//! manifest's release for this binary on this host's architecture. A
//! persistent VM keeps the set it was created with (its `asset_pins`).
use super::*;

use capsem_assets::asset_manager::{host_manifest_arch, AssetEntry, ResolvedAssets};

/// The runtime asset set, resolved against the assets directory.
pub(crate) struct RuntimeAssetSet {
    pub(crate) resolved: ResolvedAssets,
    pub(crate) pins: BootAssetPins,
    /// (kind, logical name, manifest entry) for kernel, initrd and rootfs.
    entries: [(&'static str, String, AssetEntry); 3],
}

impl ServiceState {
    /// The runtime asset set every new VM boots.
    pub(crate) fn runtime_asset_set(&self) -> Result<RuntimeAssetSet> {
        let manifest = self
            .manifest
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow!("no asset manifest is installed in {}", self.assets_dir.display()))?;
        let arch = host_manifest_arch();
        let resolved = manifest.resolve(&self.current_version, arch, &self.assets_dir)?;
        let assets = manifest
            .assets
            .releases
            .get(&resolved.asset_version)
            .and_then(|release| release.arches.get(arch))
            .ok_or_else(|| anyhow!("asset release {} has no {arch} assets", resolved.asset_version))?;
        let entry = |kind: &'static str, name: &str| -> Result<(&'static str, String, AssetEntry)> {
            let entry = assets
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow!("asset release {} has no {name}", resolved.asset_version))?;
            Ok((kind, name.to_string(), entry))
        };
        // The logical names `ManifestV2::resolve` boots.
        let entries = [
            entry("kernel", "vmlinuz")?,
            entry("initrd", "initrd.img")?,
            entry("rootfs", "rootfs.erofs")?,
        ];
        let pin = |(_, name, entry): &(&'static str, String, AssetEntry)| BootAssetPin {
            name: name.clone(),
            hash: format!("blake3:{}", entry.hash.trim_start_matches("blake3:")),
        };
        let pins = BootAssetPins {
            kernel: pin(&entries[0]),
            initrd: pin(&entries[1]),
            rootfs: pin(&entries[2]),
        };
        Ok(RuntimeAssetSet {
            resolved,
            pins,
            entries,
        })
    }
}

impl RuntimeAssetSet {
    fn paths(&self) -> [&StdPath; 3] {
        [&self.resolved.kernel, &self.resolved.initrd, &self.resolved.rootfs]
    }

    /// Presence of each asset by its size; full hash verification stays with
    /// boot and reconciliation, so a polled status never hashes a rootfs.
    fn file_statuses(&self) -> Vec<api::AssetFileStatus> {
        self.entries
            .iter()
            .zip(self.paths())
            .map(|((kind, name, entry), path)| {
                let actual_size = std::fs::metadata(path)
                    .ok()
                    .filter(|metadata| metadata.is_file())
                    .map(|metadata| metadata.len());
                let status = match actual_size {
                    None => api::AssetFileState::Missing,
                    Some(size) if size != entry.size => api::AssetFileState::Invalid,
                    Some(_) => api::AssetFileState::Present,
                };
                api::AssetFileStatus {
                    kind: (*kind).to_string(),
                    name: name.clone(),
                    path: path.display().to_string(),
                    status,
                    expected_hash: entry.hash.clone(),
                    expected_size: Some(entry.size),
                    actual_size,
                }
            })
            .collect()
    }
}

/// Why a new VM cannot boot the runtime asset set now, if it cannot.
pub(super) fn vm_asset_block_reason(state: &ServiceState) -> Option<String> {
    let set = match state.runtime_asset_set() {
        Ok(set) => set,
        Err(error) => return Some(format!("VM assets are not ready: {error:#}")),
    };
    let missing = set
        .entries
        .iter()
        .zip(set.paths())
        .filter(|(_, path)| !path.exists())
        .map(|((_, name, _), _)| name.as_str())
        .collect::<Vec<_>>();
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

/// Readiness of the runtime asset set, with any reconciliation in flight.
pub(super) fn asset_status(state: &ServiceState) -> Result<api::AssetStatus, AppError> {
    let reconcile = state.asset_reconcile.lock().map(|s| s.clone()).unwrap_or_default();
    let downloading = reconcile.in_progress || state.asset_reconcile_inflight.load(Ordering::Acquire);
    let mut errors = Vec::new();
    let (asset_version, assets) = match state.runtime_asset_set() {
        Ok(set) => (Some(set.resolved.asset_version.clone()), set.file_statuses()),
        Err(error) => {
            errors.push(format!("{error:#}"));
            (None, Vec::new())
        }
    };
    for asset in &assets {
        match asset.status {
            api::AssetFileState::Present => {}
            api::AssetFileState::Missing => errors.push(format!("{} is missing at {}", asset.name, asset.path)),
            api::AssetFileState::Invalid => errors.push(format!(
                "{} at {} is {} bytes, the manifest records {}",
                asset.name,
                asset.path,
                asset.actual_size.unwrap_or_default(),
                asset.expected_size.unwrap_or_default()
            )),
        }
    }
    Ok(api::AssetStatus {
        // A status taken during a repair is not ready: reconciliation
        // publishes its replacement before releasing the inflight claim.
        ready: errors.is_empty() && !downloading,
        downloading,
        current_arch: host_manifest_arch().to_string(),
        asset_version,
        assets,
        errors,
        manifest: asset_manifest_status(state)?.as_ref().clone(),
        current_asset: reconcile.current_asset.clone(),
        bytes_done: reconcile.current_asset.as_ref().map(|_| reconcile.bytes_done),
        bytes_total: reconcile.current_asset.as_ref().and(reconcile.bytes_total),
        downloaded: reconcile.last_downloaded,
        reconcile_error: reconcile.last_error,
        started: None,
    })
}

/// GET /assets/status -- readiness of the runtime asset set.
pub(super) async fn handle_asset_status(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<api::AssetStatus>, AppError> {
    state.off_worker(|state| asset_status(&state)).await?.map(Json)
}

/// POST /assets/ensure -- download missing or corrupt runtime assets in the
/// background and answer the status that started it.
pub(super) async fn handle_asset_ensure(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<api::AssetStatus>, AppError> {
    let started = asset_background::start_asset_ensure(&state);
    let mut status = state.off_worker(|state| asset_status(&state)).await??;
    status.started = Some(started);
    Ok(Json(status))
}

// ---------------------------------------------------------------------------
// The installed manifest
// ---------------------------------------------------------------------------

/// Identity of one manifest input: its bytes, not its size or mtime, since a
/// same-size edit must show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ManifestFileIdentity {
    Missing,
    Digest(String),
    Unreadable(String),
}

fn manifest_file_identity(path: &StdPath) -> ManifestFileIdentity {
    if !path.exists() {
        return ManifestFileIdentity::Missing;
    }
    match capsem_assets::asset_manager::hash_file(path) {
        Ok(digest) => ManifestFileIdentity::Digest(digest),
        Err(error) => ManifestFileIdentity::Unreadable(error.to_string()),
    }
}

/// The manifest status as last built, and the inputs it was built from.
pub(super) struct CachedManifestStatus {
    inputs: [ManifestFileIdentity; 2],
    status: Arc<api::AssetManifestStatus>,
}

/// The installed manifest's provenance and validation. Polled through
/// `/status`: it is rebuilt only when manifest.json or its metadata changes.
pub(super) fn asset_manifest_status(state: &ServiceState) -> Result<Arc<api::AssetManifestStatus>, AppError> {
    let path = state.assets_dir.join("manifest.json");
    let metadata_path = state.assets_dir.join("manifest-metadata.json");
    let inputs = [manifest_file_identity(&path), manifest_file_identity(&metadata_path)];
    let cache = || {
        state.asset_manifest_cache.lock().map_err(|error| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("asset manifest cache lock poisoned: {error}"),
            )
        })
    };
    let cached = cache()?
        .as_ref()
        .filter(|cached| cached.inputs == inputs)
        .map(|cached| Arc::clone(&cached.status));
    if let Some(status) = cached {
        return Ok(status);
    }
    // Built outside the lock; two concurrent rebuilds of one input agree.
    let status = Arc::new(build_asset_manifest_status(state, &path, &metadata_path, &inputs[0]));
    *cache()? = Some(CachedManifestStatus {
        inputs,
        status: Arc::clone(&status),
    });
    Ok(status)
}

fn build_asset_manifest_status(
    state: &ServiceState,
    path: &StdPath,
    metadata_path: &StdPath,
    identity: &ManifestFileIdentity,
) -> api::AssetManifestStatus {
    let metadata = std::fs::read_to_string(metadata_path)
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok());
    let metadata_text = |key: &str| {
        metadata
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    };
    let validation = validate_asset_manifest_file(path);
    let origin = metadata_text("origin").unwrap_or_else(|| {
        if path.is_file() {
            "installed".to_string()
        } else {
            "missing".to_string()
        }
    });
    let installed_manifest = state.manifest.read().unwrap();
    let manifest = validation.manifest.as_ref().or_else(|| {
        if validation.status == api::ValidationStatus::Missing {
            installed_manifest.as_deref()
        } else {
            None
        }
    });
    api::AssetManifestStatus {
        origin,
        path: path.display().to_string(),
        validation_status: validation.status,
        blake3: match identity {
            ManifestFileIdentity::Digest(digest) => Some(digest.clone()),
            _ => None,
        },
        refreshed_at: std::fs::metadata(path)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .map(format_system_time_rfc3339),
        validation_error: validation.error,
        origin_path: metadata.as_ref().map(|_| metadata_path.display().to_string()),
        origin_source: metadata_text("manifest_url"),
        packaged_at: metadata_text("packaged_at"),
        format: manifest.map(|manifest| manifest.format),
        refresh_policy: manifest.map(|manifest| manifest.refresh_policy.clone()),
        assets_current: manifest.map(|manifest| manifest.assets.current.clone()),
        binaries_current: manifest.map(|manifest| manifest.binaries.current.clone()),
    }
}

pub(super) struct AssetManifestValidation {
    status: api::ValidationStatus,
    manifest: Option<capsem_assets::asset_manager::ManifestV2>,
    error: Option<String>,
}

pub(super) fn validate_asset_manifest_file(path: &StdPath) -> AssetManifestValidation {
    if !path.is_file() {
        return AssetManifestValidation {
            status: api::ValidationStatus::Missing,
            manifest: None,
            error: None,
        };
    }
    let parsed = std::fs::read_to_string(path)
        .map_err(anyhow::Error::new)
        .and_then(|content| capsem_assets::asset_manager::ManifestV2::from_json(&content));
    match parsed {
        Ok(manifest) => AssetManifestValidation {
            status: api::ValidationStatus::Valid,
            manifest: Some(manifest),
            error: None,
        },
        Err(error) => AssetManifestValidation {
            status: api::ValidationStatus::Invalid,
            manifest: None,
            error: Some(error.to_string()),
        },
    }
}

pub(super) fn format_system_time_rfc3339(time: std::time::SystemTime) -> String {
    humantime::format_rfc3339_seconds(time).to_string()
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

pub(super) fn json_bytes_response(body: Bytes) -> axum::response::Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Reconciliation: one rail shared by service startup and /assets/ensure
// ---------------------------------------------------------------------------

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
    let json =
        serde_json::to_vec_pretty(status).map_err(|e| format!("serialize asset status {}: {e}", path.display()))?;
    capsem_foundation::unix::fs::atomic_write_private(path, &json).map_err(|e| format!("write {}: {e}", path.display()))
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
        let downloaded = capsem_assets::asset_manager::download_missing_assets(
            &manifest,
            &state.current_version,
            host_manifest_arch(),
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
