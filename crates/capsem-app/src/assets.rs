use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use capsem_core::asset_manager::{self, AssetManager};
use tracing::{debug, debug_span, info, warn};

/// Return the host architecture name used for per-arch asset subdirectories.
fn host_arch() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        "arm64"
    }
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64"
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        "arm64"
    }
}

/// Check if a candidate assets directory contains vmlinuz, checking the
/// per-arch subdirectory first (e.g., `assets/arm64/vmlinuz`), then the
/// flat layout (`assets/vmlinuz`) for backward compatibility.
fn resolve_with_arch(candidate: &Path) -> Option<PathBuf> {
    // Per-arch layout: assets/{arch}/vmlinuz
    let arch_dir = candidate.join(host_arch());
    if arch_dir.join("vmlinuz").exists() {
        info!(
            path = %arch_dir.display(),
            arch = host_arch(),
            "found per-arch assets"
        );
        return Some(arch_dir);
    }
    // Flat layout: assets/vmlinuz (backward compat)
    if candidate.join("vmlinuz").exists() {
        return Some(candidate.to_path_buf());
    }
    None
}

/// Find the assets directory containing kernel, initrd, and rootfs.
///
/// For each candidate location, checks for per-arch subdirectory first
/// (e.g., `assets/arm64/vmlinuz`), then falls back to flat layout
/// (`assets/vmlinuz`) for backward compatibility.
///
/// Checks (in order):
/// 1. `CAPSEM_ASSETS_DIR` env var (development override)
/// 2. macOS .app bundle: `Contents/Resources/` (sibling of `Contents/MacOS/`)
/// 3. `./assets` (workspace root, for `cargo run`)
/// 4. `../../assets` (when CWD is `crates/capsem-app/`)
pub(crate) fn resolve_assets_dir() -> Result<PathBuf> {
    let _span = debug_span!("resolve_assets").entered();
    // 1. Explicit env var (development override)
    if let Ok(dir) = std::env::var("CAPSEM_ASSETS_DIR") {
        let p = PathBuf::from(&dir);
        if let Some(resolved) = resolve_with_arch(&p) {
            return Ok(resolved);
        }
    }

    // 2. macOS .app bundle: Contents/Resources/ (sibling of Contents/MacOS/)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(macos_dir) = exe.parent() {
            if let Some(resources) = macos_dir.parent().map(|p| p.join("Resources")) {
                let search_paths = [
                    resources.clone(),
                    resources.join("assets"),
                    // Tauri v2 relative structure fallback
                    resources.join("_up_/_up_/assets"),
                ];
                for path in search_paths {
                    if let Some(resolved) = resolve_with_arch(&path) {
                        info!(path = %resolved.display(), "found bundled assets");
                        return Ok(resolved);
                    }
                }
            }
        }
    }

    // 3. ./assets (workspace root, for `cargo run`)
    let cwd_assets = PathBuf::from("assets");
    if let Some(resolved) = resolve_with_arch(&cwd_assets) {
        return Ok(resolved);
    }

    // 4. ../../assets (when CWD is crates/capsem-app/)
    let parent_assets = PathBuf::from("../../assets");
    if let Some(resolved) = resolve_with_arch(&parent_assets) {
        return Ok(resolved);
    }

    Err(anyhow::anyhow!(
        "VM assets not found. Set CAPSEM_ASSETS_DIR or run from workspace root."
    ))
}

/// Resolve rootfs path, checking bundled assets first, then versioned download dir,
/// then legacy flat download dir.
pub(crate) fn resolve_rootfs(bundled_assets: &Path) -> Option<PathBuf> {
    let bundled = bundled_assets.join("rootfs.squashfs");
    if bundled.exists() {
        info!(path = %bundled.display(), "rootfs found in app bundle");
        return Some(bundled);
    }
    debug!(path = %bundled.display(), "rootfs not in app bundle");
    if let Some(download_dir) = asset_manager::default_assets_dir() {
        // Check versioned directory first.
        let version = env!("CARGO_PKG_VERSION");
        let versioned = download_dir.join(format!("v{version}")).join("rootfs.squashfs");
        if versioned.exists() {
            info!(path = %versioned.display(), "rootfs found (versioned download)");
            return Some(versioned);
        }
        debug!(path = %versioned.display(), "rootfs not in versioned dir");
        // Fallback to legacy flat layout.
        let downloaded = download_dir.join("rootfs.squashfs");
        if downloaded.exists() {
            info!(path = %downloaded.display(), "rootfs found (legacy flat layout)");
            return Some(downloaded);
        }
        debug!(path = %downloaded.display(), "rootfs not in flat layout");
    } else {
        warn!("cannot determine assets download dir (HOME not set?)");
    }
    info!("rootfs not found locally, download required");
    None
}

