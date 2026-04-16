//! Asset manager for downloading and verifying VM assets.
//!
//! VM assets (rootfs) are too large to bundle in the DMG. The asset manager
//! downloads them on first launch and verifies integrity via blake3 hashes.
//!
//! ## Versioning
//!
//! Binary version (`1.0.{timestamp}`) and asset version (`YYYY.MMDD.patch`)
//! are independent. The manifest tracks both with compatibility ranges
//! (`min_binary`, `min_assets`).
//!
//! ## Storage
//!
//! Flat `~/.capsem/assets/` with hash-based filenames
//! (`vmlinuz-{hash16}`, `rootfs-{hash16}.squashfs`). Same hash = same file =
//! natural dedup across asset versions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tracing::info;

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Validate a version string (no path traversal).
fn validate_version(version: &str) -> Result<()> {
    if version.is_empty() {
        bail!("version string is empty");
    }
    if version.contains("..") || version.contains('/') || version.contains('\\') {
        bail!("version contains path traversal: {version}");
    }
    Ok(())
}

/// Validate a filename (no path separators or traversal).
fn validate_filename(filename: &str) -> Result<()> {
    if filename.is_empty() {
        bail!("filename is empty");
    }
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        bail!("filename contains path traversal: {filename}");
    }
    Ok(())
}

/// Validate a blake3 hash string (exactly 64 hex characters).
fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid blake3 hash (expected 64 hex chars): {hash}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Manifest types
// ---------------------------------------------------------------------------

/// A single asset entry (keyed by logical name in the map).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetEntry {
    pub hash: String,
    pub size: u64,
}

/// An asset release.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetRelease {
    pub date: String,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated_date: Option<String>,
    /// Oldest binary version compatible with these assets.
    pub min_binary: String,
    /// Per-arch asset maps: arch -> { logical_name -> AssetEntry }.
    pub arches: HashMap<String, HashMap<String, AssetEntry>>,
}

/// A binary release.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryRelease {
    pub date: String,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated_date: Option<String>,
    /// Oldest asset version this binary can boot.
    pub min_assets: String,
}

/// The assets section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetsSection {
    pub current: String,
    pub releases: HashMap<String, AssetRelease>,
}

/// The binaries section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinariesSection {
    pub current: String,
    pub releases: HashMap<String, BinaryRelease>,
}

/// Manifest with orthogonal binary and asset version tracks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestV2 {
    pub format: u32,
    pub assets: AssetsSection,
    pub binaries: BinariesSection,
}

/// Resolved file paths for booting a VM.
#[derive(Debug, Clone)]
pub struct ResolvedAssets {
    pub kernel: PathBuf,
    pub initrd: PathBuf,
    pub rootfs: PathBuf,
    pub asset_version: String,
}

// ---------------------------------------------------------------------------
// Hash-based filename derivation
// ---------------------------------------------------------------------------

/// Derive a hash-based filename from a logical asset name and its blake3 hash.
///
/// Splits on the first `.` to get stem and extension:
/// - `"vmlinuz"` + `"2c0bd752..."` -> `"vmlinuz-2c0bd752db929642"`
/// - `"initrd.img"` + `"e5e910e9..."` -> `"initrd-e5e910e9ab38b873.img"`
/// - `"rootfs.squashfs"` + `"89eb92b8..."` -> `"rootfs-89eb92b83534d9d0.squashfs"`
pub fn hash_filename(logical_name: &str, hash: &str) -> String {
    let prefix = &hash[..16.min(hash.len())];
    if let Some(dot_pos) = logical_name.find('.') {
        let stem = &logical_name[..dot_pos];
        let ext = &logical_name[dot_pos..];
        format!("{stem}-{prefix}{ext}")
    } else {
        format!("{logical_name}-{prefix}")
    }
}

// ---------------------------------------------------------------------------
// ManifestV2 implementation
// ---------------------------------------------------------------------------

