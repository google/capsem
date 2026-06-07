//! Shared helpers for Profile V2 VM assets.
//!
//! Profile manifests are the source of truth for VM asset identity. This
//! module deliberately does not parse or download legacy VM asset manifests.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::info;

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

/// Per-file download progress for profile-owned VM assets.
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub logical_name: String,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub done: bool,
}

/// Minisign public key baked into the binary. Stored in
/// `config/manifest-sign.pub` (key id 93A070CBB288AC9B).
const MANIFEST_SIGN_PUBKEY_FILE: &str = include_str!("../../../config/manifest-sign.pub");

/// Verify a signed JSON payload against a given minisign pubkey.
///
/// `pubkey_file` is the full two-line minisign pubkey file content (with the
/// `untrusted comment:` header); `payload_bytes` is exactly what was signed
/// (the bytes on disk, not a parsed-and-reserialized copy); `sig_file` is the
/// four-line `.minisig` file content.
pub fn verify_manifest_signature(
    pubkey_file: &str,
    payload_bytes: &[u8],
    sig_file: &str,
) -> Result<()> {
    let pubkey = minisign_verify::PublicKey::decode(pubkey_file.trim())
        .map_err(|e| anyhow::anyhow!("decode pubkey: {e}"))?;
    let sig = minisign_verify::Signature::decode(sig_file)
        .map_err(|e| anyhow::anyhow!("decode signature: {e}"))?;
    pubkey
        .verify(payload_bytes, &sig, false)
        .map_err(|e| anyhow::anyhow!("verify: {e}"))?;
    Ok(())
}

/// Verify a signed JSON payload against the baked-in release key.
pub fn verify_manifest_with_baked_key(payload_bytes: &[u8], sig_file: &str) -> Result<()> {
    verify_manifest_signature(MANIFEST_SIGN_PUBKEY_FILE, payload_bytes, sig_file)
}

/// Verify a signed JSON payload against the baked release key OR -- if
/// that fails and `dev_pub_path` points at a readable file -- against an
/// optional developer pubkey.
pub fn verify_manifest_with_baked_or_dev_key(
    payload_bytes: &[u8],
    sig_file: &str,
    dev_pub_path: Option<&Path>,
) -> Result<()> {
    match verify_manifest_with_baked_key(payload_bytes, sig_file) {
        Ok(()) => Ok(()),
        Err(baked_err) => {
            let dev = dev_pub_path.filter(|p| p.is_file()).ok_or(baked_err)?;
            let dev_pub =
                std::fs::read_to_string(dev).with_context(|| format!("read {}", dev.display()))?;
            verify_manifest_signature(&dev_pub, payload_bytes, sig_file)
                .with_context(|| format!("dev key at {} did not verify either", dev.display()))
        }
    }
}

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
    if let Ok(v) = std::env::var("CAPSEM_ASSETS_DIR") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    crate::paths::capsem_home_opt().map(|h| h.join("assets"))
}

/// Remove asset files not referenced by installed profiles or saved VMs.
///
/// Legacy manifest metadata is not an authority in Profile V2, so cleanup
/// removes stale `manifest.json`/signature files instead of preserving them.
pub fn cleanup_unreferenced_assets_preserving<I, S>(
    base_dir: &Path,
    referenced: I,
) -> Result<Vec<PathBuf>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let referenced: HashSet<String> = referenced
        .into_iter()
        .map(|name| name.as_ref().to_string())
        .collect();
    let mut removed = Vec::new();
    if !base_dir.exists() {
        return Ok(removed);
    }

    cleanup_asset_dir(base_dir, &referenced, &mut removed)?;

    for entry in read_dir_sorted(base_dir)? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.starts_with('.') || name_str.ends_with(".tmp") {
            continue;
        }

        let path = entry.path();
        if entry.file_type()?.is_dir() {
            if name_str.starts_with("v1.0.") {
                info!(path = %path.display(), "removing legacy asset directory");
                std::fs::remove_dir_all(&path)?;
                removed.push(path);
            } else {
                cleanup_asset_dir(&path, &referenced, &mut removed)?;
            }
            continue;
        }

        if is_legacy_asset_metadata_file(&name_str) {
            info!(path = %path.display(), "removing legacy asset metadata");
            std::fs::remove_file(&path)?;
            removed.push(path);
        }
    }

    Ok(removed)
}

