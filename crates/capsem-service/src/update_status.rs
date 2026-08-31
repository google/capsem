use std::path::Path;

use serde::Deserialize;

use crate::api;

#[cfg(test)]
mod tests;

const UPDATE_CACHE_TTL_SECS: u64 = 24 * 3600;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct UpdateCheckCache {
    #[serde(default)]
    checked_at: u64,
    #[serde(default)]
    latest_version: Option<String>,
    #[serde(default)]
    update_available: bool,
    #[serde(default)]
    latest_assets: Option<String>,
    #[serde(default)]
    assets_update_available: bool,
    #[serde(default)]
    assets_state: Option<String>,
    #[serde(default)]
    assets_blocked_reason: Option<String>,
    #[serde(default)]
    latest_profiles: Option<String>,
    #[serde(default)]
    current_profiles: Option<String>,
    #[serde(default)]
    profiles_update_available: bool,
    #[serde(default)]
    profiles_state: Option<String>,
    #[serde(default)]
    profiles_blocked_reason: Option<String>,
    #[serde(default)]
    latest_images: Option<String>,
    #[serde(default)]
    images_update_available: bool,
    #[serde(default)]
    images_state: Option<String>,
    #[serde(default)]
    images_blocked_reason: Option<String>,
    #[serde(default, rename = "checked_url")]
    source: Option<String>,
    #[serde(default)]
    channel_hash: Option<String>,
    #[serde(default)]
    validation_status: Option<String>,
    #[serde(default)]
    validation_error: Option<String>,
}
pub(super) fn update_status_response_from_paths(
    current_binary: &str,
    assets_dir: &Path,
    cache_path: &Path,
    now: u64,
) -> api::UpdateStatusResponse {
    let current_assets = current_asset_version_from_manifest(assets_dir);
    let manifest_channel = manifest_channel_source(assets_dir);
    let cache_result = read_update_check_cache(cache_path);
    let (cache, parse_error) = match cache_result {
        Ok(cache) => (cache, None),
        Err(error) => (None, Some(error)),
    };
    let checked_at = cache.as_ref().map(|cache| cache.checked_at);
    let stale = checked_at
        .map(|checked_at| now.saturating_sub(checked_at) > UPDATE_CACHE_TTL_SECS)
        .unwrap_or(true);
    let channel_url = cache
        .as_ref()
        .and_then(|cache| cache.source.clone())
        .or(manifest_channel);
    let channel_hash = cache.as_ref().and_then(|cache| cache.channel_hash.clone());
    let validation_status = cache.as_ref().and_then(|cache| cache.validation_status.clone());
    let validation_error = cache.as_ref().and_then(|cache| cache.validation_error.clone());
    let last_error = parse_error.or_else(|| validation_error.clone());
    let supply_chain = supply_chain_evidence_from_paths(assets_dir, channel_url.clone(), channel_hash.clone());

    api::UpdateStatusResponse {
        checked_at,
        channel_url,
        channel_hash,
        validation_status,
        validation_error,
        stale,
        last_error,
        binary: update_track(
            Some(current_binary.to_string()),
            cache.as_ref().and_then(|cache| cache.latest_version.clone()),
            cache.as_ref().is_some_and(|cache| cache.update_available),
        ),
        assets: cache
            .as_ref()
            .map(|cache| {
                channel_update_track(
                    current_assets.clone(),
                    cache.latest_assets.clone(),
                    cache.assets_update_available,
                    cache.assets_state.as_deref(),
                    cache.assets_blocked_reason.clone(),
                )
            })
            .unwrap_or_else(|| update_track(current_assets, None, false)),
        profiles: cache
            .as_ref()
            .map(|cache| {
                channel_update_track(
                    cache.current_profiles.clone(),
                    cache.latest_profiles.clone(),
                    cache.profiles_update_available,
                    cache.profiles_state.as_deref(),
                    cache.profiles_blocked_reason.clone(),
                )
            })
            .unwrap_or_else(not_published_update_track),
        images: cache
            .as_ref()
            .map(|cache| {
                channel_update_track(
                    None,
                    cache.latest_images.clone(),
                    cache.images_update_available,
                    cache.images_state.as_deref(),
                    cache.images_blocked_reason.clone(),
                )
            })
            .unwrap_or_else(not_published_update_track),
        supply_chain,
    }
}

