//! settings.toml and the corp config: read, edit, provision and reload.
use super::*;

/// Re-materialize every running VM's active policy from the current files and
/// have each reload it.
pub(super) async fn handle_reload_config(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mutation = state.policy_mutation.begin().await;
    let reloaded = push_policy_to_running_instances(&state, &mutation).await?;
    drop(mutation);
    Ok(Json(serde_json::json!({ "success": true, "reloaded": reloaded })))
}

/// GET /settings/info -- unified settings tree + issues.
pub(super) async fn handle_get_settings() -> Json<serde_json::Value> {
    let resp = capsem_core::net::policy_config::load_settings_response();
    Json(serde_json::to_value(resp).unwrap_or_default())
}

/// PATCH /settings/edit -- batch-update settings and return the refreshed tree.
pub(super) async fn handle_save_settings(
    State(state): State<Arc<ServiceState>>,
    Json(raw): Json<HashMap<String, serde_json::Value>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _mutation = state.policy_mutation.begin().await;
    capsem_core::net::policy_config::batch_update_settings_json(&raw)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
    let resp = capsem_core::net::policy_config::load_settings_response();
    Ok(Json(serde_json::to_value(resp).unwrap_or_default()))
}

/// PUT /corp/edit -- apply corporate config from URL or inline TOML.
pub(super) async fn handle_corp_config(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<CorpConfigRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    use capsem_core::net::policy_config::corp_provision;
    let _mutation = state.policy_mutation.begin().await;
    let capsem_dir = capsem_foundation::paths::capsem_home_opt()
        .ok_or(AppError(StatusCode::INTERNAL_SERVER_ERROR, "HOME not set".into()))?;

    if let Some(source) = &payload.source {
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

/// POST /corp/reload -- refresh/re-read the corp overlay and reload running VMs.
pub(super) async fn handle_corp_reload(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    use capsem_core::net::policy_config::corp_provision;

    let capsem_dir = capsem_foundation::paths::capsem_home_opt()
        .ok_or(AppError(StatusCode::INTERNAL_SERVER_ERROR, "HOME not set".into()))?;
    corp_provision::refresh_corp_config_if_stale(capsem_dir).await;
    handle_reload_config(State(state)).await
}
