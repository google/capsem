//! Host configuration detection and API key validation.
//!
//! Scans the user's macOS host for pre-existing developer configuration
//! (git identity, SSH keys, API keys, GitHub tokens) to pre-fill the
//! first-run setup wizard. All detection is best-effort -- any error
//! returns None for that field.
//!
//! Also provides async API key validation against provider endpoints.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Detected host configuration for the setup wizard.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HostConfig {
    pub git_name: Option<String>,
    pub git_email: Option<String>,
    pub ssh_public_key: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub github_token: Option<String>,
    pub claude_oauth_credentials: Option<String>,
    pub google_adc: Option<String>,
}

/// Safe summary of detected config for API responses.
/// Contains presence booleans instead of raw secret values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedConfigSummary {
    pub git_name: Option<String>,
    pub git_email: Option<String>,
    pub ssh_public_key_present: bool,
    pub anthropic_api_key_present: bool,
    pub google_api_key_present: bool,
    pub openai_api_key_present: bool,
    pub github_token_present: bool,
    pub claude_oauth_present: bool,
    pub google_adc_present: bool,
    /// Setting IDs that were written during detection.
    pub settings_written: Vec<String>,
}

impl From<&HostConfig> for DetectedConfigSummary {
    fn from(config: &HostConfig) -> Self {
        Self {
            git_name: config.git_name.clone(),
            git_email: config.git_email.clone(),
            ssh_public_key_present: config.ssh_public_key.is_some(),
            anthropic_api_key_present: config.anthropic_api_key.is_some(),
            google_api_key_present: config.google_api_key.is_some(),
            openai_api_key_present: config.openai_api_key.is_some(),
            github_token_present: config.github_token.is_some(),
            claude_oauth_present: config.claude_oauth_credentials.is_some(),
            google_adc_present: config.google_adc.is_some(),
            settings_written: Vec::new(),
        }
    }
}

/// Result of validating an API key against a provider endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyValidation {
    pub valid: bool,
    pub message: String,
}

/// Mapping from HostConfig fields to setting IDs.
/// Text settings use SettingValue::Text, file settings use SettingValue::File.
const DETECT_SETTING_MAP: &[(&str, &str)] = &[
    // (field_name, setting_id)
    ("anthropic_api_key", "ai.anthropic.api_key"),
    ("openai_api_key", "ai.openai.api_key"),
    ("google_api_key", "ai.google.api_key"),
    ("github_token", "repository.providers.github.token"),
    ("git_name", "repository.git.identity.author_name"),
    ("git_email", "repository.git.identity.author_email"),
    ("ssh_public_key", "vm.environment.ssh.public_key"),
];

/// File-type settings that need SettingValue::File instead of Text.
const DETECT_FILE_MAP: &[(&str, &str, &str)] = &[
    // (field_name, setting_id, file_path)
    ("claude_oauth_credentials", "ai.anthropic.claude.credentials_json", "/root/.claude/.credentials.json"),
    ("google_adc", "ai.google.gemini.google_adc_json", "/root/.config/gcloud/application_default_credentials.json"),
];

/// Detect host config and write found values to user settings.
///
/// Only writes to settings that are currently empty (does not overwrite
/// user-configured values). Returns a summary with presence booleans
/// and the list of setting IDs that were written.
pub fn detect_and_write_to_settings() -> DetectedConfigSummary {
    use crate::net::policy_config::{self, SettingValue};

    let config = detect();
    let mut summary = DetectedConfigSummary::from(&config);

    // Load current user settings to check which are already populated
    let (user_settings, _corp) = policy_config::load_settings_files();
    let mut changes: HashMap<String, SettingValue> = HashMap::new();

    // Helper: get the detected value for a field name
    let field_value = |field: &str| -> Option<&str> {
        match field {
            "anthropic_api_key" => config.anthropic_api_key.as_deref(),
            "openai_api_key" => config.openai_api_key.as_deref(),
            "google_api_key" => config.google_api_key.as_deref(),
            "github_token" => config.github_token.as_deref(),
            "git_name" => config.git_name.as_deref(),
            "git_email" => config.git_email.as_deref(),
            "ssh_public_key" => config.ssh_public_key.as_deref(),
            _ => None,
        }
    };

    // Text settings
    for &(field, setting_id) in DETECT_SETTING_MAP {
        if let Some(value) = field_value(field) {
            // Only write if the setting is currently empty
            let existing = user_settings.settings.get(setting_id);
            let is_empty = match existing {
                None => true,
                Some(entry) => match &entry.value {
                    SettingValue::Text(t) => t.is_empty(),
                    _ => false,
                },
            };
            if is_empty {
                changes.insert(setting_id.to_string(), SettingValue::Text(value.to_string()));
                summary.settings_written.push(setting_id.to_string());
            }
        }
    }

    // File settings (credentials, ADC)
    let file_field_value = |field: &str| -> Option<&str> {
        match field {
            "claude_oauth_credentials" => config.claude_oauth_credentials.as_deref(),
            "google_adc" => config.google_adc.as_deref(),
            _ => None,
        }
    };

    for &(field, setting_id, file_path) in DETECT_FILE_MAP {
        if let Some(content) = file_field_value(field) {
            let existing = user_settings.settings.get(setting_id);
            let is_empty = match existing {
                None => true,
                Some(entry) => match &entry.value {
                    SettingValue::File { content: c, .. } => c.is_empty(),
                    _ => false,
                },
            };
            if is_empty {
                changes.insert(
                    setting_id.to_string(),
                    SettingValue::File {
                        path: file_path.to_string(),
                        content: content.to_string(),
                    },
                );
                summary.settings_written.push(setting_id.to_string());
            }
        }
    }

    // Write all changes in one batch
    if !changes.is_empty() {
        if let Err(e) = policy_config::batch_update_settings(&changes) {
            tracing::warn!(error = %e, "failed to write detected config to settings");
        }
    }

    summary
}

