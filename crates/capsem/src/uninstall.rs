use std::path::Path;

use anyhow::{Context, Result};

use crate::platform;

const CAPSEM_BINARIES: &[&str] = &[
    "capsem",
    "capsem-service",
    "capsem-process",
    "capsem-mcp",
    "capsem-mcp-aggregator",
    "capsem-mcp-builtin",
    "capsem-gateway",
    "capsem-tray",
];

const RUNTIME_PROCESSES: &[&str] = &[
    "capsem-service",
    "capsem-process",
    "capsem-mcp",
    "capsem-mcp-aggregator",
    "capsem-mcp-builtin",
    "capsem-gateway",
    "capsem-tray",
];

/// Run runtime uninstall: stop service, remove units, binaries, and temp state.
pub async fn run_uninstall(yes: bool) -> Result<()> {
    let capsem_dir = capsem_core::paths::capsem_home_opt().context("HOME not set")?;

    if !yes {
        println!("This will remove:");
        println!("  - Capsem service (LaunchAgent / systemd unit)");
        println!("  - Runtime binaries in {}/bin/", capsem_dir.display());
        println!("  - Runtime sockets, pid files, and temporary VM state");
        println!();
        println!("This will preserve:");
        println!("  - user.toml, corp.toml, setup-state.json");
        println!("  - assets, logs, persistent VM state, and session/audit data");

        let confirm = inquire::Confirm::new("Proceed with uninstall?")
            .with_default(false)
            .prompt()
            .context("uninstall cancelled")?;
        if !confirm {
            println!("Uninstall cancelled.");
            return Ok(());
        }
    }

    if !capsem_dir.exists() {
        println!(
            "Nothing to uninstall at {}; checking service/runtime anyway.",
            capsem_dir.display()
        );
    }

    // Stop and uninstall service. In test isolation, CAPSEM_HOME/CAPSEM_RUN_DIR
    // point at a throwaway layout while the platform service unit still lives
    // under the real HOME, so service-manager mutation would hit the wrong
    // install.
    if crate::service_install::test_isolation_env_active() {
        println!("Skipping service-manager uninstall because test-isolation env vars are set.");
    } else {
        println!("Stopping service...");
        if let Err(e) = crate::service_install::uninstall_service().await {
            eprintln!(
                "Warning: service uninstall failed: {}. Continuing anyway.",
                e
            );
        }
    }

    // Kill any running processes (SIGKILL to prevent respawn by KeepAlive).
    //
    // Scope the match to this binary's install dir so `capsem uninstall`
    // from ~/.capsem/bin never touches unrelated capsem-* processes running
    // from other locations (for example dev services under target/debug/, or
    // parallel pytest workers). Users uninstalling the installation should
    // only affect the installation -- `-x <name>` matches too broadly.
    let install_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));
    for name in RUNTIME_PROCESSES {
        let pattern = match install_dir.as_ref() {
            Some(dir) => format!("{}/{name}", dir.display()),
            None => (*name).to_string(),
        };
        let _ = tokio::process::Command::new("pkill")
            .args(["-9", "-f", &pattern])
            .status()
            .await;
    }

    // Brief wait for processes to die before removing files
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Remove binaries from the detected install location.
    if let Some(bin_dir) = platform::install_bin_dir() {
        if bin_dir.exists() {
            println!("Removing binaries from {}...", bin_dir.display());
            match platform::detect_install_layout() {
                platform::InstallLayout::MacosPkg | platform::InstallLayout::LinuxDeb => {
                    // NEVER remove_dir_all on a shared dir like /usr/local/bin or /usr/bin.
                    // Remove only known capsem binaries.
                    remove_known_binaries_from_dir(&bin_dir);
                }
                _ => {
                    // UserDir layout: ~/.capsem/bin/ is ours entirely
                    remove_path(&bin_dir);
                }
            }
        }
    } else {
        // Development layout: remove ~/.capsem/bin if present
        let bin_dir = capsem_dir.join("bin");
        if bin_dir.exists() {
            println!("Removing binaries...");
            remove_path(&bin_dir);
        }
    }

    println!("Removing temporary runtime state...");
    remove_runtime_state(&capsem_dir, &capsem_core::paths::capsem_run_dir())?;

    println!("Capsem runtime uninstalled. Durable user state was preserved.");
    Ok(())
}