/// Load manifest from bundled assets and create an AssetManager.
///
/// Tries manifest.json first (multi-version), falls back to B3SUMS (legacy).
/// Uses version-scoped directories: `~/.capsem/assets/v{version}/`.
///
/// For per-arch layouts, `bundled_assets` is `assets/{arch}/` so the manifest
/// is searched both in the assets dir and its parent (where manifest.json lives
/// at `assets/manifest.json`).
pub(crate) fn create_asset_manager(bundled_assets: &Path) -> Result<AssetManager> {
    let version = env!("CARGO_PKG_VERSION");
    let download_dir = asset_manager::default_assets_dir()
        .context("cannot determine home directory")?;
    info!(version, download_dir = %download_dir.display(), "initializing asset manager");

    // Try manifest.json -- check both bundled dir and parent (per-arch layout).
    let candidates = [
        bundled_assets.join("manifest.json"),
        bundled_assets
            .parent()
            .map(|p| p.join("manifest.json"))
            .unwrap_or_default(),
    ];

    for manifest_path in &candidates {
        if !manifest_path.exists() {
            continue;
        }
        info!(path = %manifest_path.display(), "loading manifest.json");
        let content = std::fs::read_to_string(manifest_path)
            .context("failed to read manifest.json")?;

        // Use arch-aware parsing to handle per-arch manifest format.
        let manifest = asset_manager::Manifest::from_json_for_arch(&content, host_arch())
            .or_else(|_| asset_manager::Manifest::from_json(&content))
            .context("invalid manifest.json")?;
        info!(
            releases = manifest.releases.len(),
            latest = %manifest.latest,
            "manifest parsed"
        );

        // Migrate flat layout if present.
        let _ = asset_manager::migrate_flat_layout(&download_dir, version);

        return AssetManager::from_manifest(&manifest, version, download_dir, Some(host_arch()));
    }

    // Fall back to legacy B3SUMS (check both bundled dir and parent).
    let b3sums_candidates = [
        bundled_assets.join("B3SUMS"),
        bundled_assets
            .parent()
            .map(|p| p.join("B3SUMS"))
            .unwrap_or_default(),
    ];

    for b3sums_path in &b3sums_candidates {
        if !b3sums_path.exists() {
            continue;
        }
        info!(path = %b3sums_path.display(), "loading legacy B3SUMS");
        let b3sums_content = std::fs::read_to_string(b3sums_path)
            .context("failed to read B3SUMS")?;
        let base_url = asset_manager::release_url(version);
        return AssetManager::new(download_dir, base_url, &b3sums_content);
    }

    Err(anyhow::anyhow!(
        "neither manifest.json nor B3SUMS found in {}",
        bundled_assets.display()
    ))
}

/// Find the rootfs filename in the manifest.
pub(crate) fn rootfs_manifest_name(mgr: &AssetManager) -> Result<String> {
    mgr.manifest_filenames()
        .into_iter()
        .find(|f| f.starts_with("rootfs"))
        .map(String::from)
        .context("no rootfs entry in B3SUMS manifest")
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- resolve_with_arch tests --

    #[test]
    fn resolve_with_arch_per_arch_layout() {
        let dir = tempfile::tempdir().unwrap();
        let arch_dir = dir.path().join(host_arch());
        std::fs::create_dir_all(&arch_dir).unwrap();
        std::fs::write(arch_dir.join("vmlinuz"), b"kernel").unwrap();
        let result = resolve_with_arch(dir.path());
        assert_eq!(result, Some(arch_dir));
    }

    #[test]
    fn resolve_with_arch_flat_layout() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("vmlinuz"), b"kernel").unwrap();
        let result = resolve_with_arch(dir.path());
        assert_eq!(result, Some(dir.path().to_path_buf()));
    }

    #[test]
    fn resolve_with_arch_prefers_per_arch_over_flat() {
        let dir = tempfile::tempdir().unwrap();
        let arch_dir = dir.path().join(host_arch());
        std::fs::create_dir_all(&arch_dir).unwrap();
        std::fs::write(arch_dir.join("vmlinuz"), b"arch kernel").unwrap();
        std::fs::write(dir.path().join("vmlinuz"), b"flat kernel").unwrap();
        let result = resolve_with_arch(dir.path());
        assert_eq!(result, Some(arch_dir), "per-arch layout should be preferred over flat");
    }

    #[test]
    fn resolve_with_arch_empty_dir_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve_with_arch(dir.path()), None);
    }

    #[test]
    fn resolve_with_arch_wrong_arch_subdir_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let wrong = if host_arch() == "arm64" { "x86_64" } else { "arm64" };
        let wrong_dir = dir.path().join(wrong);
        std::fs::create_dir_all(&wrong_dir).unwrap();
        std::fs::write(wrong_dir.join("vmlinuz"), b"kernel").unwrap();
        assert_eq!(resolve_with_arch(dir.path()), None);
    }

    #[test]
    fn resolve_with_arch_no_vmlinuz_in_arch_dir() {
        let dir = tempfile::tempdir().unwrap();
        let arch_dir = dir.path().join(host_arch());
        std::fs::create_dir_all(&arch_dir).unwrap();
        std::fs::write(arch_dir.join("initrd.img"), b"initrd").unwrap();
        assert_eq!(resolve_with_arch(dir.path()), None);
    }

    // -- resolve_rootfs tests --

    #[test]
    fn resolve_rootfs_bundled_exists() {
        let dir = tempfile::tempdir().unwrap();
        let rootfs = dir.path().join("rootfs.squashfs");
        std::fs::write(&rootfs, b"rootfs").unwrap();
        let result = resolve_rootfs(dir.path());
        assert_eq!(result, Some(rootfs));
    }

    #[test]
    fn resolve_rootfs_bundled_missing_checks_download_dirs() {
        let dir = tempfile::tempdir().unwrap();
        // No rootfs in bundled dir -- result depends on ~/.capsem/assets/ state
        let result = resolve_rootfs(dir.path());
        // Bundled path should NOT be returned
        assert_ne!(result, Some(dir.path().join("rootfs.squashfs")));
    }
}
