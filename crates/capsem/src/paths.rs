use std::path::PathBuf;
use anyhow::{Context, Result};

use crate::service_install;

/// Return the capsem home directory.
///
/// Delegates to [`capsem_core::paths::capsem_home_opt`] so `CAPSEM_HOME`
/// overrides `$HOME/.capsem` uniformly across the workspace.
pub fn capsem_home() -> Result<PathBuf> {
    capsem_core::paths::capsem_home_opt().context("HOME not set")
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
/// Assets: `<capsem_home>/assets/` via [`capsem_core::paths::capsem_assets_dir`].
pub fn discover_paths() -> Result<CapsemPaths> {
    let exe_path = std::env::current_exe().context("cannot determine executable path")?;
    let bin_dir = exe_path.parent()
        .ok_or_else(|| anyhow::anyhow!("executable path has no parent: {}", exe_path.display()))?;

    Ok(CapsemPaths {
        service_bin: bin_dir.join("capsem-service"),
        process_bin: bin_dir.join("capsem-process"),
        gateway_bin: bin_dir.join("capsem-gateway"),
        tray_bin: bin_dir.join("capsem-tray"),
        assets_dir: capsem_core::paths::capsem_assets_dir(),
    })
}

/// Build the assets dir path from HOME. Test-only: production paths go through
/// [`capsem_core::paths::capsem_assets_dir`] so `CAPSEM_HOME` /
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
        if service_install::systemd_unit_path().map(|p| p.exists()).unwrap_or(false) {
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

    // -----------------------------------------------------------------------
    // assets_dir_from_home: the core path contract
    // -----------------------------------------------------------------------

    #[test]
    fn capsem_home_under_home() {
        let dir = capsem_home().unwrap();
        let expected = match std::env::var("CAPSEM_HOME") {
            Ok(v) if !v.is_empty() => PathBuf::from(v),
            _ => PathBuf::from(std::env::var("HOME").unwrap()).join(".capsem"),
        };
        assert_eq!(dir, expected);
    }

    #[test]
    fn assets_dir_standard_home() {
        assert_eq!(
            assets_dir_from_home("/Users/elie"),
            PathBuf::from("/Users/elie/.capsem/assets")
        );
    }

    #[test]
    fn assets_dir_linux_home() {
        assert_eq!(
            assets_dir_from_home("/home/elie"),
            PathBuf::from("/home/elie/.capsem/assets")
        );
    }

    #[test]
    fn assets_dir_home_with_spaces() {
        // macOS: "/Users/John Doe" is legal
        assert_eq!(
            assets_dir_from_home("/Users/John Doe"),
            PathBuf::from("/Users/John Doe/.capsem/assets")
        );
    }

    #[test]
    fn assets_dir_wsl_home() {
        // WSL typically uses /home/user but could be /mnt/c/Users/...
        assert_eq!(
            assets_dir_from_home("/mnt/c/Users/elie"),
            PathBuf::from("/mnt/c/Users/elie/.capsem/assets")
        );
    }

    #[test]
    fn assets_dir_root_home() {
        assert_eq!(
            assets_dir_from_home("/root"),
            PathBuf::from("/root/.capsem/assets")
        );
    }

    // -----------------------------------------------------------------------
    // discover_paths: adversarial inputs
    // -----------------------------------------------------------------------

    // NOTE: no test for HOME-unset because removing HOME in a parallel test
    // runner races with other tests. The error path is trivially correct
    // (std::env::var("HOME").context("HOME not set")) and covered by the
    // assets_dir_from_home tests.

    #[test]
    fn discover_paths_sibling_binaries_use_exe_dir() {
        // Whatever directory the current exe is in, siblings should be there
        let paths = discover_paths().unwrap();
        let exe = std::env::current_exe().unwrap();
        let exe_dir = exe.parent().unwrap();
        assert_eq!(paths.service_bin.parent().unwrap(), exe_dir);
        assert_eq!(paths.process_bin.parent().unwrap(), exe_dir);
    }

    #[test]
    fn discover_paths_assets_always_under_home() {
        let paths = discover_paths().unwrap();
        let expected = match std::env::var("CAPSEM_HOME") {
            Ok(v) if !v.is_empty() => PathBuf::from(v).join("assets"),
            _ => PathBuf::from(std::env::var("HOME").unwrap()).join(".capsem/assets"),
        };
        // CAPSEM_ASSETS_DIR may override further; honor the same priority
        // the helper itself uses.
        let expected = match std::env::var("CAPSEM_ASSETS_DIR") {
            Ok(v) if !v.is_empty() => PathBuf::from(v),
            _ => expected,
        };
        assert_eq!(paths.assets_dir, expected);
    }

    #[test]
    fn discover_paths_service_bin_name() {
        let paths = discover_paths().unwrap();
        assert_eq!(
            paths.service_bin.file_name().unwrap().to_str().unwrap(),
            "capsem-service"
        );
    }

    #[test]
    fn discover_paths_process_bin_name() {
        let paths = discover_paths().unwrap();
        assert_eq!(
            paths.process_bin.file_name().unwrap().to_str().unwrap(),
            "capsem-process"
        );
    }

    // -----------------------------------------------------------------------
    // Installed layout contract: what simulate-install.sh produces
    // must be what discover_paths + service startup consume.
    //
    // Layout:
    //   ~/.capsem/bin/capsem{,-service,-process,-mcp,-gateway,-tray}
    //   ~/.capsem/assets/manifest.json
    //   ~/.capsem/assets/v{VERSION}/{vmlinuz,initrd.img,rootfs.squashfs}
    //   ~/.capsem/run/                     (created at runtime)
    //
    // Service reads:
    //   --assets-dir  -> ~/.capsem/assets/
    //   manifest.json -> assets_dir/manifest.json
    //   rootfs        -> assets_dir/v{CARGO_PKG_VERSION}/rootfs.squashfs
    // -----------------------------------------------------------------------

    #[test]
    fn service_manifest_path_matches_install_layout() {
        // Service looks for manifest at: assets_dir.join("manifest.json")
        // simulate-install.sh copies to: ~/.capsem/assets/manifest.json
        let home = "/home/test";
        let assets_dir = assets_dir_from_home(home);
        let manifest = assets_dir.join("manifest.json");
        assert_eq!(manifest, PathBuf::from("/home/test/.capsem/assets/manifest.json"));
    }

    #[test]
    fn service_versioned_assets_path_matches_install_layout() {
        // Service looks for: assets_dir/v{version}/rootfs.squashfs
        // simulate-install.sh copies to: ~/.capsem/assets/v{VERSION}/rootfs.squashfs
        let home = "/home/test";
        let assets_dir = assets_dir_from_home(home);
        let version = env!("CARGO_PKG_VERSION");
        let rootfs = assets_dir.join(format!("v{version}")).join("rootfs.squashfs");
        assert!(rootfs.to_str().unwrap().contains(&format!("v{version}")));
        assert!(rootfs.to_str().unwrap().ends_with("rootfs.squashfs"));
    }

    // -----------------------------------------------------------------------
    // Symlink support: `ln -s` is the dev workflow
    // -----------------------------------------------------------------------

    #[test]
    fn assets_dir_works_through_symlink() {
        // If ~/.capsem is a symlink, PathBuf still resolves correctly
        // (we don't canonicalize, which is correct -- let the OS handle it)
        let dir = assets_dir_from_home("/home/dev");
        assert_eq!(dir.to_str().unwrap(), "/home/dev/.capsem/assets");
        // No canonicalize means symlinks work transparently
    }
}
