//! Profile OBOM (operational bill of materials) lookup and the route that
//! serves it.

use super::*;

pub(crate) fn profile_obom_info(profile: &ProfileConfigFile) -> Option<api::ProfileObomInfo> {
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

pub(crate) async fn handle_profile_obom(
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
        let path = PathBuf::from(path);
        let info = obom.clone();
        Some(
            tokio::task::spawn_blocking(move || read_local_profile_obom(&path, &info))
                .await
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("obom read task failed: {e}")))??,
        )
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

pub(crate) fn read_local_profile_obom(
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
