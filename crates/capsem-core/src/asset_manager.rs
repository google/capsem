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
//! (`vmlinuz-{hash16}`, `rootfs-{hash16}.erofs`). Same hash = same file =
//! natural dedup across asset versions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
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

fn validate_sha256(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid sha256 hash (expected 64 hex chars): {hash}");
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
    pub size: u64,
}

/// An asset release.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetRelease {
    /// Build date (YYYY-MM-DD). Pure metadata. Optional because the CI
    /// release-pipeline writer historically omitted it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub date: String,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated_date: Option<String>,
    /// Oldest binary version compatible with these assets. Optional; when set,
    /// runtime asset selection refuses this release for older binaries.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub min_binary: String,
    /// Per-arch asset maps: arch -> { logical_name -> AssetEntry }.
    pub arches: HashMap<String, HashMap<String, AssetEntry>>,
}

/// A binary release.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryRelease {
    /// Build date (YYYY-MM-DD). Pure metadata. Optional because the CI
    /// release-pipeline writer omits it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub date: String,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated_date: Option<String>,
    /// Oldest asset version this binary can boot. Optional -- when empty,
    /// `pick_asset_version` falls back to `assets.current`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub min_assets: String,
    /// Echo of the version key (release.yaml writes this; harmless).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// pkg/deb metadata published by the release pipeline. Not consulted
    /// at runtime; preserved on round-trip so external tooling can read it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<BinaryFile>,
}

/// One downloadable binary asset (e.g. .pkg, .deb) listed under a
/// `BinaryRelease`. Metadata only -- the runtime resolver never reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blake3: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binaries: Vec<BinaryExecutable>,
}

/// One executable file contained inside a host package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryExecutable {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub installed_path: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blake3: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sbom_component_ref: String,
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
    pub refresh_policy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_base: Option<String>,
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

/// BLAKE3 hashes for the three canonical boot assets of one arch.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedAssetHashes {
    pub kernel: String,
    pub initrd: String,
    pub rootfs: String,
}

/// Map `std::env::consts::ARCH` names to the keys used under
/// `manifest.assets.releases.<ver>.arches`. Unknown arches pass through.
pub fn map_rustc_arch_to_manifest(rustc_arch: &str) -> &str {
    match rustc_arch {
        "aarch64" => "arm64",
        other => other,
    }
}

/// Host arch as a manifest key (e.g. "arm64", "x86_64").
pub fn host_manifest_arch() -> &'static str {
    map_rustc_arch_to_manifest(std::env::consts::ARCH)
}

const ROOTFS_ASSET_NAMES: [&str; 1] = ["rootfs.erofs"];

fn canonical_rootfs_asset_name(assets: &HashMap<String, AssetEntry>) -> Option<&'static str> {
    ROOTFS_ASSET_NAMES
        .iter()
        .copied()
        .find(|name| assets.contains_key(*name))
}

/// Load `manifest.json` from the assets dir (installed layout) or its parent
/// (dev tree layout where `assets` is already `assets/<arch>/`). Returns
/// `None` on missing file, read error, parse error, or schema mismatch --
/// profile-selected asset hashes remain the runtime authority.
pub fn load_manifest_for_assets(assets: &Path) -> Option<ManifestV2> {
    let mut candidates: Vec<PathBuf> = vec![assets.join("manifest.json")];
    if let Some(parent) = assets.parent() {
        candidates.push(parent.join("manifest.json"));
    }
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => match ManifestV2::from_json(&content) {
                Ok(m) => return Some(m),
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "manifest parse failed");
                    return None;
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "manifest read failed");
                return None;
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Hash-based filename derivation
// ---------------------------------------------------------------------------

/// Derive a hash-based filename from a logical asset name and its blake3 hash.
///
/// Splits on the first `.` to get stem and extension:
/// - `"vmlinuz"` + `"2c0bd752..."` -> `"vmlinuz-2c0bd752db929642"`
/// - `"initrd.img"` + `"e5e910e9..."` -> `"initrd-e5e910e9ab38b873.img"`
/// - `"rootfs.erofs"` + `"89eb92b8..."` -> `"rootfs-89eb92b83534d9d0.erofs"`
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
        let manifest: ManifestV2 =
            serde_json::from_str(content).context("failed to parse manifest JSON")?;
        if manifest.format != 2 {
            bail!("expected manifest format 2, got {}", manifest.format);
        }
        if manifest.refresh_policy.trim().is_empty() {
            bail!("manifest refresh_policy must not be empty");
        }
        validate_version(&manifest.assets.current)?;
        validate_version(&manifest.binaries.current)?;
        for (version, release) in &manifest.assets.releases {
            validate_version(version)?;
            for assets in release.arches.values() {
                if assets.is_empty() {
                    bail!("asset release {version} has empty arch entry");
                }
                for (name, entry) in assets {
                    validate_filename(name)?;
                    validate_hash(&entry.hash)?;
                    if !entry.sha256.is_empty() {
                        validate_sha256(&entry.sha256)?;
                    }
                }
            }
        }
        for version in manifest.binaries.releases.keys() {
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
        let asset_version = pick_asset_version(self, binary_version)?;

        let release =
            self.assets.releases.get(&asset_version).with_context(|| {
                format!("asset version {} not found in manifest", asset_version)
            })?;
        let arch_assets = release.arches.get(arch).with_context(|| {
            format!("arch {} not found in asset release {}", arch, asset_version)
        })?;

        let resolve_one = |name: &str| -> Result<PathBuf> {
            let entry = arch_assets.get(name).with_context(|| {
                format!(
                    "{} not found in asset release {} / {}",
                    name, asset_version, arch
                )
            })?;
            let hname = hash_filename(name, &entry.hash);
            // Check flat layout first (base_dir/{hash}), then arch subdir (base_dir/{arch}/{hash})
            let flat = base_dir.join(&hname);
            if flat.exists() {
                return Ok(flat);
            }
            let arch_path = base_dir.join(arch).join(&hname);
            if arch_path.exists() {
                return Ok(arch_path);
            }
            // Return the flat path (caller will report the error)
            Ok(flat)
        };
        let rootfs_name = canonical_rootfs_asset_name(arch_assets).with_context(|| {
            format!(
                "rootfs not found in asset release {} / {}",
                asset_version, arch
            )
        })?;

        Ok(ResolvedAssets {
            kernel: resolve_one("vmlinuz")?,
            initrd: resolve_one("initrd.img")?,
            rootfs: resolve_one(rootfs_name)?,
            asset_version,
        })
    }

    /// Expected hashes for the canonical boot triple (kernel/initrd/rootfs)
    /// from the current asset release on the given arch. Returns `None` if
    /// the current release or arch entry is missing, or if any of the three
    /// canonical filenames is absent from that arch's asset map.
    pub fn expected_hashes_current(&self, arch: &str) -> Option<ExpectedAssetHashes> {
        let release = self.assets.releases.get(&self.assets.current)?;
        let assets = release.arches.get(arch)?;
        Some(ExpectedAssetHashes {
            kernel: assets.get("vmlinuz")?.hash.clone(),
            initrd: assets.get("initrd.img")?.hash.clone(),
            rootfs: assets
                .get(canonical_rootfs_asset_name(assets)?)?
                .hash
                .clone(),
        })
    }

    /// Merge another manifest into this one, preserving existing entries.
    pub fn merge(&mut self, other: &ManifestV2) {
        for (version, entry) in &other.assets.releases {
            self.assets
                .releases
                .entry(version.clone())
                .or_insert_with(|| entry.clone());
        }
        if other.assets.current > self.assets.current {
            self.assets.current = other.assets.current.clone();
        }
        for (version, entry) in &other.binaries.releases {
            self.binaries
                .releases
                .entry(version.clone())
                .or_insert_with(|| entry.clone());
        }
        if other.binaries.current > self.binaries.current {
            self.binaries.current = other.binaries.current.clone();
        }
    }
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

