const UNSIGNED_MANIFEST: &str = r#"{
    "format": 2,
    "assets": {
        "current": "2026.0415.1",
        "releases": {
            "2026.0415.1": {
                "date": "2026-04-15",
                "deprecated": false,
                "min_binary": "1.0.0",
                "arches": {
                    "arm64": {
                        "vmlinuz": { "hash": "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c", "size": 7797248 },
                        "initrd.img": { "hash": "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456", "size": 2270154 },
                        "rootfs.squashfs": { "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 454230016 }
                    }
                }
            }
        }
    },
    "binaries": {
        "current": "1.0.1776269479",
        "releases": {
            "1.0.1776269479": {
                "date": "2026-04-15",
                "deprecated": false,
                "min_assets": "2026.0415.1"
            }
        }
    }
}"#;

#[cfg(unix)]
fn write_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, "#!/bin/sh\n").unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

#[cfg(unix)]
fn write_executable_script(path: &std::path::Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, script).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

#[test]
fn doctor_preflight_fails_when_status_has_issues() {
    let issues = vec![super::HealthIssue::ServiceNotRunning];
    let err = super::doctor_preflight_from_issues(&issues).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("capsem status reported issues"));
    assert!(msg.contains("[error/service_not_running]"));
    assert!(msg.contains("Service is not running"));
}

#[test]
fn status_gate_fails_without_doctor_wording() {
    let issues = vec![super::HealthIssue::ServiceNotRunning];
    let err = super::status_result_from_issues(&issues).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("capsem status reported issues"));
    assert!(msg.contains("[error/service_not_running]"));
    assert!(msg.contains("Service is not running"));
    assert!(!msg.contains("before running capsem doctor"));
}

#[test]
fn health_issue_is_typed_before_rendering() {
    let issue = super::HealthIssue::GatewayTokenMismatch {
        port: "19222".to_string(),
    };

    assert_eq!(
        issue,
        super::HealthIssue::GatewayTokenMismatch {
            port: "19222".to_string()
        }
    );
    assert_eq!(
        issue.to_string(),
        "Gateway token MISMATCH (port 19222) -- restart service"
    );
}

#[test]
fn health_issue_has_stable_machine_identity() {
    let issue = super::HealthIssue::GatewayDown {
        port: "19222".to_string(),
    };

    assert_eq!(issue.code(), super::HealthIssueCode::GatewayDown);
    assert_eq!(issue.code().as_str(), "gateway_down");
    assert_eq!(issue.severity(), super::HealthSeverity::Error);
    assert_eq!(issue.severity().as_str(), "error");
    assert!(matches!(
        issue,
        super::HealthIssue::GatewayDown { ref port } if port == "19222"
    ));
}

#[test]
fn health_issue_report_is_machine_readable() {
    let issue = super::HealthIssue::ServiceStale {
        running_version: "1.0.0".to_string(),
        binary_version: "1.1.0".to_string(),
    };

    let report = issue.to_report();
    assert_eq!(report.code, "service_stale");
    assert_eq!(report.severity, "error");
    assert_eq!(report.details["running_version"], "1.0.0");
    assert_eq!(report.details["binary_version"], "1.1.0");
    assert!(report.message.contains("Service is STALE"));

    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["code"], "service_stale");
    assert_eq!(json["severity"], "error");
    assert_eq!(json["details"]["running_version"], "1.0.0");
}

#[test]
fn status_report_contains_service_and_typed_issues() {
    let service = crate::service_install::ServiceStatus {
        installed: true,
        running: false,
        pid: None,
        unit_path: Some(std::path::PathBuf::from("/tmp/capsem.service")),
        service_unit_required: true,
    };
    let issues = vec![super::HealthIssue::ServiceNotRunning];

    let report = super::status_report_from_parts(&service, &issues);
    assert_eq!(report.schema, "capsem.status.v1");
    assert!(!report.ok);
    assert_eq!(report.state, "blocked");
    assert!(report.service.installed);
    assert!(!report.service.running);
    assert_eq!(
        report.service.unit_path.as_deref(),
        Some("/tmp/capsem.service")
    );
    assert_eq!(report.checks.service_endpoint.state, "blocked");
    assert_eq!(
        report.checks.service_endpoint.issue_codes,
        vec!["service_not_running"]
    );
    assert_eq!(report.checks.gateway.state, "skipped");
    assert_eq!(report.issues[0].code, "service_not_running");

    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["schema"], "capsem.status.v1");
    assert_eq!(json["ok"], false);
    assert_eq!(json["state"], "blocked");
    assert_eq!(json["service"]["installed"], true);
    assert_eq!(json["checks"]["service_endpoint"]["state"], "blocked");
    assert_eq!(
        json["checks"]["service_endpoint"]["issue_codes"][0],
        "service_not_running"
    );
    assert_eq!(json["checks"]["gateway"]["state"], "skipped");
    assert_eq!(json["issues"][0]["code"], "service_not_running");
}

