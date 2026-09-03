use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::service_install;

/// Return the capsem home directory.
///
/// Delegates to [`capsem_foundation::paths::capsem_home_opt`] so `CAPSEM_HOME`
/// overrides `$HOME/.capsem` uniformly across the workspace.
pub fn capsem_home() -> Result<PathBuf> {
    capsem_foundation::paths::capsem_home_opt().context("HOME not set")
}

/// Resolved paths for capsem binaries and assets.
#[derive(Debug)]
pub struct CapsemPaths {
    pub service_bin: PathBuf,
    pub process_bin: PathBuf,
    pub gateway_bin: PathBuf,
    pub tray_bin: PathBuf,
    pub assets_dir: PathBuf,
}

/// Discover paths for sibling binaries and assets.
///
/// Binaries: current_exe() parent -> sibling capsem-service, capsem-process.
/// Assets: `<capsem_home>/assets/` via [`capsem_foundation::paths::capsem_assets_dir`].
pub fn discover_paths() -> Result<CapsemPaths> {
    let exe_path = std::env::current_exe().context("cannot determine executable path")?;
    let bin_dir = exe_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("executable path has no parent: {}", exe_path.display()))?;

    Ok(CapsemPaths {
        service_bin: bin_dir.join("capsem-service"),
        process_bin: bin_dir.join("capsem-process"),
        gateway_bin: bin_dir.join("capsem-gateway"),
        tray_bin: bin_dir.join("capsem-tray"),
        assets_dir: capsem_foundation::paths::capsem_assets_dir(),
    })
}

/// Build the assets dir path from HOME. Test-only: production paths go through
/// [`capsem_foundation::paths::capsem_assets_dir`] so `CAPSEM_HOME` /
/// `CAPSEM_ASSETS_DIR` are honored.
#[cfg(test)]
fn assets_dir_from_home(home: &str) -> PathBuf {
    PathBuf::from(home).join(".capsem").join("assets")
}

/// Try to start the service via the platform service manager.
/// Returns Ok(true) if started via service manager, Ok(false) if no unit installed.
pub async fn try_start_via_service_manager() -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        if service_install::systemd_unit_path()
            .map(|p| p.exists())
            .unwrap_or(false)
        {
            let status = tokio::process::Command::new("systemctl")
                .args(["--user", "start", "capsem"])
                .status()
                .await?;
            if status.success() {
                return Ok(true);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if service_install::plist_path().map(|p| p.exists()).unwrap_or(false) {
            let uid = capsem_foundation::unix::process::current_uid();
            let status = tokio::process::Command::new("launchctl")
                .args(["kickstart", &format!("gui/{}/com.capsem.service", uid)])
                .status()
                .await?;
            if status.success() {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests;