/// Return the default assets directory.
///
/// Resolves via [`crate::paths::capsem_home_opt`], so the `CAPSEM_HOME` /
/// `CAPSEM_ASSETS_DIR` env overrides are honored.
pub fn default_assets_dir() -> Option<PathBuf> {
    // Honor CAPSEM_ASSETS_DIR first, then <capsem_home>/assets.
    if let Ok(v) = std::env::var("CAPSEM_ASSETS_DIR") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    crate::paths::capsem_home_opt().map(|h| h.join("assets"))
}

/// Build the GitHub Releases download base URL for the given **binary**
/// version.
///
/// This is retained for binary update/download metadata. VM assets use
/// [`asset_release_base_url`] so the asset track can move independently of tag
/// releases.
pub fn release_url(binary_version: &str) -> String {
    let base = std::env::var("CAPSEM_RELEASE_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://github.com/google/capsem/releases/download".into());
    format!("{}/v{binary_version}", base.trim_end_matches('/'))
}

/// Default immutable VM asset blob base.
///
/// The stable channel manifest lives at
/// `https://release.capsem.org/assets/stable/manifest.json`, while blobs live
/// under `assets/releases/<asset-version>/...` so older manifests continue to
/// hydrate even after `stable` advances.
pub fn asset_release_base_url() -> String {
    std::env::var("CAPSEM_ASSET_BASE_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://release.capsem.org/assets/releases".into())
        .trim_end_matches('/')
        .to_string()
}

