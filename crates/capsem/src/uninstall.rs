use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::platform;

/// Run full uninstall: stop service, remove unit, remove binaries and data.
pub async fn run_uninstall(yes: bool) -> Result<()> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let capsem_dir = PathBuf::from(&home).join(".capsem");

    if !capsem_dir.exists() {
        println!("Nothing to uninstall (~/.capsem does not exist).");
        return Ok(());
    }

    if !yes {
        println!("This will remove:");
        println!("  - Capsem service (LaunchAgent / systemd unit)");
        println!("  - All binaries in ~/.capsem/bin/");
        println!("  - All data in ~/.capsem/ (assets, config, state)");

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

    // Kill any running processes (SIGKILL to prevent respawn by KeepAlive)
    let _ = tokio::process::Command::new("pkill")
        .args(["-9", "-x", "capsem-service"])
        .status()
        .await;
    let _ = tokio::process::Command::new("pkill")
        .args(["-9", "-x", "capsem-process"])
        .status()
        .await;
    let _ = tokio::process::Command::new("pkill")
        .args(["-9", "-x", "capsem-gateway"])
        .status()
        .await;
    let _ = tokio::process::Command::new("pkill")
        .args(["-9", "-x", "capsem-tray"])
        .status()
        .await;

    // Brief wait for processes to die before removing files
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Remove binaries from the detected install location
    const CAPSEM_BINARIES: &[&str] = &[
        "capsem", "capsem-service", "capsem-process", "capsem-mcp",
        "capsem-gateway", "capsem-tray",
    ];
    if let Some(bin_dir) = platform::install_bin_dir() {
        if bin_dir.exists() {
            println!("Removing binaries from {}...", bin_dir.display());
            match platform::detect_install_layout() {
                platform::InstallLayout::MacosPkg => {
                    // NEVER remove_dir_all on a shared dir like /usr/local/bin.
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

    // Remove ~/.capsem entirely
    println!("Removing ~/.capsem...");
    std::fs::remove_dir_all(&capsem_dir).ok();

    // Remove macOS logs
    let log_dir = PathBuf::from(&home).join("Library/Logs/capsem");
    if log_dir.exists() {
        std::fs::remove_dir_all(&log_dir).ok();
    }

    println!("Capsem uninstalled.");
    Ok(())
}
