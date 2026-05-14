use super::*;

fn manifest_with_asset_hashes(
    asset_version: &str,
    binary_version: &str,
    kernel_hash: &str,
    initrd_hash: &str,
    rootfs_hash: &str,
) -> capsem_core::asset_manager::ManifestV2 {
    let text = format!(
        r#"{{
    "format": 2,
    "assets": {{
        "current": "{asset_version}",
        "releases": {{
            "{asset_version}": {{
                "date": "2026-05-12",
                "deprecated": false,
                "min_binary": "1.0.0",
                "arches": {{
                    "arm64": {{
                        "vmlinuz": {{ "hash": "{kernel_hash}", "size": 6 }},
                        "initrd.img": {{ "hash": "{initrd_hash}", "size": 6 }},
                        "rootfs.squashfs": {{ "hash": "{rootfs_hash}", "size": 6 }}
                    }}
                }}
            }}
        }}
    }},
    "binaries": {{
        "current": "{binary_version}",
        "releases": {{
            "{binary_version}": {{
                "date": "2026-05-12",
                "deprecated": false,
                "min_assets": "{asset_version}"
            }}
        }}
    }}
}}"#
    );
    capsem_core::asset_manager::ManifestV2::from_json(&text).unwrap()
}

fn write_hash_named_asset(dir: &Path, logical: &str, bytes: &[u8]) -> String {
    let tmp = dir.join(logical.replace('/', "-"));
    std::fs::write(&tmp, bytes).unwrap();
    let hash = capsem_core::asset_manager::hash_file(&tmp).unwrap();
    let final_path = dir.join(capsem_core::asset_manager::hash_filename(logical, &hash));
    std::fs::rename(tmp, final_path).unwrap();
    hash
}

#[test]
fn attributes_the_installed_initrd() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    let arch_dir = assets_dir.join("arm64");
    std::fs::create_dir_all(&arch_dir).unwrap();
    let kernel_hash = write_hash_named_asset(&arch_dir, "vmlinuz", b"kernel");
    let initrd_hash = write_hash_named_asset(&arch_dir, "initrd.img", b"initd!");
    let rootfs_hash = write_hash_named_asset(&arch_dir, "rootfs.squashfs", b"rootfs");
    let manifest = manifest_with_asset_hashes(
        "2026.0512.1",
        "1.1.1778542197",
        &kernel_hash,
        &initrd_hash,
        &rootfs_hash,
    );

    let report = build_debug_report(DebugReportInput {
        generated_at: "2026-05-12T12:00:00Z".into(),
        version: "1.1.1778542197".into(),
        build_hash: "1d95b80.1778545863".into(),
        build_ts: "dev".into(),
        platform: "macos/aarch64".into(),
        capsem_home: dir.path().join(".capsem"),
        run_dir: dir.path().join(".capsem/run"),
        assets_dir: assets_dir.clone(),
        manifest: Some(manifest),
        running_vm_count: 1,
        total_vm_count: 2,
        status_issues: Vec::new(),
        defunct_sessions: Vec::new(),
        install: None,
        process_pids: Vec::new(),
    })
    .unwrap();

    assert!(report
        .text
        .contains("asset_version_for_binary: 2026.0512.1"));
    assert!(report
        .text
        .contains(&format!("initrd_manifest_hash: {initrd_hash}")));
    assert!(report.text.contains("initrd_path: "));
    assert!(report
        .text
        .contains("initrd_actual_hash_matches_manifest: true"));
    assert!(report.text.contains("running_vm_count: 1"));
    assert!(report.text.contains("total_vm_count: 2"));
}

#[test]
fn json_report_captures_setup_runtime_assets_and_redacted_logs() {
    let dir = tempfile::tempdir().unwrap();
    let capsem_home = dir.path().join(".capsem");
    let run_dir = capsem_home.join("run");
    let assets_dir = capsem_home.join("assets");
    let arch_dir = assets_dir.join("arm64");
    std::fs::create_dir_all(&arch_dir).unwrap();
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

    let kernel_hash = write_hash_named_asset(&arch_dir, "vmlinuz", b"kernel");
    let initrd_hash = write_hash_named_asset(&arch_dir, "initrd.img", b"initd!");
    let rootfs_hash = write_hash_named_asset(&arch_dir, "rootfs.squashfs", b"rootfs");
    let manifest = manifest_with_asset_hashes(
        "2026.0512.1",
        "1.1.1778542197",
        &kernel_hash,
        &initrd_hash,
        &rootfs_hash,
    );

    let report = build_debug_report(DebugReportInput {
        generated_at: "2026-05-12T12:00:00Z".into(),
        version: "1.1.1778542197".into(),
        build_hash: "1d95b80.1778545863".into(),
        build_ts: "dev".into(),
        platform: "macos/aarch64".into(),
        capsem_home: capsem_home.clone(),
        run_dir,
        assets_dir,
        manifest: Some(manifest),
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
    assert_eq!(json["assets"]["manifest"]["assets_current"], "2026.0512.1");
    assert_eq!(
        json["assets"]["files"]["initrd"]["manifest_hash"],
        initrd_hash
    );
    assert_eq!(
        json["assets"]["files"]["initrd"]["actual_hash_matches_manifest"],
        true
    );
    assert_eq!(json["assets"]["files"]["initrd"]["size_bytes"], 6);

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
fn missing_assets_are_reported_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    let missing_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let manifest = manifest_with_asset_hashes(
        "2026.0512.1",
        "1.1.1778542197",
        missing_hash,
        missing_hash,
        missing_hash,
    );

    let report = build_debug_report(DebugReportInput {
        generated_at: "2026-05-12T12:00:00Z".into(),
        version: "1.1.1778542197".into(),
        build_hash: "1d95b80.1778545863".into(),
        build_ts: "dev".into(),
        platform: "macos/aarch64".into(),
        capsem_home: dir.path().join(".capsem"),
        run_dir: dir.path().join(".capsem/run"),
        assets_dir,
        manifest: Some(manifest),
        running_vm_count: 0,
        total_vm_count: 0,
        status_issues: Vec::new(),
        defunct_sessions: Vec::new(),
        install: None,
        process_pids: Vec::new(),
    })
    .unwrap();

    assert!(report.text.contains("initrd_exists: false"));
    assert!(report
        .text
        .contains("initrd_actual_hash_matches_manifest: false"));
}
