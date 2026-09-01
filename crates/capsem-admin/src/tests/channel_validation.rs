use super::*;

#[test]
fn assets_channel_build_externalizes_shared_blobs_but_owns_profile_blobs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("cache/target/distribution");
    let asset_base = "https://github.com/google/capsem/releases/download/assets-v{asset_version}";

    let report = build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-02-03T00:00:00Z",
        Some(asset_base),
    )
    .expect("externalized channel builds without local blobs");

    assert_eq!(report.copied_assets, 0);
    assert!(!out_dir.join("assets/releases").exists());
    assert!(out_dir.join("profiles/releases/stable").is_dir());
    let channel_manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("assets/stable/manifest.json")).unwrap())
            .expect("channel manifest parses");
    let health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("health.json")).unwrap()).expect("health parses");
    let rootfs_url = "https://github.com/google/capsem/releases/download/assets-v2030.0101.1/arm64-rootfs.erofs";
    assert_eq!(health["urls"]["asset_base"].as_str(), Some(asset_base));
    let health_files = health["assets"]["files"].as_array().expect("asset files");
    assert!(health_files.iter().any(|file| file["url"].as_str() == Some(rootfs_url)));
    assert!(
        serde_json::to_string(&channel_manifest)
            .expect("serialize channel manifest")
            .contains("/profiles/releases/stable/code/"),
        "selected channel manifest should carry channel/profile-owned URLs"
    );
    check_assets_channel(&out_dir, "stable").expect("externalized channel checks");
}

#[test]
fn assets_channel_check_rejects_bad_health_schema() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let assets_dir = temp.path().join("assets");
    let profiles_dir = repo_config_profiles_dir();
    let out_dir = temp.path().join("cache/target/distribution");
    build_assets_channel(
        &file_url(&manifest_path),
        &assets_dir,
        &profiles_dir,
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health")).expect("health json");
    health["schema"] = serde_json::Value::String("capsem.bad_schema".to_string());
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap()).expect("write bad health");

    let error = check_assets_channel(&out_dir, "stable").expect_err("bad health schema rejected");

    assert!(
        format!("{error:#}").contains("health.json schema mismatch"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_allows_package_owned_sbom_without_host_sbom_summary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("cache/target/distribution");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health")).expect("health json");
    health["evidence"]["host_sboms"] = serde_json::json!([]);
    health["evidence"]["attestations"]
        .as_array_mut()
        .expect("attestations")
        .retain(|attestation| {
            attestation.get("name").and_then(|name| name.as_str()) != Some("github_attestations_host_sbom")
        });
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap()).expect("write health without host SBOM");

    check_assets_channel(&out_dir, "stable").expect("package-owned SBOMs are allowed");
}

