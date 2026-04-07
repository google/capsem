use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

use crate::paths;

/// Service installation status.
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub pid: Option<u32>,
    pub unit_path: Option<PathBuf>,
}

/// Generate a macOS LaunchAgent plist for capsem-service.
///
/// All paths are absolute. Uses discover_paths() for binary locations.
pub fn generate_plist(
    service_bin: &Path,
    process_bin: &Path,
    assets_dir: &Path,
    home: &str,
) -> String {
    let log_dir = format!("{}/Library/Logs/capsem", home);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.capsem.service</string>
    <key>ProgramArguments</key>
    <array>
        <string>{service_bin}</string>
        <string>--foreground</string>
        <string>--assets-dir</string>
        <string>{assets_dir}</string>
        <string>--process-binary</string>
        <string>{process_bin}</string>
    </array>
    <key>KeepAlive</key>
    <true/>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{log_dir}/service.log</string>
    <key>StandardErrorPath</key>
    <string>{log_dir}/service.log</string>
</dict>
</plist>
"#,
        service_bin = service_bin.display(),
        process_bin = process_bin.display(),
        assets_dir = assets_dir.display(),
        log_dir = log_dir,
    )
}

/// Generate a systemd user unit file for capsem-service.
///
/// All paths are absolute.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn generate_systemd_unit(
    service_bin: &Path,
    process_bin: &Path,
    assets_dir: &Path,
) -> String {
    format!(
        r#"[Unit]
Description=Capsem sandbox service

[Service]
ExecStart={service_bin} --foreground --assets-dir {assets_dir} --process-binary {process_bin}
Restart=always
RestartSec=2

[Install]
WantedBy=default.target
"#,
        service_bin = service_bin.display(),
        process_bin = process_bin.display(),
        assets_dir = assets_dir.display(),
    )
}

/// Check if the capsem service is installed on the current platform.
pub fn is_service_installed() -> bool {
    plist_path().map(|p| p.exists()).unwrap_or(false)
        || systemd_unit_path().map(|p| p.exists()).unwrap_or(false)
}

/// Install the capsem service as a LaunchAgent (macOS) or systemd user unit (Linux).
pub async fn install_service() -> Result<()> {
    let capsem_paths = paths::discover_paths()
        .context("cannot discover paths for service installation")?;
    let home = std::env::var("HOME").context("HOME not set")?;

    if !capsem_paths.service_bin.exists() {
        anyhow::bail!(
            "capsem-service not found at {}",
            capsem_paths.service_bin.display()
        );
    }

    #[cfg(target_os = "macos")]
    {
        install_launchagent(&capsem_paths, &home).await?;
    }

    #[cfg(target_os = "linux")]
    {
        install_systemd_unit(&capsem_paths, &home).await?;
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        anyhow::bail!("service installation not supported on this platform");
    }

    Ok(())
}

/// Uninstall the capsem service.
pub async fn uninstall_service() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        uninstall_launchagent().await?;
    }

    #[cfg(target_os = "linux")]
    {
        uninstall_systemd_unit().await?;
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        anyhow::bail!("service uninstallation not supported on this platform");
    }

    Ok(())
}

/// Get the current service status.
pub async fn service_status() -> Result<ServiceStatus> {
    let plist_installed = plist_path().map(|p| p.exists()).unwrap_or(false);
    let unit_installed = systemd_unit_path().map(|p| p.exists()).unwrap_or(false);
    let installed = plist_installed || unit_installed;

    let unit_path = if plist_installed {
        plist_path()
    } else if unit_installed {
        systemd_unit_path()
    } else {
        None
    };

    let (running, pid) = check_running().await;

    Ok(ServiceStatus {
        installed,
        running,
        pid,
        unit_path,
    })
}

// --- macOS LaunchAgent ---

pub fn plist_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join("Library/LaunchAgents/com.capsem.service.plist"))
}

#[cfg(target_os = "macos")]
async fn install_launchagent(capsem_paths: &paths::CapsemPaths, home: &str) -> Result<()> {
    let plist_dir = PathBuf::from(home).join("Library/LaunchAgents");
    std::fs::create_dir_all(&plist_dir)
        .context("cannot create LaunchAgents directory")?;

    let log_dir = PathBuf::from(home).join("Library/Logs/capsem");
    std::fs::create_dir_all(&log_dir)
        .context("cannot create log directory")?;

    let plist_content = generate_plist(
        &capsem_paths.service_bin,
        &capsem_paths.process_bin,
        &capsem_paths.assets_dir,
        home,
    );

    let plist_file = plist_dir.join("com.capsem.service.plist");
    std::fs::write(&plist_file, &plist_content)
        .context("cannot write plist")?;

    // Try modern bootstrap, fall back to legacy load
    let uid = nix::unistd::getuid();
    let domain = format!("gui/{}", uid);
    let status = tokio::process::Command::new("launchctl")
        .args(["bootstrap", &domain, &plist_file.to_string_lossy()])
        .status()
        .await?;

    if !status.success() {
        // Fallback to legacy load
        let status = tokio::process::Command::new("launchctl")
            .args(["load", &plist_file.to_string_lossy()])
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("launchctl load failed");
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
async fn uninstall_launchagent() -> Result<()> {
    let plist_file = plist_path().context("HOME not set")?;

    if !plist_file.exists() {
        println!("Service not installed.");
        return Ok(());
    }

    // Try modern bootout, fall back to legacy unload
    let uid = nix::unistd::getuid();
    let target = format!("gui/{}/com.capsem.service", uid);
    let status = tokio::process::Command::new("launchctl")
        .args(["bootout", &target])
        .status()
        .await?;

    if !status.success() {
        let _ = tokio::process::Command::new("launchctl")
            .args(["unload", &plist_file.to_string_lossy()])
            .status()
            .await;
    }

    std::fs::remove_file(&plist_file).ok();
    Ok(())
}

// --- Linux systemd ---

pub fn systemd_unit_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".config/systemd/user/capsem.service"))
}

