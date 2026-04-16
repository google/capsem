use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

use crate::paths;

/// Escape a string for safe embedding in XML `<string>` elements.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape a path for systemd ExecStart (spaces must be escaped).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn systemd_escape_path(p: &Path) -> String {
    p.display().to_string().replace(' ', "\\x20")
}

/// Service installation status.
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub pid: Option<u32>,
    pub unit_path: Option<PathBuf>,
}

/// Generate a macOS LaunchAgent plist for capsem-service.
///
/// All paths are absolute and XML-escaped for safe embedding.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn generate_plist(
    service_bin: &Path,
    process_bin: &Path,
    gateway_bin: &Path,
    tray_bin: &Path,
    assets_dir: &Path,
    home: &str,
) -> String {
    let log_dir = xml_escape(&format!("{}/Library/Logs/capsem", home));
    let service_bin = xml_escape(&service_bin.display().to_string());
    let process_bin = xml_escape(&process_bin.display().to_string());
    let gateway_bin = xml_escape(&gateway_bin.display().to_string());
    let tray_bin = xml_escape(&tray_bin.display().to_string());
    let assets_dir = xml_escape(&assets_dir.display().to_string());
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
        <string>--gateway-binary</string>
        <string>{gateway_bin}</string>
        <string>--tray-binary</string>
        <string>{tray_bin}</string>
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
    )
}