impl ManifestV2 {
    /// Parse a manifest from JSON.
    pub fn from_json(content: &str) -> Result<Self> {
        let manifest: ManifestV2 = serde_json::from_str(content)
            .context("failed to parse manifest JSON")?;
        if manifest.format != 2 {
            bail!("expected manifest format 2, got {}", manifest.format);
        }
        validate_version(&manifest.assets.current)?;
        validate_version(&manifest.binaries.current)?;
        for (version, release) in &manifest.assets.releases {
            validate_version(version)?;
            for (_arch, assets) in &release.arches {
                if assets.is_empty() {
                    bail!("asset release {version} has empty arch entry");
                }
                for (name, entry) in assets {
                    validate_filename(name)?;
                    validate_hash(&entry.hash)?;
                }
            }
        }
        for (version, _release) in &manifest.binaries.releases {
            validate_version(version)?;
        }
        Ok(manifest)
    }

    /// Resolve asset file paths for a given binary version and architecture.
    ///
    /// Finds the best compatible asset release and returns hash-based file paths.
    pub fn resolve(
        &self,
        binary_version: &str,
        arch: &str,
        base_dir: &Path,
    ) -> Result<ResolvedAssets> {
        // Find the asset version to use: prefer assets.current.
        // If the binary specifies min_assets, verify compatibility.
        let asset_version = if let Some(bin_rel) = self.binaries.releases.get(binary_version) {
            let min = &bin_rel.min_assets;
            if self.assets.current >= *min {
                self.assets.current.clone()
            } else {
                // Current assets are too old for this binary -- find newest compatible
                let mut best: Option<&str> = None;
                for (v, _rel) in &self.assets.releases {
                    if v.as_str() >= min.as_str() {
                        if best.is_none() || v.as_str() > best.unwrap() {
                            best = Some(v.as_str());
                        }
                    }
                }
                best.map(String::from).unwrap_or_else(|| self.assets.current.clone())
            }
        } else {
            // Binary version not in manifest -- use current assets
            self.assets.current.clone()
        };

        let release = self.assets.releases.get(&asset_version)
            .with_context(|| format!("asset version {} not found in manifest", asset_version))?;
        let arch_assets = release.arches.get(arch)
            .with_context(|| format!("arch {} not found in asset release {}", arch, asset_version))?;

        let resolve_one = |name: &str| -> Result<PathBuf> {
            let entry = arch_assets.get(name)
                .with_context(|| format!("{} not found in asset release {} / {}", name, asset_version, arch))?;
            Ok(base_dir.join(hash_filename(name, &entry.hash)))
        };

        Ok(ResolvedAssets {
            kernel: resolve_one("vmlinuz")?,
            initrd: resolve_one("initrd.img")?,
            rootfs: resolve_one("rootfs.squashfs")?,
            asset_version,
        })
    }

    /// Merge another manifest into this one, preserving existing entries.
    pub fn merge(&mut self, other: &ManifestV2) {
        for (version, entry) in &other.assets.releases {
            self.assets.releases
                .entry(version.clone())
                .or_insert_with(|| entry.clone());
        }
        if other.assets.current > self.assets.current {
            self.assets.current = other.assets.current.clone();
        }
        for (version, entry) in &other.binaries.releases {
            self.binaries.releases
                .entry(version.clone())
                .or_insert_with(|| entry.clone());
        }
        if other.binaries.current > self.binaries.current {
            self.binaries.current = other.binaries.current.clone();
        }
    }
}

/// Check if a JSON string is a v2 manifest (has `"format": 2`).
pub fn is_v2_manifest(content: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|v| v.get("format")?.as_u64())
        .map_or(false, |f| f == 2)
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Compute the blake3 hash of a file.
pub fn hash_file(path: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut file =
        std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut buf = [0u8; 256 * 1024];
    loop {
        use std::io::Read;
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Return the default assets directory: `~/.capsem/assets/`.
pub fn default_assets_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".capsem").join("assets"))
}

/// Build the GitHub Releases download base URL for the given version.
pub fn release_url(version: &str) -> String {
    format!("https://github.com/google/capsem/releases/download/v{version}")
}

// ---------------------------------------------------------------------------
// Cleanup
// ---------------------------------------------------------------------------

