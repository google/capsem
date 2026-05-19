//! Self-update: check GitHub for new versions, prompt to update.
//!
//! Binary swap is still future work. Profile-owned VM asset updates are
//! delegated to the running service so `capsem update --assets` uses the same
//! Profile V2 asset reconciler as background checks.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::client::{ApiResponse, UdsClient};
use crate::platform::{self, InstallLayout};

/// Cached update check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheck {
    /// Unix timestamp of when we last checked.
    pub checked_at: u64,
    /// Latest version available (None if check failed).
    pub latest_version: Option<String>,
    /// Whether an update is available.
    pub update_available: bool,
}

const CACHE_TTL_SECS: u64 = 24 * 3600; // 24 hours

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn cache_path() -> Option<PathBuf> {
    crate::paths::capsem_home()
        .ok()
        .map(|d| d.join("update-check.json"))
}

/// Read cached update notice. Sync file read, no latency.
/// Returns a message to display if an update is available and cache is fresh.
pub fn read_cached_update_notice() -> Option<String> {
    let path = cache_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let check: UpdateCheck = serde_json::from_str(&content).ok()?;

    if !check.update_available {
        return None;
    }

    // Only show if cache is still fresh
    let age = now_secs().saturating_sub(check.checked_at);
    if age > CACHE_TTL_SECS {
        return None;
    }

    let current = env!("CARGO_PKG_VERSION");
    check.latest_version.and_then(|latest| {
        if is_newer(&latest, current) {
            Some(format!(
                "Update available: {} -> {}. Run `capsem update` to upgrade.",
                current, latest
            ))
        } else {
            None
        }
    })
}

/// Write update check cache atomically (write tmp + rename).
fn write_cache(check: &UpdateCheck) -> Result<()> {
    let path = cache_path().context("HOME not set")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(check)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Background refresh: check GitHub for updates if cache is stale.
/// Fire-and-forget via tokio::spawn.
pub async fn refresh_update_cache_if_stale() {
    let path = match cache_path() {
        Some(p) => p,
        None => return,
    };

    // Check if cache exists and is fresh
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(check) = serde_json::from_str::<UpdateCheck>(&content) {
            let age = now_secs().saturating_sub(check.checked_at);
            if age < CACHE_TTL_SECS {
                return; // Still fresh
            }
        }
    }

    info!("update cache stale, checking for updates");

    // Fetch latest release tag from GitHub API
    let client = reqwest::Client::new();
    let resp = match client
        .get("https://api.github.com/repos/google/capsem/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "capsem")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            warn!(status = %r.status(), "update check: GitHub API error");
            return;
        }
        Err(e) => {
            warn!(error = %e, "update check failed");
            let check = UpdateCheck {
                checked_at: now_secs(),
                latest_version: None,
                update_available: false,
            };
            let _ = write_cache(&check);
            return;
        }
    };

    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return,
    };

    let tag = match body.get("tag_name").and_then(|v| v.as_str()) {
        Some(t) => t.strip_prefix('v').unwrap_or(t).to_string(),
        None => return,
    };

    let current = env!("CARGO_PKG_VERSION");
    let update_available = is_newer(&tag, current);
    let check = UpdateCheck {
        checked_at: now_secs(),
        latest_version: Some(tag),
        update_available,
    };
    let _ = write_cache(&check);
}

/// Compare versions: is `latest` newer than `current`?
/// Returns false for malformed versions (conservative: don't prompt for bad data).
fn is_newer(latest: &str, current: &str) -> bool {
    match (
        semver::Version::parse(latest),
        semver::Version::parse(current),
    ) {
        (Ok(l), Ok(c)) => l > c,
        _ => false,
    }
}