/// Detect all available host configuration.
pub fn detect() -> HostConfig {
    let home = match std::env::var("HOME").ok() {
        Some(h) => PathBuf::from(h),
        None => return HostConfig::default(),
    };

    let git = detect_git_identity(&home);
    HostConfig {
        git_name: git.0,
        git_email: git.1,
        ssh_public_key: detect_ssh_public_key(&home),
        anthropic_api_key: detect_anthropic_key(&home),
        google_api_key: detect_google_key(&home),
        openai_api_key: detect_openai_key(&home),
        github_token: detect_github_token(),
        claude_oauth_credentials: detect_claude_oauth(&home),
        google_adc: detect_google_adc(&home),
    }
}

/// Parse ~/.gitconfig for [user] name and email.
fn detect_git_identity(home: &Path) -> (Option<String>, Option<String>) {
    let path = home.join(".gitconfig");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return (None, None),
    };

    let mut name = None;
    let mut email = None;
    let mut in_user_section = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_user_section = trimmed.eq_ignore_ascii_case("[user]");
            continue;
        }
        if !in_user_section {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim().to_lowercase();
            let value = value.trim().to_string();
            if !value.is_empty() {
                match key.as_str() {
                    "name" => name = Some(value),
                    "email" => email = Some(value),
                    _ => {}
                }
            }
        }
    }

    (name, email)
}

/// Read ~/.ssh/id_ed25519.pub or ~/.ssh/id_rsa.pub.
fn detect_ssh_public_key(home: &Path) -> Option<String> {
    let candidates = ["id_ed25519.pub", "id_ecdsa.pub", "id_rsa.pub"];
    for name in &candidates {
        let path = home.join(".ssh").join(name);
        if let Ok(content) = std::fs::read_to_string(&path) {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

/// Detect Anthropic API key: env > ~/.claude/settings.json > ~/.anthropic/api_key.
fn detect_anthropic_key(home: &Path) -> Option<String> {
    if let Some(key) = non_empty_env("ANTHROPIC_API_KEY") {
        return Some(key);
    }
    // Try ~/.claude/settings.json
    let path = home.join(".claude").join("settings.json");
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Some(key) = extract_json_string_field(&content, "apiKey") {
            return Some(key);
        }
    }
    // Try ~/.anthropic/api_key (Anthropic SDK file)
    if let Some(key) = read_key_file(&home.join(".anthropic").join("api_key")) {
        return Some(key);
    }
    None
}

/// Detect Google AI API key from env var or ~/.gemini/settings.json.
fn detect_google_key(home: &Path) -> Option<String> {
    if let Some(key) = non_empty_env("GEMINI_API_KEY") {
        return Some(key);
    }
    // Try ~/.gemini/settings.json
    let path = home.join(".gemini").join("settings.json");
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Some(key) = extract_json_string_field(&content, "apiKey") {
            return Some(key);
        }
    }
    None
}

/// Detect OpenAI API key: env > ~/.config/openai/api_key.
fn detect_openai_key(home: &Path) -> Option<String> {
    if let Some(key) = non_empty_env("OPENAI_API_KEY") {
        return Some(key);
    }
    // Try ~/.config/openai/api_key (OpenAI CLI file)
    if let Some(key) = read_key_file(&home.join(".config").join("openai").join("api_key")) {
        return Some(key);
    }
    None
}

/// Detect GitHub token via `gh auth token`.
fn detect_github_token() -> Option<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() { None } else { Some(token) }
}

