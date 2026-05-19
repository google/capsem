use super::*;

fn ready_asset_health() -> crate::api::AssetHealth {
    crate::api::AssetHealth {
        ready: true,
        state: crate::api::AssetHealthState::Ready,
        profile_id: Some("everyday-work".to_string()),
        profile_revision: Some("2026.0520.1".to_string()),
        version: Some("everyday-work@2026.0520.1".to_string()),
        arch: Some("arm64".to_string()),
        missing: Vec::new(),
        progress: None,
        error: None,
        retry_count: 0,
        retryable: false,
        saved_vm_dependencies: Vec::new(),
        checked_at_unix_secs: Some(1_779_264_000),
    }
}

#[test]
fn attributes_profile_v2_asset_health() {
    let dir = tempfile::tempdir().unwrap();

    let report = build_debug_report(DebugReportInput {
        generated_at: "2026-05-12T12:00:00Z".into(),
        version: "1.1.1778542197".into(),
        build_hash: "1d95b80.1778545863".into(),
        build_ts: "dev".into(),
        platform: "macos/aarch64".into(),
        capsem_home: dir.path().join(".capsem"),
        run_dir: dir.path().join(".capsem/run"),
        assets_dir: dir.path().join("assets"),
        asset_locations: None,
        asset_health: Some(ready_asset_health()),
        running_vm_count: 1,
        total_vm_count: 2,
        status_issues: Vec::new(),
        defunct_sessions: Vec::new(),
        install: None,
        process_pids: Vec::new(),
        settings_profiles: None,
    })
    .unwrap();

    assert!(report.text.contains("source: profile_v2_asset_health"));
    assert!(report.text.contains("profile_asset_health_present: true"));
    assert!(report.text.contains("profile_asset_ready: true"));
    assert!(report
        .text
        .contains("profile_asset_profile_id: everyday-work"));
    assert!(report
        .text
        .contains("profile_asset_profile_revision: 2026.0520.1"));
    assert!(report
        .text
        .contains("profile_asset_version: everyday-work@2026.0520.1"));
    assert!(report.text.contains("profile_asset_arch: arm64"));
    assert!(report.text.contains("running_vm_count: 1"));
    assert!(report.text.contains("total_vm_count: 2"));
}

