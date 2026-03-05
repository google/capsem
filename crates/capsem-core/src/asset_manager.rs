//! Asset manager for downloading and verifying VM assets.
//!
//! VM assets (rootfs) are too large to bundle in the DMG. The asset manager
//! downloads them on first launch and verifies integrity via blake3 hashes
//! embedded in the app bundle (B3SUMS file).
//!
//! Asset storage: `~/.capsem/assets/`
//! Hash source: B3SUMS file in app bundle (compile-time embedded hashes)

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

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
    /// Directory where downloaded assets are stored (~/.capsem/assets/).
    assets_dir: PathBuf,
    /// Base URL for downloading assets (GitHub Releases).
    base_url: String,
    /// Parsed manifest entries from B3SUMS.
    manifest: Vec<ManifestEntry>,
}

impl AssetManager {
    /// Create a new asset manager.
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
        })
    }

    /// Return the assets directory path.
    pub fn assets_dir(&self) -> &Path {
        &self.assets_dir
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
            url: format!("{}/{}", self.base_url, filename),
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

        let url = format!("{}/{}", self.base_url, filename);
        let dest = self.assets_dir.join(filename);
        let tmp = self.assets_dir.join(format!("{filename}.tmp"));

        // Ensure assets directory exists.
        tokio::fs::create_dir_all(&self.assets_dir)
            .await
            .context("failed to create assets directory")?;

        info!(url = %url, dest = %dest.display(), "downloading asset");

        progress_cb(DownloadProgress {
            asset: filename.to_string(),
            bytes_downloaded: 0,
            total_bytes: 0,
            phase: "connecting".to_string(),
        });

        let response = client
            .get(&url)
            .send()
            .await
            .context("download request failed")?;

        if !response.status().is_success() {
            bail!(
                "download failed: HTTP {} for {}",
                response.status(),
                url
            );
        }

        let total_bytes = response.content_length().unwrap_or(0);
        let mut bytes_downloaded: u64 = 0;

        // Stream to temp file.
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .context("failed to create temp file")?;

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

        let actual_hash = hash_file(&tmp)?;
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
            // Skip temp files (will be cleaned up on next download).
            if name_str.ends_with(".tmp") {
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_B3SUMS: &str = "\
a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c  vmlinuz
cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456  initrd.img
b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee  rootfs.img
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
        assert_eq!(entries[2].filename, "rootfs.img");
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
        assert_eq!(mgr.manifest_filenames(), vec!["vmlinuz", "initrd.img", "rootfs.img"]);
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
    fn cleanup_removes_unknown_files() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join("assets");
        std::fs::create_dir_all(&assets).unwrap();

        // Create recognized and unrecognized files.
        std::fs::write(assets.join("vmlinuz"), b"kernel").unwrap();
        std::fs::write(assets.join("stale.ext4"), b"old rootfs").unwrap();
        std::fs::write(assets.join("rootfs.squashfs.tmp"), b"partial download").unwrap();

        let mgr = AssetManager::new(
            assets.clone(),
            "https://example.com/v1".to_string(),
            SAMPLE_B3SUMS,
        )
        .unwrap();

        let removed = mgr.cleanup_unrecognized().unwrap();
        assert_eq!(removed.len(), 2);
        assert!(assets.join("vmlinuz").exists());
        assert!(!assets.join("stale.ext4").exists());
        assert!(!assets.join("rootfs.squashfs.tmp").exists());
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
}