#[test]
fn status_report_groups_issue_codes_by_install_surface() {
    let service = crate::service_install::ServiceStatus {
        installed: true,
        running: true,
        pid: Some(42),
        unit_path: None,
        service_unit_required: true,
    };
    let issues = vec![
        super::HealthIssue::HostBinaryMissing {
            name: "capsem-tray",
            path: "/tmp/capsem-tray".into(),
        },
        super::HealthIssue::ServiceAssetError {
            state: "error".to_string(),
            error: Some("profile assets unavailable".to_string()),
        },
        super::HealthIssue::GatewayDown {
            port: "19222".into(),
        },
        super::HealthIssue::SetupIncomplete {
            path: "/tmp/setup-state.json".into(),
        },
    ];

    let report = super::status_report_from_parts(&service, &issues);
    assert_eq!(report.state, "blocked");
    assert_eq!(report.checks.host.issue_codes, vec!["host_binary_missing"]);
    assert_eq!(
        report.checks.assets.issue_codes,
        vec!["service_asset_error"]
    );
    assert_eq!(report.checks.gateway.issue_codes, vec!["gateway_down"]);
    assert_eq!(report.checks.setup.issue_codes, vec!["setup_incomplete"]);
    assert_eq!(report.checks.service_endpoint.state, "ok");
    assert_eq!(report.checks.gateway.state, "blocked");
}

#[test]
fn status_report_preserves_service_asset_updating_state() {
    let service = crate::service_install::ServiceStatus {
        installed: true,
        running: true,
        pid: Some(42),
        unit_path: None,
        service_unit_required: true,
    };
    let asset_health = crate::client::AssetHealth {
        ready: false,
        state: "updating".into(),
        profile_id: Some("everyday-work".into()),
        profile_revision: Some("2026.0513.1".into()),
        version: Some("2026.0513.1".into()),
        arch: Some("arm64".into()),
        missing: vec!["rootfs.squashfs".into()],
        progress: Some(crate::client::AssetProgress {
            logical_name: "rootfs.squashfs".into(),
            bytes_done: 12,
            bytes_total: Some(24),
            done: false,
        }),
        error: None,
        retry_count: 0,
        retryable: false,
        saved_vm_dependencies: Vec::new(),
        checked_at_unix_secs: Some(1_779_000_000),
    };

    let report = super::status_report_from_parts_with_assets(&service, &[], Some(asset_health));

    assert!(!report.ok);
    assert_eq!(report.state, "updating");
    assert_eq!(
        report.asset_health.as_ref().unwrap().missing,
        vec!["rootfs.squashfs"]
    );
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["state"], "updating");
    assert_eq!(json["asset_health"]["state"], "updating");
    assert_eq!(json["asset_health"]["profile_id"], "everyday-work");
    assert_eq!(json["asset_health"]["profile_revision"], "2026.0513.1");
    assert_eq!(json["asset_health"]["checked_at_unix_secs"], 1_779_000_000);
    assert_eq!(
        json["asset_health"]["progress"]["logical_name"],
        "rootfs.squashfs"
    );
}