#[test]
fn json_report_captures_setup_runtime_assets_and_redacted_logs() {
    let dir = tempfile::tempdir().unwrap();
    let capsem_home = dir.path().join(".capsem");
    let run_dir = capsem_home.join("run");
    let assets_dir = capsem_home.join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(run_dir.join("gateway.port"), "19222\n").unwrap();
    std::fs::write(run_dir.join("gateway.pid"), "4242\n").unwrap();
    std::fs::write(
        run_dir.join("service.log"),
        "starting from /Users/alice/.capsem Authorization: Bearer supersecret\n\
         token=supersecret api_key=sk-ant-real-secret\n\
         dns failed for elie.net\n",
    )
    .unwrap();
    std::fs::write(
        capsem_home.join("setup-state.json"),
        r#"{
            "schema_version": 1,
            "completed_steps": ["assets", "providers"],
            "security_preset": "medium",
            "providers_done": true,
            "service_installed": true,
            "install_completed": true,
            "onboarding_completed": false,
            "onboarding_version": 0
        }"#,
    )
    .unwrap();
    let bin_dir = capsem_home.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::write(bin_dir.join("capsem"), b"cli").unwrap();
    std::fs::write(bin_dir.join("capsem-service"), b"service").unwrap();

    let report = build_debug_report(DebugReportInput {
        generated_at: "2026-05-12T12:00:00Z".into(),
        version: "1.1.1778542197".into(),
        build_hash: "1d95b80.1778545863".into(),
        build_ts: "dev".into(),
        platform: "macos/aarch64".into(),
        capsem_home: capsem_home.clone(),
        run_dir,
        assets_dir,
        asset_locations: None,
        asset_health: Some(ready_asset_health()),
        running_vm_count: 1,
        total_vm_count: 2,
        status_issues: vec!["Initrd asset is MISSING: ~/.capsem/assets/initrd.img".into()],
        defunct_sessions: vec![DefunctSessionReport {
            name: "broken-vm".into(),
            last_error: Some("boot failed before ready".into()),
        }],
        install: Some(InstallReportInput {
            bin_dir: bin_dir.clone(),
            current_exe: bin_dir.join("capsem"),
            service_unit_path: Some(
                capsem_home.join("Library/LaunchAgents/com.capsem.service.plist"),
            ),
        }),
        process_pids: vec![
            ProcessReportInput {
                name: "service".into(),
                pid: Some(4242),
                executable_path: Some(bin_dir.join("capsem-service")),
            },
            ProcessReportInput {
                name: "gateway".into(),
                pid: Some(5151),
                executable_path: None,
            },
        ],
        settings_profiles: None,
    })
    .unwrap();

    let json = serde_json::to_value(&report.json).unwrap();
    assert_eq!(json["schema"], "capsem.debug.v2");
    assert_eq!(json["redacted"], true);
    assert_eq!(json["setup"]["present"], true);
    assert_eq!(json["setup"]["install_completed"], true);
    assert_eq!(json["setup"]["providers_done"], true);
    assert_eq!(json["runtime"]["gateway_port_file"]["contents"], "19222");
    assert_eq!(
        json["status"]["issues"][0],
        "Initrd asset is MISSING: ~/.capsem/assets/initrd.img"
    );
    assert_eq!(json["status"]["defunct_sessions"][0]["name"], "broken-vm");
    assert_eq!(
        json["status"]["defunct_sessions"][0]["last_error"],
        "boot failed before ready"
    );
    assert_eq!(json["host"]["os"], "macos");
    assert_eq!(json["host"]["arch"], "aarch64");
    assert_eq!(json["install"]["bin_dir"], redact_path_for_report(&bin_dir));
    assert_eq!(
        json["install"]["current_exe"],
        redact_path_for_report(&bin_dir.join("capsem"))
    );
    assert_eq!(
        json["install"]["service_unit_path"],
        redact_path_for_report(&capsem_home.join("Library/LaunchAgents/com.capsem.service.plist"))
    );
    assert!(json["host_binaries"]["capsem"]["exists"].as_bool().unwrap());
    assert!(
        json["host_binaries"]["capsem"]["hash"]
            .as_str()
            .unwrap()
            .len()
            >= 32
    );
    assert_eq!(json["processes"][0]["name"], "service");
    assert_eq!(json["processes"][0]["pid"], 4242);
    assert_eq!(
        json["processes"][0]["executable_path"],
        redact_path_for_report(&bin_dir.join("capsem-service"))
    );
    assert!(json["disk"]["capsem_home"]["available_bytes"].is_number());
    assert_eq!(json["assets"]["source"], "profile_v2_asset_health");
    assert_eq!(json["assets"]["health"]["ready"], true);
    assert_eq!(json["assets"]["health"]["state"], "ready");
    assert_eq!(json["assets"]["health"]["profile_id"], "everyday-work");
    assert_eq!(json["assets"]["health"]["profile_revision"], "2026.0520.1");
    assert_eq!(
        json["assets"]["health"]["version"],
        "everyday-work@2026.0520.1"
    );
    assert_eq!(json["assets"]["health"]["arch"], "arm64");

    let serialized = serde_json::to_string(&json).unwrap();
    assert!(serialized.contains("dns failed for elie.net"));
    assert!(!serialized.contains("supersecret"));
    assert!(!serialized.contains("sk-ant-real-secret"));
    assert!(!serialized.contains("/Users/alice"));
    assert!(serialized.contains("Bearer <redacted>"));
}

