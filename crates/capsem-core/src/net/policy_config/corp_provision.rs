//! Corp config provisioning from URL or local file path.
//!
//! Enterprise users installing via CLI can provision corp config without
//! requiring root access to /etc/capsem/. Config is installed to
//! ~/.capsem/corp.toml with source metadata in ~/.capsem/corp-source.json.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::SettingsFile;

/// Default refresh interval in hours.
const DEFAULT_REFRESH_INTERVAL_HOURS: u32 = 24;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Corp source metadata stored in ~/.capsem/corp-source.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpSource {
    /// URL the config was fetched from (None if provisioned from local file).
    pub url: Option<String>,
    /// Local file path the config was copied from (None if provisioned from URL).
    pub file_path: Option<String>,
    /// Unix timestamp (seconds) of when the config was fetched/installed.
    pub fetched_at: u64,
    /// HTTP ETag for conditional refresh.
    pub etag: Option<String>,
    /// Blake3 hash of the corp.toml content.
    pub content_hash: String,
    /// Refresh interval in hours (from corp.toml, default 24).
    pub refresh_interval_hours: u32,
}

/// Fetch corp config from a URL, validate it as TOML, and return the content + ETag.
pub async fn fetch_corp_config(client: &reqwest::Client, url: &str) -> Result<(String, Option<String>)> {
    info!(url = %url, "fetching corp config");

    let resp = client
        .get(url)
        .header("User-Agent", "capsem")
        .send()
        .await
        .context("failed to fetch corp config")?;

    if !resp.status().is_success() {
        anyhow::bail!("corp config fetch failed: HTTP {} for {}", resp.status(), url);
    }

    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let body = resp.text().await.context("failed to read corp config body")?;
    validate_corp_toml(&body)?;

    Ok((body, etag))
}

/// Validate that a string is valid corp TOML (parseable as SettingsFile).
pub fn validate_corp_toml(content: &str) -> Result<SettingsFile> {
    let file: SettingsFile = toml::from_str(content)
        .context("invalid corp TOML")?;
    Ok(file)
}

/// Parse refresh_interval_hours from corp TOML content.
/// Returns DEFAULT_REFRESH_INTERVAL_HOURS if not present or unparseable.
pub fn parse_refresh_interval(content: &str) -> u32 {
    if let Ok(table) = content.parse::<toml::Table>() {
        if let Some(toml::Value::Integer(hours)) = table.get("refresh_interval_hours") {
            if *hours >= 0 {
                return *hours as u32;
            }
        }
    }
    DEFAULT_REFRESH_INTERVAL_HOURS
}

/// Install corp config: write to ~/.capsem/corp.toml + corp-source.json.
pub fn install_corp_config(capsem_dir: &Path, content: &str, source: &CorpSource) -> Result<()> {
    std::fs::create_dir_all(capsem_dir)
        .context("cannot create ~/.capsem")?;

    let corp_path = capsem_dir.join("corp.toml");
    std::fs::write(&corp_path, content)
        .context("cannot write corp.toml")?;
    info!(path = %corp_path.display(), "installed corp config");

    write_corp_source(capsem_dir, source)
}

/// Read corp source metadata (returns None if no corp-source.json).
pub fn read_corp_source(capsem_dir: &Path) -> Option<CorpSource> {
    let path = capsem_dir.join("corp-source.json");
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Background refresh: if corp was provisioned from URL and TTL expired, re-fetch.
///
/// Uses conditional GET with If-None-Match (ETag) to avoid unnecessary downloads.
/// Fire-and-forget: errors are logged but not propagated.
pub async fn refresh_corp_config_if_stale(capsem_dir: PathBuf) {
    let source = match read_corp_source(&capsem_dir) {
        Some(s) => s,
        None => return,
    };

    let url = match &source.url {
        Some(u) => u.clone(),
        None => return, // Provisioned from local file
    };

    if source.refresh_interval_hours == 0 {
        return; // Refresh disabled
    }

    // Check TTL
    let age_secs = now_secs().saturating_sub(source.fetched_at);
    let ttl_secs = source.refresh_interval_hours as u64 * 3600;
    if age_secs < ttl_secs {
        return; // Not stale yet
    }

    let age_hours = age_secs / 3600;
    info!(url = %url, age_hours, "corp config stale, refreshing");

    let client = reqwest::Client::new();
    let mut req = client.get(&url).header("User-Agent", "capsem");
    if let Some(etag) = &source.etag {
        req = req.header("If-None-Match", etag);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "corp config refresh failed");
            return;
        }
    };

    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        let mut updated = source.clone();
        updated.fetched_at = now_secs();
        let _ = write_corp_source(&capsem_dir, &updated);
        return;
    }

    if !resp.status().is_success() {
        warn!(status = %resp.status(), "corp config refresh returned error");
        return;
    }

    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "failed to read refreshed corp config");
            return;
        }
    };

    if validate_corp_toml(&body).is_err() {
        warn!("refreshed corp config is invalid TOML, keeping existing");
        return;
    }

    let content_hash = blake3::hash(body.as_bytes()).to_hex().to_string();
    let new_source = CorpSource {
        url: Some(url),
        file_path: None,
        fetched_at: now_secs(),
        etag,
        content_hash,
        refresh_interval_hours: parse_refresh_interval(&body),
    };

    if let Err(e) = install_corp_config(&capsem_dir, &body, &new_source) {
        warn!(error = %e, "failed to install refreshed corp config");
    } else {
        info!("corp config refreshed successfully");
    }
}

