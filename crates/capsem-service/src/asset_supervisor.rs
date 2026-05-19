use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use capsem_core::asset_manager::{
    hash_filename, DownloadProgress, ExpectedAssetHashes, ResolvedAssets,
};
use capsem_core::settings_profiles::{EffectiveVmSettings, VmArchAssets, VmAssetDeclaration};
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error, info, warn};

use crate::api::{AssetHealth, AssetHealthState, AssetProgress};
use crate::registry::SavedVmBaseAssets;

#[derive(Debug)]
pub struct AssetSupervisor {
    assets_dir: PathBuf,
    requirement: AssetRequirement,
    check_interval: Duration,
    state: Mutex<AssetHealth>,
    run_lock: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone)]
pub enum AssetRequirement {
    Profile(ProfileAssetRequirement),
    DevLogical { arch: String },
}

#[derive(Debug, Clone)]
pub struct ProfileAssetRequirement {
    profile_id: String,
    revision: Option<String>,
    arch: String,
    assets: VmArchAssets,
}

#[derive(Debug)]
struct LocalAssetStatus {
    version: String,
    arch: String,
    missing: Vec<String>,
    resolved: ResolvedAssets,
}

impl AssetSupervisor {
    pub fn new(
        assets_dir: PathBuf,
        requirement: AssetRequirement,
        check_interval: Duration,
    ) -> Self {
        Self {
            assets_dir,
            requirement,
            check_interval,
            state: Mutex::new(AssetHealth {
                ready: false,
                state: AssetHealthState::Checking,
                version: None,
                arch: None,
                missing: Vec::new(),
                progress: None,
                error: None,
                retry_count: 0,
                retryable: false,
                saved_vm_dependencies: Vec::new(),
                checked_at_unix_secs: Some(now_unix_secs()),
            }),
            run_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn spawn(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                self.ensure_assets_once().await;
                tokio::time::sleep(self.check_interval).await;
            }
        })
    }

    pub fn snapshot(&self) -> AssetHealth {
        self.state.lock().unwrap().clone()
    }

    pub fn resolve_asset_paths(&self) -> Result<ResolvedAssets> {
        self.inspect_required_assets().map(|status| status.resolved)
    }

    pub fn expected_hashes(&self) -> Option<ExpectedAssetHashes> {
        match &self.requirement {
            AssetRequirement::Profile(required) => Some(required.expected_hashes()),
            AssetRequirement::DevLogical { .. } => None,
        }
    }

    pub fn current_base_assets(&self) -> Option<SavedVmBaseAssets> {
        match &self.requirement {
            AssetRequirement::Profile(required) => Some(required.base_assets()),
            AssetRequirement::DevLogical { .. } => None,
        }
    }

    pub fn refresh_local_state(&self) {
        match self.inspect_required_assets() {
            Ok(status) if status.missing.is_empty() => self.record_ready(status),
            Ok(status) => self.record_updating(status),
            Err(e) => self.record_error(format!("{e:#}"), false),
        }
    }

    pub fn record_download_progress(&self, progress: DownloadProgress) {
        let mut state = self.state.lock().unwrap();
        state.ready = false;
        state.state = AssetHealthState::Updating;
        state.progress = Some(AssetProgress {
            logical_name: progress.logical_name,
            bytes_done: progress.bytes_done,
            bytes_total: progress.bytes_total,
            done: progress.done,
        });
        state.error = None;
        state.retryable = false;
        state.checked_at_unix_secs = Some(now_unix_secs());
    }

    pub fn record_error(&self, error: impl Into<String>, retryable: bool) {
        let mut state = self.state.lock().unwrap();
        state.ready = false;
        state.state = AssetHealthState::Error;
        state.progress = None;
        state.error = Some(error.into());
        state.retryable = retryable;
        if retryable {
            state.retry_count = state.retry_count.saturating_add(1);
        }
        state.checked_at_unix_secs = Some(now_unix_secs());
    }

    pub async fn ensure_assets_once(&self) {
        let _guard = self.run_lock.lock().await;
        info!(
            event = "profile_asset_check_start",
            "profile asset supervisor check started"
        );
        self.set_checking();

        let status = match self.inspect_required_assets() {
            Ok(status) if status.missing.is_empty() => {
                info!(
                    event = "profile_asset_check_ready",
                    asset_version = %status.version,
                    arch = %status.arch,
                    "profile assets already ready"
                );
                self.record_ready(status);
                return;
            }
            Ok(status) => status,
            Err(e) => {
                error!(
                    event = "profile_asset_check_error",
                    error = %e,
                    "profile asset check failed"
                );
                self.record_error(format!("{e:#}"), false);
                return;
            }
        };

        info!(
            event = "profile_asset_missing",
            asset_version = %status.version,
            arch = %status.arch,
            missing = ?status.missing,
            "profile assets missing"
        );
        self.record_updating(status);
        let result = match &self.requirement {
            AssetRequirement::Profile(required) => {
                download_missing_profile_assets(required, &self.assets_dir, |progress| {
                    self.record_download_progress(progress)
                })
                .await
            }
            AssetRequirement::DevLogical { .. } => {
                self.record_error("required development assets are missing", false);
                return;
            }
        };

        match result {
            Ok(_) => self.refresh_local_state(),
            Err(e) => {
                warn!(
                    event = "profile_asset_download_retryable_error",
                    error = %e,
                    "profile asset download failed; will retry"
                );
                self.record_error(format!("{e:#}"), true);
            }
        }
    }

    fn set_checking(&self) {
        let mut state = self.state.lock().unwrap();
        state.ready = false;
        state.state = AssetHealthState::Checking;
        state.progress = None;
        state.error = None;
        state.retryable = false;
        state.checked_at_unix_secs = Some(now_unix_secs());
    }

    fn record_ready(&self, status: LocalAssetStatus) {
        let mut state = self.state.lock().unwrap();
        state.ready = true;
        state.state = AssetHealthState::Ready;
        state.version = Some(status.version);
        state.arch = Some(status.arch);
        state.missing.clear();
        state.progress = None;
        state.error = None;
        state.retryable = false;
        state.checked_at_unix_secs = Some(now_unix_secs());
    }

    fn record_updating(&self, status: LocalAssetStatus) {
        let mut state = self.state.lock().unwrap();
        state.ready = false;
        state.state = AssetHealthState::Updating;
        state.version = Some(status.version);
        state.arch = Some(status.arch);
        state.missing = status.missing;
        state.progress = None;
        state.error = None;
        state.retryable = false;
        state.checked_at_unix_secs = Some(now_unix_secs());
    }

    fn inspect_required_assets(&self) -> Result<LocalAssetStatus> {
        let (arch, resolved) = match &self.requirement {
            AssetRequirement::Profile(required) => (
                required.arch.clone(),
                required.resolved_assets(&self.assets_dir),
            ),
            AssetRequirement::DevLogical { arch } => {
                let base = dev_asset_base(&self.assets_dir, arch);
                (
                    arch.clone(),
                    ResolvedAssets {
                        kernel: base.join("vmlinuz"),
                        initrd: base.join("initrd.img"),
                        rootfs: base.join("rootfs.squashfs"),
                        asset_version: "dev".to_string(),
                    },
                )
            }
        };

        let mut missing = Vec::new();
        if !resolved.kernel.exists() {
            missing.push("vmlinuz".to_string());
        }
        if !resolved.initrd.exists() {
            missing.push("initrd.img".to_string());
        }
        if !resolved.rootfs.exists() {
            missing.push("rootfs.squashfs".to_string());
        }

        Ok(LocalAssetStatus {
            version: resolved.asset_version.clone(),
            arch,
            missing,
            resolved,
        })
    }
}