#[test]
fn redacts_home_paths() {
    assert_eq!(
        redact_path_for_report(Path::new("/Users/alice/.capsem/assets/arm64/initrd.img")),
        "~/.capsem/assets/arm64/initrd.img"
    );
    assert_eq!(
        redact_path_for_report(Path::new("/home/bob/.capsem/run/service.sock")),
        "~/.capsem/run/service.sock"
    );
}

#[test]
fn updating_profile_assets_are_reported_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let mut health = ready_asset_health();
    health.ready = false;
    health.state = crate::api::AssetHealthState::Updating;
    health.missing = vec!["initrd.img".to_string(), "rootfs.squashfs".to_string()];

    let report = build_debug_report(DebugReportInput {
        generated_at: "2026-05-12T12:00:00Z".into(),
        version: "1.1.1778542197".into(),
        build_hash: "1d95b80.1778545863".into(),
        build_ts: "dev".into(),
        platform: "macos/aarch64".into(),
        capsem_home: dir.path().join(".capsem"),
        run_dir: dir.path().join(".capsem/run"),
        assets_dir: dir.path().join("assets"),
        asset_locations: None,
        asset_health: Some(health),
        running_vm_count: 0,
        total_vm_count: 0,
        status_issues: Vec::new(),
        defunct_sessions: Vec::new(),
        install: None,
        process_pids: Vec::new(),
        settings_profiles: None,
    })
    .unwrap();

    assert!(report
        .text
        .contains("profile_asset_missing: initrd.img,rootfs.squashfs"));
}

#[test]
fn includes_settings_profiles_without_leaking_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = capsem_core::settings_profiles::ServiceSettings::default();
    settings.profiles.base_dirs = vec![dir.path().join("profiles/base")];
    settings.profiles.user_dirs = vec![dir.path().join("profiles/user")];
    settings.assets.assets_dir = Some(dir.path().join("corp/assets"));
    settings.assets.image_roots = vec![dir.path().join("corp/images")];
    settings.assets.download_base_url = Some("https://assets.example.test/capsem".to_string());
    settings.telemetry.enabled = true;
    settings.telemetry.endpoint = Some("https://otel.example.test/v1/traces".to_string());
    settings.remote_policy.enabled = true;
    settings.remote_policy.endpoint = Some("https://policy.example.test/decision".to_string());
    settings.remote_policy.auth_token = Some("policy-token-should-not-leak".to_string());
    settings.credentials.items.insert(
        "openai".to_string(),
        capsem_core::settings_profiles::TomlCredential {
            description: Some("OpenAI".to_string()),
            value: "sk-secret-should-not-leak".to_string(),
        },
    );
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles).unwrap();
    let (effective, trace) =
        capsem_core::settings_profiles::resolve_effective_vm_settings_with_corp(&settings, None)
            .unwrap();
    let snapshot =
        capsem_core::settings_profiles::SettingsProfilesDebugSnapshot::from_parts_with_trace(
            &settings,
            &catalog,
            Some(&effective),
            Some(&trace),
        );
    let asset_locations = capsem_core::settings_profiles::resolve_service_asset_locations(
        &settings,
        None,
        None,
        dir.path().join("assets"),
    )
    .unwrap();

    let report = build_debug_report(DebugReportInput {
        generated_at: "2026-05-12T12:00:00Z".into(),
        version: "1.1.1778542197".into(),
        build_hash: "1d95b80.1778545863".into(),
        build_ts: "dev".into(),
        platform: "macos/aarch64".into(),
        capsem_home: dir.path().join(".capsem"),
        run_dir: dir.path().join(".capsem/run"),
        assets_dir: dir.path().join("assets"),
        asset_locations: Some(asset_locations),
        asset_health: None,
        running_vm_count: 0,
        total_vm_count: 0,
        status_issues: Vec::new(),
        defunct_sessions: Vec::new(),
        install: None,
        process_pids: Vec::new(),
        settings_profiles: Some(snapshot),
    })
    .unwrap();

    assert!(report.text.contains("[settings_profiles]"));
    assert!(report.text.contains("default_profile: everyday-work"));
    assert!(report.text.contains("selected_profile: everyday-work"));
    assert!(report
        .text
        .contains("profile: everyday-work source=built-in locked=true"));
    assert!(report
        .text
        .contains("asset_download_base_url: https://assets.example.test/capsem"));
    assert!(report.text.contains("assets_dir: "));
    assert!(report
        .text
        .contains("resolved_assets_dir_origin: service_settings"));
    assert!(report.text.contains("image_roots: "));
    assert!(report
        .text
        .contains("resolved_image_roots_origin: service_settings"));
    assert!(report
        .text
        .contains("telemetry_endpoint: https://otel.example.test/v1/traces"));
    assert!(report
        .text
        .contains("remote_policy_endpoint: https://policy.example.test/decision"));
    assert!(report.text.contains("credential_ids: openai"));
    assert!(!report.text.contains("sk-secret-should-not-leak"));
    assert!(!report.text.contains("policy-token-should-not-leak"));
}