/// Provision corp config from a URL: fetch, validate, install.
/// Convenience wrapper combining fetch + install for the service API.
pub async fn provision_from_source(capsem_dir: &Path, source_url: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let (body, etag) = fetch_corp_config(&client, source_url).await?;
    let content_hash = blake3::hash(body.as_bytes()).to_hex().to_string();
    let cs = CorpSource {
        url: Some(source_url.to_string()),
        file_path: None,
        fetched_at: now_secs(),
        etag,
        content_hash,
        refresh_interval_hours: parse_refresh_interval(&body),
    };
    install_corp_config(capsem_dir, &body, &cs)
}

/// Install corp config from inline TOML content (no URL fetch).
/// Convenience wrapper for the service API.
pub fn install_inline_corp_config(capsem_dir: &Path, toml_content: &str) -> Result<()> {
    validate_corp_toml(toml_content)?;
    let content_hash = blake3::hash(toml_content.as_bytes()).to_hex().to_string();
    let cs = CorpSource {
        url: None,
        file_path: None,
        fetched_at: now_secs(),
        etag: None,
        content_hash,
        refresh_interval_hours: parse_refresh_interval(toml_content),
    };
    install_corp_config(capsem_dir, toml_content, &cs)
}

/// Write just the corp-source.json.
fn write_corp_source(capsem_dir: &Path, source: &CorpSource) -> Result<()> {
    let path = capsem_dir.join("corp-source.json");
    let json = serde_json::to_string_pretty(source)
        .context("cannot serialize corp source")?;
    std::fs::write(&path, json).context("cannot write corp-source.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_valid_corp_toml() {
        let content = r#"
[settings]
"ai.anthropic.allow" = { value = true, modified = "2024-01-01T00:00:00Z" }
"#;
        let result = validate_corp_toml(content);
        assert!(result.is_ok());
        let file = result.unwrap();
        assert!(file.settings.contains_key("ai.anthropic.allow"));
    }

    #[test]
    fn test_validate_empty_corp_toml() {
        let result = validate_corp_toml("");
        assert!(result.is_ok());
        assert!(result.unwrap().settings.is_empty());
    }

    #[test]
    fn test_validate_invalid_toml_syntax() {
        let result = validate_corp_toml("this is not [ valid toml {{{");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid corp TOML"));
    }

    #[test]
    fn test_validate_toml_with_unknown_keys() {
        let content = r#"
[settings]
"future.setting.v99" = { value = "hello", modified = "2024-01-01T00:00:00Z" }
"#;
        assert!(validate_corp_toml(content).is_ok());
    }

    #[test]
    fn test_validate_toml_wrong_types() {
        // Raw string without SettingEntry wrapper should fail
        let content = r#"
[settings]
"ai.anthropic.allow" = "yes"
"#;
        assert!(validate_corp_toml(content).is_err());
    }

    #[test]
    fn test_refresh_interval_parsing() {
        assert_eq!(parse_refresh_interval("refresh_interval_hours = 12\n\n[settings]\n"), 12);
        assert_eq!(parse_refresh_interval("[settings]\n"), DEFAULT_REFRESH_INTERVAL_HOURS);
    }

    #[test]
    fn test_refresh_interval_zero_means_no_refresh() {
        assert_eq!(parse_refresh_interval("refresh_interval_hours = 0\n\n[settings]\n"), 0);
    }

    #[test]
    fn test_corp_source_roundtrip() {
        let source = CorpSource {
            url: Some("https://example.com/corp.toml".into()),
            file_path: None,
            fetched_at: 1718444400,
            etag: Some("\"abc123\"".into()),
            content_hash: "a".repeat(64),
            refresh_interval_hours: 12,
        };
        let json = serde_json::to_string(&source).unwrap();
        let rt: CorpSource = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.url, source.url);
        assert_eq!(rt.etag, source.etag);
        assert_eq!(rt.refresh_interval_hours, 12);
        assert_eq!(rt.fetched_at, 1718444400);
        assert!(rt.file_path.is_none());
    }
}
