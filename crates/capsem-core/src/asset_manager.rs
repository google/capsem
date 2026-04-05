//! Asset manager for downloading and verifying VM assets.
//!
//! VM assets (rootfs) are too large to bundle in the DMG. The asset manager
//! downloads them on first launch and verifies integrity via blake3 hashes.
//!
//! Asset storage: `~/.capsem/assets/v{version}/` (versioned subdirectories)
//! Hash source: manifest.json in app bundle (multi-version rolling manifest),
//!              with backward compatibility for legacy B3SUMS files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Manifest types (multi-version rolling manifest)
// ---------------------------------------------------------------------------

/// A single asset entry in a release.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestAsset {
    pub filename: String,
    pub hash: String,
    pub size: u64,
}

/// Assets for a single release version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReleaseEntry {
    pub assets: Vec<ManifestAsset>,
}

/// Multi-version rolling manifest listing all released asset versions.
///
/// Bundled in the DMG as a build-time snapshot. Remote copy accumulates
/// entries across releases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub latest: String,
    pub releases: HashMap<String, ReleaseEntry>,
}

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

impl Manifest {
    /// Parse a manifest from JSON.
    pub fn from_json(content: &str) -> Result<Self> {
        let manifest: Manifest =
            serde_json::from_str(content).context("invalid manifest JSON")?;
        // Validate all versions and filenames.
        validate_version(&manifest.latest)?;
        for (version, entry) in &manifest.releases {
            validate_version(version)?;
            if entry.assets.is_empty() {
                bail!("release {version} has no assets");
            }
            for asset in &entry.assets {
                validate_filename(&asset.filename)?;
                if asset.hash.len() != 64 || !asset.hash.chars().all(|c| c.is_ascii_hexdigit()) {
                    bail!(
                        "invalid blake3 hash for {}/{}: {}",
                        version,
                        asset.filename,
                        asset.hash
                    );
                }
            }
        }
        if manifest.releases.is_empty() {
            bail!("manifest has no releases");
        }
        Ok(manifest)
    }

    /// Parse a manifest from JSON, normalizing per-arch format for a specific
    /// architecture.
    ///
    /// Per-arch format: `{"releases": {"0.13.0": {"arm64": {"assets": [...]}}}}`
    /// is normalized to flat: `{"releases": {"0.13.0": {"assets": [...]}}}` for
    /// the given `arch_key` (e.g., "arm64" or "x86_64").
    ///
    /// Falls through to `from_json` if the format is already flat.
    pub fn from_json_for_arch(content: &str, arch_key: &str) -> Result<Self> {
        let mut raw: serde_json::Value =
            serde_json::from_str(content).context("invalid manifest JSON")?;

        // Check each release entry for per-arch keys and normalize.
        if let Some(releases) = raw.get_mut("releases").and_then(|r| r.as_object_mut()) {
            for (_version, entry) in releases.iter_mut() {
                if let Some(obj) = entry.as_object_mut() {
                    // If the entry has an arch key with nested assets, flatten it.
                    if let Some(arch_val) = obj.get(arch_key).cloned() {
                        if arch_val.get("assets").is_some() {
                            // Replace entry with the arch-specific sub-object.
                            *entry = arch_val;
                        }
                    }
                }
            }
        }

        let normalized = serde_json::to_string(&raw)?;
        Self::from_json(&normalized)
    }

    /// Create a Manifest from a legacy B3SUMS file content.
    ///
    /// Wraps a single B3SUMS into the multi-version format with one release entry.
    pub fn from_b3sums(content: &str, version: &str) -> Result<Self> {
        validate_version(version)?;
        let entries = parse_b3sums(content)?;
        if entries.is_empty() {
            bail!("B3SUMS manifest is empty");
        }
        let assets = entries
            .into_iter()
            .map(|e| ManifestAsset {
                filename: e.filename,
                hash: e.hash,
                size: 0, // unknown from B3SUMS
            })
            .collect();
        let mut releases = HashMap::new();
        releases.insert(version.to_string(), ReleaseEntry { assets });
        Ok(Manifest {
            latest: version.to_string(),
            releases,
        })
    }

    /// Look up a release entry by version.
    pub fn release_for(&self, version: &str) -> Option<&ReleaseEntry> {
        self.releases.get(version)
    }

    /// Merge another manifest into this one. Newer `latest` wins.
    pub fn merge(&mut self, other: &Manifest) {
        for (version, entry) in &other.releases {
            self.releases
                .entry(version.clone())
                .or_insert_with(|| entry.clone());
        }
        // Simple semver-ish comparison: the other's latest wins if it is
        // lexicographically greater, which works for well-formed versions.
        if other.latest > self.latest {
            self.latest = other.latest.clone();
        }
    }

    /// Convert the release entry for a given version into legacy ManifestEntry list.
    fn entries_for(&self, version: &str) -> Option<Vec<ManifestEntry>> {
        self.release_for(version).map(|r| {
            r.assets
                .iter()
                .map(|a| ManifestEntry {
                    hash: a.hash.clone(),
                    filename: a.filename.clone(),
                })
                .collect()
        })
    }
}

// ---------------------------------------------------------------------------
// Asset status and download types
// ---------------------------------------------------------------------------

