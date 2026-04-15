//! Self-update: check GitHub, download new binaries + assets, restart service.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

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
    crate::paths::capsem_home().ok().map(|d| d.join("update-check.json"))
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

    let client = reqwest::Client::new();
    match capsem_core::asset_manager::fetch_latest_manifest(&client).await {
        Ok((latest_version, _manifest)) => {
            let current = env!("CARGO_PKG_VERSION");
            let update_available = is_newer(&latest_version, current);
            let check = UpdateCheck {
                checked_at: now_secs(),
                latest_version: Some(latest_version),
                update_available,
            };
            let _ = write_cache(&check);
        }
        Err(e) => {
            warn!(error = %e, "update check failed");
            // Write cache anyway so we don't hammer GitHub on every command
            let check = UpdateCheck {
                checked_at: now_secs(),
                latest_version: None,
                update_available: false,
            };
            let _ = write_cache(&check);
        }
    }
}

/// Compare versions: is `latest` newer than `current`?
/// Returns false for malformed versions (conservative: don't prompt for bad data).
fn is_newer(latest: &str, current: &str) -> bool {
    match (semver::Version::parse(latest), semver::Version::parse(current)) {
        (Ok(l), Ok(c)) => l > c,
        _ => false,
    }
}

/// Run the update flow.
pub async fn run_update(yes: bool) -> Result<()> {
    let layout = platform::detect_install_layout();
    if layout == InstallLayout::Development {
        println!("Development build detected. Update from source with `git pull && just install`.");
        return Ok(());
    }

    println!("Checking for updates...");
    let client = reqwest::Client::new();
    let (latest_version, manifest) = capsem_core::asset_manager::fetch_latest_manifest(&client)
        .await
        .context("failed to check for updates")?;

    let current = env!("CARGO_PKG_VERSION");
    if !is_newer(&latest_version, current) {
        println!("Already up to date (v{}).", current);
        // Update cache
        let check = UpdateCheck {
            checked_at: now_secs(),
            latest_version: Some(latest_version),
            update_available: false,
        };
        let _ = write_cache(&check);
        return Ok(());
    }

    println!("Update available: {} -> {}", current, latest_version);

    if !yes {
        let confirm = inquire::Confirm::new("Install update?")
            .with_default(true)
            .prompt()
            .context("update cancelled")?;
        if !confirm {
            println!("Update cancelled.");
            return Ok(());
        }
    }

    // Download phase: download new assets
    println!("Downloading v{}...", latest_version);

    let assets_base_dir = capsem_core::asset_manager::default_assets_dir()
        .context("cannot determine assets directory")?;
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };

    let am = capsem_core::asset_manager::AssetManager::from_manifest(
        &manifest,
        &latest_version,
        assets_base_dir.clone(),
        Some(arch),
    )?;

    let statuses = am.check_all()?;
    let needs_download: Vec<_> = statuses.iter()
        .filter(|(_, s)| matches!(s, capsem_core::asset_manager::AssetStatus::NeedsDownload { .. }))
        .collect();

    if !needs_download.is_empty() {
        println!("Downloading {} asset(s)...", needs_download.len());
        for (filename, _) in &needs_download {
            let fname = filename.clone();
            am.download_asset(filename, &client, move |p| {
                if p.total_bytes > 0 {
                    let pct = (p.bytes_downloaded as f64 / p.total_bytes as f64 * 100.0) as u32;
                    if pct.is_multiple_of(25) {
                        eprint!("\r  {} {}%", fname, pct);
                    }
                }
            }).await.with_context(|| format!("failed to download {}", filename))?;
            eprintln!();
        }
    }

    // TODO: Binary download and swap phase requires release infrastructure (WB6).
    // For now, assets are updated and the binary swap is a no-op placeholder.
    println!("Assets updated to v{}.", latest_version);
    println!("Binary update requires release infrastructure (WB6). Rebuild from source for now.");

    // Pin the running binary's version so cleanup doesn't delete its assets.
    // Until WB6 (binary swap), the binary is still `current` even after
    // downloading `latest_version` assets.
    let pinned_path = assets_base_dir.join("pinned.json");
    let mut pinned: std::collections::HashSet<String> = std::fs::read_to_string(&pinned_path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default();
    pinned.insert(current.to_string());
    let _ = std::fs::write(&pinned_path, serde_json::to_string(&pinned).unwrap_or_default());

    let removed = capsem_core::asset_manager::cleanup_old_versions(
        &assets_base_dir,
        &latest_version,
        &[],
    )?;
    if !removed.is_empty() {
        println!("Cleaned up {} old asset version(s).", removed.len());
    }

    // Update cache
    let check = UpdateCheck {
        checked_at: now_secs(),
        latest_version: Some(latest_version),
        update_available: false,
    };
    let _ = write_cache(&check);

    Ok(())
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
        // Malformed version from server should not trigger update
        assert!(!is_newer("error", "0.16.1"));
        assert!(!is_newer("", "0.16.1"));
        assert!(!is_newer("not-a-version", "0.16.1"));
    }

    #[test]
    fn is_newer_rejects_malformed_current() {
        // If our own version is somehow malformed, don't update
        assert!(!is_newer("0.17.0", "garbage"));
    }

    #[test]
    fn is_newer_prerelease() {
        // Pre-release of same version should not be "newer" than the release
        assert!(!is_newer("0.17.0-beta.1", "0.17.0"));
        // But pre-release of a higher version IS newer than current
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
        assert_eq!(CACHE_TTL_SECS, 86400); // 24 hours
    }
}
