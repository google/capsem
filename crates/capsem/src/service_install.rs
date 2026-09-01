use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::paths;

const EXPLICIT_STOP_MARKER: &str = "service.explicitly-stopped";

pub fn explicit_stop_marker_path() -> PathBuf {
    capsem_foundation::paths::capsem_run_dir().join(EXPLICIT_STOP_MARKER)
}

pub fn service_explicitly_stopped() -> bool {
    explicit_stop_marker_path().exists()
}

pub fn clear_explicit_stop_marker() -> Result<()> {
    let marker = explicit_stop_marker_path();
    match std::fs::remove_file(&marker) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("remove {}", marker.display())),
    }
}

fn write_explicit_stop_marker() -> Result<()> {
    let marker = explicit_stop_marker_path();
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(&marker, b"stopped\n").with_context(|| format!("write {}", marker.display()))
}

/// Escape a string for safe embedding in XML `<string>` elements.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
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
    let credential_store_path = xml_escape(&format!("{}/.capsem/credentials/credential-store.json", home));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.capsem.service</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>CAPSEM_CREDENTIAL_STORE_PATH</key>
        <string>{credential_store_path}</string>
    </dict>
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
    plist_path().map(|p| p.exists()).unwrap_or(false) || systemd_unit_path().map(|p| p.exists()).unwrap_or(false)
}

/// Refuse service installation when test-isolation env vars are set.
///
/// `capsem install` writes a persistent LaunchAgent / systemd unit whose
/// `--assets-dir` argument is resolved at install time from
/// `capsem_foundation::paths::capsem_assets_dir()`. That helper honors
/// `CAPSEM_HOME` / `CAPSEM_ASSETS_DIR` / `CAPSEM_RUN_DIR`, which the test
/// harness sets to transient paths like `cache/target/test-home/.capsem`. If
/// the install inherits any of them the generated unit permanently points
/// at a directory that gets wiped on every subsequent `just test`,
/// leaving the installed service pointing at non-existent assets. Fail
/// loud instead; the caller must unset these vars before installing.
fn reject_test_isolation_env() -> Result<()> {
    const ISOLATION_VARS: &[&str] = &["CAPSEM_HOME", "CAPSEM_RUN_DIR", "CAPSEM_ASSETS_DIR"];
    let set: Vec<&str> = ISOLATION_VARS
        .iter()
        .filter(|k| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false))
        .copied()
        .collect();
    if set.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "refusing to install service with test-isolation env vars set: {}.\n\
         These point at transient test directories (e.g. cache/target/test-home) \
         that are wiped by `just test`, so an install that inherits them \
         permanently embeds a non-existent path in the LaunchAgent / systemd \
         unit. Unset them and retry: `unset {}`.",
        set.join(", "),
        set.join(" "),
    );
}