/// Detect Claude Code OAuth credentials from ~/.claude/.credentials.json.
/// Returns the raw JSON content if the file contains a valid `claudeAiOauth` object.
fn detect_claude_oauth(home: &Path) -> Option<String> {
    let path = home.join(".claude").join(".credentials.json");
    let content = std::fs::read_to_string(&path).ok()?;
    // Validate it's real OAuth credentials (not an empty or unrelated file).
    if content.contains("claudeAiOauth") && content.contains("refreshToken") {
        Some(content.trim().to_string())
    } else {
        None
    }
}

/// Detect Google Cloud Application Default Credentials.
/// Returns the raw JSON content if ~/.config/gcloud/application_default_credentials.json exists.
fn detect_google_adc(home: &Path) -> Option<String> {
    let path = home
        .join(".config")
        .join("gcloud")
        .join("application_default_credentials.json");
    let content = std::fs::read_to_string(&path).ok()?;
    if content.contains("refresh_token") {
        Some(content.trim().to_string())
    } else {
        None
    }
}

/// Read an env var, returning None if empty or unset.
fn non_empty_env(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    }
}

/// Read a key from a plain-text file, trimming whitespace. Returns None if
/// the file is missing, unreadable, or contains only whitespace.
fn read_key_file(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

/// Validate an API key by hitting a lightweight provider endpoint.
///
/// Returns `KeyValidation { valid, message }`. Network errors produce
/// descriptive messages rather than Err -- only truly unexpected failures
/// (unknown provider) return Err.
pub async fn validate_api_key(provider: &str, key: &str) -> Result<KeyValidation, String> {
    // Trim whitespace and strip surrounding quotes (common copy-paste artifact).
    let key = key.trim();
    let key = key.strip_prefix('"').unwrap_or(key);
    let key = key.strip_suffix('"').unwrap_or(key);
    let key = key.strip_prefix('\'').unwrap_or(key);
    let key = key.strip_suffix('\'').unwrap_or(key);
    let key = key.trim();
    if key.is_empty() {
        return Ok(KeyValidation {
            valid: false,
            message: "API key is empty".to_string(),
        });
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let response = match provider {
        "anthropic" => {
            client
                .get("https://api.anthropic.com/v1/models")
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await
        }
        "google" => {
            client
                .get(format!(
                    "https://generativelanguage.googleapis.com/v1beta/models?key={}",
                    key
                ))
                .send()
                .await
        }
        "openai" => {
            client
                .get("https://api.openai.com/v1/models")
                .header("Authorization", format!("Bearer {key}"))
                .send()
                .await
        }
        "github" => {
            client
                .get("https://api.github.com/user")
                .header("Authorization", format!("Bearer {key}"))
                .header("User-Agent", "capsem")
                .send()
                .await
        }
        _ => {
            return Err(format!("unknown provider: {provider}"));
        }
    };

    match response {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                Ok(KeyValidation {
                    valid: true,
                    message: "Valid".to_string(),
                })
            } else if status.as_u16() == 401 || status.as_u16() == 403 {
                Ok(KeyValidation {
                    valid: false,
                    message: "Invalid API key".to_string(),
                })
            } else {
                Ok(KeyValidation {
                    valid: false,
                    message: format!("HTTP {status}"),
                })
            }
        }
        Err(e) => {
            let msg = if e.is_timeout() {
                "Request timed out".to_string()
            } else if e.is_connect() {
                "Connection failed".to_string()
            } else {
                format!("Network error: {e}")
            };
            Ok(KeyValidation {
                valid: false,
                message: msg,
            })
        }
    }
}

/// Extract a string value for a given key from a JSON string (simple search).
/// Not a full JSON parser -- looks for `"key": "value"` patterns.
fn extract_json_string_field(json: &str, field: &str) -> Option<String> {
    // Look for "field" followed by : and a quoted string value
    let pattern = format!("\"{}\"", field);
    let idx = json.find(&pattern)?;
    let after_key = &json[idx + pattern.len()..];
    // Skip whitespace and colon
    let after_colon = after_key.trim_start().strip_prefix(':')?;
    let after_ws = after_colon.trim_start();
    if !after_ws.starts_with('"') {
        return None;
    }
    let value_start = &after_ws[1..];
    let end = value_start.find('"')?;
    let value = value_start[..end].trim();
    if value.is_empty() { None } else { Some(value.to_string()) }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