fn cleanup_asset_dir(
    dir: &Path,
    referenced: &HashSet<String>,
    removed: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in read_dir_sorted(dir)? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.starts_with('.') || name_str.ends_with(".tmp") {
            continue;
        }
        if entry.file_type()?.is_dir() {
            continue;
        }

        let path = entry.path();
        if is_legacy_asset_metadata_file(&name_str)
            || (name_str.contains('-') && !referenced.contains(name_str.as_ref()))
        {
            let event = if is_legacy_asset_metadata_file(&name_str) {
                "removing legacy asset metadata"
            } else {
                "removing unreferenced asset"
            };
            info!(path = %path.display(), event);
            std::fs::remove_file(&path)?;
            removed.push(path);
        }
    }
    Ok(())
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<std::fs::DirEntry>> {
    let mut entries = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn is_legacy_asset_metadata_file(name: &str) -> bool {
    matches!(
        name,
        "manifest.json" | "manifest.json.minisig" | "manifest-sign.dev.pub" | "B3SUMS"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_filename_cases() {
        assert_eq!(
            hash_filename(
                "vmlinuz",
                "2c0bd752db92964268c198f655fa95f5157e75a5e5f3ccf5b0c2072aaf8ea62d"
            ),
            "vmlinuz-2c0bd752db929642"
        );
        assert_eq!(
            hash_filename(
                "initrd.img",
                "e5e910e9ab38b873a1e1d5e2f6d04c5e3a47d2a88061ab37d8bd280003e2a5fb"
            ),
            "initrd-e5e910e9ab38b873.img"
        );
        assert_eq!(
            hash_filename(
                "rootfs.squashfs",
                "89eb92b83534d9d0e08fd6ac4b5d6cb09f431d9bbf6bbdff0d7aab86d6c57a56"
            ),
            "rootfs-89eb92b83534d9d0.squashfs"
        );
    }

    #[test]
    fn hash_file_known_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, b"hello").unwrap();

        let h = hash_file(&path).unwrap();

        assert_eq!(h, blake3::hash(b"hello").to_hex().to_string());
    }

    #[test]
    fn cleanup_unreferenced_assets_preserves_profile_references() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let keep = base.join("rootfs-aaaaaaaaaaaaaaaa.squashfs");
        let remove = base.join("rootfs-bbbbbbbbbbbbbbbb.squashfs");
        std::fs::write(&keep, b"keep").unwrap();
        std::fs::write(&remove, b"remove").unwrap();

        let removed =
            cleanup_unreferenced_assets_preserving(base, ["rootfs-aaaaaaaaaaaaaaaa.squashfs"])
                .unwrap();

        assert_eq!(removed, vec![remove]);
        assert!(keep.exists());
    }

    #[test]
    fn cleanup_unreferenced_assets_removes_legacy_manifest_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("manifest.json");
        let signature = dir.path().join("manifest.json.minisig");
        let b3sums = dir.path().join("B3SUMS");
        std::fs::write(&manifest, b"old manifest").unwrap();
        std::fs::write(&signature, b"old signature").unwrap();
        std::fs::write(&b3sums, b"old checksums").unwrap();

        let removed =
            cleanup_unreferenced_assets_preserving(dir.path(), std::iter::empty::<&str>()).unwrap();

        assert_eq!(removed, vec![b3sums, manifest, signature]);
    }

    #[test]
    fn cleanup_unreferenced_assets_removes_legacy_release_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("v1.0.1234");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("rootfs.squashfs"), b"old").unwrap();

        let removed =
            cleanup_unreferenced_assets_preserving(dir.path(), std::iter::empty::<&str>()).unwrap();

        assert_eq!(removed, vec![legacy]);
    }
}