#[test]
fn includes_settings_profiles_load_error() {
    let dir = tempfile::tempdir().unwrap();
    let snapshot = capsem_core::settings_profiles::SettingsProfilesDebugSnapshot::from_error(
        "profiles.default_profile: profile id cannot be empty",
    );

    let report = build_debug_report(DebugReportInput {
        generated_at: "2026-05-12T12:00:00Z".into(),
        version: "1.1.1778542197".into(),
        build_hash: "1d95b80.1778545863".into(),
        build_ts: "dev".into(),
        platform: "macos/aarch64".into(),
        capsem_home: dir.path().join(".capsem"),
        run_dir: dir.path().join(".capsem/run"),
        assets_dir: dir.path().join("assets"),
        asset_locations: None,
        asset_health: None,
        running_vm_count: 0,
        total_vm_count: 0,
        status_issues: Vec::new(),
        defunct_sessions: Vec::new(),
        install: None,
        process_pids: Vec::new(),
        settings_profiles: Some(snapshot),
    })
    .unwrap();

    assert!(report.text.contains("[settings_profiles]"));
    assert!(report.text.contains("present: true"));
    assert!(report
        .text
        .contains("load_error: profiles.default_profile: profile id cannot be empty"));
}

#[test]
fn settings_profiles_section_includes_resolver_trace_summary_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let settings = capsem_core::settings_profiles::ServiceSettings::default();
    let catalog = capsem_core::settings_profiles::discover_profiles(&settings.profiles).unwrap();
    let (effective, trace) =
        capsem_core::settings_profiles::resolve_effective_vm_settings_with_corp(&settings, None)
            .unwrap();
    let snapshot =
        capsem_core::settings_profiles::SettingsProfilesDebugSnapshot::from_parts_with_trace(
            &settings,
            &catalog,
            Some(&effective),
            Some(&trace),
        );

    let report = build_debug_report(DebugReportInput {
        generated_at: "2026-05-12T12:00:00Z".into(),
        version: "1.1.1778542197".into(),
        build_hash: "1d95b80.1778545863".into(),
        build_ts: "dev".into(),
        platform: "macos/aarch64".into(),
        capsem_home: dir.path().join(".capsem"),
        run_dir: dir.path().join(".capsem/run"),
        assets_dir: dir.path().join("assets"),
        asset_locations: None,
        asset_health: None,
        running_vm_count: 0,
        total_vm_count: 0,
        status_issues: Vec::new(),
        defunct_sessions: Vec::new(),
        install: None,
        process_pids: Vec::new(),
        settings_profiles: Some(snapshot),
    })
    .unwrap();

    assert!(report.text.contains("resolver_trace_event_count:"));
    assert!(report.text.contains("resolver_trace_corp_event_count: 0"));
    assert!(report.text.contains("resolver_trace_event:"));
}
