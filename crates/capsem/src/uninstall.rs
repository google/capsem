use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::platform;

/// Run full uninstall: stop service, remove unit, remove binaries and data.
pub async fn run_uninstall(yes: bool) -> Result<()> {
    let capsem_dir = capsem_foundation::paths::capsem_home_opt().context("HOME not set")?;

    if !capsem_dir.exists() {
        println!("Nothing to uninstall ({} does not exist).", capsem_dir.display());
        return Ok(());
    }

    if !yes {
        println!("This will remove:");
        println!("  - Capsem service (LaunchAgent / systemd unit)");
        println!("  - All binaries in {}/bin/", capsem_dir.display());
        println!("  - All data in {}/ (assets, config, state)", capsem_dir.display());

        let confirm = inquire::Confirm::new("Proceed with uninstall?")
            .with_default(false)
            .prompt()
            .context("uninstall cancelled")?;
        if !confirm {
            println!("Uninstall cancelled.");
            return Ok(());
        }
    }

    // Stop and uninstall service
    println!("Stopping service...");
    if let Err(e) = crate::service_install::uninstall_service().await {
        eprintln!("Warning: service uninstall failed: {}. Continuing anyway.", e);
    }

    // Retire only native helpers whose kernel executable path belongs to this
    // installation. A command line containing that path proves nothing.
    let install_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));
    if let Some(dir) = install_dir {
        crate::service_install::retire_installed_helpers(&dir).await?;
    }

    // Remove binaries from the detected install location
    const CAPSEM_BINARIES: &[&str] = &[
        "capsem",
        "capsem-admin",
        "capsem-app",
        "capsem-bench-rs",
        "capsem-gateway",
        "capsem-ledger",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
        "capsem-mock-server",
        "capsem-service",
        "capsem-process",
        "capsem-proxy",
        "capsem-router",
        "capsem-tray",
        "capsem-tui",
    ];
    if let Some(bin_dir) = platform::install_bin_dir() {
        if bin_dir.exists() {
            println!("Removing binaries from {}...", bin_dir.display());
            match platform::detect_install_layout() {
                platform::InstallLayout::MacosPkg | platform::InstallLayout::LinuxDeb => {
                    // NEVER remove_dir_all on a shared dir like /usr/local/bin or /usr/bin.
                    // Remove only known capsem binaries.
                    for name in CAPSEM_BINARIES {
                        std::fs::remove_file(bin_dir.join(name)).ok();
                    }
                }
                _ => {
                    // UserDir layout: ~/.capsem/bin/ is ours entirely
                    std::fs::remove_dir_all(&bin_dir).ok();
                }
            }
        }
    } else {
        // Development layout: remove ~/.capsem/bin if present
        let bin_dir = capsem_dir.join("bin");
        if bin_dir.exists() {
            println!("Removing binaries...");
            std::fs::remove_dir_all(&bin_dir).ok();
        }
    }

    // Remove the capsem home entirely. Overlayfs workdirs under sessions/*/work
    // end up with mode 000 while the VM is running; chmod the tree back to 0o700
    // so remove_dir_all can traverse it.
    println!("Removing {}...", capsem_dir.display());
    restore_perms(&capsem_dir);
    if let Err(e) = std::fs::remove_dir_all(&capsem_dir) {
        eprintln!("Warning: failed to remove {}: {}", capsem_dir.display(), e);
    }

    // Remove macOS logs (always under the real user $HOME, not CAPSEM_HOME).
    if let Ok(home) = std::env::var("HOME") {
        let log_dir = PathBuf::from(&home).join("Library/Logs/capsem");
        if log_dir.exists() {
            std::fs::remove_dir_all(&log_dir).ok();
        }
    }

    println!("Capsem uninstalled.");
    Ok(())
}

/// Recursively chmod directories to 0o700 so remove_dir_all can traverse
/// overlayfs workdirs (which end up mode 000 while mounted).
fn restore_perms(root: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            if meta.is_dir() {
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
                restore_perms(&path);
            }
        }
    }
}