/// Remove hash-named asset files not referenced by any non-deprecated release.
/// Also removes legacy `v*/` directories.
///
/// Returns paths that were removed.
pub fn cleanup_unused_assets(
    base_dir: &Path,
    manifest: &ManifestV2,
) -> Result<Vec<PathBuf>> {
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (_version, release) in &manifest.assets.releases {
        if release.deprecated {
            continue;
        }
        for (_arch, assets) in &release.arches {
            for (name, entry) in assets {
                referenced.insert(hash_filename(name, &entry.hash));
            }
        }
    }

    let mut removed = Vec::new();
    if !base_dir.exists() {
        return Ok(removed);
    }

    for entry in std::fs::read_dir(base_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip manifest.json and hidden/temp files
        if name_str == "manifest.json" || name_str == "pinned.json"
            || name_str.starts_with('.') || name_str.ends_with(".tmp")
        {
            continue;
        }

        // Remove legacy v* directories
        if entry.file_type()?.is_dir() && name_str.starts_with('v') {
            info!(path = %entry.path().display(), "removing legacy versioned asset dir");
            std::fs::remove_dir_all(entry.path())?;
            removed.push(entry.path());
            continue;
        }

        // Remove hash-named files not referenced by any release
        if name_str.contains('-') && !referenced.contains(name_str.as_ref()) {
            info!(path = %entry.path().display(), "removing unreferenced asset");
            std::fs::remove_file(entry.path())?;
            removed.push(entry.path());
        }
    }

    Ok(removed)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_V2_MANIFEST: &str = r#"{
        "format": 2,
        "assets": {
            "current": "2026.0415.1",
            "releases": {
                "2026.0415.1": {
                    "date": "2026-04-15",
                    "deprecated": false,
                    "min_binary": "1.0.0",
                    "arches": {
                        "arm64": {
                            "vmlinuz": { "hash": "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c", "size": 7797248 },
                            "initrd.img": { "hash": "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456", "size": 2270154 },
                            "rootfs.squashfs": { "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 454230016 }
                        }
                    }
                }
            }
        },
        "binaries": {
            "current": "1.0.1776269479",
            "releases": {
                "1.0.1776269479": {
                    "date": "2026-04-15",
                    "deprecated": false,
                    "min_assets": "2026.0415.1"
                }
            }
        }
    }"#;

    #[test]
    fn manifest_parse() {
        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        assert_eq!(m.format, 2);
        assert_eq!(m.assets.current, "2026.0415.1");
        assert_eq!(m.binaries.current, "1.0.1776269479");
        assert_eq!(m.assets.releases.len(), 1);
        assert_eq!(m.binaries.releases.len(), 1);
        let rel = &m.assets.releases["2026.0415.1"];
        assert!(!rel.deprecated);
        assert_eq!(rel.min_binary, "1.0.0");
        let arm64 = &rel.arches["arm64"];
        assert_eq!(arm64.len(), 3);
        assert_eq!(arm64["vmlinuz"].size, 7797248);
    }

    #[test]
    fn manifest_resolve() {
        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();
        assert_eq!(resolved.asset_version, "2026.0415.1");
        assert!(resolved.kernel.to_str().unwrap().contains("vmlinuz-a65f925ebe0b0cc7"));
        assert!(resolved.initrd.to_str().unwrap().contains("initrd-cba052ee1e3fc7de.img"));
        assert!(resolved.rootfs.to_str().unwrap().contains("rootfs-b8199dc4a83069b9.squashfs"));
    }

    #[test]
    fn manifest_resolve_unknown_binary_uses_current_assets() {
        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let resolved = m.resolve("1.0.9999999999", "arm64", dir.path()).unwrap();
        assert_eq!(resolved.asset_version, "2026.0415.1");
    }

    #[test]
    fn hash_filename_cases() {
        assert_eq!(
            hash_filename("vmlinuz", "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c"),
            "vmlinuz-a65f925ebe0b0cc7"
        );
        assert_eq!(
            hash_filename("initrd.img", "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456"),
            "initrd-cba052ee1e3fc7de.img"
        );
        assert_eq!(
            hash_filename("rootfs.squashfs", "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee"),
            "rootfs-b8199dc4a83069b9.squashfs"
        );
    }

    #[test]
    fn is_v2_manifest_detection() {
        assert!(is_v2_manifest(SAMPLE_V2_MANIFEST));
        assert!(!is_v2_manifest(r#"{"latest":"0.9.0","releases":{}}"#));
        assert!(!is_v2_manifest("not json"));
    }

    #[test]
    fn manifest_rejects_wrong_format() {
        let json = SAMPLE_V2_MANIFEST.replace("\"format\": 2", "\"format\": 99");
        assert!(ManifestV2::from_json(&json).is_err());
    }

    #[test]
    fn manifest_merge() {
        let mut m1 = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let json2 = SAMPLE_V2_MANIFEST
            .replace("2026.0415.1", "2026.0416.1")
            .replace("1.0.1776269479", "1.0.1776300000");
        let m2 = ManifestV2::from_json(&json2).unwrap();
        m1.merge(&m2);
        assert_eq!(m1.assets.releases.len(), 2);
        assert_eq!(m1.binaries.releases.len(), 2);
        assert_eq!(m1.assets.current, "2026.0416.1");
        assert_eq!(m1.binaries.current, "1.0.1776300000");
    }

    #[test]
    fn version_traversal_rejected() {
        assert!(validate_version("../etc").is_err());
        assert!(validate_version("foo/bar").is_err());
        assert!(validate_version("").is_err());
        assert!(validate_version("0.9.0").is_ok());
    }

    #[test]
    fn filename_traversal_rejected() {
        assert!(validate_filename("../../x").is_err());
        assert!(validate_filename("foo/bar").is_err());
        assert!(validate_filename("").is_err());
        assert!(validate_filename("vmlinuz").is_ok());
    }

    #[test]
    fn hash_file_known_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test");
        std::fs::write(&path, b"hello world").unwrap();
        let h = hash_file(&path).unwrap();
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_file_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty");
        std::fs::write(&path, b"").unwrap();
        let h = hash_file(&path).unwrap();
        assert_eq!(h.len(), 64);
    }

    #[test]
    fn hash_file_nonexistent() {
        assert!(hash_file(Path::new("/nonexistent/file")).is_err());
    }

    #[test]
    fn default_assets_dir_under_home() {
        if let Some(dir) = default_assets_dir() {
            assert!(dir.to_str().unwrap().contains(".capsem/assets"));
        }
    }

    #[test]
    fn release_url_format() {
        assert_eq!(
            release_url("1.0.1776269479"),
            "https://github.com/google/capsem/releases/download/v1.0.1776269479"
        );
    }

    #[test]
    fn cleanup_removes_unreferenced_and_legacy_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        // Create a referenced hash-named file
        std::fs::write(base.join("vmlinuz-a65f925ebe0b0cc7"), b"kernel").unwrap();
        // Create an unreferenced hash-named file
        std::fs::write(base.join("vmlinuz-deadbeef12345678"), b"old").unwrap();
        // Create a legacy v* directory
        std::fs::create_dir(base.join("v0.16.1")).unwrap();
        // Create manifest.json (should be preserved)
        std::fs::write(base.join("manifest.json"), b"{}").unwrap();

        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let removed = cleanup_unused_assets(base, &m).unwrap();

        assert_eq!(removed.len(), 2);
        assert!(base.join("vmlinuz-a65f925ebe0b0cc7").exists());
        assert!(!base.join("vmlinuz-deadbeef12345678").exists());
        assert!(!base.join("v0.16.1").exists());
        assert!(base.join("manifest.json").exists());
    }

    #[test]
    fn cleanup_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let removed = cleanup_unused_assets(dir.path(), &m).unwrap();
        assert!(removed.is_empty());
    }

    #[test]
    fn cleanup_nonexistent_dir() {
        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let removed = cleanup_unused_assets(Path::new("/nonexistent"), &m).unwrap();
        assert!(removed.is_empty());
    }
}