#[test]
fn status_report_blocks_on_saved_vm_asset_dependencies() {
    let service = crate::service_install::ServiceStatus {
        installed: true,
        running: true,
        pid: Some(42),
        unit_path: None,
        service_unit_required: true,
    };
    let asset_health = crate::client::AssetHealth {
        ready: true,
        state: "ready".into(),
        profile_id: Some("everyday-work".into()),
        profile_revision: Some("2026.0513.1".into()),
        version: Some("2026.0513.1".into()),
        arch: Some("arm64".into()),
        missing: Vec::new(),
        progress: None,
        error: None,
        retry_count: 0,
        retryable: false,
        saved_vm_dependencies: vec![crate::client::SavedVmAssetDependency {
            vm: "saved-old".into(),
            asset_version: "2026.0415.1".into(),
            arch: "arm64".into(),
            missing: vec!["rootfs.squashfs".into()],
            recovery_hint: "restore assets".into(),
        }],
        checked_at_unix_secs: None,
    };
    let issues = super::service_asset_health_issues(&asset_health);

    let report = super::status_report_from_parts_with_assets(&service, &issues, Some(asset_health));

    assert!(!report.ok);
    assert_eq!(report.state, "blocked");
    assert_eq!(
        report.checks.assets.issue_codes,
        vec!["saved_vm_asset_missing"]
    );
    assert_eq!(report.issues[0].details["vm"], "saved-old");
    assert_eq!(
        report.asset_health.unwrap().saved_vm_dependencies[0].missing,
        vec!["rootfs.squashfs"]
    );
}

#[cfg(unix)]
#[test]
fn host_binary_check_reports_missing_binary() {
    let dir = tempfile::tempdir().unwrap();
    let cli_bin = dir.path().join("capsem");
    let process_bin = dir.path().join("capsem-process");
    let mcp_bin = dir.path().join("capsem-mcp");
    let mcp_aggregator_bin = dir.path().join("capsem-mcp-aggregator");
    let mcp_builtin_bin = dir.path().join("capsem-mcp-builtin");
    let gateway_bin = dir.path().join("capsem-gateway");
    let tray_bin = dir.path().join("capsem-tray");
    write_executable(&cli_bin);
    write_executable(&process_bin);
    write_executable(&mcp_bin);
    write_executable(&mcp_aggregator_bin);
    write_executable(&mcp_builtin_bin);
    write_executable(&gateway_bin);
    write_executable(&tray_bin);
    let paths = crate::paths::CapsemPaths {
        cli_bin,
        service_bin: dir.path().join("capsem-service"),
        process_bin,
        mcp_bin,
        mcp_aggregator_bin,
        mcp_builtin_bin,
        gateway_bin,
        tray_bin,
        assets_dir: dir.path().join("assets"),
    };

    let issues = super::check_host_binaries(&paths);
    assert!(matches!(
        issues.as_slice(),
        [super::HealthIssue::HostBinaryMissing { name, .. }] if *name == "capsem-service"
    ));
    assert_eq!(issues[0].code().as_str(), "host_binary_missing");
}

#[cfg(unix)]
#[test]
fn host_binary_check_reports_non_executable_binary() {
    let dir = tempfile::tempdir().unwrap();
    let cli_bin = dir.path().join("capsem");
    let service_bin = dir.path().join("capsem-service");
    let process_bin = dir.path().join("capsem-process");
    let mcp_bin = dir.path().join("capsem-mcp");
    let mcp_aggregator_bin = dir.path().join("capsem-mcp-aggregator");
    let mcp_builtin_bin = dir.path().join("capsem-mcp-builtin");
    let gateway_bin = dir.path().join("capsem-gateway");
    let tray_bin = dir.path().join("capsem-tray");
    std::fs::write(&service_bin, "#!/bin/sh\n").unwrap();
    write_executable(&cli_bin);
    write_executable(&process_bin);
    write_executable(&mcp_bin);
    write_executable(&mcp_aggregator_bin);
    write_executable(&mcp_builtin_bin);
    write_executable(&gateway_bin);
    write_executable(&tray_bin);
    let paths = crate::paths::CapsemPaths {
        cli_bin,
        service_bin,
        process_bin,
        mcp_bin,
        mcp_aggregator_bin,
        mcp_builtin_bin,
        gateway_bin,
        tray_bin,
        assets_dir: dir.path().join("assets"),
    };

    let issues = super::check_host_binaries(&paths);
    assert!(matches!(
        issues.as_slice(),
        [super::HealthIssue::HostBinaryNotExecutable { name, .. }] if *name == "capsem-service"
    ));
    assert_eq!(issues[0].code().as_str(), "host_binary_not_executable");
}