#[test]
fn assets_channel_check_rejects_missing_asset_release_date() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("cache/target/distribution");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health")).expect("health json");
    health["asset_releases"][0]
        .as_object_mut()
        .expect("asset release object")
        .remove("date");
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without asset release date");

    let error = check_assets_channel(&out_dir, "stable").expect_err("missing release date rejected");

    assert!(
        format!("{error:#}").contains("health.json asset release date mismatch"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_missing_evidence_vm_obom() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("cache/target/distribution");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health")).expect("health json");
    health["evidence"]["vm_oboms"] = serde_json::json!([]);
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap()).expect("write health without VM OBOM");

    let error = check_assets_channel(&out_dir, "stable").expect_err("missing VM OBOM rejected");

    assert!(
        format!("{error:#}").contains("health.json missing VM OBOM evidence"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_missing_evidence_vm_attestation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("cache/target/distribution");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health")).expect("health json");
    let attestations = health["evidence"]["attestations"]
        .as_array()
        .expect("attestations")
        .iter()
        .filter(|attestation| {
            attestation.get("name").and_then(|name| name.as_str()) != Some("github_attestations_vm_assets")
        })
        .cloned()
        .collect::<Vec<_>>();
    health["evidence"]["attestations"] = serde_json::Value::Array(attestations);
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without VM attestation");

    let error = check_assets_channel(&out_dir, "stable").expect_err("missing VM attestation rejected");

    assert!(
        format!("{error:#}").contains("health.json VM asset attestation evidence missing"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_missing_vm_attestation_predicate() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("cache/target/distribution");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health")).expect("health json");
    let attestations = health["evidence"]["attestations"].as_array_mut().expect("attestations");
    let vm_attestation = attestations
        .iter_mut()
        .find(|attestation| {
            attestation.get("name").and_then(|name| name.as_str()) == Some("github_attestations_vm_assets")
        })
        .expect("VM asset attestation");
    vm_attestation
        .as_object_mut()
        .expect("attestation object")
        .remove("predicate_url");
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap()).expect("write health without VM predicate");

    let error = check_assets_channel(&out_dir, "stable").expect_err("missing VM attestation predicate rejected");

    assert!(
        format!("{error:#}").contains("health.json VM asset attestation predicate_url missing"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_missing_host_sbom_attestation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("cache/target/distribution");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health")).expect("health json");
    let attestations = health["evidence"]["attestations"]
        .as_array()
        .expect("attestations")
        .iter()
        .filter(|attestation| {
            attestation.get("name").and_then(|name| name.as_str()) != Some("github_attestations_host_sbom")
        })
        .cloned()
        .collect::<Vec<_>>();
    health["evidence"]["attestations"] = serde_json::Value::Array(attestations);
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without host SBOM attestation");

    let error = check_assets_channel(&out_dir, "stable").expect_err("missing host SBOM attestation rejected");

    assert!(
        format!("{error:#}").contains("health.json host SBOM attestation evidence missing"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_host_sbom_attestation_missing_package_subject() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest")).expect("manifest json");
    manifest["binaries"]["releases"]["1.0.0"]["files"]
        .as_array_mut()
        .expect("binary files")
        .push(serde_json::json!({
            "name": "Capsem_1.0.0_arm64.deb",
            "size": 789,
            "sha256": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210",
            "blake3": "3333333333333333333333333333333333333333333333333333333333333333",
            "binaries": [
                {
                    "name": "capsem-tray",
                    "installed_path": "/usr/bin/capsem-tray",
                    "size": 19,
                    "sha256": "4444444444444444444444444444444444444444444444444444444444444444",
                    "blake3": "5555555555555555555555555555555555555555555555555555555555555555",
                    "sbom_component_ref": "SPDXRef-File-capsem-tray"
                }
            ]
        }));
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("manifest json"),
    )
    .expect("write manifest with deb");
    let out_dir = temp.path().join("cache/target/distribution");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health")).expect("health json");
    let host_attestation = health["evidence"]["attestations"]
        .as_array_mut()
        .expect("attestations")
        .iter_mut()
        .find(|attestation| attestation.get("name").and_then(|name| name.as_str()) == Some("github_attestations_host"))
        .expect("host package attestation");
    let subjects = host_attestation["subjects"]
        .as_array_mut()
        .expect("host package subjects");
    *subjects = vec![serde_json::json!(
        "https://github.com/google/capsem/releases/download/v1.0.0/not-a-package"
    )];
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without deb SBOM subject");

    let error = check_assets_channel(&out_dir, "stable").expect_err("missing host package SBOM subject rejected");

    assert!(
        format!("{error:#}").contains("health.json host package attestation subjects missing"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_attestation_without_verification_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let out_dir = temp.path().join("cache/target/distribution");
    build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let health_path = out_dir.join("health.json");
    let mut health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&health_path).expect("health")).expect("health json");
    health["evidence"]["attestations"][0]
        .as_object_mut()
        .expect("attestation object")
        .remove("verify_command");
    fs::write(&health_path, serde_json::to_string_pretty(&health).unwrap())
        .expect("write health without verification metadata");

    let error =
        check_assets_channel(&out_dir, "stable").expect_err("missing attestation verification metadata rejected");

    assert!(
        format!("{error:#}").contains("health.json attestation verify_command missing"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_check_rejects_missing_current_asset_blob() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let assets_dir = temp.path().join("assets");
    let profiles_dir = repo_config_profiles_dir();
    let out_dir = temp.path().join("cache/target/distribution");
    build_assets_channel(
        &file_url(&manifest_path),
        &assets_dir,
        &profiles_dir,
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");
    fs::remove_file(out_dir.join("assets/releases/2030.0101.1/arm64-rootfs.erofs")).expect("remove published rootfs");

    let error = check_assets_channel(&out_dir, "stable").expect_err("missing asset blob rejected");

    assert!(format!("{error:#}").contains("arm64-rootfs.erofs"), "{error:#}");
}

#[test]
fn assets_channel_rejects_unsafe_channel_names() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let manifest_url = file_url(&manifest_path);
    let assets_dir = temp.path().join("assets");
    let profiles_dir = repo_config_profiles_dir();
    for channel in ["../stable", "stable.v1", "stable channel", "<stable>"] {
        let error = build_assets_channel(
            &manifest_url,
            &assets_dir,
            &profiles_dir,
            channel,
            "1.0.2",
            &temp.path().join("cache/target/distribution"),
            "2030-01-01T00:00:00Z",
            None,
        )
        .expect_err("unsafe channel rejected");

        assert!(error.to_string().contains("invalid asset channel name"));
    }
}

#[test]
fn assets_channel_manifest_source_must_be_url() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let error = build_assets_channel(
        &manifest_path.display().to_string(),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &temp.path().join("cache/target/distribution"),
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect_err("bare manifest path rejected");

    assert!(format!("{error:#}").contains("manifest must be a URL"), "{error:#}");
}
