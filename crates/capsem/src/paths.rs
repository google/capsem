use std::path::PathBuf;
use anyhow::{Context, Result};

/// Resolved paths for capsem binaries and assets.
pub struct CapsemPaths {
    pub service_bin: PathBuf,
    pub process_bin: PathBuf,
    pub assets_dir: PathBuf,
}

/// Discover paths for sibling binaries and assets.
///
/// Binaries: current_exe() parent -> sibling capsem-service, capsem-process.
/// Assets: ~/.capsem/assets/ (the only layout -- use `just install` or symlink for dev).
pub fn discover_paths() -> Result<CapsemPaths> {
    let exe_path = std::env::current_exe().context("cannot determine executable path")?;
    let bin_dir = exe_path.parent()
        .ok_or_else(|| anyhow::anyhow!("executable path has no parent: {}", exe_path.display()))?;

    let home = std::env::var("HOME").context("HOME not set")?;
    let assets_dir = PathBuf::from(&home).join(".capsem").join("assets");

    Ok(CapsemPaths {
        service_bin: bin_dir.join("capsem-service"),
        process_bin: bin_dir.join("capsem-process"),
        assets_dir,
    })
}

/// Check if a systemd user unit for capsem is installed.
#[cfg(target_os = "linux")]
pub fn systemd_unit_installed() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        let unit = PathBuf::from(home).join(".config/systemd/user/capsem.service");
        if unit.exists() {
            return Some(unit);
        }
    }
    None
}

/// Check if a LaunchAgent plist for capsem is installed.
#[cfg(target_os = "macos")]
pub fn launchagent_installed() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        let plist = PathBuf::from(home).join("Library/LaunchAgents/com.capsem.service.plist");
        if plist.exists() {
            return Some(plist);
        }
    }
    None
}

/// Try to start the service via the platform service manager.
/// Returns Ok(true) if started via service manager, Ok(false) if no unit installed.
pub async fn try_start_via_service_manager() -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        if systemd_unit_installed().is_some() {
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
        if launchagent_installed().is_some() {
            let uid = nix::unistd::getuid();
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