/// Derive the immutable asset blob base from a manifest URL.
///
/// Canonical channel manifests use `<prefix>/assets/<channel>/manifest.json`
/// and resolve blobs from `<prefix>/assets/releases/<asset-version>/...`.
pub fn asset_release_base_url_from_manifest_url(manifest_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(manifest_url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let mut segments: Vec<String> = url
        .path_segments()
        .map(|segments| segments.map(ToOwned::to_owned).collect())
        .unwrap_or_default();
    if segments.len() < 3 || segments.last().map(String::as_str) != Some("manifest.json") {
        return None;
    }
    let channel_index = segments.len() - 2;
    if channel_index == 0 || segments[channel_index - 1] != "assets" {
        return None;
    }
    segments.truncate(channel_index);
    segments.push("releases".to_string());
    let mut out = url;
    out.set_path(&segments.join("/"));
    Some(out.as_str().trim_end_matches('/').to_string())
}

/// Derive a remote asset blob base from `manifest-origin.json`, when present.
pub fn asset_release_base_url_from_manifest_origin(assets_dir: &Path) -> Result<Option<String>> {
    let origin_path = assets_dir.join("manifest-origin.json");
    if !origin_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&origin_path)
        .with_context(|| format!("read {}", origin_path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("parse {}", origin_path.display()))?;
    let Some(source) = value.get("source").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    Ok(asset_release_base_url_from_manifest_url(source))
}

fn remote_asset_release_base_url(manifest: &ManifestV2, assets_dir: &Path) -> Result<String> {
    let asset_base_url = manifest
        .asset_base
        .clone()
        .or(asset_release_base_url_from_manifest_origin(assets_dir)?)
        .unwrap_or_else(asset_release_base_url);
    let asset_base_url = asset_base_url.trim_end_matches('/').to_string();
    let validation_url = asset_base_url.replace("{asset_version}", "0");
    let parsed = reqwest::Url::parse(&validation_url).map_err(|_| {
        anyhow::anyhow!(
            "asset base URL must be a URL: use https://... or http://..., got {asset_base_url}"
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!(
            "unsupported asset base URL scheme {}: use https:// or http://",
            parsed.scheme()
        );
    }
    Ok(asset_base_url)
}

/// Full per-asset download URL:
/// `{asset_release_base_url}/{asset_version}/{arch}-{logical_name}`.
///
/// Single source of truth for the URL `download_missing_assets` constructs.
/// Pinned by unit tests so the layout the binary fetches stays in lock-step
/// with the layout `release-assets.yaml` deploys.
pub fn asset_download_url(asset_version: &str, arch: &str, logical_name: &str) -> String {
    asset_download_url_with_base(&asset_release_base_url(), asset_version, arch, logical_name)
}

pub fn asset_download_url_with_base(
    asset_base_url: &str,
    asset_version: &str,
    arch: &str,
    logical_name: &str,
) -> String {
    let asset_base_url = asset_base_url.trim_end_matches('/');
    let version_base = if asset_base_url.contains("{asset_version}") {
        asset_base_url.replace("{asset_version}", asset_version)
    } else {
        format!("{asset_base_url}/{asset_version}")
    };
    format!(
        "{}/{}-{}",
        version_base.trim_end_matches('/'),
        arch,
        logical_name
    )
}

fn asset_storage_dir(base_dir: &Path, arch: &str) -> PathBuf {
    if base_dir.file_name().and_then(|name| name.to_str()) == Some(arch) {
        base_dir.to_path_buf()
    } else {
        base_dir.join(arch)
    }
}

// ---------------------------------------------------------------------------
// Cleanup
// ---------------------------------------------------------------------------

/// Remove hash-named asset files not referenced by any non-deprecated release.
///
/// Returns paths that were removed.
pub fn cleanup_unused_assets(base_dir: &Path, manifest: &ManifestV2) -> Result<Vec<PathBuf>> {
    cleanup_unused_assets_preserving(base_dir, manifest, std::iter::empty::<String>())
}

/// Remove hash-named asset files not referenced by any non-deprecated release
/// or explicitly listed in `preserve_filenames`.
///
/// `preserve_filenames` is intentionally filename-only. Callers that own
/// higher-level contracts, such as profiles or saved VMs, translate those
/// contracts into hash-prefixed asset basenames before cleanup.
pub fn cleanup_unused_assets_preserving<I, S>(
    base_dir: &Path,
    manifest: &ManifestV2,
    preserve_filenames: I,
) -> Result<Vec<PathBuf>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();

    for release in manifest.assets.releases.values() {
        if release.deprecated {
            continue;
        }
        for assets in release.arches.values() {
            for (name, entry) in assets {
                referenced.insert(hash_filename(name, &entry.hash));
            }
        }
    }
    referenced.extend(
        preserve_filenames
            .into_iter()
            .map(|filename| filename.as_ref().to_string()),
    );

    let mut removed = Vec::new();
    if !base_dir.exists() {
        return Ok(removed);
    }

    for entry in std::fs::read_dir(base_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == "manifest.json"
            || name_str == "manifest-origin.json"
            || name_str.starts_with('.')
            || name_str.ends_with(".tmp")
        {
            continue;
        }

        // Skip directories (arch subdirs like arm64/, x86_64/)
        if entry.file_type()?.is_dir() {
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
// Download
// ---------------------------------------------------------------------------

/// Per-file download progress for [`download_missing_assets`].
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub logical_name: String,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub done: bool,
}

/// Resolve the compatible asset release for `binary_version`, then download
/// any missing or hash-mismatched files from the asset channel into
/// `base_dir/{arch}/{hash_filename}`.
///
/// Per-arch upload convention (see commit aef5269): remote filenames are
/// `{arch}-{logical_name}` (e.g. `arm64-rootfs.erofs`). The downloaded
/// bytes are blake3-verified before atomic rename.
///
/// Returns the set of paths that were freshly downloaded. Already-present
/// files with matching hashes are skipped silently.
pub async fn download_missing_assets<F>(
    manifest: &ManifestV2,
    binary_version: &str,
    arch: &str,
    base_dir: &Path,
    on_progress: F,
) -> Result<Vec<PathBuf>>
where
    F: Fn(DownloadProgress) + Send + Sync,
{
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    // Pick and validate the same bootable asset release the service resolver
    // will use. This rejects channel manifests missing kernel/initrd/rootfs
    // before they can become the installed manifest.
    let asset_version = manifest
        .resolve(binary_version, arch, base_dir)?
        .asset_version;
    let release = manifest
        .assets
        .releases
        .get(&asset_version)
        .with_context(|| format!("asset version {asset_version} not found in manifest"))?;
    let arch_assets = release
        .arches
        .get(arch)
        .with_context(|| format!("arch {arch} not found in asset release {asset_version}"))?;

    let asset_base_url = remote_asset_release_base_url(manifest, base_dir)?;
    let arch_dir = asset_storage_dir(base_dir, arch);
    std::fs::create_dir_all(&arch_dir)
        .with_context(|| format!("cannot create {}", arch_dir.display()))?;

    let client = reqwest::Client::builder()
        .user_agent(concat!("capsem/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build reqwest client")?;

    let mut downloaded = Vec::new();

    // Deterministic order for stable progress output.
    let mut names: Vec<&String> = arch_assets.keys().collect();
    names.sort();

    for name in names {
        let entry = &arch_assets[name];
        let hname = hash_filename(name, &entry.hash);
        let target = arch_dir.join(&hname);

        let mut candidates = vec![base_dir.join(&hname), target.clone()];
        candidates.dedup();
        let mut needs_download = true;
        for candidate in candidates {
            if candidate.exists() {
                match hash_file(&candidate) {
                    Ok(h) if h == entry.hash => {
                        needs_download = false;
                        break;
                    }
                    _ => {
                        info!(path = %candidate.display(), "existing file hash mismatch, redownloading");
                        let _ = std::fs::remove_file(&candidate);
                    }
                }
            }
        }
        if !needs_download {
            on_progress(DownloadProgress {
                logical_name: name.clone(),
                bytes_done: entry.size,
                bytes_total: Some(entry.size),
                done: true,
            });
            continue;
        }

        let url = asset_download_url_with_base(&asset_base_url, &asset_version, arch, name);
        info!(name = %name, url = %url, "downloading asset");

        let resp = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            bail!("GET {} returned {}", url, resp.status());
        }
        let total = resp.content_length().or(Some(entry.size));

        let tmp = arch_dir.join(format!("{hname}.tmp"));
        // Best-effort: clean up any stale tmp from a prior aborted run.
        let _ = std::fs::remove_file(&tmp);

        let mut file = tokio::fs::File::create(&tmp)
            .await
            .with_context(|| format!("create {}", tmp.display()))?;
        let mut hasher = blake3::Hasher::new();
        let mut bytes_done: u64 = 0;
        let mut stream = resp.bytes_stream();

        let cleanup_tmp = |tmp: &Path| {
            let _ = std::fs::remove_file(tmp);
        };

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    cleanup_tmp(&tmp);
                    return Err(anyhow::Error::new(e).context(format!("stream {url}")));
                }
            };
            if let Err(e) = file.write_all(&chunk).await {
                cleanup_tmp(&tmp);
                return Err(anyhow::Error::new(e).context(format!("write {}", tmp.display())));
            }
            hasher.update(&chunk);
            bytes_done += chunk.len() as u64;
            on_progress(DownloadProgress {
                logical_name: name.clone(),
                bytes_done,
                bytes_total: total,
                done: false,
            });
        }
        if let Err(e) = file.flush().await {
            cleanup_tmp(&tmp);
            return Err(anyhow::Error::new(e).context(format!("flush {}", tmp.display())));
        }
        drop(file);

        let actual = hasher.finalize().to_hex().to_string();
        if actual != entry.hash {
            cleanup_tmp(&tmp);
            bail!(
                "{}: hash mismatch (expected {}, got {})",
                name,
                entry.hash,
                actual
            );
        }

        std::fs::rename(&tmp, &target)
            .with_context(|| format!("rename {} -> {}", tmp.display(), target.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o444));
        }

        on_progress(DownloadProgress {
            logical_name: name.clone(),
            bytes_done,
            bytes_total: total,
            done: true,
        });
        downloaded.push(target);
    }

    Ok(downloaded)
}