#[cfg(unix)]
#[tokio::test]
async fn host_binary_version_check_reports_stale_process_binary() {
    let dir = tempfile::tempdir().unwrap();
    let service_bin = dir.path().join("capsem-service");
    let process_bin = dir.path().join("capsem-process");
    write_executable_script(
        &service_bin,
        &format!(
            "#!/bin/sh\nprintf 'capsem-service {}\\n'\n",
            env!("CARGO_PKG_VERSION")
        ),
    );
    write_executable_script(
        &process_bin,
        "#!/bin/sh\nprintf 'capsem-process 0.0.0\\n'\n",
    );

    let paths = crate::paths::CapsemPaths {
        cli_bin: dir.path().join("capsem"),
        service_bin,
        process_bin,
        mcp_bin: dir.path().join("capsem-mcp"),
        mcp_aggregator_bin: dir.path().join("capsem-mcp-aggregator"),
        mcp_builtin_bin: dir.path().join("capsem-mcp-builtin"),
        gateway_bin: dir.path().join("capsem-gateway"),
        tray_bin: dir.path().join("capsem-tray"),
        assets_dir: dir.path().join("assets"),
    };

    let issues = super::check_host_binary_versions(&paths).await;
    assert!(matches!(
        issues.as_slice(),
        [super::HealthIssue::HostBinaryVersionMismatch {
            name,
            actual_version,
            expected_version,
            ..
        }] if *name == "capsem-process"
            && actual_version == "0.0.0"
            && expected_version == env!("CARGO_PKG_VERSION")
    ));
    assert_eq!(issues[0].code().as_str(), "host_binary_version_mismatch");
    assert_eq!(issues[0].to_report().details["actual_version"], "0.0.0");
}

#[cfg(unix)]
#[tokio::test]
async fn host_binary_version_check_reports_stale_gateway_and_tray() {
    let dir = tempfile::tempdir().unwrap();
    let service_bin = dir.path().join("capsem-service");
    let process_bin = dir.path().join("capsem-process");
    let gateway_bin = dir.path().join("capsem-gateway");
    let tray_bin = dir.path().join("capsem-tray");
    for (path, name, version) in [
        (&service_bin, "capsem-service", env!("CARGO_PKG_VERSION")),
        (&process_bin, "capsem-process", env!("CARGO_PKG_VERSION")),
        (&gateway_bin, "capsem-gateway", "0.0.0"),
        (&tray_bin, "capsem-tray", "0.0.0"),
    ] {
        write_executable_script(path, &format!("#!/bin/sh\nprintf '{name} {version}\\n'\n"));
    }

    let paths = crate::paths::CapsemPaths {
        cli_bin: dir.path().join("capsem"),
        service_bin,
        process_bin,
        mcp_bin: dir.path().join("capsem-mcp"),
        mcp_aggregator_bin: dir.path().join("capsem-mcp-aggregator"),
        mcp_builtin_bin: dir.path().join("capsem-mcp-builtin"),
        gateway_bin,
        tray_bin,
        assets_dir: dir.path().join("assets"),
    };

    let issues = super::check_host_binary_versions(&paths).await;
    let names: std::collections::BTreeSet<_> = issues
        .iter()
        .map(|issue| issue.to_report().details["name"].clone())
        .collect();
    assert_eq!(
        names,
        ["capsem-gateway".to_string(), "capsem-tray".to_string()]
            .into_iter()
            .collect()
    );
    assert!(issues
        .iter()
        .all(|issue| issue.code().as_str() == "host_binary_version_mismatch"));
}

#[test]
fn version_output_parser_uses_second_token() {
    assert_eq!(
        super::parse_version_output("capsem-process 1.2.3\n"),
        Some("1.2.3".to_string())
    );
}

#[test]
fn asset_check_accepts_empty_profile_v2_assets_directory() {
    let dir = tempfile::tempdir().unwrap();

    let issues = super::check_assets_dir(dir.path());
    assert!(issues.is_empty(), "unexpected issues: {issues:?}");
}

#[test]
fn service_unit_check_reports_missing_unit() {
    let dir = tempfile::tempdir().unwrap();
    let paths = crate::paths::CapsemPaths {
        cli_bin: dir.path().join("capsem"),
        service_bin: dir.path().join("capsem-service"),
        process_bin: dir.path().join("capsem-process"),
        mcp_bin: dir.path().join("capsem-mcp"),
        mcp_aggregator_bin: dir.path().join("capsem-mcp-aggregator"),
        mcp_builtin_bin: dir.path().join("capsem-mcp-builtin"),
        gateway_bin: dir.path().join("capsem-gateway"),
        tray_bin: dir.path().join("capsem-tray"),
        assets_dir: dir.path().join("assets"),
    };
    let service = crate::service_install::ServiceStatus {
        installed: false,
        running: false,
        pid: None,
        unit_path: None,
        service_unit_required: true,
    };

    let issues = super::check_service_unit(&service, &paths);
    assert!(matches!(
        issues.as_slice(),
        [super::HealthIssue::ServiceUnitMissing]
    ));
    assert_eq!(issues[0].code().as_str(), "service_unit_missing");
}

