use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use capsem_core::asset_manager::{DownloadProgress, ManifestV2, ResolvedAssets};

use crate::api::{AssetHealth, AssetHealthState, AssetProgress};

#[derive(Debug)]
pub struct AssetSupervisor {
    assets_dir: PathBuf,
    manifest: Option<Arc<ManifestV2>>,
    binary_version: String,
    arch: String,
    check_interval: Duration,
    state: Mutex<AssetHealth>,
    run_lock: tokio::sync::Mutex<()>,
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
        manifest: Option<Arc<ManifestV2>>,
        binary_version: String,
        arch: String,
        check_interval: Duration,
    ) -> Self {
        Self {
            assets_dir,
            manifest,
            binary_version,
            arch,
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
        self.set_checking();

        let status = match self.inspect_required_assets() {
            Ok(status) if status.missing.is_empty() => {
                self.record_ready(status);
                return;
            }
            Ok(status) => status,
            Err(e) => {
                self.record_error(format!("{e:#}"), false);
                return;
            }
        };

        self.record_updating(status);
        let Some(manifest) = self.manifest.as_ref().cloned() else {
            self.record_error("required development assets are missing", false);
            return;
        };

        let result = capsem_core::asset_manager::download_missing_assets(
            &manifest,
            &self.binary_version,
            &self.arch,
            &self.assets_dir,
            |progress| self.record_download_progress(progress),
        )
        .await;

        match result {
            Ok(_) => self.refresh_local_state(),
            Err(e) => self.record_error(format!("{e:#}"), true),
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
        let resolved = if let Some(manifest) = &self.manifest {
            manifest.resolve(&self.binary_version, &self.arch, &self.assets_dir)?
        } else {
            let base = dev_asset_base(&self.assets_dir, &self.arch);
            ResolvedAssets {
                kernel: base.join("vmlinuz"),
                initrd: base.join("initrd.img"),
                rootfs: base.join("rootfs.squashfs"),
                asset_version: "dev".to_string(),
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
            arch: self.arch.clone(),
            missing,
            resolved,
        })
    }
}

fn dev_asset_base(assets_dir: &Path, arch: &str) -> PathBuf {
    let arch_dir = assets_dir.join(arch);
    if arch_dir.join("rootfs.squashfs").exists() {
        arch_dir
    } else {
        assets_dir.to_path_buf()
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
