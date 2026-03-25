use capsem_core::net::policy_config::{self, ConfigIssue, ResolvedSetting, SecurityPreset, SettingEntry, SettingsNode, SettingValue};
use capsem_core::session;

use crate::state::AppState;
use super::{active_vm_id, reload_all_policies};

/// Set a guest env var in ~/.capsem/user.toml via settings system.
#[tauri::command]
pub async fn set_guest_env(key: String, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let path = policy_config::user_config_path()
            .ok_or("HOME not set")?;
        let mut file = policy_config::load_settings_file(&path)?;
        let setting_id = format!("guest.env.{key}");
        file.settings.insert(setting_id, SettingEntry {
            value: SettingValue::Text(value),
            modified: session::now_iso(),
        });
        policy_config::write_settings_file(&path, &file)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Remove a guest env var from ~/.capsem/user.toml via settings system.
#[tauri::command]
pub async fn remove_guest_env(key: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let path = policy_config::user_config_path()
            .ok_or("HOME not set")?;
        let mut file = policy_config::load_settings_file(&path)?;
        let setting_id = format!("guest.env.{key}");
        file.settings.remove(&setting_id);
        policy_config::write_settings_file(&path, &file)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Returns all resolved settings for the UI.
#[tauri::command]
pub async fn get_settings() -> Result<Vec<ResolvedSetting>, String> {
    tokio::task::spawn_blocking(policy_config::load_merged_settings)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// Returns the settings tree (nested groups + leaves) for the UI.
#[tauri::command]
pub async fn get_settings_tree() -> Result<Vec<SettingsNode>, String> {
    tokio::task::spawn_blocking(policy_config::load_settings_tree)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// Validate all settings and return a list of issues.
#[tauri::command]
pub async fn lint_config() -> Result<Vec<ConfigIssue>, String> {
    tokio::task::spawn_blocking(policy_config::load_merged_lint)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// Returns all available security presets.
#[tauri::command]
pub async fn list_presets() -> Result<Vec<SecurityPreset>, String> {
    tokio::task::spawn_blocking(|| Ok(policy_config::security_presets()))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Apply a security preset by ID. Hot-reloads all policies (network, domain, MCP).
#[tauri::command]
pub async fn apply_preset(id: String, app_handle: tauri::AppHandle) -> Result<Vec<String>, String> {
    let skipped = tokio::task::spawn_blocking(move || {
        policy_config::apply_preset(&id)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))??;

    use tauri::Manager;
    let state = app_handle.state::<AppState>();
    if let Ok(vm_id) = active_vm_id(&state) {
        reload_all_policies(&state, &vm_id).await;
    }

    Ok(skipped)
}

/// Update a single user setting by ID. Hot-reloads all policies so
/// changes take effect immediately for new MITM proxy connections.
#[tauri::command]
pub async fn update_setting(id: String, value: SettingValue, app_handle: tauri::AppHandle) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        policy_config::validate_setting_value(&id, &value)?;
        let path = policy_config::user_config_path()
            .ok_or("HOME not set")?;
        let mut file = policy_config::load_settings_file(&path)?;
        file.settings.insert(id, SettingEntry {
            value,
            modified: session::now_iso(),
        });
        policy_config::write_settings_file(&path, &file)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))??;

    use tauri::Manager;
    let state = app_handle.state::<AppState>();
    if let Ok(vm_id) = active_vm_id(&state) {
        reload_all_policies(&state, &vm_id).await;
    }

    Ok(())
}
