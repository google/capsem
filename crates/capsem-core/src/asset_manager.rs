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

/// Minisign public key baked into the binary. Used to verify signatures on
/// downloaded manifests in release builds. Stored in `config/manifest-sign.pub`
/// (key id 93A070CBB288AC9B).
const MANIFEST_SIGN_PUBKEY_FILE: &str = include_str!("../../../config/manifest-sign.pub");

/// Verify a manifest's minisign signature against a given pubkey.
///
/// `pubkey_file` is the full two-line minisign pubkey file content (with the
/// `untrusted comment:` header); `manifest_bytes` is exactly what was signed
/// (the bytes on disk, not a parsed-and-reserialized copy); `sig_file` is the
/// four-line `.minisig` file content.
pub fn verify_manifest_signature(
    pubkey_file: &str,
    manifest_bytes: &[u8],
    sig_file: &str,
) -> Result<()> {
    let pubkey = minisign_verify::PublicKey::decode(pubkey_file.trim())
        .map_err(|e| anyhow::anyhow!("decode pubkey: {e}"))?;
    let sig = minisign_verify::Signature::decode(sig_file)
        .map_err(|e| anyhow::anyhow!("decode signature: {e}"))?;
    pubkey
        .verify(manifest_bytes, &sig, false)
        .map_err(|e| anyhow::anyhow!("verify: {e}"))?;
    Ok(())
}

/// Verify a manifest signature against the baked-in release key.
pub fn verify_manifest_with_baked_key(manifest_bytes: &[u8], sig_file: &str) -> Result<()> {
    verify_manifest_signature(MANIFEST_SIGN_PUBKEY_FILE, manifest_bytes, sig_file)
}

/// Verify a manifest signature against the baked release key OR -- if
/// that fails and `dev_pub_path` points at a readable file -- against an
/// optional developer pubkey. Used so `just install` can deploy a dev
/// keypair once and every release-build binary installed from it trusts
/// that dev key's signatures, without a runtime bypass of verification.
/// Dev-key trust is deliberately scoped to the sibling pubkey file; an
/// attacker who can write to `~/.capsem/assets/` can already rewrite
/// both the manifest and its signature, so allowing a dev key there is
/// not a security regression.
pub fn verify_manifest_with_baked_or_dev_key(
    manifest_bytes: &[u8],
    sig_file: &str,
    dev_pub_path: Option<&Path>,
) -> Result<()> {
    match verify_manifest_with_baked_key(manifest_bytes, sig_file) {
        Ok(()) => Ok(()),
        Err(baked_err) => {
            let dev = dev_pub_path.filter(|p| p.is_file()).ok_or(baked_err)?;
            let dev_pub = std::fs::read_to_string(dev)
                .with_context(|| format!("read {}", dev.display()))?;
            verify_manifest_signature(&dev_pub, manifest_bytes, sig_file).with_context(|| {
                format!("dev key at {} did not verify either", dev.display())
            })
        }
    }
}