impl ProfileAssetRequirement {
    pub fn new(
        profile_id: String,
        revision: Option<String>,
        arch: String,
        assets: VmArchAssets,
    ) -> Self {
        Self {
            profile_id,
            revision,
            arch,
            assets,
        }
    }

    pub fn from_effective(effective: &EffectiveVmSettings, arch: &str) -> Result<Self> {
        let assets = effective
            .vm
            .value
            .assets
            .get(arch)
            .cloned()
            .with_context(|| {
                format!(
                    "profile {} does not declare VM assets for arch {arch}",
                    effective.profile_id
                )
            })?;
        Ok(Self::new(
            effective.profile_id.clone(),
            None,
            arch.to_string(),
            assets,
        ))
    }

    fn resolved_assets(&self, base_dir: &Path) -> ResolvedAssets {
        ResolvedAssets {
            kernel: self.resolve_one(base_dir, "vmlinuz", &self.assets.kernel),
            initrd: self.resolve_one(base_dir, "initrd.img", &self.assets.initrd),
            rootfs: self.resolve_one(base_dir, "rootfs.squashfs", &self.assets.rootfs),
            asset_version: self.asset_version(),
        }
    }

    fn resolve_one(
        &self,
        base_dir: &Path,
        logical_name: &str,
        asset: &VmAssetDeclaration,
    ) -> PathBuf {
        let hash = profile_asset_hash_hex(asset);
        let filename = hash_filename(logical_name, hash);
        let flat = base_dir.join(&filename);
        if flat.exists() {
            return flat;
        }
        base_dir.join(&self.arch).join(filename)
    }

    pub fn expected_hashes(&self) -> ExpectedAssetHashes {
        ExpectedAssetHashes {
            kernel: profile_asset_hash_hex(&self.assets.kernel).to_string(),
            initrd: profile_asset_hash_hex(&self.assets.initrd).to_string(),
            rootfs: profile_asset_hash_hex(&self.assets.rootfs).to_string(),
        }
    }

    fn base_assets(&self) -> SavedVmBaseAssets {
        let hashes = self.expected_hashes();
        SavedVmBaseAssets {
            asset_version: self.asset_version(),
            arch: self.arch.clone(),
            kernel_hash: hashes.kernel,
            initrd_hash: hashes.initrd,
            rootfs_hash: hashes.rootfs,
            guest_abi: Some("capsem-guest-v2".to_string()),
        }
    }