fn supply_chain_evidence_from_paths(
    assets_dir: &Path,
    channel_url: Option<String>,
    channel_hash: Option<String>,
) -> api::SupplyChainEvidence {
    let manifest_path = assets_dir.join("manifest.json");
    let manifest_metadata = std::fs::read_to_string(assets_dir.join("manifest-metadata.json"))
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok());
    let metadata_origin = manifest_metadata
        .as_ref()
        .and_then(|value| value.get("origin"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            if manifest_path.is_file() {
                Some("installed".to_string())
            } else {
                None
            }
        });
    let manifest_source = manifest_metadata
        .as_ref()
        .and_then(|value| value.get("manifest_url"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let manifest_hash = if manifest_path.is_file() {
        capsem_assets::asset_manager::hash_file(&manifest_path).ok()
    } else {
        None
    };

    api::SupplyChainEvidence {
        manifest: api::SupplyChainManifestEvidence {
            origin: metadata_origin,
            source: manifest_source,
            path: manifest_path.display().to_string(),
            blake3: manifest_hash,
        },
        channel_index: api::SupplyChainChannelEvidence {
            url: channel_url,
            sha256: channel_hash,
        },
        host_sbom: api::SupplyChainReference {
            name: "host_sbom".to_string(),
            format: Some("spdx_json_2_3".to_string()),
            scope: Some("host_binaries".to_string()),
            generator: Some("cargo-sbom".to_string()),
            release_artifact: Some("capsem-sbom.spdx.json".to_string()),
            route: None,
            workflow: Some(".github/workflows/release.yaml".to_string()),
        },
        vm_obom: api::SupplyChainReference {
            name: "profile_obom".to_string(),
            format: Some("cyclonedx-obom.v1".to_string()),
            scope: Some("base_image".to_string()),
            generator: Some("cdxgen".to_string()),
            release_artifact: None,
            route: Some("/profiles/{profile_id}/obom".to_string()),
            workflow: Some(".github/workflows/release-assets.yaml".to_string()),
        },
        attestations: vec![
            api::SupplyChainReference {
                name: "github_attestations_host".to_string(),
                format: None,
                scope: Some("host_binaries".to_string()),
                generator: Some("gh attestation".to_string()),
                release_artifact: None,
                route: None,
                workflow: Some(".github/workflows/release.yaml".to_string()),
            },
            api::SupplyChainReference {
                name: "github_attestations_vm_assets".to_string(),
                format: None,
                scope: Some("vm_assets".to_string()),
                generator: Some("gh attestation".to_string()),
                release_artifact: None,
                route: None,
                workflow: Some(".github/workflows/release-assets.yaml".to_string()),
            },
        ],
    }
}

fn read_update_check_cache(path: &Path) -> std::result::Result<Option<UpdateCheckCache>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&content).map_err(|error| format!("parse {}: {error}", path.display()))?;
    if value.get("schema").and_then(serde_json::Value::as_str) != Some("capsem.manifest_metadata.v1") {
        return Err(format!(
            "parse {}: expected schema capsem.manifest_metadata.v1",
            path.display()
        ));
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| format!("parse {}: {error}", path.display()))
}

fn current_asset_version_from_manifest(assets_dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(assets_dir.join("manifest.json")).ok()?;
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Ok(state) = capsem_assets::asset_manager::release_graph_profile_state(&value) {
            return Some(state.images_revision);
        }
    }
    capsem_assets::asset_manager::ManifestV2::from_json(&content)
        .ok()
        .map(|manifest| manifest.assets.current)
}

fn manifest_channel_source(assets_dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(assets_dir.join("manifest-metadata.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value
        .get("manifest_url")
        .and_then(|source| source.as_str())
        .map(ToOwned::to_owned)
}

fn update_track(current: Option<String>, latest: Option<String>, update_available: bool) -> api::UpdateTrackStatus {
    let state = if update_available {
        api::UpdateTrackState::UpdateAvailable
    } else if latest.is_some() || current.is_some() {
        api::UpdateTrackState::Current
    } else {
        api::UpdateTrackState::Unknown
    };
    let compatibility = if current.is_some() || latest.is_some() {
        api::UpdateCompatibilityState::Compatible
    } else {
        api::UpdateCompatibilityState::Unknown
    };
    api::UpdateTrackStatus {
        current,
        latest,
        blocked_reason: None,
        update_available,
        state,
        compatibility,
    }
}

fn not_published_update_track() -> api::UpdateTrackStatus {
    api::UpdateTrackStatus {
        current: None,
        latest: None,
        blocked_reason: None,
        update_available: false,
        state: api::UpdateTrackState::NotPublished,
        compatibility: api::UpdateCompatibilityState::NotApplicable,
    }
}

fn channel_update_track(
    current: Option<String>,
    latest: Option<String>,
    update_available: bool,
    channel_state: Option<&str>,
    blocked_reason: Option<String>,
) -> api::UpdateTrackStatus {
    if blocked_reason.is_some() {
        return api::UpdateTrackStatus {
            current,
            latest,
            blocked_reason,
            update_available: false,
            state: api::UpdateTrackState::Unknown,
            compatibility: api::UpdateCompatibilityState::Unknown,
        };
    }
    match channel_state {
        Some("not_published") => not_published_update_track(),
        Some("unknown") => api::UpdateTrackStatus {
            current,
            latest,
            blocked_reason: None,
            update_available: false,
            state: api::UpdateTrackState::Unknown,
            compatibility: api::UpdateCompatibilityState::Unknown,
        },
        Some("update_available") => update_track(current, latest, true),
        Some("current") | Some("published") => update_track(current, latest, update_available),
        _ if latest.is_some() || update_available => update_track(current, latest, update_available),
        _ => not_published_update_track(),
    }
}