/// Load a manifest from disk with minisign signature verification.
///
/// Looks for `manifest.json` in `assets/` and `assets.parent()`, the same
/// search used by `load_manifest_for_assets`. For each candidate, if a
/// sibling `manifest.json.minisig` exists, verifies the signature against
/// the baked release pubkey. `require_signature` controls what happens when
/// the `.minisig` is missing:
///
///   * `true` (release) -- bail. A manifest on disk with no signature is
///     untrusted and must not drive hash verification.
///   * `false` (debug)  -- warn + proceed. Keeps dev loops working when a
///     locally built manifest hasn't been signed.
///
/// Signature-mismatch always bails, regardless of the flag.
///
/// Returns `Ok(None)` only if no `manifest.json` is found at any candidate
/// path.
pub fn load_verified_manifest_for_assets(
    assets: &Path,
    require_signature: bool,
) -> Result<Option<ManifestV2>> {
    let mut candidates: Vec<PathBuf> = vec![assets.join("manifest.json")];
    if let Some(parent) = assets.parent() {
        candidates.push(parent.join("manifest.json"));
    }
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        let manifest_bytes = std::fs::read(&path)
            .with_context(|| format!("read {}", path.display()))?;
        let sig_path = {
            let mut p = path.clone();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("manifest.json");
            p.set_file_name(format!("{name}.minisig"));
            p
        };
        if sig_path.is_file() {
            let sig_text = std::fs::read_to_string(&sig_path)
                .with_context(|| format!("read {}", sig_path.display()))?;
            // Accept either the baked release key or a sibling dev key at
            // `<manifest_dir>/manifest-sign.dev.pub` (deployed by
            // `just install`). See `verify_manifest_with_baked_or_dev_key`.
            let dev_pub = path.parent().map(|p| p.join("manifest-sign.dev.pub"));
            verify_manifest_with_baked_or_dev_key(
                &manifest_bytes,
                &sig_text,
                dev_pub.as_deref(),
            )
            .with_context(|| format!("verify {}", sig_path.display()))?;
            tracing::info!(path = %path.display(), "manifest signature verified");
        } else if require_signature {
            anyhow::bail!(
                "manifest signature missing at {} (required in release builds)",
                sig_path.display()
            );
        } else {
            tracing::warn!(
                path = %path.display(),
                "manifest.json.minisig not found; skipping signature verification (debug build)"
            );
        }
        let content = std::str::from_utf8(&manifest_bytes)
            .context("manifest is not valid UTF-8")?;
        return Ok(Some(ManifestV2::from_json(content)?));
    }
    Ok(None)
}