/// Status of a single asset after checking local storage against expected hash.
#[derive(Debug, Clone)]
pub enum AssetStatus {
    /// Asset exists locally and hash matches.
    Ready(PathBuf),
    /// Asset needs to be downloaded (missing or hash mismatch).
    NeedsDownload {
        url: String,
        expected_hash: String,
        dest: PathBuf,
    },
}

/// Progress of an asset download.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DownloadProgress {
    pub asset: String,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub phase: String,
}

/// Manifest entry parsed from B3SUMS file.
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestEntry {
    pub hash: String,
    pub filename: String,
}

/// The asset manager checks, downloads, and verifies VM assets.
pub struct AssetManager {
    /// Directory where downloaded assets are stored (version-scoped).
    /// For versioned layout: `~/.capsem/assets/v{version}/`
    assets_dir: PathBuf,
    /// Base URL for downloading assets (GitHub Releases).
    base_url: String,
    /// Parsed manifest entries for the target version.
    manifest: Vec<ManifestEntry>,
    /// Architecture prefix for download URLs. When set, download URLs become
    /// `{base_url}/{arch}-{filename}` to match CI's per-arch release uploads.
    arch_prefix: Option<String>,
}

impl AssetManager {
    /// Create a new asset manager from a B3SUMS file (legacy API).
    ///
    /// `assets_dir` is where downloaded assets live (~/.capsem/assets/).
    /// `base_url` is the GitHub Releases download base, e.g.
    ///   `https://github.com/google/capsem/releases/download/v0.8.0`
    /// `b3sums_content` is the raw content of the B3SUMS file from the app bundle.
    pub fn new(assets_dir: PathBuf, base_url: String, b3sums_content: &str) -> Result<Self> {
        let manifest = parse_b3sums(b3sums_content)?;
        if manifest.is_empty() {
            bail!("B3SUMS manifest is empty");
        }
        Ok(Self {
            assets_dir,
            base_url,
            manifest,
            arch_prefix: None,
        })
    }

    /// Create a new asset manager from a multi-version Manifest.
    ///
    /// `manifest` is the parsed rolling manifest.
    /// `version` is the target release version (e.g. "0.9.0").
    /// `assets_base_dir` is `~/.capsem/assets/` -- the version subdirectory is appended.
    pub fn from_manifest(
        manifest: &Manifest,
        version: &str,
        assets_base_dir: PathBuf,
        arch: Option<&str>,
    ) -> Result<Self> {
        validate_version(version)?;
        let entries = manifest
            .entries_for(version)
            .with_context(|| format!("version {version} not found in manifest"))?;
        if entries.is_empty() {
            bail!("no assets for version {version}");
        }
        let assets_dir = assets_base_dir.join(format!("v{version}"));
        let base_url = release_url(version);
        Ok(Self {
            assets_dir,
            base_url,
            manifest: entries,
            arch_prefix: arch.map(String::from),
        })
    }

    /// Return the assets directory path.
    pub fn assets_dir(&self) -> &Path {
        &self.assets_dir
    }

    /// Build the download URL for a given filename. When `arch_prefix` is set,
    /// URLs use `{base_url}/{arch}-{filename}` to match CI's per-arch uploads.
    fn download_url(&self, filename: &str) -> String {
        match &self.arch_prefix {
            Some(arch) => format!("{}/{}-{}", self.base_url, arch, filename),
            None => format!("{}/{}", self.base_url, filename),
        }
    }

    /// Check the status of a specific asset by filename.
    ///
    /// Returns `Ready` if the file exists and its blake3 hash matches the manifest,
    /// or `NeedsDownload` if missing or corrupted.
    pub fn check_asset(&self, filename: &str) -> Result<AssetStatus> {
        let entry = self
            .manifest
            .iter()
            .find(|e| e.filename == filename)
            .with_context(|| format!("{filename} not found in B3SUMS manifest"))?;

        let local_path = self.assets_dir.join(filename);

        if local_path.exists() {
            let actual_hash = hash_file(&local_path)?;
            if actual_hash == entry.hash {
                debug!(filename, "asset verified");
                return Ok(AssetStatus::Ready(local_path));
            }
            warn!(
                filename,
                expected = %entry.hash,
                actual = %actual_hash,
                "asset hash mismatch, needs re-download"
            );
        }

        Ok(AssetStatus::NeedsDownload {
            url: self.download_url(filename),
            expected_hash: entry.hash.clone(),
            dest: local_path,
        })
    }

    /// Check all assets in the manifest.
    pub fn check_all(&self) -> Result<Vec<(String, AssetStatus)>> {
        let mut results = Vec::new();
        for entry in &self.manifest {
            let status = self.check_asset(&entry.filename)?;
            results.push((entry.filename.clone(), status));
        }
        Ok(results)
    }

