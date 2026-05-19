use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

use anyhow::{bail, Result};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use tokio::io::AsyncWriteExt;

use crate::client::{self, UdsClient};
use crate::service_install;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HealthIssueReport {
    pub code: &'static str,
    pub severity: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<&'static str, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StatusReport {
    pub schema: &'static str,
    pub version: String,
    pub ok: bool,
    pub state: &'static str,
    pub service: StatusServiceReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_health: Option<client::AssetHealth>,
    pub checks: StatusChecksReport,
    pub issues: Vec<HealthIssueReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StatusServiceReport {
    pub installed: bool,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StatusChecksReport {
    pub host: StatusCheckReport,
    pub service_unit: StatusCheckReport,
    pub setup: StatusCheckReport,
    pub assets: StatusCheckReport,
    pub app: StatusCheckReport,
    pub service_endpoint: StatusCheckReport,
    pub gateway: StatusCheckReport,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StatusCheckReport {
    pub state: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub issue_codes: Vec<&'static str>,
}

impl StatusCheckReport {
    fn from_issues(issues: Vec<&HealthIssue>, skipped: bool) -> Self {
        let issue_codes = issue_codes(issues);
        let state = if !issue_codes.is_empty() {
            "blocked"
        } else if skipped {
            "skipped"
        } else {
            "ok"
        };
        Self { state, issue_codes }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthSeverity {
    Error,
}

impl HealthSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            HealthSeverity::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthIssueCode {
    HostPathDiscoveryFailed,
    HostBinaryMissing,
    HostBinaryNotExecutable,
    HostBinaryVersionMismatch,
    ServiceUnitMissing,
    ServiceUnitUnreadable,
    ServiceUnitStalePath,
    SetupStatePathUnavailable,
    SetupStateMissing,
    SetupStateUnreadable,
    SetupStateInvalid,
    SetupIncomplete,
    ServiceNotRunning,
    ServiceStale,
    ServiceEndpointUnavailable,
    GatewayFilesMissing,
    GatewayStale,
    GatewayTokenMismatch,
    GatewayDown,
    AssetsDirMissing,
    ServiceAssetError,
    SavedVmAssetMissing,
    AppBundleMissing,
}

impl HealthIssueCode {
    pub fn as_str(self) -> &'static str {
        match self {
            HealthIssueCode::HostPathDiscoveryFailed => "host_path_discovery_failed",
            HealthIssueCode::HostBinaryMissing => "host_binary_missing",
            HealthIssueCode::HostBinaryNotExecutable => "host_binary_not_executable",
            HealthIssueCode::HostBinaryVersionMismatch => "host_binary_version_mismatch",
            HealthIssueCode::ServiceUnitMissing => "service_unit_missing",
            HealthIssueCode::ServiceUnitUnreadable => "service_unit_unreadable",
            HealthIssueCode::ServiceUnitStalePath => "service_unit_stale_path",
            HealthIssueCode::SetupStatePathUnavailable => "setup_state_path_unavailable",
            HealthIssueCode::SetupStateMissing => "setup_state_missing",
            HealthIssueCode::SetupStateUnreadable => "setup_state_unreadable",
            HealthIssueCode::SetupStateInvalid => "setup_state_invalid",
            HealthIssueCode::SetupIncomplete => "setup_incomplete",
            HealthIssueCode::ServiceNotRunning => "service_not_running",
            HealthIssueCode::ServiceStale => "service_stale",
            HealthIssueCode::ServiceEndpointUnavailable => "service_endpoint_unavailable",
            HealthIssueCode::GatewayFilesMissing => "gateway_files_missing",
            HealthIssueCode::GatewayStale => "gateway_stale",
            HealthIssueCode::GatewayTokenMismatch => "gateway_token_mismatch",
            HealthIssueCode::GatewayDown => "gateway_down",
            HealthIssueCode::AssetsDirMissing => "assets_dir_missing",
            HealthIssueCode::ServiceAssetError => "service_asset_error",
            HealthIssueCode::SavedVmAssetMissing => "saved_vm_asset_missing",
            HealthIssueCode::AppBundleMissing => "app_bundle_missing",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthIssue {
    HostPathDiscoveryFailed {
        error: String,
    },
    HostBinaryMissing {
        name: &'static str,
        path: PathBuf,
    },
    HostBinaryNotExecutable {
        name: &'static str,
        path: PathBuf,
    },
    HostBinaryVersionMismatch {
        name: &'static str,
        path: PathBuf,
        actual_version: String,
        expected_version: String,
    },
    ServiceUnitMissing,
    ServiceUnitUnreadable {
        unit_path: PathBuf,
        error: String,
    },
    ServiceUnitStalePath {
        unit_path: PathBuf,
        expected_path: PathBuf,
    },
    SetupStatePathUnavailable {
        error: String,
    },
    SetupStateMissing {
        path: PathBuf,
    },
    SetupStateUnreadable {
        path: PathBuf,
        error: String,
    },
    SetupStateInvalid {
        path: PathBuf,
        error: String,
    },
    SetupIncomplete {
        path: PathBuf,
    },
    ServiceNotRunning,
    ServiceStale {
        running_version: String,
        binary_version: String,
    },
    ServiceEndpointUnavailable,
    GatewayFilesMissing,
    GatewayStale {
        running_version: String,
        binary_version: String,
    },
    GatewayTokenMismatch {
        port: String,
    },
    GatewayDown {
        port: String,
    },
    AssetsDirMissing,
    ServiceAssetError {
        state: String,
        error: Option<String>,
    },
    SavedVmAssetMissing {
        vm: String,
        asset_version: String,
        arch: String,
        missing: Vec<String>,
        recovery_hint: String,
    },
    AppBundleMissing {
        path: PathBuf,
    },
}

impl HealthIssue {
    pub fn code(&self) -> HealthIssueCode {
        match self {
            HealthIssue::HostPathDiscoveryFailed { .. } => HealthIssueCode::HostPathDiscoveryFailed,
            HealthIssue::HostBinaryMissing { .. } => HealthIssueCode::HostBinaryMissing,
            HealthIssue::HostBinaryNotExecutable { .. } => HealthIssueCode::HostBinaryNotExecutable,
            HealthIssue::HostBinaryVersionMismatch { .. } => {
                HealthIssueCode::HostBinaryVersionMismatch
            }
            HealthIssue::ServiceUnitMissing => HealthIssueCode::ServiceUnitMissing,
            HealthIssue::ServiceUnitUnreadable { .. } => HealthIssueCode::ServiceUnitUnreadable,
            HealthIssue::ServiceUnitStalePath { .. } => HealthIssueCode::ServiceUnitStalePath,
            HealthIssue::SetupStatePathUnavailable { .. } => {
                HealthIssueCode::SetupStatePathUnavailable
            }
            HealthIssue::SetupStateMissing { .. } => HealthIssueCode::SetupStateMissing,
            HealthIssue::SetupStateUnreadable { .. } => HealthIssueCode::SetupStateUnreadable,
            HealthIssue::SetupStateInvalid { .. } => HealthIssueCode::SetupStateInvalid,
            HealthIssue::SetupIncomplete { .. } => HealthIssueCode::SetupIncomplete,
            HealthIssue::ServiceNotRunning => HealthIssueCode::ServiceNotRunning,
            HealthIssue::ServiceStale { .. } => HealthIssueCode::ServiceStale,
            HealthIssue::ServiceEndpointUnavailable => HealthIssueCode::ServiceEndpointUnavailable,
            HealthIssue::GatewayFilesMissing => HealthIssueCode::GatewayFilesMissing,
            HealthIssue::GatewayStale { .. } => HealthIssueCode::GatewayStale,
            HealthIssue::GatewayTokenMismatch { .. } => HealthIssueCode::GatewayTokenMismatch,
            HealthIssue::GatewayDown { .. } => HealthIssueCode::GatewayDown,
            HealthIssue::AssetsDirMissing => HealthIssueCode::AssetsDirMissing,
            HealthIssue::ServiceAssetError { .. } => HealthIssueCode::ServiceAssetError,
            HealthIssue::SavedVmAssetMissing { .. } => HealthIssueCode::SavedVmAssetMissing,
            HealthIssue::AppBundleMissing { .. } => HealthIssueCode::AppBundleMissing,
        }
    }

    pub fn severity(&self) -> HealthSeverity {
        HealthSeverity::Error
    }

    pub fn to_report(&self) -> HealthIssueReport {
        HealthIssueReport {
            code: self.code().as_str(),
            severity: self.severity().as_str(),
            message: self.to_string(),
            details: self.details(),
        }
    }

    fn details(&self) -> BTreeMap<&'static str, String> {
        let mut details = BTreeMap::new();
        match self {
            HealthIssue::HostPathDiscoveryFailed { error } => {
                details.insert("error", error.clone());
            }
            HealthIssue::HostBinaryMissing { name, path }
            | HealthIssue::HostBinaryNotExecutable { name, path } => {
                details.insert("name", (*name).to_string());
                details.insert("path", path.display().to_string());
            }
            HealthIssue::HostBinaryVersionMismatch {
                name,
                path,
                actual_version,
                expected_version,
            } => {
                details.insert("name", (*name).to_string());
                details.insert("path", path.display().to_string());
                details.insert("actual_version", actual_version.clone());
                details.insert("expected_version", expected_version.clone());
            }
            HealthIssue::ServiceUnitUnreadable { unit_path, error } => {
                details.insert("unit_path", unit_path.display().to_string());
                details.insert("error", error.clone());
            }
            HealthIssue::ServiceUnitStalePath {
                unit_path,
                expected_path,
            } => {
                details.insert("unit_path", unit_path.display().to_string());
                details.insert("expected_path", expected_path.display().to_string());
            }
            HealthIssue::SetupStatePathUnavailable { error } => {
                details.insert("error", error.clone());
            }
            HealthIssue::SetupStateMissing { path } | HealthIssue::SetupIncomplete { path } => {
                details.insert("path", path.display().to_string());
            }
            HealthIssue::SetupStateUnreadable { path, error }
            | HealthIssue::SetupStateInvalid { path, error } => {
                details.insert("path", path.display().to_string());
                details.insert("error", error.clone());
            }
            HealthIssue::ServiceStale {
                running_version,
                binary_version,
            }
            | HealthIssue::GatewayStale {
                running_version,
                binary_version,
            } => {
                details.insert("running_version", running_version.clone());
                details.insert("binary_version", binary_version.clone());
            }
            HealthIssue::GatewayTokenMismatch { port } | HealthIssue::GatewayDown { port } => {
                details.insert("port", port.clone());
            }
            HealthIssue::AppBundleMissing { path } => {
                details.insert("path", path.display().to_string());
            }
            HealthIssue::ServiceAssetError { state, error } => {
                details.insert("state", state.clone());
                if let Some(error) = error {
                    details.insert("error", error.clone());
                }
            }
            HealthIssue::SavedVmAssetMissing {
                vm,
                asset_version,
                arch,
                missing,
                recovery_hint,
            } => {
                details.insert("vm", vm.clone());
                details.insert("asset_version", asset_version.clone());
                details.insert("arch", arch.clone());
                details.insert("missing", missing.join(","));
                details.insert("recovery_hint", recovery_hint.clone());
            }
            HealthIssue::ServiceUnitMissing
            | HealthIssue::ServiceNotRunning
            | HealthIssue::ServiceEndpointUnavailable
            | HealthIssue::GatewayFilesMissing
            | HealthIssue::AssetsDirMissing => {}
        }
        details
    }
}

impl fmt::Display for HealthIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HealthIssue::HostPathDiscoveryFailed { error } => {
                write!(f, "Install path discovery failed: {}", error)
            }
            HealthIssue::HostBinaryMissing { name, path } => {
                write!(f, "Host binary is MISSING: {} ({})", name, path.display())
            }
            HealthIssue::HostBinaryNotExecutable { name, path } => write!(
                f,
                "Host binary is not executable: {} ({})",
                name,
                path.display()
            ),
            HealthIssue::HostBinaryVersionMismatch {
                name,
                path,
                actual_version,
                expected_version,
            } => write!(
                f,
                "Host binary version mismatch: {} ({}) is v{}, expected v{}",
                name,
                path.display(),
                actual_version,
                expected_version
            ),
            HealthIssue::ServiceUnitMissing => {
                write!(f, "Service unit is not installed")
            }
            HealthIssue::ServiceUnitUnreadable { unit_path, error } => write!(
                f,
                "Service unit is unreadable: {} ({})",
                unit_path.display(),
                error
            ),
            HealthIssue::ServiceUnitStalePath {
                unit_path,
                expected_path,
            } => write!(
                f,
                "Service unit is stale: {} does not reference {}",
                unit_path.display(),
                expected_path.display()
            ),
            HealthIssue::SetupStatePathUnavailable { error } => {
                write!(f, "Setup state path is unavailable: {}", error)
            }
            HealthIssue::SetupStateMissing { path } => {
                write!(f, "Setup state is MISSING: {}", path.display())
            }
            HealthIssue::SetupStateUnreadable { path, error } => {
                write!(
                    f,
                    "Setup state is unreadable: {} ({})",
                    path.display(),
                    error
                )
            }
            HealthIssue::SetupStateInvalid { path, error } => {
                write!(f, "Setup state is invalid: {} ({})", path.display(), error)
            }
            HealthIssue::SetupIncomplete { path } => {
                write!(f, "Setup has not completed: {}", path.display())
            }
            HealthIssue::ServiceNotRunning => {
                write!(
                    f,
                    "Service is not running. Run `capsem start` to start the service."
                )
            }
            HealthIssue::ServiceStale {
                running_version,
                binary_version,
            } => write!(
                f,
                "Service is STALE (running v{}, binary is v{}) -- restart service",
                running_version, binary_version
            ),
            HealthIssue::ServiceEndpointUnavailable => {
                write!(f, "Service is STALE (socket dead or no /version endpoint)")
            }
            HealthIssue::GatewayFilesMissing => {
                write!(f, "Gateway files not found (no token/port files)")
            }
            HealthIssue::GatewayStale {
                running_version,
                binary_version,
            } => write!(
                f,
                "Gateway is STALE (running v{}, binary is v{}) -- restart service",
                running_version, binary_version
            ),
            HealthIssue::GatewayTokenMismatch { port } => {
                write!(
                    f,
                    "Gateway token MISMATCH (port {}) -- restart service",
                    port
                )
            }
            HealthIssue::GatewayDown { port } => {
                write!(f, "Gateway is DOWN (port {} not responding)", port)
            }
            HealthIssue::AssetsDirMissing => write!(f, "Assets directory not found"),
            HealthIssue::ServiceAssetError { state, error } => write!(
                f,
                "Service asset supervisor is {}: {}",
                state,
                error.as_deref().unwrap_or("no error detail")
            ),
            HealthIssue::SavedVmAssetMissing {
                vm,
                asset_version,
                arch,
                missing,
                recovery_hint,
            } => write!(
                f,
                "Saved VM asset dependency is missing: {} needs {} ({}, {}) -- {}",
                vm,
                missing.join(", "),
                asset_version,
                arch,
                recovery_hint
            ),
            HealthIssue::AppBundleMissing { path } => {
                write!(f, "Desktop app bundle is missing: {}", path.display())
            }
        }
    }
}

pub async fn run(json: bool) -> Result<()> {
    let service = service_install::service_status().await?;
    let asset_health = fetch_service_asset_health(service.running).await;
    let mut issues = check_service_health_from_status(&service).await?;
    if let Some(asset_health) = &asset_health {
        issues.extend(service_asset_health_issues(asset_health));
    }

    if json {
        let report = status_report_from_parts_with_assets(&service, &issues, asset_health.clone());
        println!("{}", serde_json::to_string_pretty(&report)?);
        return status_result_from_report(&report, &issues);
    }

    print_text_status(&service, asset_health.as_ref()).await;
    if let Some(report_asset_health) = asset_health {
        let report =
            status_report_from_parts_with_assets(&service, &issues, Some(report_asset_health));
        status_result_from_report(&report, &issues)
    } else {
        status_result_from_issues(&issues)
    }
}

fn service_asset_health_issues(asset_health: &client::AssetHealth) -> Vec<HealthIssue> {
    let mut issues = Vec::new();
    if asset_health.state == "error" {
        issues.push(HealthIssue::ServiceAssetError {
            state: asset_health.state.clone(),
            error: asset_health.error.clone(),
        });
    }
    issues.extend(asset_health.saved_vm_dependencies.iter().map(|dependency| {
        HealthIssue::SavedVmAssetMissing {
            vm: dependency.vm.clone(),
            asset_version: dependency.asset_version.clone(),
            arch: dependency.arch.clone(),
            missing: dependency.missing.clone(),
            recovery_hint: dependency.recovery_hint.clone(),
        }
    }));
    issues
}

pub async fn doctor_preflight() -> Result<()> {
    let issues = check_service_health().await?;
    doctor_preflight_from_issues(&issues)
}

pub async fn debug_report(uds_client: &UdsClient) -> Result<()> {
    let resp: client::ApiResponse<serde_json::Value> = uds_client.get("/debug/report").await?;
    let report = resp.into_result()?;
    let payload = debug_report_payload(report);
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

pub(crate) fn debug_report_payload(report: serde_json::Value) -> serde_json::Value {
    report.get("json").cloned().unwrap_or(report)
}

pub(crate) fn doctor_preflight_from_issues(issues: &[HealthIssue]) -> Result<()> {
    if issues.is_empty() {
        return Ok(());
    }

    bail!(
        "capsem status reported issues; fix these before running capsem doctor:\n  - {}",
        format_issue_list(issues)
    )
}

pub(crate) fn status_result_from_issues(issues: &[HealthIssue]) -> Result<()> {
    if issues.is_empty() {
        return Ok(());
    }

    bail!(
        "capsem status reported issues:\n  - {}",
        format_issue_list(issues)
    )
}

fn status_result_from_report(report: &StatusReport, issues: &[HealthIssue]) -> Result<()> {
    if report.ok {
        return Ok(());
    }
    if issues.is_empty() {
        bail!("capsem status reported state: {}", report.state);
    }
    status_result_from_issues(issues)
}

#[cfg(test)]
pub(crate) fn status_report_from_parts(
    service: &service_install::ServiceStatus,
    issues: &[HealthIssue],
) -> StatusReport {
    status_report_from_parts_with_assets(service, issues, None)
}

pub(crate) fn status_report_from_parts_with_assets(
    service: &service_install::ServiceStatus,
    issues: &[HealthIssue],
    asset_health: Option<client::AssetHealth>,
) -> StatusReport {
    let state = status_state(issues, asset_health.as_ref());
    StatusReport {
        schema: "capsem.status.v1",
        version: env!("CARGO_PKG_VERSION").to_string(),
        ok: issues.is_empty() && state == "ready",
        state,
        service: StatusServiceReport {
            installed: service.installed,
            running: service.running,
            pid: service.pid,
            unit_path: service
                .unit_path
                .as_ref()
                .map(|path| path.display().to_string()),
        },
        asset_health,
        checks: checks_report_from_issues(service, issues),
        issues: issues.iter().map(HealthIssue::to_report).collect(),
    }
}

fn status_state(
    issues: &[HealthIssue],
    asset_health: Option<&client::AssetHealth>,
) -> &'static str {
    if !issues.is_empty() {
        return "blocked";
    }
    match asset_health.map(|health| health.state.as_str()) {
        Some("checking") => "checking",
        Some("updating") => "updating",
        Some("error") => "blocked",
        _ => "ready",
    }
}

fn checks_report_from_issues(
    service: &service_install::ServiceStatus,
    issues: &[HealthIssue],
) -> StatusChecksReport {
    StatusChecksReport {
        host: StatusCheckReport::from_issues(
            issues
                .iter()
                .filter(|issue| {
                    matches!(
                        issue.code(),
                        HealthIssueCode::HostPathDiscoveryFailed
                            | HealthIssueCode::HostBinaryMissing
                            | HealthIssueCode::HostBinaryNotExecutable
                            | HealthIssueCode::HostBinaryVersionMismatch
                    )
                })
                .collect(),
            false,
        ),
        service_unit: StatusCheckReport::from_issues(
            issues
                .iter()
                .filter(|issue| {
                    matches!(
                        issue.code(),
                        HealthIssueCode::ServiceUnitMissing
                            | HealthIssueCode::ServiceUnitUnreadable
                            | HealthIssueCode::ServiceUnitStalePath
                    )
                })
                .collect(),
            !service.service_unit_required,
        ),
        setup: StatusCheckReport::from_issues(
            issues
                .iter()
                .filter(|issue| {
                    matches!(
                        issue.code(),
                        HealthIssueCode::SetupStatePathUnavailable
                            | HealthIssueCode::SetupStateMissing
                            | HealthIssueCode::SetupStateUnreadable
                            | HealthIssueCode::SetupStateInvalid
                            | HealthIssueCode::SetupIncomplete
                    )
                })
                .collect(),
            false,
        ),
        assets: StatusCheckReport::from_issues(
            issues
                .iter()
                .filter(|issue| {
                    matches!(
                        issue.code(),
                        HealthIssueCode::AssetsDirMissing
                            | HealthIssueCode::ServiceAssetError
                            | HealthIssueCode::SavedVmAssetMissing
                    )
                })
                .collect(),
            false,
        ),
        app: StatusCheckReport::from_issues(
            issues
                .iter()
                .filter(|issue| matches!(issue.code(), HealthIssueCode::AppBundleMissing))
                .collect(),
            false,
        ),
        service_endpoint: StatusCheckReport::from_issues(
            issues
                .iter()
                .filter(|issue| {
                    matches!(
                        issue.code(),
                        HealthIssueCode::ServiceNotRunning
                            | HealthIssueCode::ServiceStale
                            | HealthIssueCode::ServiceEndpointUnavailable
                    )
                })
                .collect(),
            false,
        ),
        gateway: StatusCheckReport::from_issues(
            issues
                .iter()
                .filter(|issue| {
                    matches!(
                        issue.code(),
                        HealthIssueCode::GatewayFilesMissing
                            | HealthIssueCode::GatewayStale
                            | HealthIssueCode::GatewayTokenMismatch
                            | HealthIssueCode::GatewayDown
                    )
                })
                .collect(),
            !service.running,
        ),
    }
}

fn issue_codes(issues: Vec<&HealthIssue>) -> Vec<&'static str> {
    let mut codes = Vec::new();
    for issue in issues {
        let code = issue.code().as_str();
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    codes
}

fn format_issue_list(issues: &[HealthIssue]) -> String {
    issues
        .iter()
        .map(|issue| {
            let report = issue.to_report();
            format!("[{}/{}] {}", report.severity, report.code, report.message)
        })
        .collect::<Vec<_>>()
        .join("\n  - ")
}

pub async fn check_service_health() -> Result<Vec<HealthIssue>> {
    let status = service_install::service_status().await?;
    check_service_health_from_status(&status).await
}

async fn check_service_health_from_status(
    status: &service_install::ServiceStatus,
) -> Result<Vec<HealthIssue>> {
    let mut issues = Vec::new();
    match crate::paths::discover_paths() {
        Ok(paths) => {
            issues.extend(check_host_binaries(&paths));
            issues.extend(check_host_binary_versions(&paths).await);
            issues.extend(check_service_unit(status, &paths));
            issues.extend(check_desktop_app_bundle(&paths));
        }
        Err(e) => issues.push(HealthIssue::HostPathDiscoveryFailed {
            error: format!("{e:#}"),
        }),
    }
    issues.extend(check_default_assets());
    issues.extend(check_default_setup_state());

    if !status.running {
        issues.push(HealthIssue::ServiceNotRunning);
        return Ok(issues);
    }

    let home = crate::paths::capsem_home().unwrap_or_default();
    let sock = home.join("run/service.sock");
    let my_version = env!("CARGO_PKG_VERSION");

    match service_version(&sock).await {
        Some(ref v) if v == my_version => {}
        Some(ref v) => issues.push(HealthIssue::ServiceStale {
            running_version: v.clone(),
            binary_version: my_version.to_string(),
        }),
        None => issues.push(HealthIssue::ServiceEndpointUnavailable),
    }

    let port_path = home.join("run/gateway.port");
    let token_path = home.join("run/gateway.token");
    match (
        std::fs::read_to_string(&port_path),
        std::fs::read_to_string(&token_path),
    ) {
        (Ok(port_str), Ok(token)) => {
            let port = port_str.trim();
            let token = token.trim();
            match gateway_status(port, token).await {
                (Some(ref v), true) if v == my_version => {}
                (Some(ref v), true) => {
                    issues.push(HealthIssue::GatewayStale {
                        running_version: v.clone(),
                        binary_version: my_version.to_string(),
                    });
                }
                (Some(_), false) => {
                    issues.push(HealthIssue::GatewayTokenMismatch {
                        port: port.to_string(),
                    });
                }
                (None, _) => {
                    issues.push(HealthIssue::GatewayDown {
                        port: port.to_string(),
                    });
                }
            }
        }
        _ => issues.push(HealthIssue::GatewayFilesMissing),
    }

    Ok(issues)
}

pub(crate) fn check_host_binaries(paths: &crate::paths::CapsemPaths) -> Vec<HealthIssue> {
    [
        ("capsem", &paths.cli_bin),
        ("capsem-service", &paths.service_bin),
        ("capsem-process", &paths.process_bin),
        ("capsem-mcp", &paths.mcp_bin),
        ("capsem-mcp-aggregator", &paths.mcp_aggregator_bin),
        ("capsem-mcp-builtin", &paths.mcp_builtin_bin),
        ("capsem-gateway", &paths.gateway_bin),
        ("capsem-tray", &paths.tray_bin),
    ]
    .into_iter()
    .filter_map(|(name, path)| {
        if !path.exists() {
            return Some(HealthIssue::HostBinaryMissing {
                name,
                path: path.clone(),
            });
        }
        if !is_executable_file(path) {
            return Some(HealthIssue::HostBinaryNotExecutable {
                name,
                path: path.clone(),
            });
        }
        None
    })
    .collect()
}

pub(crate) async fn check_host_binary_versions(
    paths: &crate::paths::CapsemPaths,
) -> Vec<HealthIssue> {
    let mut issues = Vec::new();
    for (name, path) in [
        ("capsem-service", &paths.service_bin),
        ("capsem-process", &paths.process_bin),
        ("capsem-gateway", &paths.gateway_bin),
        ("capsem-tray", &paths.tray_bin),
    ] {
        if let Some(issue) = host_binary_version_mismatch(name, path).await {
            issues.push(issue);
        }
    }
    issues
}

pub(crate) fn check_desktop_app_bundle(paths: &crate::paths::CapsemPaths) -> Vec<HealthIssue> {
    if should_check_desktop_app_bundle(paths) {
        check_app_bundle_path(Path::new("/Applications/Capsem.app"))
    } else {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn should_check_desktop_app_bundle(paths: &crate::paths::CapsemPaths) -> bool {
    if crate::service_install::test_isolation_env_active() {
        return false;
    }
    let Ok(home) = crate::paths::capsem_home() else {
        return false;
    };
    paths.cli_bin == home.join("bin/capsem")
}

#[cfg(not(target_os = "macos"))]
fn should_check_desktop_app_bundle(_paths: &crate::paths::CapsemPaths) -> bool {
    false
}

pub(crate) fn check_app_bundle_path(path: &Path) -> Vec<HealthIssue> {
    if path.is_dir() {
        Vec::new()
    } else {
        vec![HealthIssue::AppBundleMissing {
            path: path.to_path_buf(),
        }]
    }
}

async fn host_binary_version_mismatch(name: &'static str, path: &Path) -> Option<HealthIssue> {
    if !is_executable_file(path) {
        return None;
    }

    let expected_version = env!("CARGO_PKG_VERSION").to_string();
    let actual_version = helper_binary_version(path)
        .await
        .unwrap_or_else(|| "unknown".to_string());
    if actual_version == expected_version {
        return None;
    }

    Some(HealthIssue::HostBinaryVersionMismatch {
        name,
        path: path.to_path_buf(),
        actual_version,
        expected_version,
    })
}

async fn helper_binary_version(path: &Path) -> Option<String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::process::Command::new(path).arg("--version").output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_version_output(&stdout)
}

fn parse_version_output(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.split_whitespace().nth(1).map(str::to_string))
}

pub(crate) fn check_service_unit(
    service: &service_install::ServiceStatus,
    paths: &crate::paths::CapsemPaths,
) -> Vec<HealthIssue> {
    if !service.service_unit_required {
        return Vec::new();
    }

    if !service.installed {
        return vec![HealthIssue::ServiceUnitMissing];
    }

    let Some(unit_path) = service.unit_path.as_ref() else {
        return vec![HealthIssue::ServiceUnitMissing];
    };

    let unit = match std::fs::read_to_string(unit_path) {
        Ok(unit) => unit,
        Err(e) => {
            return vec![HealthIssue::ServiceUnitUnreadable {
                unit_path: unit_path.clone(),
                error: e.to_string(),
            }];
        }
    };

    [
        &paths.service_bin,
        &paths.process_bin,
        &paths.gateway_bin,
        &paths.tray_bin,
        &paths.assets_dir,
    ]
    .into_iter()
    .filter_map(|expected_path| {
        if unit_references_path(&unit, expected_path) {
            None
        } else {
            Some(HealthIssue::ServiceUnitStalePath {
                unit_path: unit_path.clone(),
                expected_path: expected_path.clone(),
            })
        }
    })
    .collect()
}

fn unit_references_path(unit: &str, path: &Path) -> bool {
    let raw = path.display().to_string();
    let systemd_escaped = raw.replace(' ', "\\x20");
    let xml_escaped = raw
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    unit.contains(&raw) || unit.contains(&systemd_escaped) || unit.contains(&xml_escaped)
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn check_default_assets() -> Vec<HealthIssue> {
    if let Some(assets_dir) = capsem_core::asset_manager::default_assets_dir() {
        check_assets_dir(&assets_dir)
    } else {
        vec![HealthIssue::AssetsDirMissing]
    }
}

pub(crate) fn check_assets_dir(assets_dir: &Path) -> Vec<HealthIssue> {
    if assets_dir.is_dir() {
        Vec::new()
    } else {
        vec![HealthIssue::AssetsDirMissing]
    }
}

fn check_default_setup_state() -> Vec<HealthIssue> {
    match crate::paths::capsem_home() {
        Ok(home) => check_setup_state_path(&home.join("setup-state.json")),
        Err(e) => vec![HealthIssue::SetupStatePathUnavailable {
            error: format!("{e:#}"),
        }],
    }
}

pub(crate) fn check_setup_state_path(path: &Path) -> Vec<HealthIssue> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return vec![HealthIssue::SetupStateMissing {
                path: path.to_path_buf(),
            }];
        }
        Err(e) => {
            return vec![HealthIssue::SetupStateUnreadable {
                path: path.to_path_buf(),
                error: e.to_string(),
            }];
        }
    };

    let state = match serde_json::from_str::<capsem_core::setup_state::SetupState>(&contents) {
        Ok(state) => state,
        Err(e) => {
            return vec![HealthIssue::SetupStateInvalid {
                path: path.to_path_buf(),
                error: e.to_string(),
            }];
        }
    };

    if state.install_completed || state.is_step_done("summary") {
        Vec::new()
    } else {
        vec![HealthIssue::SetupIncomplete {
            path: path.to_path_buf(),
        }]
    }
}

async fn print_text_status(
    service: &service_install::ServiceStatus,
    asset_health: Option<&client::AssetHealth>,
) {
    println!("Version:   {}", env!("CARGO_PKG_VERSION"));
    println!("Installed: {}", service.installed);
    println!("Running:   {}", service.running);
    if let Some(pid) = service.pid {
        println!("PID:       {}", pid);
    }
    if let Some(path) = &service.unit_path {
        println!("Unit:      {}", path.display());
    }

    if service.running {
        print_service_and_gateway_status().await;
    }
    if let Some(asset_health) = asset_health {
        print_service_asset_status(asset_health);
    } else {
        print_offline_asset_status();
    }
    print_defunct_sessions(service.running).await;
}

fn print_service_asset_status(asset_health: &client::AssetHealth) {
    let version = asset_health.version.as_deref().unwrap_or("unknown");
    let arch = asset_health.arch.as_deref().unwrap_or("unknown");
    println!("Assets:    {} (v{}, {})", asset_health.state, version, arch);
    if let Some(profile_id) = &asset_health.profile_id {
        match asset_health.profile_revision.as_deref() {
            Some(revision) => println!("  profile: {} ({})", profile_id, revision),
            None => println!("  profile: {}", profile_id),
        }
    }
    if !asset_health.missing.is_empty() {
        println!("  missing: {}", asset_health.missing.join(", "));
    }
    if let Some(progress) = &asset_health.progress {
        match progress.bytes_total {
            Some(total) => println!(
                "  updating: {} {}/{} bytes",
                progress.logical_name, progress.bytes_done, total
            ),
            None => println!(
                "  updating: {} {} bytes",
                progress.logical_name, progress.bytes_done
            ),
        }
    }
    if let Some(error) = &asset_health.error {
        println!("  error: {}", error);
    }
    if let Some(checked_at) = asset_health.checked_at_unix_secs {
        println!("  checked_at_unix_secs: {}", checked_at);
    }
    for dependency in &asset_health.saved_vm_dependencies {
        println!(
            "  saved VM missing: {} needs {} ({}, {})",
            dependency.vm,
            dependency.missing.join(", "),
            dependency.asset_version,
            dependency.arch
        );
    }
}

async fn fetch_service_asset_health(service_running: bool) -> Option<client::AssetHealth> {
    if !service_running {
        return None;
    }
    let home = crate::paths::capsem_home().ok()?;
    let sock = home.join("run/service.sock");
    let list_client = UdsClient::new(sock, false);
    let resp = list_client
        .get::<client::ApiResponse<client::ListResponse>>("/list")
        .await
        .ok()?;
    resp.into_result().ok()?.asset_health
}

async fn print_service_and_gateway_status() {
    let home = crate::paths::capsem_home().unwrap_or_default();
    let sock = home.join("run/service.sock");
    let my_version = env!("CARGO_PKG_VERSION");

    match service_version(&sock).await {
        Some(ref v) if v == my_version => println!("Service:   ok (v{})", v),
        Some(ref v) => println!(
            "Service:   STALE (running v{}, binary is v{}) -- restart service",
            v, my_version
        ),
        None => println!("Service:   STALE (socket dead or no /version endpoint)"),
    }

    let port_path = home.join("run/gateway.port");
    let token_path = home.join("run/gateway.token");
    match (
        std::fs::read_to_string(&port_path),
        std::fs::read_to_string(&token_path),
    ) {
        (Ok(port_str), Ok(token)) => {
            let port = port_str.trim();
            let token = token.trim();
            match gateway_status(port, token).await {
                (Some(ref v), true) if v == my_version => {
                    println!("Gateway:   ok (port {}, v{})", port, v);
                }
                (Some(ref v), true) => {
                    println!(
                        "Gateway:   STALE (running v{}, binary is v{}) -- restart service",
                        v, my_version
                    );
                }
                (Some(_), false) => {
                    println!(
                        "Gateway:   token MISMATCH (port {}) -- restart service",
                        port
                    );
                }
                (None, _) => {
                    println!("Gateway:   DOWN (port {} not responding)", port);
                }
            }
        }
        _ => println!("Gateway:   no token/port files"),
    }
}

fn print_offline_asset_status() {
    if let Some(assets_dir) = capsem_core::asset_manager::default_assets_dir() {
        if assets_dir.is_dir() {
            println!(
                "Assets:    service not running; Profile V2 health unavailable ({})",
                assets_dir.display()
            );
        } else {
            println!("Assets:    directory missing ({})", assets_dir.display());
        }
    }
}

async fn print_defunct_sessions(service_running: bool) {
    if !service_running {
        return;
    }

    let home = crate::paths::capsem_home().unwrap_or_default();
    let sock = home.join("run/service.sock");
    let list_client = UdsClient::new(sock, false);
    if let Ok(resp) = list_client
        .get::<client::ApiResponse<client::ListResponse>>("/list")
        .await
    {
        if let Ok(list) = resp.into_result() {
            let defunct: Vec<&client::SessionInfo> = list
                .sessions
                .iter()
                .filter(|s| s.status == "Defunct")
                .collect();
            if !defunct.is_empty() {
                println!();
                println!(
                    "Defunct:   {} sandbox(es) failed to boot -- run `capsem logs <name>`",
                    defunct.len()
                );
                for s in &defunct {
                    let name = s.name.as_deref().unwrap_or(&s.id);
                    if let Some(err) = &s.last_error {
                        let last = err
                            .lines()
                            .rev()
                            .find(|line| !line.trim().is_empty())
                            .unwrap_or("(log empty)");
                        println!("  - {}: {}", name, last);
                    } else {
                        println!("  - {}", name);
                    }
                }
            }
        }
    }
}

async fn service_version(sock: &Path) -> Option<String> {
    let stream = tokio::net::UnixStream::connect(sock).await.ok()?;
    let (reader, mut writer) = tokio::io::split(stream);
    writer
        .write_all(b"GET /version HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .ok()?;
    let mut buf = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut tokio::io::BufReader::new(reader), &mut buf)
        .await
        .ok()?;
    let body = String::from_utf8_lossy(&buf);
    let json_start = body.find('{')?;
    let v: serde_json::Value = serde_json::from_str(&body[json_start..]).ok()?;
    v.get("version")?.as_str().map(String::from)
}

async fn gateway_status(port: &str, token: &str) -> (Option<String>, bool) {
    let client = reqwest::Client::new();

    let health_url = format!("http://127.0.0.1:{}/health", port);
    let gw_version: Option<String> = async {
        let r = client
            .get(&health_url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .ok()?;
        let v: serde_json::Value = r.json().await.ok()?;
        v.get("version")?.as_str().map(String::from)
    }
    .await;

    let auth_url = format!("http://127.0.0.1:{}/list", port);
    let token_ok = client
        .get(&auth_url)
        .header("Authorization", format!("Bearer {}", token))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    (gw_version, token_ok)
}

#[cfg(test)]
mod tests;
