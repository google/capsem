use super::{
    build_profile_status_cache, load_profile_catalog_for_service, AppError, Bytes, ServiceState,
    StatusCode,
};
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::Path as StdPath;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(super) struct ProfileStatusCache {
    pub(super) inputs: ProfileStatusInputs,
    pub(super) catalog: serde_json::Value,
    pub(super) catalog_body: Bytes,
    pub(super) profiles: BTreeMap<String, serde_json::Value>,
    pub(super) profile_bodies: BTreeMap<String, Bytes>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProfileStatusInputs {
    pub(super) manifest: ProfileStatusFileIdentity,
    pub(super) metadata: ProfileStatusFileIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ProfileStatusFileIdentity {
    Missing,
    Digest(String),
    Unreadable(String),
}

fn profile_status_file_identity(path: &StdPath) -> ProfileStatusFileIdentity {
    if !path.exists() {
        return ProfileStatusFileIdentity::Missing;
    }
    match capsem_assets::asset_manager::hash_file(path) {
        Ok(digest) => ProfileStatusFileIdentity::Digest(digest),
        Err(error) => ProfileStatusFileIdentity::Unreadable(error.to_string()),
    }
}

pub(super) fn profile_status_inputs(state: &ServiceState) -> ProfileStatusInputs {
    ProfileStatusInputs {
        manifest: profile_status_file_identity(&state.assets_dir.join("manifest.json")),
        metadata: profile_status_file_identity(&state.assets_dir.join("manifest-metadata.json")),
    }
}

pub(super) fn build_stable_profile_status_cache(
    state: &ServiceState,
) -> Result<Arc<ProfileStatusCache>, AppError> {
    let inputs = profile_status_inputs(state);
    let catalog = load_profile_catalog_for_service()?;
    let cache = Arc::new(build_profile_status_cache(state, &catalog, inputs.clone()));
    if profile_status_inputs(state) != inputs {
        return Err(AppError(
            StatusCode::CONFLICT,
            "asset manifest changed while profile status was being built".to_string(),
        ));
    }
    Ok(cache)
}

pub(super) fn rebuild_profile_status_cache(
    state: &ServiceState,
) -> Result<Arc<ProfileStatusCache>, AppError> {
    let cache = build_stable_profile_status_cache(state)?;
    *state.profile_status_cache.lock().map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("profile status cache lock poisoned: {error}"),
        )
    })? = Some(Arc::clone(&cache));
    Ok(cache)
}

pub(super) fn profile_status_cache(
    state: &ServiceState,
) -> Result<Arc<ProfileStatusCache>, AppError> {
    let inputs = profile_status_inputs(state);
    if let Some(cache) = state
        .profile_status_cache
        .lock()
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("profile status cache lock poisoned: {error}"),
            )
        })?
        .as_ref()
        .filter(|cache| cache.inputs == inputs)
        .cloned()
    {
        return Ok(cache);
    }
    rebuild_profile_status_cache(state)
}

pub(super) fn profile_status_catalog_body(state: &ServiceState) -> Result<Bytes, AppError> {
    Ok(profile_status_cache(state)?.catalog_body.clone())
}
