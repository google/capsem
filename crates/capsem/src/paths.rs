use anyhow::{Context, Result};
use std::path::PathBuf;

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
    pub cli_bin: PathBuf,
    pub service_bin: PathBuf,
    pub process_bin: PathBuf,
    pub mcp_bin: PathBuf,
    pub mcp_aggregator_bin: PathBuf,
    pub mcp_builtin_bin: PathBuf,
    pub gateway_bin: PathBuf,
    pub tray_bin: PathBuf,
    pub assets_dir: PathBuf,
}

/// Discover paths for sibling binaries and assets.
///
/// Binaries: current_exe() parent -> sibling capsem-service, capsem-process.
/// Assets: `<capsem_home>/assets/` via [`capsem_core::paths::capsem_assets_dir`].
pub fn discover_paths() -> Result<CapsemPaths> {
    let exe_path = invoked_executable_path()
        .or_else(|| std::env::current_exe().ok())
        .context("cannot determine executable path")?;
    let bin_dir = exe_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("executable path has no parent: {}", exe_path.display()))?;

    Ok(CapsemPaths {
        cli_bin: bin_dir.join("capsem"),
        service_bin: bin_dir.join("capsem-service"),
        process_bin: bin_dir.join("capsem-process"),
        mcp_bin: bin_dir.join("capsem-mcp"),
        mcp_aggregator_bin: bin_dir.join("capsem-mcp-aggregator"),
        mcp_builtin_bin: bin_dir.join("capsem-mcp-builtin"),
        gateway_bin: bin_dir.join("capsem-gateway"),
        tray_bin: bin_dir.join("capsem-tray"),
        assets_dir: capsem_core::paths::capsem_assets_dir(),
    })
}

fn invoked_executable_path() -> Option<PathBuf> {
    let argv0 = std::env::args_os().next()?;
    invoked_executable_path_from_argv0(PathBuf::from(argv0), std::env::current_dir().ok()?)
}

fn invoked_executable_path_from_argv0(path: PathBuf, cwd: PathBuf) -> Option<PathBuf> {
    if path.is_absolute() {
        return Some(path);
    }
    if path
        .parent()
        .is_some_and(|parent| parent.as_os_str().is_empty())
    {
        return None;
    }
    if path.parent().is_some() {
        return Some(cwd.join(path));
    }
    None
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
        if service_install::systemd_unit_path()
            .map(|p| p.exists())
            .unwrap_or(false)
        {
            let mut command = tokio::process::Command::new("systemctl");
            command.args(["--user", "start", "--no-block", "capsem"]);
            let status = command_status_quiet(command).await?;
            if status.success() {
                return Ok(true);
            }
        }
        if service_install::systemd_system_unit_path().exists() {
            let mut command = tokio::process::Command::new("systemctl");
            command.args(["start", "--no-block", "capsem"]);
            let status = command_status_quiet(command).await?;
            if status.success() {
                return Ok(true);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if service_install::plist_path()
            .map(|p| p.exists())
            .unwrap_or(false)
        {
            let uid = nix::unistd::getuid();
            let mut command = tokio::process::Command::new("launchctl");
            command.args(["kickstart", &format!("gui/{}/com.capsem.service", uid)]);
            let status = command_status_quiet(command).await?;
            if status.success() {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

async fn command_status_quiet(
    mut command: tokio::process::Command,
) -> std::io::Result<std::process::ExitStatus> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
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
    fn invoked_path_preserves_absolute_symlink_entrypoint() {
        assert_eq!(
            invoked_executable_path_from_argv0(
                PathBuf::from("/home/user/.capsem/bin/capsem"),
                PathBuf::from("/work")
            ),
            Some(PathBuf::from("/home/user/.capsem/bin/capsem"))
        );
    }

    #[test]
    fn invoked_path_resolves_relative_entrypoint_with_slash() {
        assert_eq!(
            invoked_executable_path_from_argv0(
                PathBuf::from("target/debug/capsem"),
                PathBuf::from("/work")
            ),
            Some(PathBuf::from("/work/target/debug/capsem"))
        );
    }

    #[test]
    fn invoked_path_ignores_path_lookup_entrypoint() {
        assert_eq!(
            invoked_executable_path_from_argv0(PathBuf::from("capsem"), PathBuf::from("/work")),
            None
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
        assert_eq!(paths.mcp_bin.parent().unwrap(), exe_dir);
        assert_eq!(paths.mcp_aggregator_bin.parent().unwrap(), exe_dir);
        assert_eq!(paths.mcp_builtin_bin.parent().unwrap(), exe_dir);
    }

    #[test]
    fn discover_paths_assets_always_under_home() {
        let paths = discover_paths().unwrap();
        assert_eq!(paths.assets_dir, capsem_core::paths::capsem_assets_dir());
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

    #[test]
    fn discover_paths_mcp_helper_bin_names() {
        let paths = discover_paths().unwrap();
        assert_eq!(
            paths.mcp_bin.file_name().unwrap().to_str().unwrap(),
            "capsem-mcp"
        );
        assert_eq!(
            paths
                .mcp_aggregator_bin
                .file_name()
                .unwrap()
                .to_str()
                .unwrap(),
            "capsem-mcp-aggregator"
        );
        assert_eq!(
            paths.mcp_builtin_bin.file_name().unwrap().to_str().unwrap(),
            "capsem-mcp-builtin"
        );
    }

    // -----------------------------------------------------------------------
    // Installed layout contract: what simulate-install.sh produces
    // must be what discover_paths + service startup consume.
    //
    // Layout:
    //   ~/.capsem/bin/capsem{,-service,-process,-mcp,-mcp-aggregator,-mcp-builtin,-gateway,-tray}
    //   ~/.capsem/assets/manifest.json
    //   ~/.capsem/assets/manifest.json.minisig
    //   ~/.capsem/assets/{arch}/{vmlinuz-<hash16>,initrd-<hash16>.img,rootfs-<hash16>.squashfs}
    //   ~/.capsem/run/                     (created at runtime)
    //
    // Service reads:
    //   --assets-dir  -> ~/.capsem/assets/
    //   manifest.json -> assets_dir/manifest.json
    //   rootfs        -> manifest-selected hash-named asset under assets_dir/{arch}/
    // -----------------------------------------------------------------------

    #[test]
    fn service_manifest_path_matches_install_layout() {
        // Service looks for manifest at: assets_dir.join("manifest.json")
        // simulate-install.sh copies to: ~/.capsem/assets/manifest.json
        let home = "/home/test";
        let assets_dir = assets_dir_from_home(home);
        let manifest = assets_dir.join("manifest.json");
        assert_eq!(
            manifest,
            PathBuf::from("/home/test/.capsem/assets/manifest.json")
        );
    }

    #[test]
    fn service_hash_named_assets_path_matches_install_layout() {
        // Service resolves hash-named files from manifest entries.
        // simulate-install.sh copies to: ~/.capsem/assets/{arch}/{hash-named file}
        let home = "/home/test";
        let assets_dir = assets_dir_from_home(home);
        let rootfs = assets_dir
            .join("arm64")
            .join(capsem_core::asset_manager::hash_filename(
                "rootfs.squashfs",
                "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee",
            ));
        assert!(rootfs.to_str().unwrap().contains("/assets/arm64/"));
        assert!(rootfs
            .to_str()
            .unwrap()
            .ends_with("rootfs-b8199dc4a83069b9.squashfs"));
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