/// Generate a systemd user unit file for capsem-service.
///
/// All paths are absolute. Spaces are escaped with `\x20` per systemd syntax.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn generate_systemd_unit(
    service_bin: &Path,
    process_bin: &Path,
    gateway_bin: &Path,
    tray_bin: &Path,
    assets_dir: &Path,
) -> String {
    let service_bin = systemd_escape_path(service_bin);
    let process_bin = systemd_escape_path(process_bin);
    let gateway_bin = systemd_escape_path(gateway_bin);
    let tray_bin = systemd_escape_path(tray_bin);
    let assets_dir = systemd_escape_path(assets_dir);
    format!(
        r#"[Unit]
Description=Capsem sandbox service

[Service]
ExecStart={service_bin} --foreground --assets-dir {assets_dir} --process-binary {process_bin} --gateway-binary {gateway_bin} --tray-binary {tray_bin}
Restart=always
RestartSec=2

[Install]
WantedBy=default.target
"#,
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
    if !capsem_paths.process_bin.exists() {
        anyhow::bail!(
            "capsem-process not found at {}",
            capsem_paths.process_bin.display()
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

/// Start the capsem service via the platform service manager.
pub async fn start_service() -> Result<()> {
    if !is_service_installed() {
        anyhow::bail!("Service not installed. Run `capsem install` first.");
    }

    #[cfg(target_os = "macos")]
    {
        let uid = nix::unistd::getuid();
        let target = format!("gui/{}/com.capsem.service", uid);
        let status = tokio::process::Command::new("launchctl")
            .args(["kickstart", "-k", &target])
            .status()
            .await?;
        if !status.success() {
            // Fallback: bootstrap the plist
            if let Some(plist) = plist_path() {
                let domain = format!("gui/{}", uid);
                let _ = tokio::process::Command::new("launchctl")
                    .args(["bootstrap", &domain, &plist.to_string_lossy()])
                    .status()
                    .await;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let status = tokio::process::Command::new("systemctl")
            .args(["--user", "start", "capsem"])
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("systemctl --user start capsem failed");
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        anyhow::bail!("service start not supported on this platform");
    }

    Ok(())
}

/// Stop the capsem service via the platform service manager.
pub async fn stop_service() -> Result<()> {
    if !is_service_installed() {
        anyhow::bail!("Service not installed. Run `capsem install` first.");
    }

    #[cfg(target_os = "macos")]
    {
        let uid = nix::unistd::getuid();
        let target = format!("gui/{}/com.capsem.service", uid);
        let status = tokio::process::Command::new("launchctl")
            .args(["kill", "SIGTERM", &target])
            .status()
            .await?;
        if !status.success() {
            // Fallback: unload/load cycle
            if let Some(plist) = plist_path() {
                let _ = tokio::process::Command::new("launchctl")
                    .args(["unload", &plist.to_string_lossy()])
                    .status()
                    .await;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let status = tokio::process::Command::new("systemctl")
            .args(["--user", "stop", "capsem"])
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("systemctl --user stop capsem failed");
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        anyhow::bail!("service stop not supported on this platform");
    }

    Ok(())
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

    let uid = nix::unistd::getuid();
    let domain = format!("gui/{}", uid);

    // Stop existing launchd jobs and kill ALL capsem processes.
    // 1. Bootout (tells launchd to stop managing + kills managed processes)
    for label in ["com.capsem.service", "com.capsem.tray"] {
        let _ = tokio::process::Command::new("launchctl")
            .args(["bootout", &format!("{domain}/{label}")])
            .output().await;
    }
    // 2. Remove old plist files so launchd doesn't auto-start them
    //    during the bootstrap of other services.
    let _ = std::fs::remove_file(plist_dir.join("com.capsem.service.plist"));
    let _ = std::fs::remove_file(plist_dir.join("com.capsem.tray.plist"));
    // 3. Kill strays not managed by launchd (dev _ensure-service, manual launches)
    let _ = tokio::process::Command::new("pkill")
        .args(["-9", "-x", "capsem-service"]).output().await;
    let _ = tokio::process::Command::new("pkill")
        .args(["-9", "-x", "capsem-tray"]).output().await;
    // 4. Wait until all are dead (prevents stale socket EADDRINUSE on bootstrap)
    for _ in 0..30 {
        let svc = tokio::process::Command::new("pgrep")
            .args(["-x", "capsem-service"]).output().await;
        let tray = tokio::process::Command::new("pgrep")
            .args(["-x", "capsem-tray"]).output().await;
        let svc_dead = svc.map(|o| o.stdout.is_empty()).unwrap_or(true);
        let tray_dead = tray.map(|o| o.stdout.is_empty()).unwrap_or(true);
        if svc_dead && tray_dead { break; }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    // 5. Remove stale socket so the new service can bind cleanly
    let sock_path = PathBuf::from(home).join(".capsem/run/service.sock");
    let _ = std::fs::remove_file(&sock_path);

    // Install service plist
    let plist_content = generate_plist(
        &capsem_paths.service_bin,
        &capsem_paths.process_bin,
        &capsem_paths.gateway_bin,
        &capsem_paths.tray_bin,
        &capsem_paths.assets_dir,
        home,
    );
    let plist_file = plist_dir.join("com.capsem.service.plist");
    std::fs::write(&plist_file, &plist_content)
        .context("cannot write service plist")?;
    bootstrap_launchagent(&domain, &plist_file).await?;

    Ok(())
}

#[cfg(target_os = "macos")]
async fn bootstrap_launchagent(domain: &str, plist_file: &Path) -> Result<()> {
    let status = tokio::process::Command::new("launchctl")
        .args(["bootstrap", domain, &plist_file.to_string_lossy()])
        .status()
        .await?;
    if !status.success() {
        let status = tokio::process::Command::new("launchctl")
            .args(["load", &plist_file.to_string_lossy()])
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("launchctl load failed for {}", plist_file.display());
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn uninstall_launchagent() -> Result<()> {
    let uid = nix::unistd::getuid();

    // Uninstall service
    if let Some(plist_file) = plist_path() {
        if plist_file.exists() {
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
        }
    }

    // Clean up stale tray plist if it exists (tray is spawned by the service)
    let home = std::env::var("HOME").unwrap_or_default();
    let tray_plist = PathBuf::from(&home).join("Library/LaunchAgents/com.capsem.tray.plist");
    if tray_plist.exists() {
        let _ = tokio::process::Command::new("launchctl")
            .args(["bootout", &format!("gui/{}/com.capsem.tray", uid)])
            .output().await;
        std::fs::remove_file(&tray_plist).ok();
    }

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
        &capsem_paths.gateway_bin,
        &capsem_paths.tray_bin,
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
        // Get actual PID via pgrep (pidfile may be stale)
        let pid = tokio::process::Command::new("pgrep")
            .args(["-x", "capsem-service"])
            .output()
            .await
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.lines().next().and_then(|l| l.trim().parse::<u32>().ok()));
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
            Path::new("/Users/test/.capsem/bin/capsem-gateway"),
            Path::new("/Users/test/.capsem/bin/capsem-tray"),
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
            Path::new("/usr/local/bin/capsem-gateway"),
            Path::new("/usr/local/bin/capsem-tray"),
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
            Path::new("/bin/capsem-gateway"),
            Path::new("/bin/capsem-tray"),
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
            Path::new("/home/test/.capsem/bin/capsem-gateway"),
            Path::new("/home/test/.capsem/bin/capsem-tray"),
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
            Path::new("/bin/gw"),
            Path::new("/bin/tray"),
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
            Path::new("/bin/gw"),
            Path::new("/bin/tray"),
            Path::new("/assets"),
        );
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    // -- XML escaping ---------------------------------------------------------

    #[test]
    fn test_xml_escape_clean_path() {
        assert_eq!(xml_escape("/usr/local/bin"), "/usr/local/bin");
    }

    #[test]
    fn test_xml_escape_ampersand() {
        assert_eq!(xml_escape("/Users/AT&T/bin"), "/Users/AT&amp;T/bin");
    }

    #[test]
    fn test_xml_escape_angle_brackets() {
        assert_eq!(xml_escape("a<b>c"), "a&lt;b&gt;c");
    }

    #[test]
    fn test_plist_with_special_chars_in_path() {
        let plist = generate_plist(
            Path::new("/Users/AT&T Corp/.capsem/bin/capsem-service"),
            Path::new("/Users/AT&T Corp/.capsem/bin/capsem-process"),
            Path::new("/Users/AT&T Corp/.capsem/bin/capsem-gateway"),
            Path::new("/Users/AT&T Corp/.capsem/bin/capsem-tray"),
            Path::new("/Users/AT&T Corp/.capsem/assets"),
            "/Users/AT&T Corp",
        );
        // Must contain escaped ampersands, not raw &
        assert!(plist.contains("AT&amp;T"), "plist must XML-escape ampersands");
        assert!(!plist.contains("AT&T "), "plist must not have unescaped &");
        // Must still be valid-ish XML (balanced tags)
        assert!(plist.contains("</plist>"));
    }

    // -- systemd space escaping -----------------------------------------------

    #[test]
    fn test_systemd_escape_path_no_spaces() {
        let p = Path::new("/home/user/.capsem/bin/capsem-service");
        assert_eq!(systemd_escape_path(p), "/home/user/.capsem/bin/capsem-service");
    }

    #[test]
    fn test_systemd_escape_path_with_spaces() {
        let p = Path::new("/home/John Doe/.capsem/bin/capsem-service");
        let escaped = systemd_escape_path(p);
        assert_eq!(escaped, "/home/John\\x20Doe/.capsem/bin/capsem-service");
        assert!(!escaped.contains(' '), "spaces must be escaped for systemd");
    }

    #[test]
    fn test_systemd_unit_with_spaces_in_path() {
        let unit = generate_systemd_unit(
            Path::new("/home/John Doe/.capsem/bin/capsem-service"),
            Path::new("/home/John Doe/.capsem/bin/capsem-process"),
            Path::new("/home/John Doe/.capsem/assets"),
        );
        let exec_line = unit.lines().find(|l| l.starts_with("ExecStart=")).unwrap();
        // Spaces must be escaped as \x20 in ExecStart
        assert!(!exec_line.contains("John Doe"), "unescaped space in ExecStart: {}", exec_line);
        assert!(exec_line.contains("John\\x20Doe"), "missing \\x20 escape: {}", exec_line);
    }
}
