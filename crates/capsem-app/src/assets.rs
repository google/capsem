use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use capsem_core::asset_manager::{self, AssetManager};
use tracing::{debug, debug_span, info, warn};

/// Find the assets directory containing kernel, initrd, and rootfs.
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
        let p = PathBuf::from(dir);
        if p.join("vmlinuz").exists() {
            return Ok(p);
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
                    if path.join("vmlinuz").exists() {
                        info!(path = %path.display(), "found bundled assets");
                        return Ok(path);
                    }
                }
            }
        }
    }

    // 3. ./assets (workspace root, for `cargo run`)
    let cwd_assets = PathBuf::from("assets");
    if cwd_assets.join("vmlinuz").exists() {
        return Ok(cwd_assets);
    }

    // 4. ../../assets (when CWD is crates/capsem-app/)
    let parent_assets = PathBuf::from("../../assets");
    if parent_assets.join("vmlinuz").exists() {
        return Ok(parent_assets);
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
pub(crate) fn create_asset_manager(bundled_assets: &Path) -> Result<AssetManager> {
    let version = env!("CARGO_PKG_VERSION");
    let download_dir = asset_manager::default_assets_dir()
        .context("cannot determine home directory")?;
    info!(version, download_dir = %download_dir.display(), "initializing asset manager");

    // Try manifest.json first (new multi-version format).
    let manifest_path = bundled_assets.join("manifest.json");
    if manifest_path.exists() {
        info!(path = %manifest_path.display(), "loading manifest.json");
        let content = std::fs::read_to_string(&manifest_path)
            .context("failed to read manifest.json")?;
        let manifest = asset_manager::Manifest::from_json(&content)
            .context("invalid manifest.json")?;
        info!(
            releases = manifest.releases.len(),
            latest = %manifest.latest,
            "manifest parsed"
        );

        // Migrate flat layout if present.
        let _ = asset_manager::migrate_flat_layout(&download_dir, version);

        return AssetManager::from_manifest(&manifest, version, download_dir);
    }

    // Fall back to legacy B3SUMS.
    let b3sums_path = bundled_assets.join("B3SUMS");
    info!(path = %b3sums_path.display(), "manifest.json not found, trying legacy B3SUMS");
    let b3sums_content = std::fs::read_to_string(&b3sums_path)
        .context("neither manifest.json nor B3SUMS found in app bundle")?;
    let base_url = asset_manager::release_url(version);
    AssetManager::new(download_dir, base_url, &b3sums_content)
}

/// Find the rootfs filename in the manifest.
pub(crate) fn rootfs_manifest_name(mgr: &AssetManager) -> Result<String> {
    mgr.manifest_filenames()
        .into_iter()
        .find(|f| f.starts_with("rootfs"))
        .map(String::from)
        .context("no rootfs entry in B3SUMS manifest")
}