    /// Download an asset, verify its hash, and atomically move it into place.
    ///
    /// Supports resuming partial downloads: if a `.tmp` file exists from a
    /// previous attempt, sends an HTTP Range header to continue where it left
    /// off. Falls back to a fresh download if the server doesn't support Range
    /// or returns an unexpected status.
    ///
    /// Calls `progress_cb` with download progress updates.
    /// Returns the final path on success.
    pub async fn download_asset<F>(
        &self,
        filename: &str,
        client: &reqwest::Client,
        progress_cb: F,
    ) -> Result<PathBuf>
    where
        F: Fn(DownloadProgress) + Send + 'static,
    {
        let entry = self
            .manifest
            .iter()
            .find(|e| e.filename == filename)
            .with_context(|| format!("{filename} not found in B3SUMS manifest"))?;

        let url = self.download_url(filename);
        let dest = self.assets_dir.join(filename);
        let tmp = self.assets_dir.join(format!("{filename}.tmp"));

        // Ensure assets directory exists.
        tokio::fs::create_dir_all(&self.assets_dir)
            .await
            .context("failed to create assets directory")?;

        // Check for a resumable partial download.
        let existing_bytes = match tokio::fs::metadata(&tmp).await {
            Ok(m) if m.len() > 0 => m.len(),
            _ => 0,
        };

        info!(url = %url, dest = %dest.display(), resume_from = existing_bytes, "downloading asset");

        progress_cb(DownloadProgress {
            asset: filename.to_string(),
            bytes_downloaded: existing_bytes,
            total_bytes: 0,
            phase: "connecting".to_string(),
        });

        // Try a Range request if we have partial data.
        let mut request = client.get(&url);
        if existing_bytes > 0 {
            request = request.header("Range", format!("bytes={existing_bytes}-"));
        }

        let response = request.send().await.context("download request failed")?;
        let status = response.status();

        // Determine whether we're resuming or starting fresh.
        let (mut bytes_downloaded, append) = if existing_bytes > 0
            && status == reqwest::StatusCode::PARTIAL_CONTENT
        {
            info!(resume_from = existing_bytes, "resuming partial download");
            (existing_bytes, true)
        } else if status.is_success() {
            // Server returned 200 (no Range support) or we had no partial file.
            if existing_bytes > 0 {
                info!("server does not support Range, restarting download");
            }
            (0u64, false)
        } else if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            // Partial file may be corrupt or larger than remote. Start fresh.
            info!("range not satisfiable, restarting download");
            (0u64, false)
        } else {
            bail!("download failed: HTTP {} for {}", status, url);
        };

        let total_bytes = if append {
            // Content-Range response: total size is existing + remaining.
            existing_bytes + response.content_length().unwrap_or(0)
        } else {
            response.content_length().unwrap_or(0)
        };

        // Open temp file: append if resuming, create/truncate if starting fresh.
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .append(append)
            .truncate(!append)
            .open(&tmp)
            .await
            .context("failed to open temp file")?;

        let mut stream = response.bytes_stream();
        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("error reading download stream")?;
            file.write_all(&chunk)
                .await
                .context("failed to write chunk")?;
            bytes_downloaded += chunk.len() as u64;

            progress_cb(DownloadProgress {
                asset: filename.to_string(),
                bytes_downloaded,
                total_bytes,
                phase: "downloading".to_string(),
            });
        }

        file.flush().await?;
        drop(file);

        // Verify hash.
        progress_cb(DownloadProgress {
            asset: filename.to_string(),
            bytes_downloaded,
            total_bytes,
            phase: "verifying".to_string(),
        });

        let tmp_clone = tmp.clone();
        let actual_hash = tokio::task::spawn_blocking(move || hash_file(&tmp_clone))
            .await
            .context("hash verification task panicked")??;
        if actual_hash != entry.hash {
            let _ = tokio::fs::remove_file(&tmp).await;
            bail!(
                "hash verification failed for {}: expected {}, got {}",
                filename,
                entry.hash,
                actual_hash
            );
        }

        // Atomic rename (POSIX: works even if dest is open by another process).
        tokio::fs::rename(&tmp, &dest)
            .await
            .context("failed to move verified asset into place")?;

        info!(filename, hash = %actual_hash, "asset downloaded and verified");

        progress_cb(DownloadProgress {
            asset: filename.to_string(),
            bytes_downloaded,
            total_bytes,
            phase: "complete".to_string(),
        });

        Ok(dest)
    }

    /// Clean up files in assets_dir that are NOT referenced by the manifest.
    ///
    /// Keeps `.tmp` files for manifest assets (partial downloads eligible for
    /// resume). Removes `.tmp` files for non-manifest assets and all other
    /// unrecognized files.
    pub fn cleanup_unrecognized(&self) -> Result<Vec<PathBuf>> {
        let mut removed = Vec::new();
        if !self.assets_dir.exists() {
            return Ok(removed);
        }
        let known: std::collections::HashSet<&str> =
            self.manifest.iter().map(|e| e.filename.as_str()).collect();

        for entry in std::fs::read_dir(&self.assets_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".tmp") {
                // Keep .tmp files whose base name is in the manifest (resumable).
                let base = name_str.trim_end_matches(".tmp");
                if known.contains(base) {
                    debug!(file = %name_str, "keeping resumable partial download");
                    continue;
                }
                let _ = std::fs::remove_file(entry.path());
                removed.push(entry.path());
                continue;
            }
            if !known.contains(name_str.as_ref()) {
                info!(file = %name_str, "removing unrecognized asset");
                let _ = std::fs::remove_file(entry.path());
                removed.push(entry.path());
            }
        }
        Ok(removed)
    }

    /// Check available disk space and return true if there's enough for the download.
    pub fn check_disk_space(&self, needed_bytes: u64) -> Result<bool> {
        check_available_space(&self.assets_dir, needed_bytes)
    }

    /// Get the expected hash for a filename from the manifest.
    pub fn expected_hash(&self, filename: &str) -> Option<&str> {
        self.manifest
            .iter()
            .find(|e| e.filename == filename)
            .map(|e| e.hash.as_str())
    }

    /// List all filenames in the manifest.
    pub fn manifest_filenames(&self) -> Vec<&str> {
        self.manifest.iter().map(|e| e.filename.as_str()).collect()
    }
}