/// Copy any missing / hash-mismatched VM assets from a local asset tree into
/// `base_dir/{arch}/{hash_filename}`.
///
/// This is the file:// twin of [`download_missing_assets`]. It intentionally
/// preserves the same manifest resolver, hash naming, hash verification, and
/// read-only permissions so local dev/corp package manifests exercise the same
/// installed layout as remote release downloads.
pub fn copy_missing_local_assets<F>(
    manifest: &ManifestV2,
    binary_version: &str,
    arch: &str,
    source_dir: &Path,
    base_dir: &Path,
    on_progress: F,
) -> Result<Vec<PathBuf>>
where
    F: Fn(DownloadProgress),
{
    let asset_version = manifest
        .resolve(binary_version, arch, base_dir)?
        .asset_version;
    let release = manifest
        .assets
        .releases
        .get(&asset_version)
        .with_context(|| format!("asset version {asset_version} not found in manifest"))?;
    let arch_assets = release
        .arches
        .get(arch)
        .with_context(|| format!("arch {arch} not found in asset release {asset_version}"))?;

    let arch_dir = asset_storage_dir(base_dir, arch);
    std::fs::create_dir_all(&arch_dir)
        .with_context(|| format!("cannot create {}", arch_dir.display()))?;

    let mut copied = Vec::new();
    let mut names: Vec<&String> = arch_assets.keys().collect();
    names.sort();

    for name in names {
        let entry = &arch_assets[name];
        let hname = hash_filename(name, &entry.hash);
        let target = arch_dir.join(&hname);

        let mut candidates = vec![base_dir.join(&hname), target.clone()];
        candidates.dedup();
        let mut needs_copy = true;
        for candidate in candidates {
            if candidate.exists() {
                match hash_file(&candidate) {
                    Ok(h) if h == entry.hash => {
                        needs_copy = false;
                        break;
                    }
                    _ => {
                        info!(path = %candidate.display(), "existing file hash mismatch, recopying");
                        let _ = std::fs::remove_file(&candidate);
                    }
                }
            }
        }
        if !needs_copy {
            on_progress(DownloadProgress {
                logical_name: name.clone(),
                bytes_done: entry.size,
                bytes_total: Some(entry.size),
                done: true,
            });
            continue;
        }

        let source = [
            source_dir.join(arch).join(&hname),
            source_dir.join(arch).join(name),
            source_dir.join("current").join(&hname),
            source_dir.join("current").join(name),
            source_dir.join(&hname),
            source_dir.join(name),
        ]
        .into_iter()
        .find(|path| path.is_file())
        .with_context(|| {
            format!(
                "local asset source missing for {name}; checked {}/{arch}, {}/current, and {}",
                source_dir.display(),
                source_dir.display(),
                source_dir.display()
            )
        })?;

        let actual =
            hash_file(&source).with_context(|| format!("hash local asset {}", source.display()))?;
        if actual != entry.hash {
            bail!(
                "{}: local asset hash mismatch at {} (expected {}, got {})",
                name,
                source.display(),
                entry.hash,
                actual
            );
        }

        let tmp = arch_dir.join(format!("{hname}.tmp"));
        let _ = std::fs::remove_file(&tmp);
        std::fs::copy(&source, &tmp)
            .with_context(|| format!("copy {} -> {}", source.display(), tmp.display()))?;
        std::fs::rename(&tmp, &target)
            .with_context(|| format!("rename {} -> {}", tmp.display(), target.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o444));
        }

        on_progress(DownloadProgress {
            logical_name: name.clone(),
            bytes_done: entry.size,
            bytes_total: Some(entry.size),
            done: true,
        });
        copied.push(target);
    }

    Ok(copied)
}

/// Pick the asset version that [`ManifestV2::resolve`] would pick for a
/// given binary version. Extracted so `download_missing_assets` and the
/// resolver stay in lock-step.
fn pick_asset_version(manifest: &ManifestV2, binary_version: &str) -> Result<String> {
    // Empty min_assets means "no compatibility constraint declared".
    let min_assets = manifest
        .binaries
        .releases
        .get(binary_version)
        .map(|release| release.min_assets.as_str())
        .unwrap_or("");

    let mut best: Option<&str> = None;
    for (asset_version, release) in &manifest.assets.releases {
        if release.deprecated {
            continue;
        }
        if !version_at_least(asset_version, min_assets) {
            continue;
        }
        if !release.min_binary.is_empty() && !version_at_least(binary_version, &release.min_binary)
        {
            continue;
        }
        if best.is_none_or(|current| version_at_least(asset_version, current)) {
            best = Some(asset_version.as_str());
        }
    }

    best.map(ToOwned::to_owned).ok_or_else(|| {
        anyhow::anyhow!(
            "no compatible asset release for binary {binary_version} (min_assets: {})",
            if min_assets.is_empty() {
                "unspecified"
            } else {
                min_assets
            }
        )
    })
}

fn version_at_least(actual: &str, minimum: &str) -> bool {
    if minimum.is_empty() {
        return true;
    }
    match (
        numeric_version_parts(actual),
        numeric_version_parts(minimum),
    ) {
        (Some(actual), Some(minimum)) => compare_numeric_versions(&actual, &minimum).is_ge(),
        _ => actual >= minimum,
    }
}

fn numeric_version_parts(version: &str) -> Option<Vec<u64>> {
    let mut parts = Vec::new();
    for part in version.split('.') {
        if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        parts.push(part.parse().ok()?);
    }
    Some(parts)
}