#[test]
fn service_unit_check_reports_stale_paths() {
    let dir = tempfile::tempdir().unwrap();
    let unit_path = dir.path().join("capsem.service");
    std::fs::write(&unit_path, "ExecStart=/old/capsem-service\n").unwrap();
    let paths = crate::paths::CapsemPaths {
        cli_bin: dir.path().join("capsem"),
        service_bin: dir.path().join("capsem-service"),
        process_bin: dir.path().join("capsem-process"),
        mcp_bin: dir.path().join("capsem-mcp"),
        mcp_aggregator_bin: dir.path().join("capsem-mcp-aggregator"),
        mcp_builtin_bin: dir.path().join("capsem-mcp-builtin"),
        gateway_bin: dir.path().join("capsem-gateway"),
        tray_bin: dir.path().join("capsem-tray"),
        assets_dir: dir.path().join("assets"),
    };
    let service = crate::service_install::ServiceStatus {
        installed: true,
        running: false,
        pid: None,
        unit_path: Some(unit_path.clone()),
        service_unit_required: true,
    };

    let issues = super::check_service_unit(&service, &paths);
    assert!(matches!(
        issues.first(),
        Some(super::HealthIssue::ServiceUnitStalePath { unit_path: path, expected_path })
            if path == &unit_path && expected_path == &paths.service_bin
    ));
    assert_eq!(issues[0].code().as_str(), "service_unit_stale_path");
}

#[test]
fn service_unit_check_accepts_escaped_paths() {
    let dir = tempfile::tempdir().unwrap();
    let install_dir = dir.path().join("Cap Sem");
    std::fs::create_dir_all(&install_dir).unwrap();
    let unit_path = dir.path().join("capsem.service");
    let paths = crate::paths::CapsemPaths {
        cli_bin: install_dir.join("capsem"),
        service_bin: install_dir.join("capsem-service"),
        process_bin: install_dir.join("capsem-process"),
        mcp_bin: install_dir.join("capsem-mcp"),
        mcp_aggregator_bin: install_dir.join("capsem-mcp-aggregator"),
        mcp_builtin_bin: install_dir.join("capsem-mcp-builtin"),
        gateway_bin: install_dir.join("capsem-gateway"),
        tray_bin: install_dir.join("capsem-tray"),
        assets_dir: install_dir.join("assets"),
    };
    std::fs::write(
        &unit_path,
        format!(
            "ExecStart={} --process-binary {} --gateway-binary {} --tray-binary {} --assets-dir {}",
            paths
                .service_bin
                .display()
                .to_string()
                .replace(' ', "\\x20"),
            paths
                .process_bin
                .display()
                .to_string()
                .replace(' ', "\\x20"),
            paths
                .gateway_bin
                .display()
                .to_string()
                .replace(' ', "\\x20"),
            paths.tray_bin.display().to_string().replace(' ', "\\x20"),
            paths.assets_dir.display().to_string().replace(' ', "\\x20"),
        ),
    )
    .unwrap();
    let service = crate::service_install::ServiceStatus {
        installed: true,
        running: false,
        pid: None,
        unit_path: Some(unit_path),
        service_unit_required: true,
    };

    let issues = super::check_service_unit(&service, &paths);
    assert!(issues.is_empty(), "unexpected issues: {issues:?}");
}

#[test]
fn service_unit_check_skips_isolated_dev_service() {
    let dir = tempfile::tempdir().unwrap();
    let paths = crate::paths::CapsemPaths {
        cli_bin: dir.path().join("capsem"),
        service_bin: dir.path().join("capsem-service"),
        process_bin: dir.path().join("capsem-process"),
        mcp_bin: dir.path().join("capsem-mcp"),
        mcp_aggregator_bin: dir.path().join("capsem-mcp-aggregator"),
        mcp_builtin_bin: dir.path().join("capsem-mcp-builtin"),
        gateway_bin: dir.path().join("capsem-gateway"),
        tray_bin: dir.path().join("capsem-tray"),
        assets_dir: dir.path().join("assets"),
    };
    let service = crate::service_install::ServiceStatus {
        installed: false,
        running: true,
        pid: Some(42),
        unit_path: None,
        service_unit_required: false,
    };

    let issues = super::check_service_unit(&service, &paths);
    assert!(issues.is_empty(), "unexpected issues: {issues:?}");

    let report = super::status_report_from_parts(&service, &issues);
    assert_eq!(report.checks.service_unit.state, "skipped");
    assert!(report.checks.service_unit.issue_codes.is_empty());
}