#[cfg(target_os = "linux")]
async fn install_systemd_unit(capsem_paths: &paths::CapsemPaths, home: &str) -> Result<()> {
    let unit_dir = PathBuf::from(home).join(".config/systemd/user");
    std::fs::create_dir_all(&unit_dir)
        .context("cannot create systemd user unit directory")?;

    let unit_content = generate_systemd_unit(
        &capsem_paths.service_bin,
        &capsem_paths.process_bin,
        &capsem_paths.assets_dir,
    );

    let unit_file = unit_dir.join("capsem.service");
    std::fs::write(&unit_file, &unit_content)
        .context("cannot write systemd unit")?;

    // daemon-reload + enable --now
    let status = tokio::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("systemctl --user daemon-reload failed");
    }

    let status = tokio::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "capsem"])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("systemctl --user enable --now capsem failed");
    }

    Ok(())
}

#[cfg(target_os = "linux")]
async fn uninstall_systemd_unit() -> Result<()> {
    let unit_file = systemd_unit_path().context("HOME not set")?;

    if !unit_file.exists() {
        println!("Service not installed.");
        return Ok(());
    }

    let _ = tokio::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", "capsem"])
        .status()
        .await;

    let _ = tokio::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()
        .await;

    std::fs::remove_file(&unit_file).ok();
    Ok(())
}

// --- Common helpers ---

async fn check_running() -> (bool, Option<u32>) {
    // Check via socket connectivity
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return (false, None),
    };
    let sock = PathBuf::from(&home).join(".capsem/run/service.sock");
    if tokio::net::UnixStream::connect(&sock).await.is_ok() {
        // Try to get PID from pidfile
        let pidfile = PathBuf::from(&home).join(".capsem/run/service.pid");
        let pid = std::fs::read_to_string(&pidfile)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok());
        return (true, pid);
    }
    (false, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_generate_plist_absolute_paths() {
        let plist = generate_plist(
            Path::new("/Users/test/.capsem/bin/capsem-service"),
            Path::new("/Users/test/.capsem/bin/capsem-process"),
            Path::new("/Users/test/.capsem/assets"),
            "/Users/test",
        );
        // ProgramArguments binary and path args must be absolute
        assert!(plist.contains("<string>/Users/test/.capsem/bin/capsem-service</string>"));
        assert!(plist.contains("<string>/Users/test/.capsem/bin/capsem-process</string>"));
        assert!(plist.contains("<string>/Users/test/.capsem/assets</string>"));
        // Log path must be absolute
        assert!(plist.contains("<string>/Users/test/Library/Logs/capsem/service.log</string>"));
        // No tilde in paths
        assert!(!plist.contains("~"), "plist should not contain ~");
    }

    #[test]
    fn test_generate_plist_valid_xml() {
        let plist = generate_plist(
            Path::new("/usr/local/bin/capsem-service"),
            Path::new("/usr/local/bin/capsem-process"),
            Path::new("/home/test/.capsem/assets"),
            "/home/test",
        );
        assert!(plist.starts_with("<?xml"));
        assert!(plist.contains("<plist version=\"1.0\">"));
        assert!(plist.contains("</plist>"));
        // Balanced dict tags
        let open_dicts = plist.matches("<dict>").count();
        let close_dicts = plist.matches("</dict>").count();
        assert_eq!(open_dicts, close_dicts, "unbalanced <dict> tags");
    }

    #[test]
    fn test_generate_plist_has_keep_alive() {
        let plist = generate_plist(
            Path::new("/bin/capsem-service"),
            Path::new("/bin/capsem-process"),
            Path::new("/assets"),
            "/home",
        );
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("<true/>"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
    }

    #[test]
    fn test_generate_systemd_unit_absolute_paths() {
        let unit = generate_systemd_unit(
            Path::new("/home/test/.capsem/bin/capsem-service"),
            Path::new("/home/test/.capsem/bin/capsem-process"),
            Path::new("/home/test/.capsem/assets"),
        );
        // ExecStart line should have absolute path
        let exec_line = unit.lines().find(|l| l.starts_with("ExecStart=")).unwrap();
        assert!(
            exec_line.starts_with("ExecStart=/"),
            "ExecStart must use absolute path: {}",
            exec_line
        );
        // --process-binary value should be absolute
        assert!(exec_line.contains("--process-binary /"));
        // --assets-dir value should be absolute
        assert!(exec_line.contains("--assets-dir /"));
    }

    #[test]
    fn test_generate_systemd_unit_restart_policy() {
        let unit = generate_systemd_unit(
            Path::new("/bin/svc"),
            Path::new("/bin/proc"),
            Path::new("/assets"),
        );
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("RestartSec=2"));
    }

    #[test]
    fn test_generate_systemd_unit_wanted_by() {
        let unit = generate_systemd_unit(
            Path::new("/bin/svc"),
            Path::new("/bin/proc"),
            Path::new("/assets"),
        );
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("WantedBy=default.target"));
    }
}