/// Run the update flow.
///
/// With `assets = true`, refresh only the VM asset files referenced by the
/// locally-installed manifest. Binary swap is still scoped to the orthogonal
/// CI sprint and remains a "rebuild from source" step for dev builds.
pub async fn run_update(_yes: bool, assets: bool) -> Result<()> {
    let layout = platform::detect_install_layout();

    if assets {
        return refresh_assets().await;
    }

    if layout == InstallLayout::Development {
        println!("Development build detected. Update from source with `git pull && just install`.");
        return Ok(());
    }

    println!("Binary self-update is not yet wired up.");
    println!("Run `capsem update --assets` to refresh VM assets, or");
    println!("rebuild from source: `git pull && just install`.");
    Ok(())
}

/// Trigger the service-owned Profile V2 asset reconciler.
async fn refresh_assets() -> Result<()> {
    let sock = capsem_core::paths::capsem_run_dir().join("service.sock");
    let client = UdsClient::new(sock, true);
    let response: ApiResponse<serde_json::Value> = client
        .post("/setup/assets/reconcile", serde_json::json!({}))
        .await
        .context("request Profile V2 asset reconcile from service")?;
    let result = response.into_result()?;
    println!("{}", profile_asset_reconcile_summary_line(&result));
    Ok(())
}

fn profile_asset_reconcile_summary_line(result: &serde_json::Value) -> String {
    let outcome = result["outcome"].as_str().unwrap_or("unknown");
    let health = &result["health"];
    let state = health["state"].as_str().unwrap_or("unknown");
    let version = health["version"].as_str().unwrap_or("unknown");
    let arch = health["arch"].as_str().unwrap_or("unknown");
    match outcome {
        "already_ready" => format!("Profile VM assets already ready ({version}, {arch})."),
        "downloaded" => format!("Profile VM assets reconciled ({version}, {arch})."),
        "error" => {
            let error = health["error"].as_str().unwrap_or("unknown error");
            format!("Profile VM asset reconcile failed: {error}")
        }
        _ => format!("Profile VM asset reconcile {outcome} (state={state}, {version}, {arch})."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_semver() {
        assert!(is_newer("0.17.0", "0.16.1"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.16.1", "0.16.1"));
        assert!(!is_newer("0.16.0", "0.16.1"));
    }

    #[test]
    fn is_newer_rejects_garbage() {
        assert!(!is_newer("error", "0.16.1"));
        assert!(!is_newer("", "0.16.1"));
        assert!(!is_newer("not-a-version", "0.16.1"));
    }

    #[test]
    fn is_newer_rejects_malformed_current() {
        assert!(!is_newer("0.17.0", "garbage"));
    }

    #[test]
    fn is_newer_prerelease() {
        assert!(!is_newer("0.17.0-beta.1", "0.17.0"));
        assert!(is_newer("0.18.0-beta.1", "0.17.0"));
    }

    #[test]
    fn update_check_roundtrip() {
        let check = UpdateCheck {
            checked_at: 1718444400,
            latest_version: Some("0.17.0".into()),
            update_available: true,
        };
        let json = serde_json::to_string(&check).unwrap();
        let rt: UpdateCheck = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.latest_version, Some("0.17.0".into()));
        assert!(rt.update_available);
    }

    #[test]
    fn cache_ttl_constant() {
        assert_eq!(CACHE_TTL_SECS, 86400);
    }

    #[test]
    fn profile_asset_reconcile_summary_line_reports_downloaded() {
        let result = serde_json::json!({
            "outcome": "downloaded",
            "health": {
                "state": "ready",
                "version": "everyday-work@2026.0520.1",
                "arch": "arm64"
            }
        });

        assert_eq!(
            profile_asset_reconcile_summary_line(&result),
            "Profile VM assets reconciled (everyday-work@2026.0520.1, arm64)."
        );
    }

    #[test]
    fn profile_asset_reconcile_summary_line_reports_error() {
        let result = serde_json::json!({
            "outcome": "error",
            "health": {
                "state": "error",
                "error": "GET https://assets.example.test/rootfs returned 503"
            }
        });

        assert_eq!(
            profile_asset_reconcile_summary_line(&result),
            "Profile VM asset reconcile failed: GET https://assets.example.test/rootfs returned 503"
        );
    }
}
