//! Pasteable debug report for Settings -> About.
//!
//! This is intentionally smaller than `capsem support-bundle`: it produces
//! redacted text that users can paste into a bug without unpacking a tarball.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug)]
pub struct DebugReportInput {
    pub generated_at: String,
    pub version: String,
    pub build_hash: String,
    pub build_ts: String,
    pub platform: String,
    pub capsem_home: PathBuf,
    pub run_dir: PathBuf,
    pub assets_dir: PathBuf,
    pub asset_locations: Option<capsem_core::settings_profiles::ResolvedServiceAssetLocations>,
    pub manifest: Option<capsem_core::asset_manager::ManifestV2>,
    pub running_vm_count: usize,
    pub total_vm_count: usize,
    pub status_issues: Vec<String>,
    pub defunct_sessions: Vec<DefunctSessionReport>,
    pub install: Option<InstallReportInput>,
    pub process_pids: Vec<ProcessReportInput>,
    pub settings_profiles: Option<capsem_core::settings_profiles::SettingsProfilesDebugSnapshot>,
}

#[derive(Debug, Clone)]
pub struct InstallReportInput {
    pub bin_dir: PathBuf,
    pub current_exe: PathBuf,
    pub service_unit_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ProcessReportInput {
    pub name: String,
    pub pid: Option<u32>,
    pub executable_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugReport {
    pub text: String,
    pub json: DebugReportJson,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugReportJson {
    pub schema: String,
    pub redacted: bool,
    pub generated_at: String,
    pub version: VersionReport,
    pub paths: PathsReport,
    pub runtime: RuntimeReport,
    pub host: HostReport,
    pub disk: DiskReport,
    pub install: InstallReport,
    pub host_binaries: BTreeMap<String, BinaryReport>,
    pub processes: Vec<ProcessReport>,
    pub status: DebugStatusReport,
    pub setup: SetupReport,
    pub assets: AssetsReport,
    pub logs: Vec<LogTailReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionReport {
    pub capsem_version: String,
    pub build_hash: String,
    pub build_ts: String,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathsReport {
    pub capsem_home: String,
    pub run_dir: String,
    pub assets_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeReport {
    pub running_vm_count: usize,
    pub total_vm_count: usize,
    pub service_pid_file: FileSnapshot,
    pub gateway_pid_file: FileSnapshot,
    pub gateway_port_file: FileSnapshot,
    pub gateway_token_file: FileSnapshot,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostReport {
    pub os: String,
    pub arch: String,
    pub family: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskReport {
    pub capsem_home: DiskPathReport,
    pub run_dir: DiskPathReport,
    pub assets_dir: DiskPathReport,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskPathReport {
    pub path: String,
    pub exists: bool,
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallReport {
    pub bin_dir: Option<String>,
    pub current_exe: Option<String>,
    pub service_unit_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BinaryReport {
    pub path: String,
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub mode_octal: Option<String>,
    pub executable: bool,
    pub hash: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessReport {
    pub name: String,
    pub pid: Option<u32>,
    pub running: Option<bool>,
    pub executable_path: Option<String>,
    pub executable_hash: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugStatusReport {
    pub issues: Vec<String>,
    pub defunct_sessions: Vec<DefunctSessionReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DefunctSessionReport {
    pub name: String,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StatusIssuesInput {
    pub gateway_port_file_exists: bool,
    pub gateway_token_file_exists: bool,
    pub assets_dir_exists: bool,
    pub manifest_present: bool,
    pub resolved_assets: std::result::Result<StatusResolvedAssets, String>,
    pub defunct_session_count: usize,
}

#[derive(Debug, Clone)]
pub struct StatusResolvedAssets {
    pub kernel: PathBuf,
    pub initrd: PathBuf,
    pub rootfs: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupReport {
    pub path: String,
    pub present: bool,
    pub parse_error: Option<String>,
    pub schema_version: u32,
    pub current_onboarding_version: u32,
    pub completed_steps: Vec<String>,
    pub security_preset: Option<String>,
    pub providers_done: bool,
    pub repositories_done: bool,
    pub service_installed: bool,
    pub vm_verified: bool,
    pub install_completed: bool,
    pub onboarding_completed: bool,
    pub onboarding_version: u32,
    pub needs_onboarding: bool,
    pub corp_config_source_present: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetsReport {
    pub manifest: ManifestReport,
    pub asset_version_for_binary: Option<String>,
    pub files: BTreeMap<String, AssetFileReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManifestReport {
    pub present: bool,
    pub path: String,
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub hash: Option<String>,
    pub signature_file: FileSnapshot,
    pub dev_pubkey_file: FileSnapshot,
    pub assets_current: Option<String>,
    pub binaries_current: Option<String>,
    pub arch: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetFileReport {
    pub logical: String,
    pub path: String,
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub manifest_hash: String,
    pub manifest_size_bytes: u64,
    pub actual_hash: Option<String>,
    pub actual_hash_matches_manifest: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileSnapshot {
    pub path: String,
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub hash: Option<String>,
    pub contents: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogTailReport {
    pub name: String,
    pub path: String,
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub truncated: bool,
    pub tail: Vec<String>,
    pub error: Option<String>,
}

pub fn build_debug_report(input: DebugReportInput) -> Result<DebugReport> {
    let version = VersionReport {
        capsem_version: input.version.clone(),
        build_hash: input.build_hash.clone(),
        build_ts: input.build_ts.clone(),
        platform: input.platform.clone(),
    };
    let paths = PathsReport {
        capsem_home: redact_path_for_report(&input.capsem_home),
        run_dir: redact_path_for_report(&input.run_dir),
        assets_dir: redact_path_for_report(&input.assets_dir),
    };
    let runtime =
        build_runtime_report(&input.run_dir, input.running_vm_count, input.total_vm_count);
    let host = build_host_report(&input.platform);
    let disk = build_disk_report(&input);
    let install = build_install_report(input.install.as_ref());
    let host_binaries = build_host_binary_report(&input);
    let processes = build_process_report(&input.process_pids);
    let status = build_status_report(&input);
    let setup = build_setup_report(&input.capsem_home);
    let assets = build_asset_report(&input)?;
    let logs = collect_log_tails(&input.capsem_home, &input.run_dir);

    let mut lines = Vec::new();
    lines.push("Capsem Debug Report".to_string());
    lines.push("redacted: true".to_string());
    lines.push(format!("generated_at: {}", input.generated_at));
    lines.push(String::new());
    lines.push("[version]".to_string());
    lines.push(format!("capsem_version: {}", version.capsem_version));
    lines.push(format!("build_hash: {}", version.build_hash));
    lines.push(format!("build_ts: {}", version.build_ts));
    lines.push(format!("platform: {}", version.platform));
    lines.push(String::new());
    lines.push("[paths]".to_string());
    lines.push(format!("capsem_home: {}", paths.capsem_home));
    lines.push(format!("run_dir: {}", paths.run_dir));
    lines.push(format!("assets_dir: {}", paths.assets_dir));
    if let Some(locations) = input.asset_locations.as_ref() {
        append_asset_locations_report(&mut lines, locations);
    }
    lines.push(String::new());
    lines.push("[runtime]".to_string());
    append_runtime_report(&mut lines, &runtime);
    lines.push(String::new());
    lines.push("[host]".to_string());
    append_host_report(
        &mut lines,
        &host,
        &disk,
        &install,
        &host_binaries,
        &processes,
    );
    lines.push(String::new());
    lines.push("[status]".to_string());
    append_status_report(&mut lines, &status);
    lines.push(String::new());
    lines.push("[setup]".to_string());
    append_setup_report(&mut lines, &setup);
    lines.push(String::new());
    lines.push("[settings_profiles]".to_string());
    append_settings_profiles_report(&mut lines, input.settings_profiles.as_ref());
    lines.push(String::new());
    lines.push("[assets]".to_string());
    append_asset_report(&mut lines, &assets);
    lines.push(String::new());
    lines.push("[logs]".to_string());
    append_logs_report(&mut lines, &logs);

    let json = DebugReportJson {
        schema: "capsem.debug.v2".to_string(),
        redacted: true,
        generated_at: input.generated_at,
        version,
        paths,
        runtime,
        host,
        disk,
        install,
        host_binaries,
        processes,
        status,
        setup,
        assets,
        logs,
    };

    Ok(DebugReport {
        text: lines.join("\n"),
        json,
    })
}

pub fn redact_path_for_report(path: &Path) -> String {
    redact_home_prefix(&path.display().to_string())
}

pub fn status_issues(input: StatusIssuesInput) -> Vec<String> {
    let mut issues = Vec::new();

    if !input.gateway_port_file_exists || !input.gateway_token_file_exists {
        issues.push("Gateway files not found (no token/port files)".into());
    }

    if !input.assets_dir_exists {
        issues.push("Assets directory not found".into());
        return issues;
    }

    if !input.manifest_present {
        issues.push("Manifest file not found in assets directory".into());
    }

    match input.resolved_assets {
        Ok(resolved) => {
            if !resolved.kernel.exists() {
                issues.push(format!(
                    "Kernel asset is MISSING: {}",
                    resolved.kernel.display()
                ));
            }
            if !resolved.initrd.exists() {
                issues.push(format!(
                    "Initrd asset is MISSING: {}",
                    resolved.initrd.display()
                ));
            }
            if !resolved.rootfs.exists() {
                issues.push(format!(
                    "Rootfs asset is MISSING: {}",
                    resolved.rootfs.display()
                ));
            }
        }
        Err(e) => issues.push(format!("Failed to resolve assets: {e}")),
    }

    if input.defunct_session_count > 0 {
        issues.push(format!(
            "{} defunct sandbox(es) failed to boot -- run `capsem logs <name>`",
            input.defunct_session_count
        ));
    }

    issues
}

pub fn default_install_report_input() -> Option<InstallReportInput> {
    let current_exe = std::env::current_exe().ok()?;
    let bin_dir = current_exe.parent()?.to_path_buf();
    Some(InstallReportInput {
        bin_dir,
        current_exe,
        service_unit_path: default_service_unit_path(),
    })
}

pub fn default_process_report_inputs(
    run_dir: &Path,
    current_exe: &Path,
) -> Vec<ProcessReportInput> {
    let bin_dir = current_exe.parent().map(Path::to_path_buf);
    let sibling = |name: &str| bin_dir.as_ref().map(|dir| dir.join(name));
    vec![
        ProcessReportInput {
            name: "service".into(),
            pid: Some(std::process::id()),
            executable_path: Some(current_exe.to_path_buf()),
        },
        ProcessReportInput {
            name: "gateway".into(),
            pid: read_pid_file(&run_dir.join("gateway.pid")),
            executable_path: sibling("capsem-gateway"),
        },
        ProcessReportInput {
            name: "tray".into(),
            pid: read_pid_file(&run_dir.join("tray.pid")),
            executable_path: sibling("capsem-tray"),
        },
        ProcessReportInput {
            name: "mcp".into(),
            pid: read_pid_file(&run_dir.join("mcp.pid")),
            executable_path: sibling("capsem-mcp"),
        },
    ]
}

fn default_service_unit_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    #[cfg(target_os = "macos")]
    {
        Some(PathBuf::from(home).join("Library/LaunchAgents/com.capsem.service.plist"))
    }
    #[cfg(target_os = "linux")]
    {
        Some(PathBuf::from(home).join(".config/systemd/user/capsem.service"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = home;
        None
    }
}

fn read_pid_file(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| contents.trim().parse().ok())
}

fn build_asset_report(input: &DebugReportInput) -> Result<AssetsReport> {
    let manifest_path = manifest_path_for_assets(&input.assets_dir);
    let signature_path = signature_path_for_manifest(&manifest_path);
    let dev_pubkey_path = manifest_path
        .parent()
        .map(|p| p.join("manifest-sign.dev.pub"))
        .unwrap_or_else(|| input.assets_dir.join("manifest-sign.dev.pub"));
    let manifest_file = file_snapshot(&manifest_path, false, true);
    let signature_file = file_snapshot(&signature_path, false, true);
    let dev_pubkey_file = file_snapshot(&dev_pubkey_path, false, true);

    let Some(manifest) = input.manifest.as_ref() else {
        return Ok(AssetsReport {
            manifest: ManifestReport {
                present: false,
                path: manifest_file.path,
                exists: manifest_file.exists,
                size_bytes: manifest_file.size_bytes,
                hash: manifest_file.hash,
                signature_file,
                dev_pubkey_file,
                assets_current: None,
                binaries_current: None,
                arch: None,
            },
            asset_version_for_binary: None,
            files: BTreeMap::new(),
        });
    };

    let arch = capsem_core::asset_manager::host_manifest_arch();
    let resolved = manifest
        .resolve(&input.version, arch, &input.assets_dir)
        .context("resolve manifest assets for debug report")?;
    let release = manifest
        .assets
        .releases
        .get(&resolved.asset_version)
        .with_context(|| format!("asset version {} not found", resolved.asset_version))?;
    let arch_assets = release.arches.get(arch).with_context(|| {
        format!(
            "arch {arch} not found in asset release {}",
            resolved.asset_version
        )
    })?;

    let mut files = BTreeMap::new();
    for (label, logical, path) in [
        ("kernel", "vmlinuz", resolved.kernel.as_path()),
        ("initrd", "initrd.img", resolved.initrd.as_path()),
        ("rootfs", "rootfs.squashfs", resolved.rootfs.as_path()),
    ] {
        files.insert(
            label.to_string(),
            build_one_asset_report(logical, path, arch_assets)?,
        );
    }

    Ok(AssetsReport {
        manifest: ManifestReport {
            present: true,
            path: manifest_file.path,
            exists: manifest_file.exists,
            size_bytes: manifest_file.size_bytes,
            hash: manifest_file.hash,
            signature_file,
            dev_pubkey_file,
            assets_current: Some(manifest.assets.current.clone()),
            binaries_current: Some(manifest.binaries.current.clone()),
            arch: Some(arch.to_string()),
        },
        asset_version_for_binary: Some(resolved.asset_version),
        files,
    })
}

fn build_one_asset_report(
    logical: &str,
    path: &Path,
    arch_assets: &std::collections::HashMap<String, capsem_core::asset_manager::AssetEntry>,
) -> Result<AssetFileReport> {
    let expected = arch_assets
        .get(logical)
        .with_context(|| format!("{logical} missing from manifest arch assets"))?;
    let exists = path.exists();
    let size_bytes = std::fs::metadata(path).ok().map(|m| m.len());
    let actual_hash = if exists {
        Some(
            capsem_core::asset_manager::hash_file(path)
                .with_context(|| format!("hash {}", path.display()))?,
        )
    } else {
        None
    };
    let actual_hash_matches_manifest = actual_hash
        .as_ref()
        .map(|actual| actual == &expected.hash)
        .unwrap_or(false);

    Ok(AssetFileReport {
        logical: logical.to_string(),
        path: redact_path_for_report(path),
        exists,
        size_bytes,
        manifest_hash: expected.hash.clone(),
        manifest_size_bytes: expected.size,
        actual_hash,
        actual_hash_matches_manifest,
    })
}

fn build_status_report(input: &DebugReportInput) -> DebugStatusReport {
    DebugStatusReport {
        issues: input
            .status_issues
            .iter()
            .map(|issue| redact_log_line(issue))
            .collect(),
        defunct_sessions: input
            .defunct_sessions
            .iter()
            .map(|session| DefunctSessionReport {
                name: session.name.clone(),
                last_error: session.last_error.as_deref().map(redact_log_line),
            })
            .collect(),
    }
}

fn build_host_report(platform: &str) -> HostReport {
    let mut parts = platform.splitn(2, '/');
    HostReport {
        os: parts.next().unwrap_or(std::env::consts::OS).to_string(),
        arch: parts.next().unwrap_or(std::env::consts::ARCH).to_string(),
        family: std::env::consts::FAMILY.to_string(),
    }
}

fn build_disk_report(input: &DebugReportInput) -> DiskReport {
    DiskReport {
        capsem_home: disk_path_report(&input.capsem_home),
        run_dir: disk_path_report(&input.run_dir),
        assets_dir: disk_path_report(&input.assets_dir),
    }
}

fn disk_path_report(path: &Path) -> DiskPathReport {
    let exists = path.exists();
    let stat_path = existing_stat_path(path);
    match nix::sys::statvfs::statvfs(&stat_path) {
        Ok(stat) => {
            let fragment_size = stat.fragment_size();
            DiskPathReport {
                path: redact_path_for_report(path),
                exists,
                total_bytes: Some(u64::from(stat.blocks()).saturating_mul(fragment_size)),
                available_bytes: Some(
                    u64::from(stat.blocks_available()).saturating_mul(fragment_size),
                ),
                error: None,
            }
        }
        Err(e) => DiskPathReport {
            path: redact_path_for_report(path),
            exists,
            total_bytes: None,
            available_bytes: None,
            error: Some(e.to_string()),
        },
    }
}

fn existing_stat_path(path: &Path) -> PathBuf {
    if path.exists() {
        return path.to_path_buf();
    }
    let mut current = path;
    while let Some(parent) = current.parent() {
        if parent.exists() {
            return parent.to_path_buf();
        }
        current = parent;
    }
    PathBuf::from("/")
}

fn build_install_report(input: Option<&InstallReportInput>) -> InstallReport {
    InstallReport {
        bin_dir: input.map(|i| redact_path_for_report(&i.bin_dir)),
        current_exe: input.map(|i| redact_path_for_report(&i.current_exe)),
        service_unit_path: input.and_then(|i| {
            i.service_unit_path
                .as_ref()
                .map(|p| redact_path_for_report(p))
        }),
    }
}

fn build_host_binary_report(input: &DebugReportInput) -> BTreeMap<String, BinaryReport> {
    let mut binaries = BTreeMap::new();
    let Some(install) = input.install.as_ref() else {
        return binaries;
    };

    for name in [
        "capsem",
        "capsem-service",
        "capsem-gateway",
        "capsem-process",
        "capsem-tray",
        "capsem-mcp",
    ] {
        binaries.insert(
            name.to_string(),
            binary_report_for_path(&install.bin_dir.join(name)),
        );
    }
    binaries.insert(
        "current_exe".to_string(),
        binary_report_for_path(&install.current_exe),
    );
    binaries
}

fn binary_report_for_path(path: &Path) -> BinaryReport {
    let mut report = BinaryReport {
        path: redact_path_for_report(path),
        exists: false,
        size_bytes: None,
        mode_octal: None,
        executable: false,
        hash: None,
        error: None,
    };

    match std::fs::metadata(path) {
        Ok(metadata) => {
            let mode = metadata.permissions().mode() & 0o777;
            report.exists = true;
            report.size_bytes = Some(metadata.len());
            report.mode_octal = Some(format!("{mode:03o}"));
            report.executable = mode & 0o111 != 0;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return report,
        Err(e) => {
            report.error = Some(e.to_string());
            return report;
        }
    }

    match capsem_core::asset_manager::hash_file(path) {
        Ok(hash) => report.hash = Some(hash),
        Err(e) => report.error = Some(format!("hash failed: {e:#}")),
    }

    report
}

fn build_process_report(inputs: &[ProcessReportInput]) -> Vec<ProcessReport> {
    inputs
        .iter()
        .map(|input| {
            let (executable_path, executable_hash, error) =
                if let Some(path) = input.executable_path.as_ref() {
                    let mut error = None;
                    let hash = if path.exists() {
                        capsem_core::asset_manager::hash_file(path)
                            .map_err(|e| {
                                error = Some(format!("hash failed: {e:#}"));
                            })
                            .ok()
                    } else {
                        None
                    };
                    (Some(redact_path_for_report(path)), hash, error)
                } else {
                    (None, None, None)
                };
            ProcessReport {
                name: input.name.clone(),
                pid: input.pid,
                running: input.pid.map(pid_is_running),
                executable_path,
                executable_hash,
                error,
            }
        })
        .collect()
}

fn pid_is_running(pid: u32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok()
}

fn append_runtime_report(lines: &mut Vec<String>, runtime: &RuntimeReport) {
    lines.push(format!("running_vm_count: {}", runtime.running_vm_count));
    lines.push(format!("total_vm_count: {}", runtime.total_vm_count));
    lines.push(format!(
        "service_pid_file_exists: {}",
        runtime.service_pid_file.exists
    ));
    lines.push(format!(
        "gateway_pid_file_exists: {}",
        runtime.gateway_pid_file.exists
    ));
    lines.push(format!(
        "gateway_port_file_exists: {}",
        runtime.gateway_port_file.exists
    ));
    if let Some(port) = runtime.gateway_port_file.contents.as_deref() {
        lines.push(format!("gateway_port: {port}"));
    }
    lines.push(format!(
        "gateway_token_file_exists: {}",
        runtime.gateway_token_file.exists
    ));
}

fn append_asset_locations_report(
    lines: &mut Vec<String>,
    locations: &capsem_core::settings_profiles::ResolvedServiceAssetLocations,
) {
    lines.push(format!(
        "resolved_assets_dir: {}",
        redact_path_for_report(&locations.assets_dir)
    ));
    lines.push(format!(
        "resolved_assets_dir_origin: {}",
        locations.assets_dir_origin.as_str()
    ));
    let image_roots = locations
        .image_roots
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    lines.push(format!(
        "resolved_image_roots: {}",
        join_redacted_paths(&image_roots)
    ));
    lines.push(format!(
        "resolved_image_roots_origin: {}",
        locations.image_roots_origin.as_str()
    ));
    lines.push(format!(
        "resolved_manifest_source: {}",
        locations.manifest.source.as_str()
    ));
}

fn append_host_report(
    lines: &mut Vec<String>,
    host: &HostReport,
    disk: &DiskReport,
    install: &InstallReport,
    host_binaries: &BTreeMap<String, BinaryReport>,
    processes: &[ProcessReport],
) {
    lines.push(format!("os: {}", host.os));
    lines.push(format!("arch: {}", host.arch));
    lines.push(format!("family: {}", host.family));
    if let Some(bin_dir) = install.bin_dir.as_deref() {
        lines.push(format!("install_bin_dir: {bin_dir}"));
    }
    if let Some(current_exe) = install.current_exe.as_deref() {
        lines.push(format!("current_exe: {current_exe}"));
    }
    lines.push(format!(
        "capsem_home_available_bytes: {}",
        disk.capsem_home
            .available_bytes
            .map(|v| v.to_string())
            .unwrap_or_else(|| "<unknown>".into())
    ));
    for (name, binary) in host_binaries {
        lines.push(format!("{name}_binary_exists: {}", binary.exists));
        if let Some(hash) = binary.hash.as_deref() {
            lines.push(format!("{name}_binary_hash: {hash}"));
        }
    }
    for process in processes {
        lines.push(format!(
            "{}_pid: {}",
            process.name,
            process
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "<missing>".into())
        ));
    }
}

fn append_status_report(lines: &mut Vec<String>, status: &DebugStatusReport) {
    lines.push(format!("status_issue_count: {}", status.issues.len()));
    for issue in &status.issues {
        lines.push(format!("status_issue: {issue}"));
    }
    lines.push(format!(
        "defunct_session_count: {}",
        status.defunct_sessions.len()
    ));
    for session in &status.defunct_sessions {
        if let Some(last_error) = session.last_error.as_deref() {
            lines.push(format!("defunct_session: {}: {last_error}", session.name));
        } else {
            lines.push(format!("defunct_session: {}", session.name));
        }
    }
}

fn append_setup_report(lines: &mut Vec<String>, setup: &SetupReport) {
    lines.push(format!("setup_state_present: {}", setup.present));
    if let Some(err) = setup.parse_error.as_deref() {
        lines.push(format!("setup_state_parse_error: {err}"));
    }
    lines.push(format!("install_completed: {}", setup.install_completed));
    lines.push(format!(
        "onboarding_completed: {}",
        setup.onboarding_completed
    ));
    lines.push(format!("onboarding_version: {}", setup.onboarding_version));
    lines.push(format!("needs_onboarding: {}", setup.needs_onboarding));
    lines.push(format!("providers_done: {}", setup.providers_done));
    lines.push(format!("vm_verified: {}", setup.vm_verified));
}

fn append_settings_profiles_report(
    lines: &mut Vec<String>,
    snapshot: Option<&capsem_core::settings_profiles::SettingsProfilesDebugSnapshot>,
) {
    let Some(snapshot) = snapshot else {
        lines.push("present: false".to_string());
        return;
    };

    lines.push("present: true".to_string());
    if let Some(error) = &snapshot.load_error {
        lines.push(format!("load_error: {error}"));
        return;
    }

    if let Some(service) = &snapshot.service {
        lines.push(format!("default_profile: {}", service.default_profile));
        lines.push(format!(
            "profile_base_dirs: {}",
            join_redacted_paths(&service.base_dirs)
        ));
        lines.push(format!(
            "profile_corp_dirs: {}",
            join_redacted_paths(&service.corp_dirs)
        ));
        lines.push(format!(
            "profile_user_dirs: {}",
            join_redacted_paths(&service.user_dirs)
        ));
        lines.push(format!(
            "manifest_source: {}",
            service.manifest_source.as_str()
        ));
        lines.push(format!(
            "manifest_path: {}",
            redacted_optional_path(service.manifest_path.as_deref())
        ));
        lines.push(format!(
            "manifest_url: {}",
            service.manifest_url.as_deref().unwrap_or("<none>")
        ));
        lines.push(format!(
            "manifest_signature_path: {}",
            redacted_optional_path(service.manifest_signature_path.as_deref())
        ));
        lines.push(format!(
            "manifest_signature_url: {}",
            service
                .manifest_signature_url
                .as_deref()
                .unwrap_or("<none>")
        ));
        lines.push(format!(
            "assets_dir: {}",
            redacted_optional_path(service.assets_dir.as_deref())
        ));
        lines.push(format!(
            "image_roots: {}",
            join_redacted_paths(&service.image_roots)
        ));
        lines.push(format!(
            "asset_download_base_url: {}",
            service
                .asset_download_base_url
                .as_deref()
                .unwrap_or("<none>")
        ));
        lines.push(format!(
            "allow_user_profiles: {}",
            service.allow_user_profiles
        ));
        lines.push(format!("allow_user_fork: {}", service.allow_user_fork));
        lines.push(format!("allow_user_delete: {}", service.allow_user_delete));
        lines.push(format!("telemetry_enabled: {}", service.telemetry_enabled));
        lines.push(format!(
            "telemetry_endpoint_configured: {}",
            service.telemetry_endpoint_configured
        ));
        lines.push(format!(
            "telemetry_endpoint: {}",
            service.telemetry_endpoint.as_deref().unwrap_or("<none>")
        ));
        lines.push(format!(
            "remote_policy_enabled: {}",
            service.remote_policy_enabled
        ));
        lines.push(format!(
            "remote_policy_endpoint_configured: {}",
            service.remote_policy_endpoint_configured
        ));
        lines.push(format!(
            "remote_policy_endpoint: {}",
            service
                .remote_policy_endpoint
                .as_deref()
                .unwrap_or("<none>")
        ));
        lines.push(format!(
            "credential_ids: {}",
            join_or_none(&service.credential_ids)
        ));
    }

    let selected = snapshot
        .selected_profile_id
        .as_deref()
        .unwrap_or("<unresolved>");
    lines.push(format!("selected_profile: {selected}"));
    for profile in &snapshot.profiles {
        let path = profile
            .path
            .as_deref()
            .map(|path| redact_path_for_report(Path::new(path)))
            .unwrap_or_else(|| "<built-in>".to_string());
        lines.push(format!(
            "profile: {} source={} locked={} type={:?} path={}",
            profile.id,
            profile.source.as_str(),
            profile.locked,
            profile.profile_type,
            path
        ));
    }

    if let Some(effective) = &snapshot.effective {
        lines.push(format!("effective_profile: {}", effective.profile_id));
        lines.push(format!(
            "effective_vm: memory_mib={} cpus={} network={:?}",
            effective.vm_memory_mib, effective.vm_cpus, effective.vm_network
        ));
        lines.push(format!(
            "effective_mcp_connectors: {}",
            join_or_none(&effective.mcp_connector_ids)
        ));
        lines.push(format!(
            "effective_enabled_mcp_connectors: {}",
            join_or_none(&effective.enabled_mcp_connector_ids)
        ));
        lines.push(format!(
            "effective_skill_groups: {}",
            join_or_none(&effective.skill_groups)
        ));
        lines.push(format!(
            "effective_enabled_skills: {}",
            join_or_none(&effective.enabled_skills)
        ));
        lines.push(format!(
            "effective_disabled_skills: {}",
            join_or_none(&effective.disabled_skills)
        ));
        lines.push(format!("effective_rule_count: {}", effective.rule_count));
        lines.push(format!(
            "effective_derived_rule_count: {}",
            effective.derived_rule_count
        ));
        lines.push(format!(
            "effective_raw_rule_count: {}",
            effective.raw_rule_count
        ));
    }

    if let Some(trace) = &snapshot.resolver_trace {
        lines.push(format!("resolver_trace_event_count: {}", trace.event_count));
        lines.push(format!(
            "resolver_trace_corp_event_count: {}",
            trace.corp_event_count
        ));
        lines.push(format!(
            "resolver_trace_locked_paths: {}",
            join_or_none(&trace.locked_paths)
        ));
        lines.push(format!(
            "resolver_trace_rejected_paths: {}",
            join_or_none(&trace.rejected_paths)
        ));
        for event in &trace.last_events {
            lines.push(format!(
                "resolver_trace_event: step={} op={:?} source={:?} profile={} path={}",
                event.step,
                event.operation,
                event.source_kind,
                event.source_profile_id.as_deref().unwrap_or("<none>"),
                event.path,
            ));
        }
    }
}

fn append_asset_report(lines: &mut Vec<String>, assets: &AssetsReport) {
    if !assets.manifest.present {
        lines.push("manifest_present: false".to_string());
        return;
    }

    lines.push("manifest_present: true".to_string());
    if let Some(current) = assets.manifest.assets_current.as_deref() {
        lines.push(format!("manifest_assets_current: {current}"));
    }
    if let Some(current) = assets.manifest.binaries_current.as_deref() {
        lines.push(format!("manifest_binaries_current: {current}"));
    }
    if let Some(arch) = assets.manifest.arch.as_deref() {
        lines.push(format!("manifest_arch: {arch}"));
    }
    if let Some(version) = assets.asset_version_for_binary.as_deref() {
        lines.push(format!("asset_version_for_binary: {version}"));
    }

    for label in ["kernel", "initrd", "rootfs"] {
        let Some(asset) = assets.files.get(label) else {
            continue;
        };
        lines.push(format!("{label}_manifest_hash: {}", asset.manifest_hash));
        lines.push(format!("{label}_path: {}", asset.path));
        lines.push(format!("{label}_exists: {}", asset.exists));
        lines.push(format!(
            "{label}_actual_hash: {}",
            asset.actual_hash.as_deref().unwrap_or("<missing>")
        ));
        lines.push(format!(
            "{label}_actual_hash_matches_manifest: {}",
            asset.actual_hash_matches_manifest
        ));
    }
}

fn append_logs_report(lines: &mut Vec<String>, logs: &[LogTailReport]) {
    for log in logs {
        lines.push(format!("{}_log_path: {}", log.name, log.path));
        lines.push(format!("{}_log_exists: {}", log.name, log.exists));
        lines.push(format!(
            "{}_log_tail_line_count: {}",
            log.name,
            log.tail.len()
        ));
        if let Some(err) = log.error.as_deref() {
            lines.push(format!("{}_log_error: {err}", log.name));
        }
    }
}

fn build_runtime_report(
    run_dir: &Path,
    running_vm_count: usize,
    total_vm_count: usize,
) -> RuntimeReport {
    RuntimeReport {
        running_vm_count,
        total_vm_count,
        service_pid_file: file_snapshot(&run_dir.join("service.pid"), true, false),
        gateway_pid_file: file_snapshot(&run_dir.join("gateway.pid"), true, false),
        gateway_port_file: file_snapshot(&run_dir.join("gateway.port"), true, false),
        gateway_token_file: file_snapshot(&run_dir.join("gateway.token"), false, false),
    }
}

fn build_setup_report(capsem_home: &Path) -> SetupReport {
    let path = capsem_home.join("setup-state.json");
    let redacted_path = redact_path_for_report(&path);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return setup_report_from_state(redacted_path, false, None, Default::default());
        }
        Err(e) => {
            return setup_report_from_state(
                redacted_path,
                false,
                Some(format!("read failed: {e}")),
                Default::default(),
            );
        }
    };

    match serde_json::from_str::<capsem_core::setup_state::SetupState>(&contents) {
        Ok(state) => setup_report_from_state(redacted_path, true, None, state),
        Err(e) => setup_report_from_state(
            redacted_path,
            true,
            Some(format!("parse failed: {e}")),
            Default::default(),
        ),
    }
}

fn setup_report_from_state(
    path: String,
    present: bool,
    parse_error: Option<String>,
    state: capsem_core::setup_state::SetupState,
) -> SetupReport {
    let needs_onboarding = state.needs_onboarding();
    SetupReport {
        path,
        present,
        parse_error,
        schema_version: state.schema_version,
        current_onboarding_version: capsem_core::setup_state::CURRENT_ONBOARDING_VERSION,
        completed_steps: state.completed_steps,
        security_preset: state.security_preset,
        providers_done: state.providers_done,
        repositories_done: state.repositories_done,
        service_installed: state.service_installed,
        vm_verified: state.vm_verified,
        install_completed: state.install_completed,
        onboarding_completed: state.onboarding_completed,
        onboarding_version: state.onboarding_version,
        needs_onboarding,
        corp_config_source_present: state.corp_config_source.is_some(),
    }
}

fn file_snapshot(path: &Path, include_contents: bool, include_hash: bool) -> FileSnapshot {
    let mut snapshot = FileSnapshot {
        path: redact_path_for_report(path),
        exists: false,
        size_bytes: None,
        hash: None,
        contents: None,
        error: None,
    };

    match std::fs::metadata(path) {
        Ok(metadata) => {
            snapshot.exists = true;
            snapshot.size_bytes = Some(metadata.len());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return snapshot,
        Err(e) => {
            snapshot.error = Some(e.to_string());
            return snapshot;
        }
    }

    if include_hash {
        match capsem_core::asset_manager::hash_file(path) {
            Ok(hash) => snapshot.hash = Some(hash),
            Err(e) => snapshot.error = Some(format!("hash failed: {e:#}")),
        }
    }

    if include_contents {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let trimmed = contents.trim();
                snapshot.contents = Some(redact_log_line(trimmed));
            }
            Err(e) => snapshot.error = Some(format!("read failed: {e}")),
        }
    }

    snapshot
}

fn manifest_path_for_assets(assets_dir: &Path) -> PathBuf {
    let primary = assets_dir.join("manifest.json");
    if primary.exists() {
        return primary;
    }
    if let Some(parent) = assets_dir.parent() {
        let parent_manifest = parent.join("manifest.json");
        if parent_manifest.exists() {
            return parent_manifest;
        }
    }
    primary
}

fn signature_path_for_manifest(path: &Path) -> PathBuf {
    let mut sig = path.to_path_buf();
    let name = sig
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("manifest.json");
    sig.set_file_name(format!("{name}.minisig"));
    sig
}

fn collect_log_tails(capsem_home: &Path, run_dir: &Path) -> Vec<LogTailReport> {
    let mut logs = Vec::new();
    for (name, candidates) in [
        ("service", log_candidates(capsem_home, run_dir, "service")),
        ("gateway", log_candidates(capsem_home, run_dir, "gateway")),
        ("tray", log_candidates(capsem_home, run_dir, "tray")),
        ("mcp", log_candidates(capsem_home, run_dir, "mcp")),
        ("doctor_latest", vec![run_dir.join("doctor-latest.log")]),
    ] {
        let path = candidates
            .iter()
            .find(|candidate| candidate.exists())
            .cloned()
            .unwrap_or_else(|| candidates[0].clone());
        logs.push(log_tail_report(name, &path));
    }
    logs
}

fn log_candidates(capsem_home: &Path, run_dir: &Path, name: &str) -> Vec<PathBuf> {
    let mut candidates = vec![
        run_dir.join(format!("{name}.log")),
        run_dir.join("logs").join(format!("{name}.log")),
    ];
    if let Some(home) = capsem_home.parent() {
        candidates.push(home.join("Library/Logs/capsem").join(format!("{name}.log")));
    }
    candidates
}

fn log_tail_report(name: &str, path: &Path) -> LogTailReport {
    let redacted_path = redact_path_for_report(path);
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return LogTailReport {
                name: name.to_string(),
                path: redacted_path,
                exists: false,
                size_bytes: None,
                truncated: false,
                tail: Vec::new(),
                error: None,
            };
        }
        Err(e) => {
            return LogTailReport {
                name: name.to_string(),
                path: redacted_path,
                exists: false,
                size_bytes: None,
                truncated: false,
                tail: Vec::new(),
                error: Some(e.to_string()),
            };
        }
    };

    match read_tail_lines(path, 16 * 1024, 80) {
        Ok((tail, truncated)) => LogTailReport {
            name: name.to_string(),
            path: redacted_path,
            exists: true,
            size_bytes: Some(metadata.len()),
            truncated,
            tail,
            error: None,
        },
        Err(e) => LogTailReport {
            name: name.to_string(),
            path: redacted_path,
            exists: true,
            size_bytes: Some(metadata.len()),
            truncated: false,
            tail: Vec::new(),
            error: Some(e.to_string()),
        },
    }
}

fn read_tail_lines(
    path: &Path,
    max_bytes: u64,
    max_lines: usize,
) -> std::io::Result<(Vec<String>, bool)> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(max_bytes);
    let truncated = start > 0;
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        if let Some(idx) = text.find('\n') {
            text = text[idx + 1..].to_string();
        }
    }
    let mut lines = text.lines().map(redact_log_line).collect::<Vec<_>>();
    if lines.len() > max_lines {
        lines = lines.split_off(lines.len() - max_lines);
    }
    Ok((lines, truncated))
}

fn redact_home_prefix(value: &str) -> String {
    if let Some(idx) = value.find("/Users/") {
        redact_user_segment(value, idx, "/Users/".len())
    } else if let Some(idx) = value.find("/home/") {
        redact_user_segment(value, idx, "/home/".len())
    } else {
        value.to_string()
    }
}

fn redact_user_segment(value: &str, idx: usize, prefix_len: usize) -> String {
    let mut out = value.to_string();
    if let Some(end) = out[idx + prefix_len..].find('/') {
        let abs_end = idx + prefix_len + end + 1;
        out.replace_range(idx..abs_end, "~/");
    }
    out
}

fn join_redacted_paths(paths: &[String]) -> String {
    let values = paths
        .iter()
        .map(|path| redact_path_for_report(Path::new(path)))
        .collect::<Vec<_>>();
    join_or_none(&values)
}

fn redacted_optional_path(path: Option<&str>) -> String {
    path.map(|path| redact_path_for_report(Path::new(path)))
        .unwrap_or_else(|| "<none>".to_string())
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "<none>".to_string()
    } else {
        values.join(",")
    }
}

fn redact_log_line(value: &str) -> String {
    let mut out = redact_home_prefix(value);
    for prefix in [
        "Authorization: Bearer ",
        "authorization: Bearer ",
        "Bearer ",
        "token=",
        "api_key=",
        "x-api-key=",
        "authorization=",
    ] {
        out = redact_secret_after_prefix(&out, prefix);
    }
    out
}

fn redact_secret_after_prefix(value: &str, prefix: &str) -> String {
    let mut out = value.to_string();
    let mut search_start = 0;
    while let Some(relative_idx) = out[search_start..].find(prefix) {
        let value_start = search_start + relative_idx + prefix.len();
        let value_end = out[value_start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';'))
            .map(|end| value_start + end)
            .unwrap_or_else(|| out.len());
        if value_end > value_start {
            out.replace_range(value_start..value_end, "<redacted>");
            search_start = value_start + "<redacted>".len();
        } else {
            search_start = value_start;
        }
    }
    out
}

#[cfg(test)]
mod tests;
