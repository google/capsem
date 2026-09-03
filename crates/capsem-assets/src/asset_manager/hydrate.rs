//! Hydration: download or copy the VM assets a manifest promises into the
//! installed `base_dir/{arch}/{hash_filename}` layout.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tracing::info;

use super::{
    arch_assets_to_materialize, asset_download_url_with_base, asset_storage_dir, copy_hashed, hash_file, hash_filename,
    remote_asset_release_base_url, ManifestV2,
};

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

    // Validate that the release the service resolver will boot is complete,
    // rejecting a channel manifest missing kernel/initrd/rootfs before it can
    // become the installed manifest -- then fetch what *every* compatible
    // release needs, not only that one's.
    manifest.resolve(binary_version, arch, base_dir)?;
    let arch_assets = arch_assets_to_materialize(manifest, binary_version, arch)?;

    let asset_base_url = remote_asset_release_base_url(manifest, base_dir)?;
    let arch_dir = asset_storage_dir(base_dir, arch);
    std::fs::create_dir_all(&arch_dir).with_context(|| format!("cannot create {}", arch_dir.display()))?;

    let client = reqwest::Client::builder()
        .user_agent(concat!("capsem/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build reqwest client")?;

    let mut downloaded = Vec::new();

    // Sorted by (name, hash) for stable progress output.
    for (asset_version, name, entry) in arch_assets {
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

        let url = asset_download_url_with_base(&asset_base_url, asset_version, arch, name);
        info!(name = %name, url = %url, "downloading asset");

        let resp = client.get(&url).send().await.with_context(|| format!("GET {url}"))?;
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
            // The manifest size is a hard cap, not a progress hint: the origin
            // is manifest-controlled, and without this an endless body filled
            // the disk before the hash check could ever run.
            if bytes_done > entry.size {
                cleanup_tmp(&tmp);
                bail!(
                    "{}: origin sent more than the manifest size ({} > {} bytes); download refused",
                    name,
                    bytes_done,
                    entry.size
                );
            }
            on_progress(DownloadProgress {
                logical_name: name.clone(),
                bytes_done,
                bytes_total: total,
                done: false,
            });
        }
        if bytes_done != entry.size {
            cleanup_tmp(&tmp);
            bail!(
                "{}: origin sent {} bytes, manifest says {}",
                name,
                bytes_done,
                entry.size
            );
        }
        if let Err(e) = file.flush().await {
            cleanup_tmp(&tmp);
            return Err(anyhow::Error::new(e).context(format!("flush {}", tmp.display())));
        }
        drop(file);

        let actual = hasher.finalize().to_hex().to_string();
        if actual != entry.hash {
            cleanup_tmp(&tmp);
            bail!("{}: hash mismatch (expected {}, got {})", name, entry.hash, actual);
        }

        std::fs::rename(&tmp, &target).with_context(|| format!("rename {} -> {}", tmp.display(), target.display()))?;
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
    let arch_assets = arch_assets_to_materialize(manifest, binary_version, arch)?;

    let arch_dir = asset_storage_dir(base_dir, arch);
    std::fs::create_dir_all(&arch_dir).with_context(|| format!("cannot create {}", arch_dir.display()))?;

    let mut copied = Vec::new();

    for (_asset_version, name, entry) in arch_assets {
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

        // Hash the bytes that land in the store, in the same pass that copies
        // them. Hashing the source and then copying it separately verified one
        // set of bytes and installed another if the source changed in between,
        // under a filename that says it was verified.
        let tmp = arch_dir.join(format!("{hname}.tmp"));
        let _ = std::fs::remove_file(&tmp);
        let actual =
            copy_hashed(&source, &tmp).with_context(|| format!("copy {} -> {}", source.display(), tmp.display()))?;
        if actual != entry.hash {
            let _ = std::fs::remove_file(&tmp);
            bail!(
                "{}: local asset hash mismatch at {} (expected {}, got {})",
                name,
                source.display(),
                entry.hash,
                actual
            );
        }
        std::fs::rename(&tmp, &target).with_context(|| format!("rename {} -> {}", tmp.display(), target.display()))?;
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