/// Install the capsem service as a LaunchAgent (macOS) or systemd user unit (Linux).
pub async fn install_service() -> Result<()> {
    reject_test_isolation_env()?;
    clear_explicit_stop_marker()?;
    let capsem_paths = paths::discover_paths().context("cannot discover paths for service installation")?;
    let home = std::env::var("HOME").context("HOME not set")?;

    if !capsem_paths.service_bin.exists() {
        anyhow::bail!("capsem-service not found at {}", capsem_paths.service_bin.display());
    }
    if !capsem_paths.process_bin.exists() {
        anyhow::bail!("capsem-process not found at {}", capsem_paths.process_bin.display());
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
    clear_explicit_stop_marker()?;

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
    write_explicit_stop_marker()?;

    #[cfg(target_os = "macos")]
    {
        let uid = nix::unistd::getuid();
        let (primary, fallback) = macos_stop_launchagent_plan(uid.as_raw());
        let output = tokio::process::Command::new(primary.program)
            .args(primary.args.iter().map(String::as_str))
            .output()
            .await?;
        if !output.status.success() && macos_launchagent_loaded(uid.as_raw()).await? {
            if let Some(fallback) = fallback {
                let fallback_output = tokio::process::Command::new(fallback.program)
                    .args(fallback.args.iter().map(String::as_str))
                    .output()
                    .await;
                if fallback_output.as_ref().map(|o| !o.status.success()).unwrap_or(true)
                    && macos_launchagent_loaded(uid.as_raw()).await?
                {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    anyhow::bail!("failed to stop capsem service: {}", stderr.trim());
                }
            }
        }
        wait_for_macos_launchagent_unloaded(uid.as_raw()).await?;
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
#[derive(Debug, Clone, PartialEq, Eq)]
struct LaunchctlCommand {
    program: &'static str,
    args: Vec<String>,
}

#[cfg(target_os = "macos")]
fn macos_stop_launchagent_plan(uid: u32) -> (LaunchctlCommand, Option<LaunchctlCommand>) {
    let target = format!("gui/{uid}/com.capsem.service");
    (
        LaunchctlCommand {
            program: "launchctl",
            args: vec!["bootout".to_string(), target],
        },
        plist_path().map(|plist| LaunchctlCommand {
            program: "launchctl",
            args: vec!["unload".to_string(), plist.display().to_string()],
        }),
    )
}

#[cfg(target_os = "macos")]
async fn wait_for_macos_launchagent_unloaded(uid: u32) -> Result<()> {
    for _ in 0..50 {
        if !macos_launchagent_loaded(uid).await? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let target = format!("gui/{uid}/com.capsem.service");
    anyhow::bail!("capsem service still loaded after stop: {target}");
}

#[cfg(target_os = "macos")]
async fn macos_launchagent_loaded(uid: u32) -> Result<bool> {
    let target = format!("gui/{uid}/com.capsem.service");
    let output = tokio::process::Command::new("launchctl")
        .args(["print", &target])
        .output()
        .await?;
    Ok(output.status.success())
}

#[cfg(target_os = "macos")]
async fn install_launchagent(capsem_paths: &paths::CapsemPaths, home: &str) -> Result<()> {
    let plist_dir = PathBuf::from(home).join("Library/LaunchAgents");
    std::fs::create_dir_all(&plist_dir).context("cannot create LaunchAgents directory")?;

    let log_dir = PathBuf::from(home).join("Library/Logs/capsem");
    std::fs::create_dir_all(&log_dir).context("cannot create log directory")?;

    let uid = nix::unistd::getuid();
    let domain = format!("gui/{}", uid);

    // Stop existing launchd jobs and kill ALL capsem processes.
    // 1. Bootout (tells launchd to stop managing + kills managed processes)
    for label in ["com.capsem.service", "com.capsem.tray"] {
        let _ = tokio::process::Command::new("launchctl")
            .args(["bootout", &format!("{domain}/{label}")])
            .output()
            .await;
    }
    // 2. Remove old plist files so launchd doesn't auto-start them
    //    during the bootstrap of other services.
    let _ = std::fs::remove_file(plist_dir.join("com.capsem.service.plist"));
    let _ = std::fs::remove_file(plist_dir.join("com.capsem.tray.plist"));
    // 3. Kill strays not managed by launchd (dev _ensure-service, test crashes,
    //    manual launches). Must cover every binary the service may have spawned
    //    so the (re)install starts from a clean slate -- otherwise orphan
    //    capsem-process instances hold Apple VZ memory across reinstalls.
    //
    // Scope by the installed prefix so we only kill processes from this
    // installation -- `-x <name>` matches every capsem-service on the box,
    // including parallel pytest workers running cache/target/cargo/debug binaries.
    let install_dir = capsem_paths.service_bin.parent().map(|p| p.to_path_buf());
    let scoped_name = |name: &str| -> String {
        install_dir
            .as_ref()
            .map(|d| format!("{}/{name}", d.display()))
            .unwrap_or_else(|| name.to_string())
    };
    let names = ["capsem-service", "capsem-tray", "capsem-gateway", "capsem-process"];
    for name in names {
        let pattern = scoped_name(name);
        let _ = tokio::process::Command::new("pkill")
            .args(["-9", "-f", &pattern])
            .output()
            .await;
    }
    // 4. Wait until all are dead (prevents stale socket EADDRINUSE on bootstrap)
    for _ in 0..30 {
        let mut any_alive = false;
        for name in names {
            let pattern = scoped_name(name);
            let out = tokio::process::Command::new("pgrep")
                .args(["-f", &pattern])
                .output()
                .await;
            if out.map(|o| !o.stdout.is_empty()).unwrap_or(false) {
                any_alive = true;
                break;
            }
        }
        if !any_alive {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    // 5. Remove stale socket so the new service can bind cleanly
    let sock_path = capsem_foundation::paths::service_socket_path();
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
    std::fs::write(&plist_file, &plist_content).context("cannot write service plist")?;
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
            .output()
            .await;
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
    std::fs::create_dir_all(&unit_dir).context("cannot create systemd user unit directory")?;

    let unit_content = generate_systemd_unit(
        &capsem_paths.service_bin,
        &capsem_paths.process_bin,
        &capsem_paths.gateway_bin,
        &capsem_paths.tray_bin,
        &capsem_paths.assets_dir,
    );

    let unit_file = unit_dir.join("capsem.service");
    std::fs::write(&unit_file, &unit_content).context("cannot write systemd unit")?;

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
    let sock = capsem_foundation::paths::service_socket_path();
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
mod tests;