/// Load `manifest.json` from the assets dir (installed layout) or its parent
/// (dev tree layout where `assets` is already `assets/<arch>/`). Returns
/// `None` on missing file, read error, parse error, or schema mismatch --
/// boot-time hash verification then falls back to "disabled" so dev loops
/// without a manifest keep working.
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
            for assets in release.arches.values() {
                if assets.is_empty() {
                    bail!("asset release {version} has empty arch entry");
                }
                for (name, entry) in assets {
                    validate_filename(name)?;
                    validate_hash(&entry.hash)?;
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
        let asset_version = pick_asset_version(self, binary_version);

        let release = self.assets.releases.get(&asset_version)
            .with_context(|| format!("asset version {} not found in manifest", asset_version))?;
        let arch_assets = release.arches.get(arch)
            .with_context(|| format!("arch {} not found in asset release {}", arch, asset_version))?;

        let resolve_one = |name: &str| -> Result<PathBuf> {
            let entry = arch_assets.get(name)
                .with_context(|| format!("{} not found in asset release {} / {}", name, asset_version, arch))?;
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

        Ok(ResolvedAssets {
            kernel: resolve_one("vmlinuz")?,
            initrd: resolve_one("initrd.img")?,
            rootfs: resolve_one("rootfs.squashfs")?,
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
            rootfs: assets.get("rootfs.squashfs")?.hash.clone(),
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

/// Build the GitHub Releases download base URL for the given asset version.
///
/// Honors the `CAPSEM_RELEASE_URL` env override (used by integration tests that
/// point at a local HTTP fixture). The trailing path `/v{version}` is still
/// appended so local fixtures can mirror the release directory structure.
pub fn release_url(version: &str) -> String {
    let base = std::env::var("CAPSEM_RELEASE_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://github.com/google/capsem/releases/download".into());
    format!("{}/v{version}", base.trim_end_matches('/'))
}

// ---------------------------------------------------------------------------
// Cleanup
// ---------------------------------------------------------------------------

/// Remove hash-named asset files not referenced by any non-deprecated release.
///
/// Returns paths that were removed.
pub fn cleanup_unused_assets(
    base_dir: &Path,
    manifest: &ManifestV2,
) -> Result<Vec<PathBuf>> {
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

    let mut removed = Vec::new();
    if !base_dir.exists() {
        return Ok(removed);
    }

    for entry in std::fs::read_dir(base_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == "manifest.json" || name_str.starts_with('.') || name_str.ends_with(".tmp") {
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
/// any missing or hash-mismatched files from the GitHub Release (or the URL
/// in `CAPSEM_RELEASE_URL`) into `base_dir/{arch}/{hash_filename}`.
///
/// Per-arch upload convention (see commit aef5269): remote filenames are
/// `{arch}-{logical_name}` (e.g. `arm64-rootfs.squashfs`). The downloaded
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

    // Pick the same asset release the service's resolver will pick.
    let asset_version = pick_asset_version(manifest, binary_version);
    let release = manifest.assets.releases.get(&asset_version)
        .with_context(|| format!("asset version {asset_version} not found in manifest"))?;
    let arch_assets = release.arches.get(arch)
        .with_context(|| format!("arch {arch} not found in asset release {asset_version}"))?;

    let arch_dir = base_dir.join(arch);
    std::fs::create_dir_all(&arch_dir)
        .with_context(|| format!("cannot create {}", arch_dir.display()))?;

    let client = reqwest::Client::builder()
        .user_agent(concat!("capsem/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build reqwest client")?;

    let base_url = release_url(&asset_version);
    let mut downloaded = Vec::new();

    // Deterministic order for stable progress output.
    let mut names: Vec<&String> = arch_assets.keys().collect();
    names.sort();

    for name in names {
        let entry = &arch_assets[name];
        let hname = hash_filename(name, &entry.hash);
        let target = arch_dir.join(&hname);

        if target.exists() {
            match hash_file(&target) {
                Ok(h) if h == entry.hash => {
                    on_progress(DownloadProgress {
                        logical_name: name.clone(),
                        bytes_done: entry.size,
                        bytes_total: Some(entry.size),
                        done: true,
                    });
                    continue;
                }
                _ => {
                    info!(path = %target.display(), "existing file hash mismatch, redownloading");
                    let _ = std::fs::remove_file(&target);
                }
            }
        }

        let url = format!("{}/{}-{}", base_url, arch, name);
        info!(name = %name, url = %url, "downloading asset");

        let resp = client.get(&url).send().await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            bail!("GET {} returned {}", url, resp.status());
        }
        let total = resp.content_length().or(Some(entry.size));

        let tmp = arch_dir.join(format!("{hname}.tmp"));
        // Best-effort: clean up any stale tmp from a prior aborted run.
        let _ = std::fs::remove_file(&tmp);

        let mut file = tokio::fs::File::create(&tmp).await
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
                name, entry.hash, actual
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

/// Pick the asset version that [`ManifestV2::resolve`] would pick for a
/// given binary version. Extracted so `download_missing_assets` and the
/// resolver stay in lock-step.
fn pick_asset_version(manifest: &ManifestV2, binary_version: &str) -> String {
    if let Some(bin_rel) = manifest.binaries.releases.get(binary_version) {
        let min = &bin_rel.min_assets;
        if manifest.assets.current >= *min {
            return manifest.assets.current.clone();
        }
        let mut best: Option<&str> = None;
        for v in manifest.assets.releases.keys() {
            if v.as_str() >= min.as_str() && (best.is_none() || v.as_str() > best.unwrap()) {
                best = Some(v.as_str());
            }
        }
        return best.map(String::from).unwrap_or_else(|| manifest.assets.current.clone());
    }
    manifest.assets.current.clone()
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
    fn manifest_rejects_wrong_format() {
        let json = SAMPLE_V2_MANIFEST.replace("\"format\": 2", "\"format\": 99");
        assert!(ManifestV2::from_json(&json).is_err());
    }

    #[test]
    fn expected_hashes_current_returns_arch_hashes() {
        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let h = m.expected_hashes_current("arm64").unwrap();
        assert_eq!(h.kernel, "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c");
        assert_eq!(h.initrd, "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456");
        assert_eq!(h.rootfs, "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee");
    }

    #[test]
    fn expected_hashes_current_returns_none_for_unknown_arch() {
        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        assert!(m.expected_hashes_current("riscv64").is_none());
    }

    #[test]
    fn expected_hashes_current_returns_none_when_canonical_asset_missing() {
        // Manifest with arm64 present but missing rootfs.squashfs entry.
        let json = SAMPLE_V2_MANIFEST.replace(
            r#""rootfs.squashfs": { "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 454230016 }"#,
            r#""rootfs.placeholder": { "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 454230016 }"#,
        );
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

    // Test-only minisign keypair. Generated with `minisign -G -W`; only the
    // pubkey and a sample signature are baked in. Used to exercise the
    // verify_manifest_signature path without needing the real release key.
    const TEST_PUBKEY: &str = "untrusted comment: minisign public key D2FF2FA8B3C45D80\nRWSAXcSzqC//0ussmV+rXA7RVjSb7oBJxZA/Ao9jSOz3yVIv8vcHBOLS\n";
    const TEST_MANIFEST_BYTES: &[u8] = b"{\"hello\":\"world\",\"format\":2}";
    const TEST_SIGNATURE: &str = "untrusted comment: capsem test fixture\nRUSAXcSzqC//0gYG4blIb+435YYxZ665oOig9zIb4BG6alNMXB5/WnDFnKR5SHSfxsi+yyJGNuyDkmPTku5gPusVanpI9YR1MQ4=\ntrusted comment: capsem test fixture\nwyK54SForvZTNYj5/Vn/sScn9kPTutpmSZ27MaZAV8QAspbtH1NKTrCuEw9VVb8r/EOOUWycImpo95puXB/KDg==\n";

    #[test]
    fn verify_manifest_signature_accepts_valid_signature() {
        verify_manifest_signature(TEST_PUBKEY, TEST_MANIFEST_BYTES, TEST_SIGNATURE).unwrap();
    }

    #[test]
    fn verify_manifest_signature_rejects_tampered_manifest() {
        let tampered = b"{\"hello\":\"tampered\",\"format\":2}";
        assert!(verify_manifest_signature(TEST_PUBKEY, tampered, TEST_SIGNATURE).is_err());
    }

    #[test]
    fn verify_manifest_signature_rejects_mangled_signature() {
        // Flip one base64 character in the signature line.
        let mangled = TEST_SIGNATURE.replace(
            "RUSAXcSzqC//0gYG4blIb+435YYxZ665oOig9zIb4BG6alNMXB5/WnDFnKR5SHSfxsi+yyJGNuyDkmPTku5gPusVanpI9YR1MQ4=",
            "RUSAXcSzqC//0gYG4blIb+435YYxZ665oOig9zIb4BG6alNMXB5/WnDFnKR5SHSfxsi+yyJGNuyDkmPTku5gPusVanpI9YR1MQaa=",
        );
        assert!(verify_manifest_signature(TEST_PUBKEY, TEST_MANIFEST_BYTES, &mangled).is_err());
    }

    #[test]
    fn verify_manifest_signature_rejects_wrong_pubkey() {
        // Flip a byte in the pubkey's b64 body. The decode might pass (still
        // 32 bytes of valid b64) but verification must fail.
        let wrong = TEST_PUBKEY.replace(
            "RWSAXcSzqC//0ussmV+rXA7RVjSb7oBJxZA/Ao9jSOz3yVIv8vcHBOLS",
            "RWSAXcSzqC//0ussmV+rXA7RVjSb7oBJxZA/Ao9jSOz3yVIv8vcHBBBB",
        );
        assert!(verify_manifest_signature(&wrong, TEST_MANIFEST_BYTES, TEST_SIGNATURE).is_err());
    }

    #[test]
    fn load_verified_manifest_returns_none_when_no_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let got = load_verified_manifest_for_assets(dir.path(), true).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn load_verified_manifest_bails_when_sig_required_but_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), SAMPLE_V2_MANIFEST).unwrap();
        let err = load_verified_manifest_for_assets(dir.path(), true).unwrap_err();
        assert!(
            format!("{err}").contains("signature missing"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_verified_manifest_accepts_unsigned_when_allowed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), SAMPLE_V2_MANIFEST).unwrap();
        let m = load_verified_manifest_for_assets(dir.path(), false).unwrap().unwrap();
        assert_eq!(m.assets.current, "2026.0415.1");
    }

    #[test]
    fn load_verified_manifest_bails_on_bad_signature_even_if_unsigned_allowed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), SAMPLE_V2_MANIFEST).unwrap();
        std::fs::write(dir.path().join("manifest.json.minisig"), "not a signature").unwrap();
        let err = load_verified_manifest_for_assets(dir.path(), false).unwrap_err();
        assert!(format!("{err}").contains("verify"), "unexpected error: {err}");
    }

    #[test]
    fn dev_key_accepts_signature_baked_key_rejects() {
        // Test fixture is signed with TEST_PUBKEY. The baked release key
        // does NOT match, so `verify_manifest_with_baked_or_dev_key` must
        // fall through to the dev key and accept.
        let dir = tempfile::tempdir().unwrap();
        let dev = dir.path().join("manifest-sign.dev.pub");
        std::fs::write(&dev, TEST_PUBKEY).unwrap();
        verify_manifest_with_baked_or_dev_key(
            TEST_MANIFEST_BYTES,
            TEST_SIGNATURE,
            Some(dev.as_path()),
        )
        .unwrap();
    }

    #[test]
    fn dev_key_missing_falls_back_to_baked_error() {
        // No dev key supplied: the baked-key failure must propagate
        // unchanged so callers see the real reason verification failed.
        let err = verify_manifest_with_baked_or_dev_key(
            TEST_MANIFEST_BYTES,
            TEST_SIGNATURE,
            None,
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("verify"), "unexpected error: {msg}");
    }

    #[test]
    fn dev_key_path_not_a_file_falls_back_to_baked_error() {
        // Path points at something that isn't a regular file -- treat as
        // absent, preserving the baked-key error.
        let dir = tempfile::tempdir().unwrap();
        let err = verify_manifest_with_baked_or_dev_key(
            TEST_MANIFEST_BYTES,
            TEST_SIGNATURE,
            Some(dir.path()), // directory, not a file
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("verify"));
    }

    #[test]
    fn dev_key_both_invalid_surfaces_dev_error() {
        // Dev key is deployed but doesn't match either. Error chain must
        // mention the dev key path so debugging is possible.
        let dir = tempfile::tempdir().unwrap();
        let dev = dir.path().join("manifest-sign.dev.pub");
        let wrong = TEST_PUBKEY.replace(
            "RWSAXcSzqC//0ussmV+rXA7RVjSb7oBJxZA/Ao9jSOz3yVIv8vcHBOLS",
            "RWSAXcSzqC//0ussmV+rXA7RVjSb7oBJxZA/Ao9jSOz3yVIv8vcHBBBB",
        );
        std::fs::write(&dev, wrong).unwrap();
        let err = verify_manifest_with_baked_or_dev_key(
            TEST_MANIFEST_BYTES,
            TEST_SIGNATURE,
            Some(dev.as_path()),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("dev key") && msg.contains("did not verify"),
            "expected dev-key error chain, got: {msg}"
        );
    }

    #[test]
    fn baked_pubkey_file_is_parseable_minisign_format() {
        // Regression guard: if config/manifest-sign.pub ever gets replaced
        // with a malformed file, this fires before the binary starts
        // rejecting every signed manifest.
        minisign_verify::PublicKey::decode(MANIFEST_SIGN_PUBKEY_FILE.trim())
            .expect("baked pubkey must decode as minisign PublicKey");
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
        std::fs::write(arm64.join("rootfs-b8199dc4a83069b9.squashfs"), b"r").unwrap();

        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();
        assert!(resolved.kernel.exists(), "kernel not found: {:?}", resolved.kernel);
        assert!(resolved.initrd.exists(), "initrd not found: {:?}", resolved.initrd);
        assert!(resolved.rootfs.exists(), "rootfs not found: {:?}", resolved.rootfs);
        // Must resolve to the arch subdir, not the flat path
        assert!(resolved.kernel.to_str().unwrap().contains("arm64/"));
    }

    #[test]
    fn manifest_resolve_finds_files_flat() {
        // Simulates flat layout: base_dir/vmlinuz-{hash}
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("vmlinuz-a65f925ebe0b0cc7"), b"k").unwrap();
        std::fs::write(dir.path().join("initrd-cba052ee1e3fc7de.img"), b"i").unwrap();
        std::fs::write(dir.path().join("rootfs-b8199dc4a83069b9.squashfs"), b"r").unwrap();

        let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
        let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();
        assert!(resolved.kernel.exists());
        assert!(resolved.initrd.exists());
        assert!(resolved.rootfs.exists());
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
        let overridden = std::env::var("CAPSEM_ASSETS_DIR").is_ok()
            || std::env::var("CAPSEM_HOME").is_ok();
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