#[test]
fn app_bundle_check_reports_missing_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Capsem.app");

    let issues = super::check_app_bundle_path(&path);
    assert!(matches!(
        issues.as_slice(),
        [super::HealthIssue::AppBundleMissing { path: issue_path }] if issue_path == &path
    ));
    assert_eq!(issues[0].code().as_str(), "app_bundle_missing");
    assert_eq!(
        issues[0].to_report().details["path"],
        path.display().to_string()
    );
}

#[test]
fn app_bundle_check_accepts_existing_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Capsem.app");
    std::fs::create_dir(&path).unwrap();

    let issues = super::check_app_bundle_path(&path);
    assert!(issues.is_empty(), "unexpected issues: {issues:?}");
}

#[test]
fn desktop_app_bundle_check_skips_non_installed_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let paths = crate::paths::CapsemPaths {
        cli_bin: dir.path().join("capsem"),
        service_bin: dir.path().join("capsem-service"),
        process_bin: dir.path().join("capsem-process"),
        mcp_bin: dir.path().join("capsem-mcp"),
        mcp_aggregator_bin: dir.path().join("capsem-mcp-aggregator"),
        mcp_builtin_bin: dir.path().join("capsem-mcp-builtin"),
        gateway_bin: dir.path().join("capsem-gateway"),
        tray_bin: dir.path().join("capsem-tray"),
        assets_dir: dir.path().join("assets"),
    };

    let issues = super::check_desktop_app_bundle(&paths);
    assert!(issues.is_empty(), "unexpected issues: {issues:?}");
}

#[test]
fn setup_state_check_reports_missing_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup-state.json");

    let issues = super::check_setup_state_path(&path);
    assert!(matches!(
        issues.as_slice(),
        [super::HealthIssue::SetupStateMissing { path: issue_path }] if issue_path == &path
    ));
    assert_eq!(issues[0].code().as_str(), "setup_state_missing");
}

#[test]
fn setup_state_check_reports_invalid_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup-state.json");
    std::fs::write(&path, "{not json").unwrap();

    let issues = super::check_setup_state_path(&path);
    assert!(matches!(
        issues.as_slice(),
        [super::HealthIssue::SetupStateInvalid { path: issue_path, .. }] if issue_path == &path
    ));
    assert_eq!(issues[0].code().as_str(), "setup_state_invalid");
}

#[test]
fn setup_state_check_reports_incomplete_install() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup-state.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&capsem_core::setup_state::SetupState::default()).unwrap(),
    )
    .unwrap();

    let issues = super::check_setup_state_path(&path);
    assert!(matches!(
        issues.as_slice(),
        [super::HealthIssue::SetupIncomplete { path: issue_path }] if issue_path == &path
    ));
    assert_eq!(issues[0].code().as_str(), "setup_incomplete");
}

#[test]
fn doctor_preflight_accepts_clean_status() {
    super::doctor_preflight_from_issues(&[]).unwrap();
}

#[test]
fn debug_report_payload_prefers_service_json_field() {
    let payload = super::debug_report_payload(serde_json::json!({
        "text": "Capsem Debug Report",
        "json": {
            "schema": "capsem.debug.v2",
            "status": { "issues": [] }
        }
    }));
    assert_eq!(payload["schema"], "capsem.debug.v2");
    assert_eq!(payload["status"]["issues"], serde_json::json!([]));
}

#[test]
fn asset_directory_check_ignores_legacy_manifest_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("manifest.json"), UNSIGNED_MANIFEST).unwrap();

    let issues = super::check_assets_dir(dir.path());

    assert!(issues.is_empty(), "unexpected issues: {issues:?}");
}

#[test]
fn asset_directory_check_only_reports_missing_directory() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing-assets");

    let issues = super::check_assets_dir(&missing);

    assert!(matches!(
        issues.as_slice(),
        [super::HealthIssue::AssetsDirMissing]
    ));
}
