use serde::Serialize;

/// Open a URL in the host's default browser.
#[tauri::command]
pub async fn open_url(url: String, app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn detect_host_config() -> Result<capsem_core::host_config::HostConfig, String> {
    tokio::task::spawn_blocking(capsem_core::host_config::detect)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[tauri::command]
pub async fn validate_api_key(
    provider: String,
    key: String,
) -> Result<capsem_core::host_config::KeyValidation, String> {
    capsem_core::host_config::validate_api_key(&provider, &key).await
}

/// Info about an available app update.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
}

/// Manually check for an app update. Returns Some(UpdateInfo) if available.
#[tauri::command]
pub async fn check_for_app_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app.updater().map_err(|e| format!("updater not available: {e}"))?;
    let update = updater.check().await.map_err(|e| format!("update check failed: {e}"))?;

    match update {
        Some(u) => {
            let current_version = app.package_info().version.to_string();
            Ok(Some(UpdateInfo {
                version: u.version.clone(),
                current_version,
            }))
        }
        None => Ok(None),
    }
}