/// Parse B3SUMS file content into manifest entries.
///
/// Format: `<hex-hash>  <filename>` (two spaces between hash and filename,
/// matching the output of `b3sum`).
pub fn parse_b3sums(content: &str) -> Result<Vec<ManifestEntry>> {
    let mut entries = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
        if parts.len() != 2 {
            bail!("invalid B3SUMS line: {line}");
        }
        let hash = parts[0].trim().to_string();
        let filename = parts[1].trim().to_string();
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("invalid blake3 hash in B3SUMS: {hash}");
        }
        entries.push(ManifestEntry { hash, filename });
    }
    Ok(entries)
}

/// Compute the blake3 hash of a file.
pub fn hash_file(path: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut file =
        std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    // Copy in 256KB chunks for performance.
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

/// Check available disk space at a path.
fn check_available_space(path: &Path, needed: u64) -> Result<bool> {
    // Walk up to find an existing ancestor directory for statvfs.
    let mut check_path = path.to_path_buf();
    while !check_path.exists() {
        match check_path.parent() {
            Some(p) => check_path = p.to_path_buf(),
            None => bail!("cannot find existing ancestor of {}", path.display()),
        }
    }

    let c_path =
        std::ffi::CString::new(check_path.to_string_lossy().as_bytes()).context("invalid path")?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if ret != 0 {
        bail!(
            "statvfs failed for {}: {}",
            check_path.display(),
            std::io::Error::last_os_error()
        );
    }
    #[allow(clippy::unnecessary_cast)]
    let available = stat.f_bavail as u64 * stat.f_frsize as u64;
    Ok(available >= needed)
}

/// Return the default assets directory: `~/.capsem/assets/`.
pub fn default_assets_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".capsem").join("assets"))
}

/// Build the GitHub Releases download base URL for the given version.
pub fn release_url(version: &str) -> String {
    format!(
        "https://github.com/google/capsem/releases/download/v{version}"
    )
}

/// Clean up old versioned asset directories, keeping current + pinned versions.
/// Also protects any base versions referenced by images in the ImageRegistry.
///
/// Scans `base_dir/v*/` directories. Keeps `current_version` and any versions
/// listed in `base_dir/pinned.json` (a JSON array of version strings).
/// Returns paths that were removed.
pub fn cleanup_old_versions(
    base_dir: &Path,
    current_version: &str,
    image_registry: Option<&crate::image::ImageRegistry>,
) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    if !base_dir.exists() {
        return Ok(removed);
    }

    // Read pinned versions.
    let pinned_path = base_dir.join("pinned.json");
    let mut pinned: std::collections::HashSet<String> = if pinned_path.exists() {
        let content = std::fs::read_to_string(&pinned_path)
            .context("failed to read pinned.json")?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        std::collections::HashSet::new()
    };

    // Protect versions used by any user image
    if let Some(reg) = image_registry {
        if let Ok(entries) = reg.list() {
            for entry in entries {
                pinned.insert(entry.base_version);
            }
        }
    }

    let current_dir_name = format!("v{current_version}");

    for entry in std::fs::read_dir(base_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Only consider versioned directories (v*).
        if !name_str.starts_with('v') || !entry.file_type()?.is_dir() {
            continue;
        }

        // Keep current version.
        if name_str == current_dir_name {
            continue;
        }

        // Keep pinned versions.
        let version = &name_str[1..]; // strip "v" prefix
        if pinned.contains(version) {
            continue;
        }

        // Remove this old version directory.
        info!(version = %name_str, "removing old asset version");
        let path = entry.path();
        let _ = std::fs::remove_dir_all(&path);
        removed.push(path);
    }

    Ok(removed)
}

