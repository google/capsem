//! Corp config provisioning from URL sources.
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
    /// URL the config was fetched from (None if provisioned from inline TOML).
    pub url: Option<String>,
    /// Deprecated legacy field. Local config sources must use file:// URLs.
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
    let parsed = reqwest::Url::parse(url).with_context(|| {
        format!("corp config source must be a URL: use https://..., http://..., or file:///absolute/path, got {url}")
    })?;
    info!(url = %url, "fetching corp config");

    if parsed.scheme() == "file" {
        if !has_scheme_authority_prefix(url, "file") {
            anyhow::bail!("corp config file URL must start with file://: {url}");
        }
        let path = parsed
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("corp config file URL must be absolute: {url}"))?;
        let body = std::fs::read_to_string(&path).with_context(|| format!("read corp config {}", path.display()))?;
        validate_corp_toml(&body)?;
        return Ok((body, None));
    }

    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!(
            "unsupported corp config URL scheme {}: use https://, http://, or file://",
            parsed.scheme()
        );
    }
    if !has_scheme_authority_prefix(url, parsed.scheme()) {
        anyhow::bail!("corp config source must use https://, http://, or file:// URLs, got {url}");
    }

    let resp = client
        .get(parsed)
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

fn has_scheme_authority_prefix(value: &str, scheme: &str) -> bool {
    let prefix = format!("{scheme}://");
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(&prefix))
}

/// Validate that a string is valid corp TOML (parseable as SettingsFile).
pub fn validate_corp_toml(content: &str) -> Result<SettingsFile> {
    let file: SettingsFile = toml::from_str(content).context("invalid corp TOML")?;
    super::loader::reject_retired_ai_setting_ids_in_content("corp TOML", content).map_err(anyhow::Error::msg)?;
    Ok(file)
}

/// Parse refresh_policy from corp TOML content.
/// Returns DEFAULT_REFRESH_INTERVAL_HOURS if not present or unparseable.
pub fn parse_refresh_interval(content: &str) -> u32 {
    if let Ok(table) = content.parse::<toml::Table>() {
        if let Some(toml::Value::String(policy)) = table.get("refresh_policy") {
            let Some(hours) = policy.strip_suffix('h') else {
                return DEFAULT_REFRESH_INTERVAL_HOURS;
            };
            if let Ok(hours) = hours.parse::<u32>() {
                return hours;
            }
        }
    }
    DEFAULT_REFRESH_INTERVAL_HOURS
}

/// Install corp config: write to ~/.capsem/corp.toml + corp-source.json.
pub fn install_corp_config(capsem_dir: &Path, content: &str, source: &CorpSource) -> Result<()> {
    std::fs::create_dir_all(capsem_dir).context("cannot create ~/.capsem")?;

    let corp_path = capsem_dir.join("corp.toml");
    std::fs::write(&corp_path, content).context("cannot write corp.toml")?;
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
    let ttl_secs = u64::from(source.refresh_interval_hours) * 3600;
    if age_secs < ttl_secs {
        return; // Not stale yet
    }

    let age_hours = age_secs / 3600;
    info!(url = %url, age_hours, "corp config stale, refreshing");

    let client = reqwest::Client::new();
    if url.starts_with("file://") {
        match fetch_corp_config(&client, &url).await {
            Ok((body, _)) => {
                let content_hash = blake3::hash(body.as_bytes()).to_hex().to_string();
                let new_source = CorpSource {
                    url: Some(url),
                    file_path: None,
                    fetched_at: now_secs(),
                    etag: None,
                    content_hash,
                    refresh_interval_hours: parse_refresh_interval(&body),
                };
                if let Err(e) = install_corp_config(&capsem_dir, &body, &new_source) {
                    warn!(error = %e, "failed to install refreshed corp config");
                } else {
                    info!("corp config refreshed successfully");
                }
            }
            Err(e) => warn!(error = %e, "corp config refresh failed"),
        }
        return;
    }

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
    let json = serde_json::to_string_pretty(source).context("cannot serialize corp source")?;
    std::fs::write(&path, json).context("cannot write corp-source.json")
}

#[cfg(test)]
mod tests;