    pub fn asset_version(&self) -> String {
        self.revision
            .as_ref()
            .map(|revision| format!("{}@{}", self.profile_id, revision))
            .unwrap_or_else(|| self.profile_id.clone())
    }
}

async fn download_missing_profile_assets(
    required: &ProfileAssetRequirement,
    base_dir: &Path,
    mut on_progress: impl FnMut(DownloadProgress),
) -> Result<()> {
    let arch_dir = base_dir.join(&required.arch);
    tokio::fs::create_dir_all(&arch_dir)
        .await
        .with_context(|| format!("create {}", arch_dir.display()))?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("capsem/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build reqwest client")?;

    for (logical_name, asset) in [
        ("vmlinuz", &required.assets.kernel),
        ("initrd.img", &required.assets.initrd),
        ("rootfs.squashfs", &required.assets.rootfs),
    ] {
        let hash = profile_asset_hash_hex(asset);
        let filename = hash_filename(logical_name, hash);
        let target = arch_dir.join(&filename);
        if target.exists()
            && capsem_core::asset_manager::hash_file(&target)
                .ok()
                .as_deref()
                == Some(hash)
        {
            on_progress(DownloadProgress {
                logical_name: logical_name.to_string(),
                bytes_done: asset.size,
                bytes_total: Some(asset.size),
                done: true,
            });
            continue;
        }

        let url = &asset.url;
        let redacted_url = redacted_url_for_log(url);
        info!(
            event = "profile_asset_download_start",
            profile_id = %required.profile_id,
            revision = required.revision.as_deref().unwrap_or(""),
            arch = %required.arch,
            logical_name,
            expected_hash = hash,
            target = %target.display(),
            url = %redacted_url,
            "profile asset download started"
        );
        let resp = client
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            bail!("GET {} returned {}", url, resp.status());
        }
        let total = resp.content_length().or(Some(asset.size));
        let tmp = arch_dir.join(format!("{filename}.tmp"));
        let _ = tokio::fs::remove_file(&tmp).await;
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .with_context(|| format!("create {}", tmp.display()))?;
        let mut hasher = blake3::Hasher::new();
        let mut bytes_done = 0_u64;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.with_context(|| format!("stream {url}"))?;
            file.write_all(&chunk)
                .await
                .with_context(|| format!("write {}", tmp.display()))?;
            hasher.update(&chunk);
            bytes_done += chunk.len() as u64;
            debug!(
                event = "profile_asset_download_progress",
                profile_id = %required.profile_id,
                revision = required.revision.as_deref().unwrap_or(""),
                arch = %required.arch,
                logical_name,
                bytes_done,
                bytes_total = ?total,
                "profile asset download progressed"
            );
            on_progress(DownloadProgress {
                logical_name: logical_name.to_string(),
                bytes_done,
                bytes_total: total,
                done: false,
            });
        }
        file.flush()
            .await
            .with_context(|| format!("flush {}", tmp.display()))?;
        drop(file);

        let actual = hasher.finalize().to_hex().to_string();
        if actual != hash {
            let _ = tokio::fs::remove_file(&tmp).await;
            bail!("{logical_name}: hash mismatch (expected {hash}, got {actual})");
        }
        info!(
            event = "profile_asset_verify_ok",
            profile_id = %required.profile_id,
            revision = required.revision.as_deref().unwrap_or(""),
            arch = %required.arch,
            logical_name,
            expected_hash = hash,
            bytes_done,
            "profile asset hash verified"
        );
        tokio::fs::rename(&tmp, &target)
            .await
            .with_context(|| format!("install {}", target.display()))?;
        info!(
            event = "profile_asset_install_ok",
            profile_id = %required.profile_id,
            revision = required.revision.as_deref().unwrap_or(""),
            arch = %required.arch,
            logical_name,
            target = %target.display(),
            "profile asset installed"
        );
        on_progress(DownloadProgress {
            logical_name: logical_name.to_string(),
            bytes_done,
            bytes_total: total,
            done: true,
        });
    }
    Ok(())
}

fn profile_asset_hash_hex(asset: &VmAssetDeclaration) -> &str {
    asset.hash.strip_prefix("blake3:").unwrap_or(&asset.hash)
}

fn dev_asset_base(assets_dir: &Path, arch: &str) -> PathBuf {
    let arch_dir = assets_dir.join(arch);
    if arch_dir.join("rootfs.squashfs").exists() {
        arch_dir
    } else {
        assets_dir.to_path_buf()
    }
}

fn redacted_url_for_log(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(parsed) => {
            let host = parsed.host_str().unwrap_or("unknown-host");
            format!("{}://{}{}", parsed.scheme(), host, parsed.path())
        }
        Err(_) => "<invalid-url>".to_string(),
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn host_asset_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests;