fn remove_known_binaries_from_dir(bin_dir: &Path) {
    for name in CAPSEM_BINARIES {
        std::fs::remove_file(bin_dir.join(name)).ok();
    }
}

fn remove_runtime_state(capsem_dir: &Path, run_dir: &Path) -> Result<()> {
    remove_path(&capsem_dir.join("bin"));
    remove_path(&capsem_dir.join("update-check.json"));
    remove_runtime_run_entries(run_dir)?;
    Ok(())
}

fn remove_runtime_run_entries(run_dir: &Path) -> Result<()> {
    if !run_dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(run_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "persistent" || name == "persistent_registry.json" {
            continue;
        }
        remove_path(&entry.path());
    }
    Ok(())
}

fn remove_path(path: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.is_dir() && !meta.file_type().is_symlink() {
        restore_perms(path);
        std::fs::remove_dir_all(path).ok();
    } else {
        std::fs::remove_file(path).ok();
    }
}

/// Recursively chmod directories to 0o700 so remove_dir_all can traverse
/// overlayfs workdirs (which end up mode 000 while mounted).
fn restore_perms(root: &Path) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn known_binary_cleanup_covers_every_installed_helper() {
        let dir = tempfile::tempdir().unwrap();
        for name in CAPSEM_BINARIES {
            write(&dir.path().join(name), b"bin");
        }
        write(&dir.path().join("unrelated"), b"keep");

        remove_known_binaries_from_dir(dir.path());

        for name in CAPSEM_BINARIES {
            assert!(!dir.path().join(name).exists(), "{name} should be removed");
        }
        assert!(dir.path().join("unrelated").exists());
    }

    #[test]
    fn runtime_uninstall_preserves_durable_state() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let run = home.join("run");

        write(&home.join("bin/capsem"), b"bin");
        write(&home.join("bin/capsem-mcp-builtin"), b"bin");
        write(&home.join("user.toml"), b"[ai]\n");
        write(&home.join("corp.toml"), b"[net]\n");
        write(&home.join("corp-source.json"), b"{}\n");
        write(&home.join("setup-state.json"), b"{}\n");
        write(&home.join("assets/arm64/rootfs.squashfs"), b"rootfs");
        write(&home.join("logs/app.log"), b"log");
        write(&home.join("sessions/main.db"), b"session index");
        write(&home.join("update-check.json"), b"cache");

        write(&run.join("service.sock"), b"sock");
        write(&run.join("service.pid"), b"123");
        write(&run.join("gateway.port"), b"19222");
        write(&run.join("gateway.token"), b"token");
        write(&run.join("instances/vm.sock"), b"sock");
        write(&run.join("sessions/temp-vm/rootfs.img"), b"temp");
        write(&run.join("persistent/saved-vm/state.vz"), b"saved");
        write(&run.join("persistent_registry.json"), b"{\"vms\":[]}");

        remove_runtime_state(home, &run).unwrap();

        assert!(!home.join("bin").exists());
        assert!(!home.join("update-check.json").exists());
        assert!(!run.join("service.sock").exists());
        assert!(!run.join("service.pid").exists());
        assert!(!run.join("gateway.port").exists());
        assert!(!run.join("gateway.token").exists());
        assert!(!run.join("instances").exists());
        assert!(!run.join("sessions").exists());

        assert!(home.join("user.toml").exists());
        assert!(home.join("corp.toml").exists());
        assert!(home.join("corp-source.json").exists());
        assert!(home.join("setup-state.json").exists());
        assert!(home.join("assets/arm64/rootfs.squashfs").exists());
        assert!(home.join("logs/app.log").exists());
        assert!(home.join("sessions/main.db").exists());
        assert!(run.join("persistent/saved-vm/state.vz").exists());
        assert!(run.join("persistent_registry.json").exists());
    }
}