/// Migrate flat asset layout to versioned layout.
///
/// If `base_dir/rootfs.squashfs` exists (old flat layout), move it to
/// `base_dir/v{version}/rootfs.squashfs`. Idempotent.
pub fn migrate_flat_layout(base_dir: &Path, version: &str) -> Result<bool> {
    validate_version(version)?;
    let flat_rootfs = base_dir.join("rootfs.squashfs");
    if !flat_rootfs.exists() {
        return Ok(false);
    }

    let versioned_dir = base_dir.join(format!("v{version}"));
    std::fs::create_dir_all(&versioned_dir)
        .context("failed to create versioned assets directory")?;

    let dest = versioned_dir.join("rootfs.squashfs");
    if dest.exists() {
        // Already migrated; just remove the flat file.
        let _ = std::fs::remove_file(&flat_rootfs);
        return Ok(true);
    }

    std::fs::rename(&flat_rootfs, &dest)
        .context("failed to move rootfs to versioned directory")?;
    info!(
        from = %flat_rootfs.display(),
        to = %dest.display(),
        "migrated flat asset layout to versioned"
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_B3SUMS: &str = "\
a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c  vmlinuz
cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456  initrd.img
b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee  rootfs.squashfs
";

    // ---- parse_b3sums tests ----

    #[test]
    fn parse_valid_b3sums() {
        let entries = parse_b3sums(SAMPLE_B3SUMS).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].filename, "vmlinuz");
        assert_eq!(
            entries[0].hash,
            "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c"
        );
        assert_eq!(entries[1].filename, "initrd.img");
        assert_eq!(entries[2].filename, "rootfs.squashfs");
    }

    #[test]
    fn parse_b3sums_ignores_comments_and_blank_lines() {
        let content = "# comment\n\na65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c  vmlinuz\n\n";
        let entries = parse_b3sums(content).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].filename, "vmlinuz");
    }

    #[test]
    fn parse_b3sums_rejects_short_hash() {
        let content = "abcdef  vmlinuz\n";
        assert!(parse_b3sums(content).is_err());
    }

    #[test]
    fn parse_b3sums_rejects_non_hex_hash() {
        let content =
            "zzzz925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c  vmlinuz\n";
        assert!(parse_b3sums(content).is_err());
    }

    #[test]
    fn parse_b3sums_rejects_no_filename() {
        let content = "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c\n";
        // splitn with no whitespace after hash -> only 1 part -> invalid
        assert!(parse_b3sums(content).is_err());
    }

    #[test]
    fn parse_empty_b3sums() {
        let entries = parse_b3sums("").unwrap();
        assert!(entries.is_empty());
    }

    // ---- hash_file tests ----

    #[test]
    fn hash_file_known_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"hello capsem").unwrap();

        let hash = hash_file(&path).unwrap();
        // Verify against blake3 crate directly.
        let expected = blake3::hash(b"hello capsem").to_hex().to_string();
        assert_eq!(hash, expected);
    }

    #[test]
    fn hash_file_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.bin");
        std::fs::write(&path, b"").unwrap();

        let hash = hash_file(&path).unwrap();
        let expected = blake3::hash(b"").to_hex().to_string();
        assert_eq!(hash, expected);
    }

    #[test]
    fn hash_file_nonexistent() {
        assert!(hash_file(Path::new("/nonexistent/file")).is_err());
    }

    // ---- AssetManager::new tests ----

    #[test]
    fn new_with_valid_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = AssetManager::new(
            dir.path().to_path_buf(),
            "https://example.com/releases/v1".to_string(),
            SAMPLE_B3SUMS,
        )
        .unwrap();
        assert_eq!(mgr.manifest.len(), 3);
        assert_eq!(mgr.manifest_filenames(), vec!["vmlinuz", "initrd.img", "rootfs.squashfs"]);
    }

    #[test]
    fn new_rejects_empty_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let result = AssetManager::new(
            dir.path().to_path_buf(),
            "https://example.com".to_string(),
            "",
        );
        assert!(result.is_err());
    }

    // ---- check_asset tests ----

    #[test]
    fn check_asset_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = AssetManager::new(
            dir.path().to_path_buf(),
            "https://example.com/v1".to_string(),
            SAMPLE_B3SUMS,
        )
        .unwrap();

        match mgr.check_asset("vmlinuz").unwrap() {
            AssetStatus::NeedsDownload { url, dest, .. } => {
                assert_eq!(url, "https://example.com/v1/vmlinuz");
                assert_eq!(dest, dir.path().join("vmlinuz"));
            }
            AssetStatus::Ready(_) => panic!("should need download"),
        }
    }

    #[test]
    fn check_asset_present_and_valid() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"test kernel data";
        let hash = blake3::hash(content).to_hex().to_string();
        let b3sums = format!("{hash}  vmlinuz\n");

        std::fs::write(dir.path().join("vmlinuz"), content).unwrap();

        let mgr = AssetManager::new(
            dir.path().to_path_buf(),
            "https://example.com/v1".to_string(),
            &b3sums,
        )
        .unwrap();

        match mgr.check_asset("vmlinuz").unwrap() {
            AssetStatus::Ready(p) => assert_eq!(p, dir.path().join("vmlinuz")),
            AssetStatus::NeedsDownload { .. } => panic!("should be ready"),
        }
    }

    #[test]
    fn check_asset_corrupted() {
        let dir = tempfile::tempdir().unwrap();
        // Write file with wrong content (hash won't match SAMPLE_B3SUMS).
        std::fs::write(dir.path().join("vmlinuz"), b"corrupted").unwrap();

        let mgr = AssetManager::new(
            dir.path().to_path_buf(),
            "https://example.com/v1".to_string(),
            SAMPLE_B3SUMS,
        )
        .unwrap();

        match mgr.check_asset("vmlinuz").unwrap() {
            AssetStatus::NeedsDownload { .. } => {} // expected
            AssetStatus::Ready(_) => panic!("corrupted file should need re-download"),
        }
    }

    #[test]
    fn check_asset_unknown_filename() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = AssetManager::new(
            dir.path().to_path_buf(),
            "https://example.com/v1".to_string(),
            SAMPLE_B3SUMS,
        )
        .unwrap();

        assert!(mgr.check_asset("nonexistent.bin").is_err());
    }

    // ---- check_all tests ----

    #[test]
    fn check_all_nothing_present() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = AssetManager::new(
            dir.path().to_path_buf(),
            "https://example.com/v1".to_string(),
            SAMPLE_B3SUMS,
        )
        .unwrap();

        let results = mgr.check_all().unwrap();
        assert_eq!(results.len(), 3);
        for (_, status) in &results {
            assert!(matches!(status, AssetStatus::NeedsDownload { .. }));
        }
    }

    // ---- cleanup_unrecognized tests ----

    #[test]
    fn cleanup_removes_unknown_files_keeps_resumable() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join("assets");
        std::fs::create_dir_all(&assets).unwrap();

        // Create recognized, unrecognized, and partial download files.
        std::fs::write(assets.join("vmlinuz"), b"kernel").unwrap();
        std::fs::write(assets.join("stale.ext4"), b"old rootfs").unwrap();
        // .tmp for a manifest asset -- should be kept (resumable).
        std::fs::write(assets.join("rootfs.squashfs.tmp"), b"partial download").unwrap();
        // .tmp for a non-manifest asset -- should be removed.
        std::fs::write(assets.join("unknown.tmp"), b"orphan").unwrap();

        let mgr = AssetManager::new(
            assets.clone(),
            "https://example.com/v1".to_string(),
            SAMPLE_B3SUMS,
        )
        .unwrap();

        let removed = mgr.cleanup_unrecognized().unwrap();
        assert_eq!(removed.len(), 2); // stale.ext4 + unknown.tmp
        assert!(assets.join("vmlinuz").exists());
        assert!(!assets.join("stale.ext4").exists());
        assert!(assets.join("rootfs.squashfs.tmp").exists()); // kept for resume
        assert!(!assets.join("unknown.tmp").exists());
    }

    #[test]
    fn cleanup_noop_on_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = AssetManager::new(
            dir.path().to_path_buf(),
            "https://example.com/v1".to_string(),
            SAMPLE_B3SUMS,
        )
        .unwrap();
        // Directory doesn't exist yet -- should not error.
        let removed = mgr.cleanup_unrecognized().unwrap();
        assert!(removed.is_empty());
    }

    // ---- disk space tests ----

    #[test]
    fn disk_space_check_reasonable() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = AssetManager::new(
            dir.path().to_path_buf(),
            "https://example.com/v1".to_string(),
            SAMPLE_B3SUMS,
        )
        .unwrap();

        // 1 byte should always be available.
        assert!(mgr.check_disk_space(1).unwrap());
        // 1 exabyte should not be available.
        assert!(!mgr.check_disk_space(u64::MAX / 2).unwrap());
    }

    // ---- helper tests ----

    #[test]
    fn default_assets_dir_under_home() {
        let dir = default_assets_dir();
        // HOME is set in test environments.
        assert!(dir.is_some());
        let d = dir.unwrap();
        assert!(d.to_string_lossy().contains(".capsem/assets"));
    }

    #[test]
    fn release_url_format() {
        let url = release_url("0.8.0");
        assert_eq!(
            url,
            "https://github.com/google/capsem/releases/download/v0.8.0"
        );
    }

    #[test]
    fn expected_hash_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = AssetManager::new(
            dir.path().to_path_buf(),
            "https://example.com/v1".to_string(),
            SAMPLE_B3SUMS,
        )
        .unwrap();

        assert_eq!(
            mgr.expected_hash("vmlinuz"),
            Some("a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c")
        );
        assert_eq!(mgr.expected_hash("nonexistent"), None);
    }

    // ---- Manifest tests ----

    const SAMPLE_MANIFEST_JSON: &str = r#"{
        "latest": "0.9.0",
        "releases": {
            "0.9.0": {
                "assets": [
                    {"filename": "rootfs.squashfs", "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 314572800}
                ]
            },
            "0.8.8": {
                "assets": [
                    {"filename": "rootfs.squashfs", "hash": "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c", "size": 310000000}
                ]
            }
        }
    }"#;

    #[test]
    fn manifest_from_json_roundtrip() {
        let manifest = Manifest::from_json(SAMPLE_MANIFEST_JSON).unwrap();
        assert_eq!(manifest.latest, "0.9.0");
        assert_eq!(manifest.releases.len(), 2);
        let r = manifest.release_for("0.9.0").unwrap();
        assert_eq!(r.assets.len(), 1);
        assert_eq!(r.assets[0].filename, "rootfs.squashfs");
        assert_eq!(r.assets[0].size, 314572800);
    }

    #[test]
    fn manifest_from_json_rejects_empty_releases() {
        let json = r#"{"latest": "0.9.0", "releases": {}}"#;
        assert!(Manifest::from_json(json).is_err());
    }

    #[test]
    fn manifest_from_json_rejects_bad_hash() {
        let json = r#"{"latest": "0.9.0", "releases": {"0.9.0": {"assets": [{"filename": "rootfs.squashfs", "hash": "short", "size": 100}]}}}"#;
        assert!(Manifest::from_json(json).is_err());
    }

    #[test]
    fn manifest_from_json_rejects_empty_assets() {
        let json = r#"{"latest": "0.9.0", "releases": {"0.9.0": {"assets": []}}}"#;
        assert!(Manifest::from_json(json).is_err());
    }

    #[test]
    fn manifest_from_json_rejects_invalid_json() {
        assert!(Manifest::from_json("not json").is_err());
    }

    #[test]
    fn manifest_release_lookup() {
        let manifest = Manifest::from_json(SAMPLE_MANIFEST_JSON).unwrap();
        assert!(manifest.release_for("0.9.0").is_some());
        assert!(manifest.release_for("0.8.8").is_some());
        assert!(manifest.release_for("0.7.0").is_none());
    }

    #[test]
    fn manifest_merge() {
        let mut m1 = Manifest::from_json(SAMPLE_MANIFEST_JSON).unwrap();
        let m2_json = r#"{
            "latest": "0.9.1",
            "releases": {
                "0.9.1": {
                    "assets": [
                        {"filename": "rootfs.squashfs", "hash": "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456", "size": 320000000}
                    ]
                },
                "0.8.8": {
                    "assets": [
                        {"filename": "rootfs.squashfs", "hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": 1}
                    ]
                }
            }
        }"#;
        let m2 = Manifest::from_json(m2_json).unwrap();
        m1.merge(&m2);

        // Latest should be updated to newer.
        assert_eq!(m1.latest, "0.9.1");
        // Should have 3 releases: 0.8.8, 0.9.0, 0.9.1.
        assert_eq!(m1.releases.len(), 3);
        // 0.8.8 should retain original (merge doesn't overwrite existing).
        assert_eq!(
            m1.release_for("0.8.8").unwrap().assets[0].hash,
            "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c"
        );
        // 0.9.1 added.
        assert!(m1.release_for("0.9.1").is_some());
    }

    #[test]
    fn manifest_from_b3sums_compat() {
        let manifest = Manifest::from_b3sums(SAMPLE_B3SUMS, "0.8.8").unwrap();
        assert_eq!(manifest.latest, "0.8.8");
        assert_eq!(manifest.releases.len(), 1);
        let r = manifest.release_for("0.8.8").unwrap();
        assert_eq!(r.assets.len(), 3);
        assert_eq!(r.assets[0].filename, "vmlinuz");
        assert_eq!(r.assets[0].size, 0); // unknown from B3SUMS
    }

    #[test]
    fn manifest_from_b3sums_empty() {
        assert!(Manifest::from_b3sums("", "0.8.8").is_err());
    }

    #[test]
    fn version_scoped_directory() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = Manifest::from_json(SAMPLE_MANIFEST_JSON).unwrap();
        let mgr = AssetManager::from_manifest(&manifest, "0.9.0", dir.path().to_path_buf(), None).unwrap();
        assert!(mgr.assets_dir().ends_with("v0.9.0"));
    }

    #[test]
    fn from_manifest_missing_version() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = Manifest::from_json(SAMPLE_MANIFEST_JSON).unwrap();
        assert!(AssetManager::from_manifest(&manifest, "99.99.99", dir.path().to_path_buf(), None).is_err());
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
        assert!(validate_filename("path/to/file").is_err());
        assert!(validate_filename("").is_err());
        assert!(validate_filename("rootfs.squashfs").is_ok());
    }

    // ---- per-arch manifest tests ----

    #[test]
    fn manifest_from_json_for_arch_per_arch_format() {
        let json = r#"{
            "latest": "0.13.0",
            "releases": {
                "0.13.0": {
                    "arm64": {
                        "assets": [{
                            "filename": "rootfs.squashfs",
                            "hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                            "size": 100
                        }]
                    },
                    "x86_64": {
                        "assets": [{
                            "filename": "rootfs.squashfs",
                            "hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                            "size": 200
                        }]
                    }
                }
            }
        }"#;
        let manifest = Manifest::from_json_for_arch(json, "arm64").unwrap();
        let r = manifest.release_for("0.13.0").unwrap();
        assert_eq!(r.assets.len(), 1);
        assert_eq!(r.assets[0].hash, "a".repeat(64));
    }

    #[test]
    fn manifest_from_json_for_arch_flat_format_passthrough() {
        // Flat format should pass through unchanged.
        let manifest = Manifest::from_json_for_arch(SAMPLE_MANIFEST_JSON, "arm64").unwrap();
        assert_eq!(manifest.latest, "0.9.0");
        let r = manifest.release_for("0.9.0").unwrap();
        assert_eq!(r.assets[0].filename, "rootfs.squashfs");
    }

    // ---- arch-prefixed download URL tests ----

    #[test]
    fn check_asset_url_with_arch_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = Manifest::from_json(SAMPLE_MANIFEST_JSON).unwrap();
        let mgr = AssetManager::from_manifest(
            &manifest, "0.9.0", dir.path().to_path_buf(), Some("arm64"),
        ).unwrap();
        match mgr.check_asset("rootfs.squashfs").unwrap() {
            AssetStatus::NeedsDownload { url, dest, .. } => {
                assert!(url.ends_with("/arm64-rootfs.squashfs"), "url should have arch prefix: {url}");
                assert!(dest.ends_with("rootfs.squashfs"), "local path should be bare: {}", dest.display());
            }
            AssetStatus::Ready(_) => panic!("should need download"),
        }
    }

    #[test]
    fn check_asset_url_without_arch_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = Manifest::from_json(SAMPLE_MANIFEST_JSON).unwrap();
        let mgr = AssetManager::from_manifest(
            &manifest, "0.9.0", dir.path().to_path_buf(), None,
        ).unwrap();
        match mgr.check_asset("rootfs.squashfs").unwrap() {
            AssetStatus::NeedsDownload { url, .. } => {
                assert!(url.ends_with("/rootfs.squashfs"), "url should be bare: {url}");
                assert!(!url.contains("arm64-"), "url should not have arch prefix: {url}");
            }
            AssetStatus::Ready(_) => panic!("should need download"),
        }
    }

    #[test]
    fn download_url_with_x86_64_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = Manifest::from_json(SAMPLE_MANIFEST_JSON).unwrap();
        let mgr = AssetManager::from_manifest(
            &manifest, "0.9.0", dir.path().to_path_buf(), Some("x86_64"),
        ).unwrap();
        match mgr.check_asset("rootfs.squashfs").unwrap() {
            AssetStatus::NeedsDownload { url, .. } => {
                assert!(url.ends_with("/x86_64-rootfs.squashfs"), "url should have x86_64 prefix: {url}");
            }
            AssetStatus::Ready(_) => panic!("should need download"),
        }
    }

    // ---- cleanup_old_versions tests ----

    #[test]
    fn cleanup_old_versions_keeps_current_and_pinned() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::fs::create_dir_all(base.join("v0.8.7")).unwrap();
        std::fs::create_dir_all(base.join("v0.8.8")).unwrap();
        std::fs::create_dir_all(base.join("v0.9.0")).unwrap();
        std::fs::write(base.join("v0.8.7/rootfs.squashfs"), b"old").unwrap();
        std::fs::write(base.join("v0.8.8/rootfs.squashfs"), b"pinned").unwrap();
        std::fs::write(base.join("v0.9.0/rootfs.squashfs"), b"current").unwrap();

        // Pin 0.8.8.
        std::fs::write(base.join("pinned.json"), r#"["0.8.8"]"#).unwrap();

        let removed = cleanup_old_versions(base, "0.9.0", None).unwrap();
        assert_eq!(removed.len(), 1);
        assert!(!base.join("v0.8.7").exists());
        assert!(base.join("v0.8.8").exists());
        assert!(base.join("v0.9.0").exists());
    }

    #[test]
    fn cleanup_old_versions_respects_pinned_file() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::fs::create_dir_all(base.join("v0.8.7")).unwrap();
        std::fs::create_dir_all(base.join("v0.9.0")).unwrap();

        // Pin 0.8.7.
        std::fs::write(base.join("pinned.json"), r#"["0.8.7"]"#).unwrap();

        let removed = cleanup_old_versions(base, "0.9.0", None).unwrap();
        assert!(removed.is_empty());
        assert!(base.join("v0.8.7").exists());
    }

    #[test]
    fn cleanup_old_versions_empty() {
        let dir = tempfile::tempdir().unwrap();
        let removed = cleanup_old_versions(dir.path(), "0.9.0", None).unwrap();
        assert!(removed.is_empty());
    }

    #[test]
    fn cleanup_old_versions_nonexistent_dir() {
        let removed = cleanup_old_versions(Path::new("/nonexistent/dir"), "0.9.0", None).unwrap();
        assert!(removed.is_empty());
    }

    #[test]
    fn cleanup_old_versions_protects_image_referenced() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::fs::create_dir_all(base.join("v0.8.7")).unwrap();
        std::fs::create_dir_all(base.join("v0.8.8")).unwrap();
        std::fs::create_dir_all(base.join("v0.9.0")).unwrap();

        // Create an image registry that references v0.8.7
        let reg = crate::image::ImageRegistry::new(dir.path());
        reg.insert(crate::image::ImageEntry {
            name: "protected-img".into(),
            description: None,
            source_vm: "vm1".into(),
            parent_image: None,
            base_version: "0.8.7".into(),
            created_at: std::time::SystemTime::now(),
            size_bytes: 100,
        }).unwrap();

        let removed = cleanup_old_versions(base, "0.9.0", Some(&reg)).unwrap();
        // v0.8.7 should be protected by image reference, only v0.8.8 removed
        assert_eq!(removed.len(), 1);
        assert!(base.join("v0.8.7").exists(), "v0.8.7 should be protected by image");
        assert!(!base.join("v0.8.8").exists(), "v0.8.8 should be removed");
        assert!(base.join("v0.9.0").exists(), "current version should stay");
    }

    // ---- migration tests ----

    #[test]
    fn migration_flat_to_versioned() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::fs::write(base.join("rootfs.squashfs"), b"rootfs data").unwrap();

        let migrated = migrate_flat_layout(base, "0.9.0").unwrap();
        assert!(migrated);
        assert!(!base.join("rootfs.squashfs").exists());
        assert!(base.join("v0.9.0/rootfs.squashfs").exists());

        // Idempotent: second call should still succeed.
        // (flat file gone, but versioned exists -- no-op)
        let migrated2 = migrate_flat_layout(base, "0.9.0").unwrap();
        assert!(!migrated2);
    }

    #[test]
    fn migration_no_flat_file() {
        let dir = tempfile::tempdir().unwrap();
        let migrated = migrate_flat_layout(dir.path(), "0.9.0").unwrap();
        assert!(!migrated);
    }
}
