use std::path::PathBuf;

use anyhow::{Context, Result};

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
    let _ = crate::service_install::uninstall_service().await;

    // Kill any running processes
    let _ = tokio::process::Command::new("pkill")
        .args(["-f", "capsem-service"])
        .status()
        .await;
    let _ = tokio::process::Command::new("pkill")
        .args(["-f", "capsem-process"])
        .status()
        .await;

    // Remove binaries
    let bin_dir = capsem_dir.join("bin");
    if bin_dir.exists() {
        println!("Removing binaries...");
        std::fs::remove_dir_all(&bin_dir).ok();
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
