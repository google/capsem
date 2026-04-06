use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

/// Resolved paths for capsem binaries and assets.
pub struct CapsemPaths {
    pub service_bin: PathBuf,
    pub process_bin: PathBuf,
    pub assets_dir: PathBuf,
}

/// Discover paths for sibling binaries and assets.
///
/// Binary discovery: current_exe() parent -> sibling capsem-service, capsem-process.
/// Asset discovery: installed-first (~/.capsem/assets/ with manifest.json),
/// then dev fallback (bin_dir/../../assets/{arch}).
pub fn discover_paths() -> Result<CapsemPaths> {
    let exe_path = std::env::current_exe().context("cannot determine executable path")?;
    let bin_dir = exe_path.parent()
        .ok_or_else(|| anyhow::anyhow!("executable path has no parent: {}", exe_path.display()))?;

    let service_bin = bin_dir.join("capsem-service");
    let process_bin = bin_dir.join("capsem-process");

    let assets_dir = resolve_assets_dir(bin_dir)?;

    Ok(CapsemPaths {
        service_bin,
        process_bin,
        assets_dir,
    })
}

/// Resolve assets directory: installed layout first, dev fallback second.
fn resolve_assets_dir(bin_dir: &Path) -> Result<PathBuf> {
    // Installed layout: ~/.capsem/assets/ (has manifest.json after install)
    if let Ok(home) = std::env::var("HOME") {
        let installed_assets = PathBuf::from(&home).join(".capsem").join("assets");
        if installed_assets.join("manifest.json").exists() {
            return Ok(installed_assets);
        }
    }

    // Dev layout: bin_dir/../../assets/{arch}
    // e.g. target/debug/../../assets/arm64 -> assets/arm64
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
    if let Some(project_root) = bin_dir.parent().and_then(|p| p.parent()) {
        let dev_assets = project_root.join("assets").join(arch);
        if dev_assets.exists() {
            return Ok(dev_assets);
        }
    }

    Err(anyhow::anyhow!(
        "cannot find assets directory (checked ~/.capsem/assets/ and dev layout from {})",
        bin_dir.display()
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn resolve_assets_dir_installed_layout() {
        // When manifest.json exists in ~/.capsem/assets/, that path wins
        let tmp = tempfile::tempdir().unwrap();
        let assets = tmp.path().join(".capsem").join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("manifest.json"), "{}").unwrap();

        // Temporarily override HOME
        let old_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        let result = resolve_assets_dir(Path::new("/nonexistent/bin"));
        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }

        let resolved = result.unwrap();
        assert_eq!(resolved, assets);
    }

    #[test]
    fn resolve_assets_dir_dev_layout() {
        let tmp = tempfile::tempdir().unwrap();
        // Simulate target/debug/ with assets/{arch}/ at project root
        let bin_dir = tmp.path().join("target").join("debug");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
        let dev_assets = tmp.path().join("assets").join(arch);
        std::fs::create_dir_all(&dev_assets).unwrap();

        // No HOME override needed -- just ensure no installed manifest
        let old_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", "/nonexistent");
        let result = resolve_assets_dir(&bin_dir);
        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }

        let resolved = result.unwrap();
        assert_eq!(resolved, dev_assets);
    }

    #[test]
    fn resolve_assets_dir_installed_wins_over_dev() {
        let tmp = tempfile::tempdir().unwrap();

        // Set up both layouts
        let installed = tmp.path().join(".capsem").join("assets");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("manifest.json"), "{}").unwrap();

        let bin_dir = tmp.path().join("target").join("debug");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
        std::fs::create_dir_all(tmp.path().join("assets").join(arch)).unwrap();

        let old_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        let result = resolve_assets_dir(&bin_dir);
        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }

        // Installed layout should win
        assert_eq!(result.unwrap(), installed);
    }

    #[test]
    fn resolve_assets_dir_neither_exists() {
        let old_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", "/nonexistent");
        let result = resolve_assets_dir(Path::new("/nonexistent/bin"));
        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot find assets directory"));
    }
}
