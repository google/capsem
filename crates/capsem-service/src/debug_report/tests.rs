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
    })
    .unwrap();

    assert!(report.text.contains("initrd_exists: false"));
    assert!(report
        .text
        .contains("initrd_actual_hash_matches_manifest: false"));
}