fn compare_numeric_versions(left: &[u64], right: &[u64]) -> std::cmp::Ordering {
    let width = left.len().max(right.len());
    for index in 0..width {
        let left = left.get(index).copied().unwrap_or_default();
        let right = right.get(index).copied().unwrap_or_default();
        match left.cmp(&right) {
            std::cmp::Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    std::cmp::Ordering::Equal
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_V2_MANIFEST: &str = r#"{
        "format": 2,
        "refresh_policy": "24h",
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
                            "rootfs.erofs": { "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 454230016 }
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
        assert_eq!(m.refresh_policy, "24h");
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
    fn manifest_requires_refresh_policy() {
        let json = SAMPLE_V2_MANIFEST.replace(r#""refresh_policy": "24h","#, "");
        let err = ManifestV2::from_json(&json).unwrap_err();
        let error_chain = format!("{err:#}");
        assert!(
            error_chain.contains("refresh_policy"),
            "missing refresh policy must fail closed, got: {error_chain}"
        );
    }

    #[test]
    fn manifest_resolve() {
        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();
        assert_eq!(resolved.asset_version, "2026.0415.1");
        assert!(resolved
            .kernel
            .to_str()
            .unwrap()
            .contains("vmlinuz-a65f925ebe0b0cc7"));
        assert!(resolved
            .initrd
            .to_str()
            .unwrap()
            .contains("initrd-cba052ee1e3fc7de.img"));
        assert!(resolved
            .rootfs
            .to_str()
            .unwrap()
            .contains("rootfs-b8199dc4a83069b9.erofs"));
    }

    #[test]
    fn manifest_resolve_unknown_binary_uses_current_assets() {
        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let resolved = m.resolve("1.0.9999999999", "arm64", dir.path()).unwrap();
        assert_eq!(resolved.asset_version, "2026.0415.1");
    }

    #[test]
    fn manifest_resolve_rejects_current_assets_that_require_newer_binary() {
        let mut m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let future_version = "2030.0101.1".to_string();
        let mut future_release = m.assets.releases["2026.0415.1"].clone();
        future_release.min_binary = "2.0.0".to_string();
        m.assets
            .releases
            .insert(future_version.clone(), future_release);
        m.assets.current = future_version;

        let dir = tempfile::tempdir().unwrap();
        let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();

        assert_eq!(
            resolved.asset_version, "2026.0415.1",
            "older binaries must keep using the newest asset release whose min_binary allows them"
        );
    }

    #[test]
    fn manifest_resolve_avoids_deprecated_asset_releases() {
        let mut m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let deprecated_version = "2026.0416.1".to_string();
        let mut deprecated_release = m.assets.releases["2026.0415.1"].clone();
        deprecated_release.deprecated = true;
        deprecated_release.deprecated_date = Some("2026-04-17".to_string());
        m.assets
            .releases
            .insert(deprecated_version.clone(), deprecated_release);
        m.assets.current = deprecated_version;

        let dir = tempfile::tempdir().unwrap();
        let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();

        assert_eq!(
            resolved.asset_version, "2026.0415.1",
            "new sessions must avoid deprecated asset releases when a compatible release remains"
        );
    }

    #[test]
    fn manifest_resolve_fails_when_only_compatible_assets_are_deprecated() {
        let mut m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        m.assets.releases.get_mut("2026.0415.1").unwrap().deprecated = true;

        let dir = tempfile::tempdir().unwrap();
        let err = m
            .resolve("1.0.1776269479", "arm64", dir.path())
            .unwrap_err();

        assert!(
            format!("{err:#}").contains("no compatible asset release for binary 1.0.1776269479"),
            "{err:#}"
        );
    }

    #[test]
    fn manifest_resolve_fails_when_no_asset_release_supports_binary() {
        let mut m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        m.assets.releases.get_mut("2026.0415.1").unwrap().min_binary = "2.0.0".to_string();

        let dir = tempfile::tempdir().unwrap();
        let err = m
            .resolve("1.0.1776269479", "arm64", dir.path())
            .unwrap_err();

        assert!(
            format!("{err:#}").contains("no compatible asset release for binary 1.0.1776269479"),
            "{err:#}"
        );
    }

    #[test]
    fn numeric_version_comparison_handles_multi_digit_components() {
        assert!(version_at_least("10.0.0", "9.9.9"));
        assert!(version_at_least("2026.1001.1", "2026.0630.9"));
        assert!(!version_at_least("1.9.9", "1.10.0"));
    }

    #[test]
    fn hash_filename_cases() {
        assert_eq!(
            hash_filename(
                "vmlinuz",
                "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c"
            ),
            "vmlinuz-a65f925ebe0b0cc7"
        );
        assert_eq!(
            hash_filename(
                "initrd.img",
                "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456"
            ),
            "initrd-cba052ee1e3fc7de.img"
        );
        assert_eq!(
            hash_filename(
                "rootfs.erofs",
                "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee"
            ),
            "rootfs-b8199dc4a83069b9.erofs"
        );
    }

    #[test]
    fn manifest_rejects_wrong_format() {
        let json = SAMPLE_V2_MANIFEST.replace("\"format\": 2", "\"format\": 99");
        assert!(ManifestV2::from_json(&json).is_err());
    }

    #[test]
    fn expected_hashes_current_returns_arch_hashes() {
        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let h = m.expected_hashes_current("arm64").unwrap();
        assert_eq!(
            h.kernel,
            "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c"
        );
        assert_eq!(
            h.initrd,
            "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456"
        );
        assert_eq!(
            h.rootfs,
            "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee"
        );
    }

    #[test]
    fn expected_hashes_current_returns_none_for_unknown_arch() {
        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        assert!(m.expected_hashes_current("riscv64").is_none());
    }

    #[test]
    fn expected_hashes_current_returns_none_when_canonical_asset_missing() {
        // Manifest with arm64 present but missing any known rootfs entry.
        let json = SAMPLE_V2_MANIFEST.replace(
            r#""rootfs.erofs": { "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 454230016 }"#,
            r#""rootfs.placeholder": { "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 454230016 }"#,
        );
        let m = ManifestV2::from_json(&json).unwrap();
        assert!(m.expected_hashes_current("arm64").is_none());
    }

    #[test]
    fn expected_hashes_current_rejects_squashfs_manifest() {
        let json = SAMPLE_V2_MANIFEST.replace("rootfs.erofs", "rootfs.squashfs");
        let m = ManifestV2::from_json(&json).unwrap();
        assert!(m.expected_hashes_current("arm64").is_none());
    }

    #[test]
    fn host_manifest_arch_maps_aarch64_to_arm64() {
        // Static check: the function maps the rustc arch name (aarch64) to the
        // manifest arch key (arm64). On an aarch64 host this yields "arm64";
        // on x86_64 it yields "x86_64". We can only test the arm's value if
        // we run on that arch, so pin the full mapping table instead.
        assert_eq!(map_rustc_arch_to_manifest("aarch64"), "arm64");
        assert_eq!(map_rustc_arch_to_manifest("x86_64"), "x86_64");
        // Unknown arches pass through (leaves the caller to fail resolution).
        assert_eq!(map_rustc_arch_to_manifest("riscv64"), "riscv64");
    }

    #[test]
    fn load_manifest_for_assets_reads_flat_adjacent_layout() {
        // ~/.capsem/assets/ style: manifest.json lives in the assets dir.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), SAMPLE_V2_MANIFEST).unwrap();
        let m = load_manifest_for_assets(dir.path()).unwrap();
        assert_eq!(m.assets.current, "2026.0415.1");
    }

    #[test]
    fn load_manifest_for_assets_reads_per_arch_layout() {
        // Dev-tree style: assets passed in is assets/arm64/, manifest.json
        // lives at assets/manifest.json (one level up).
        let dir = tempfile::tempdir().unwrap();
        let arm64 = dir.path().join("arm64");
        std::fs::create_dir(&arm64).unwrap();
        std::fs::write(dir.path().join("manifest.json"), SAMPLE_V2_MANIFEST).unwrap();
        let m = load_manifest_for_assets(&arm64).unwrap();
        assert_eq!(m.assets.current, "2026.0415.1");
    }

    #[test]
    fn load_manifest_for_assets_returns_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_manifest_for_assets(dir.path()).is_none());
    }

    #[test]
    fn load_manifest_for_assets_returns_none_on_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), "not json").unwrap();
        assert!(load_manifest_for_assets(dir.path()).is_none());
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
    fn manifest_resolve_finds_files_in_arch_subdir() {
        // Simulates installed/dev layout: base_dir/arm64/vmlinuz-{hash}
        let dir = tempfile::tempdir().unwrap();
        let arm64 = dir.path().join("arm64");
        std::fs::create_dir(&arm64).unwrap();
        std::fs::write(arm64.join("vmlinuz-a65f925ebe0b0cc7"), b"k").unwrap();
        std::fs::write(arm64.join("initrd-cba052ee1e3fc7de.img"), b"i").unwrap();
        std::fs::write(arm64.join("rootfs-b8199dc4a83069b9.erofs"), b"r").unwrap();

        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();
        assert!(
            resolved.kernel.exists(),
            "kernel not found: {:?}",
            resolved.kernel
        );
        assert!(
            resolved.initrd.exists(),
            "initrd not found: {:?}",
            resolved.initrd
        );
        assert!(
            resolved.rootfs.exists(),
            "rootfs not found: {:?}",
            resolved.rootfs
        );
        // Must resolve to the arch subdir, not the flat path
        assert!(resolved.kernel.to_str().unwrap().contains("arm64/"));
    }

    #[test]
    fn manifest_resolve_finds_files_flat() {
        // Simulates flat layout: base_dir/vmlinuz-{hash}
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("vmlinuz-a65f925ebe0b0cc7"), b"k").unwrap();
        std::fs::write(dir.path().join("initrd-cba052ee1e3fc7de.img"), b"i").unwrap();
        std::fs::write(dir.path().join("rootfs-b8199dc4a83069b9.erofs"), b"r").unwrap();

        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();
        assert!(resolved.kernel.exists());
        assert!(resolved.initrd.exists());
        assert!(resolved.rootfs.exists());
    }

    #[test]
    fn copy_missing_local_assets_materializes_hash_named_layout() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let install = dir.path().join("install");
        let arch_dir = source.join("arm64");
        std::fs::create_dir_all(&arch_dir).unwrap();

        let kernel = b"kernel-local";
        let initrd = b"initrd-local";
        let rootfs = b"rootfs-local";
        std::fs::write(arch_dir.join("vmlinuz"), kernel).unwrap();
        std::fs::write(arch_dir.join("initrd.img"), initrd).unwrap();
        std::fs::write(arch_dir.join("rootfs.erofs"), rootfs).unwrap();

        let manifest = ManifestV2::from_json(&format!(
            r#"{{
                "format": 2,
                "refresh_policy": "24h",
                "assets": {{
                    "current": "2030.0101.1",
                    "releases": {{
                        "2030.0101.1": {{
                            "date": "2030-01-01",
                            "deprecated": false,
                            "min_binary": "1.0.0",
                            "arches": {{
                                "arm64": {{
                                    "vmlinuz": {{ "hash": "{}", "size": {} }},
                                    "initrd.img": {{ "hash": "{}", "size": {} }},
                                    "rootfs.erofs": {{ "hash": "{}", "size": {} }}
                                }}
                            }}
                        }}
                    }}
                }},
                "binaries": {{
                    "current": "9.9.9",
                    "releases": {{
                        "9.9.9": {{
                            "date": "2030-01-01",
                            "deprecated": false,
                            "min_assets": "2030.0101.1"
                        }}
                    }}
                }}
            }}"#,
            blake3::hash(kernel).to_hex(),
            kernel.len(),
            blake3::hash(initrd).to_hex(),
            initrd.len(),
            blake3::hash(rootfs).to_hex(),
            rootfs.len(),
        ))
        .unwrap();

        let copied =
            copy_missing_local_assets(&manifest, "9.9.9", "arm64", &source, &install, |_| {})
                .unwrap();

        assert_eq!(copied.len(), 3);
        for (logical, bytes) in [
            ("vmlinuz", kernel.as_slice()),
            ("initrd.img", initrd.as_slice()),
            ("rootfs.erofs", rootfs.as_slice()),
        ] {
            let digest = blake3::hash(bytes).to_hex().to_string();
            let target = install.join("arm64").join(hash_filename(logical, &digest));
            assert_eq!(std::fs::read(&target).unwrap(), bytes);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                    0o444
                );
            }
        }
    }

    #[test]
    fn copy_missing_local_assets_rejects_hash_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let install = dir.path().join("install");
        std::fs::create_dir_all(source.join("arm64")).unwrap();
        std::fs::write(source.join("arm64").join("vmlinuz"), b"wrong").unwrap();
        std::fs::write(source.join("arm64").join("initrd.img"), b"initrd").unwrap();
        std::fs::write(source.join("arm64").join("rootfs.erofs"), b"rootfs").unwrap();
        let initrd_hash = blake3::hash(b"initrd").to_hex().to_string();
        let rootfs_hash = blake3::hash(b"rootfs").to_hex().to_string();

        let manifest = ManifestV2::from_json(
            &format!(
                r#"{{
                "format": 2,
                "refresh_policy": "24h",
                "assets": {{
                    "current": "2030.0101.1",
                    "releases": {{
                        "2030.0101.1": {{
                            "date": "2030-01-01",
                            "deprecated": false,
                            "min_binary": "1.0.0",
                            "arches": {{
                                "arm64": {{
                                    "vmlinuz": {{ "hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": 5 }},
                                    "initrd.img": {{ "hash": "{initrd_hash}", "size": 6 }},
                                    "rootfs.erofs": {{ "hash": "{rootfs_hash}", "size": 6 }}
                                }}
                            }}
                        }}
                    }}
                }},
                "binaries": {{
                    "current": "9.9.9",
                    "releases": {{
                        "9.9.9": {{
                            "date": "2030-01-01",
                            "deprecated": false,
                            "min_assets": "2030.0101.1"
                        }}
                    }}
                }}
            }}"#,
            ),
        )
        .unwrap();

        let err = copy_missing_local_assets(&manifest, "9.9.9", "arm64", &source, &install, |_| {})
            .expect_err("wrong bytes must not be installed");
        assert!(err.to_string().contains("hash mismatch"), "{err:#}");
        assert!(!install
            .join("arm64")
            .join("vmlinuz-aaaaaaaaaaaaaaaa")
            .exists());
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
        // With CAPSEM_HOME / CAPSEM_ASSETS_DIR overrides the path won't contain
        // ".capsem/assets" -- it's whatever the user pointed at. Only assert
        // the substring when we're on the default layout.
        let overridden =
            std::env::var("CAPSEM_ASSETS_DIR").is_ok() || std::env::var("CAPSEM_HOME").is_ok();
        if let Some(dir) = default_assets_dir() {
            if overridden {
                assert!(dir.to_str().is_some());
            } else {
                assert!(dir.to_str().unwrap().contains(".capsem/assets"));
            }
        }
    }

    #[test]
    fn release_url_format() {
        assert_eq!(
            release_url("1.0.1776269479"),
            "https://github.com/google/capsem/releases/download/v1.0.1776269479"
        );
    }

    /// Pin the exact URL `download_missing_assets` constructs. Assets are
    /// deployed by asset version under release.capsem.org; the channel manifest
    /// can move without breaking older installed manifests.
    #[test]
    fn asset_download_url_uses_asset_version_channel_base_and_arch_prefix() {
        assert_eq!(
            asset_download_url("2026.0627.1", "arm64", "vmlinuz"),
            "https://release.capsem.org/assets/releases/2026.0627.1/arm64-vmlinuz",
        );
        assert_eq!(
            asset_download_url("2026.0627.1", "x86_64", "rootfs.erofs"),
            "https://release.capsem.org/assets/releases/2026.0627.1/x86_64-rootfs.erofs",
        );
        let url = asset_download_url("2026.0627.1", "arm64", "initrd.img");
        assert!(
            !url.contains("1.0."),
            "binary version leaked into asset URL: {url}"
        );
        assert_eq!(
            asset_download_url_with_base(
                "https://github.com/google/capsem/releases/download/assets-v{asset_version}",
                "2026.0627.1",
                "arm64",
                "rootfs.erofs",
            ),
            "https://github.com/google/capsem/releases/download/assets-v2026.0627.1/arm64-rootfs.erofs",
        );
    }

    #[test]
    fn remote_asset_release_base_preserves_asset_version_template() {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let asset_base =
            "https://github.com/google/capsem/releases/download/assets-v{asset_version}";
        manifest.asset_base = Some(asset_base.to_string());

        let resolved_base = remote_asset_release_base_url(&manifest, dir.path()).unwrap();

        assert_eq!(resolved_base, asset_base);
        assert_eq!(
            asset_download_url_with_base(&resolved_base, "2026.0415.1", "arm64", "vmlinuz"),
            "https://github.com/google/capsem/releases/download/assets-v2026.0415.1/arm64-vmlinuz",
        );
    }

    #[test]
    fn asset_release_base_derives_from_channel_manifest_url() {
        assert_eq!(
            asset_release_base_url_from_manifest_url(
                "https://release.capsem.org/assets/stable/manifest.json"
            )
            .as_deref(),
            Some("https://release.capsem.org/assets/releases")
        );
        assert_eq!(
            asset_release_base_url_from_manifest_url(
                "https://corp.example/capsem/assets/internal/manifest.json"
            )
            .as_deref(),
            Some("https://corp.example/capsem/assets/releases")
        );
        assert_eq!(
            asset_release_base_url_from_manifest_url("file:///tmp/assets/stable/manifest.json"),
            None
        );
    }

    #[tokio::test]
    async fn download_missing_assets_skips_direct_arch_dev_layout() {
        let dir = tempfile::tempdir().unwrap();
        let base_dir = dir.path().join("arm64");
        std::fs::create_dir(&base_dir).unwrap();
        let files = [
            ("vmlinuz", b"kernel".as_slice()),
            ("initrd.img", b"initrd".as_slice()),
            ("rootfs.erofs", b"rootfs".as_slice()),
        ];
        let mut assets = std::collections::HashMap::new();
        for (name, bytes) in files {
            let hash = blake3::hash(bytes).to_hex().to_string();
            assets.insert(
                name.to_string(),
                AssetEntry {
                    hash,
                    size: bytes.len() as u64,
                },
            );
        }
        let manifest = ManifestV2 {
            format: 2,
            refresh_policy: "24h".to_string(),
            asset_base: None,
            assets: AssetsSection {
                current: "2030.0101.1".to_string(),
                releases: [(
                    "2030.0101.1".to_string(),
                    AssetRelease {
                        date: "2030-01-01".to_string(),
                        deprecated: false,
                        deprecated_date: None,
                        min_binary: "1.0.0".to_string(),
                        arches: [("arm64".to_string(), assets)].into(),
                    },
                )]
                .into(),
            },
            binaries: BinariesSection {
                current: "9.9.9".to_string(),
                releases: [(
                    "9.9.9".to_string(),
                    BinaryRelease {
                        date: "2030-01-01".to_string(),
                        deprecated: false,
                        deprecated_date: None,
                        min_assets: "2030.0101.1".to_string(),
                        version: String::new(),
                        files: Vec::new(),
                    },
                )]
                .into(),
            },
        };
        for (name, entry) in &manifest.assets.releases["2030.0101.1"].arches["arm64"] {
            let hname = hash_filename(name, &entry.hash);
            let bytes = match name.as_str() {
                "vmlinuz" => b"kernel".as_slice(),
                "initrd.img" => b"initrd".as_slice(),
                "rootfs.erofs" => b"rootfs".as_slice(),
                _ => unreachable!(),
            };
            std::fs::write(base_dir.join(hname), bytes).unwrap();
        }

        let downloaded = download_missing_assets(&manifest, "9.9.9", "arm64", &base_dir, |_| {})
            .await
            .expect("direct arch layout should not try to download");

        assert!(downloaded.is_empty());
    }

    // CAPSEM_ASSET_BASE_URL override is exercised end-to-end by the Python
    // integration test in tests/capsem-install/test_asset_download.py against
    // a real local HTTP server. We deliberately don't unit-test it here:
    // env mutation is process-wide and races with other tests in this binary.

    #[test]
    fn cleanup_removes_unreferenced_files() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        // Create a referenced hash-named file
        std::fs::write(base.join("vmlinuz-a65f925ebe0b0cc7"), b"kernel").unwrap();
        // Create an unreferenced hash-named file
        std::fs::write(base.join("vmlinuz-deadbeef12345678"), b"old").unwrap();
        // Create manifest.json (should be preserved)
        std::fs::write(base.join("manifest.json"), b"{}").unwrap();

        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let removed = cleanup_unused_assets(base, &m).unwrap();

        assert_eq!(removed.len(), 1);
        assert!(base.join("vmlinuz-a65f925ebe0b0cc7").exists());
        assert!(!base.join("vmlinuz-deadbeef12345678").exists());
        assert!(base.join("manifest.json").exists());
    }

    #[test]
    fn cleanup_preserves_manifest_origin_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        std::fs::write(base.join("manifest.json"), SAMPLE_V2_MANIFEST).unwrap();
        std::fs::write(
            base.join("manifest-origin.json"),
            br#"{"schema":"capsem.manifest_origin.v1","origin":"package"}"#,
        )
        .unwrap();
        std::fs::write(base.join("rootfs-deadbeef12345678.erofs"), b"stale").unwrap();

        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let removed = cleanup_unused_assets(base, &m).unwrap();

        assert_eq!(removed, vec![base.join("rootfs-deadbeef12345678.erofs")]);
        assert!(base.join("manifest.json").exists());
        assert!(base.join("manifest-origin.json").exists());
    }

    #[test]
    fn cleanup_preserves_explicit_retention_filenames() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        std::fs::write(base.join("vmlinuz-deadbeef12345678"), b"profile kernel").unwrap();
        std::fs::write(
            base.join("rootfs-feedface87654321.erofs"),
            b"profile rootfs",
        )
        .unwrap();
        std::fs::write(base.join("rootfs-1111111111111111.erofs"), b"old rootfs").unwrap();

        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let removed = cleanup_unused_assets_preserving(
            base,
            &m,
            ["vmlinuz-deadbeef12345678", "rootfs-feedface87654321.erofs"],
        )
        .unwrap();

        assert_eq!(removed, vec![base.join("rootfs-1111111111111111.erofs")]);
        assert!(base.join("vmlinuz-deadbeef12345678").exists());
        assert!(base.join("rootfs-feedface87654321.erofs").exists());
    }

    #[test]
    fn channel_cache_isolation() {
        let dir = tempfile::tempdir().unwrap();
        let capsem_home = dir.path();
        let stable_manifest = capsem_home.join("channels/stable/manifest.json");
        let nightly_manifest = capsem_home.join("channels/nightly/manifest.json");
        std::fs::create_dir_all(stable_manifest.parent().unwrap()).unwrap();
        std::fs::create_dir_all(nightly_manifest.parent().unwrap()).unwrap();
        std::fs::write(&stable_manifest, br#"{"channel":"stable"}"#).unwrap();
        std::fs::write(&nightly_manifest, br#"{"channel":"nightly"}"#).unwrap();

        let asset_dir = capsem_home.join("assets/arm64");
        std::fs::create_dir_all(&asset_dir).unwrap();
        let stable_rootfs_hash =
            "1111111111111111111111111111111111111111111111111111111111111111";
        let nightly_rootfs_hash =
            "2222222222222222222222222222222222222222222222222222222222222222";
        let stable_rootfs = asset_dir.join(hash_filename("rootfs.erofs", stable_rootfs_hash));
        let nightly_rootfs = asset_dir.join(hash_filename("rootfs.erofs", nightly_rootfs_hash));
        std::fs::write(&stable_rootfs, b"stable profile rootfs").unwrap();
        std::fs::write(&nightly_rootfs, b"nightly profile rootfs").unwrap();

        assert_ne!(stable_manifest, nightly_manifest);
        assert_ne!(stable_rootfs, nightly_rootfs);
        assert!(stable_manifest.is_file());
        assert!(nightly_manifest.is_file());
        assert!(stable_rootfs.is_file());
        assert!(nightly_rootfs.is_file());
        assert_eq!(
            asset_release_base_url_from_manifest_url(
                "https://release.capsem.org/assets/stable/manifest.json"
            ),
            Some("https://release.capsem.org/assets/releases".to_string())
        );
        assert_eq!(
            asset_release_base_url_from_manifest_url(
                "https://release.capsem.org/assets/nightly/manifest.json"
            ),
            Some("https://release.capsem.org/assets/releases".to_string())
        );
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
