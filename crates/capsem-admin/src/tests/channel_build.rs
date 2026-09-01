use super::*;

#[test]
fn assets_channel_build_writes_manifest_under_channel_assets_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let manifest_url = file_url(&manifest_path);
    let assets_dir = temp.path().join("assets");
    let profiles_dir = repo_config_profiles_dir();
    let out_dir = temp.path().join("cache/target/distribution");

    let report = build_assets_channel(
        &manifest_url,
        &assets_dir,
        &profiles_dir,
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("asset channel builds");

    let channel_manifest = out_dir.join("assets/stable/manifest.json");
    let release_dir = out_dir.join("assets/releases/2030.0101.1");
    assert_eq!(report.manifest, channel_manifest.display().to_string());
    assert_eq!(report.copied_assets, 5);
    assert!(
        !out_dir.join("index.html").exists(),
        "human release pages are built by release-site Astro, not capsem-admin"
    );
    assert!(out_dir.join("health.json").is_file());
    assert!(channel_manifest.is_file());
    assert_eq!(
        fs::read(release_dir.join("arm64-vmlinuz")).expect("published kernel"),
        b"kernel-arm64"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let source = fs::metadata(assets_dir.join("arm64/vmlinuz")).unwrap();
        let release = fs::metadata(release_dir.join("arm64-vmlinuz")).unwrap();
        assert_ne!(
            source.ino(),
            release.ino(),
            "an external fixture is unclassified and must copy rather than fail open"
        );
    }
    assert!(release_dir.join("arm64-initrd.img").is_file());
    assert!(release_dir.join("arm64-rootfs.erofs").is_file());
    assert!(release_dir.join("arm64-obom.cdx.json").is_file());
    assert!(release_dir.join("arm64-software-inventory.json").is_file());
    let channel_manifest_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&channel_manifest).expect("channel manifest"))
            .expect("channel manifest json");
    let source_manifest_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("source manifest"))
            .expect("source manifest json");
    let kernel_artifact = channel_manifest_json["profiles"]
        .as_object()
        .expect("profiles object")
        .values()
        .flat_map(|profile| {
            profile["architectures"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|image| image["architecture"].as_str() == Some("arm64"))
                .flat_map(|image| image["images"].as_array().into_iter().flatten())
        })
        .find(|artifact| artifact["kind"].as_str() == Some("kernel"))
        .expect("arm64 kernel artifact");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let published = out_dir.join(
            kernel_artifact["url"]
                .as_str()
                .expect("kernel publication URL")
                .trim_start_matches('/'),
        );
        assert_ne!(
            fs::metadata(assets_dir.join("arm64/vmlinuz")).unwrap().ino(),
            fs::metadata(published).unwrap().ino(),
            "an external fixture must remain independent from published output"
        );
    }
    assert_eq!(
        kernel_artifact["digest"]["blake3"],
        source_manifest_json["assets"]["releases"]["2030.0101.1"]["arches"]["arm64"]["vmlinuz"]["hash"]
    );
    assert!(
        kernel_artifact["digest"]["sha256"]
            .as_str()
            .is_some_and(|hash| hash.len() == 64),
        "channel manifest must hydrate VM asset SHA-256"
    );
    let health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("health.json")).unwrap()).expect("health json parses");
    assert_eq!(health["schema"].as_str(), Some("capsem.assets_channel.health.v1"));
    assert_eq!(health["current"]["assets"].as_str(), Some("2030.0101.1"));
    assert_eq!(
        health["urls"]["manifest"].as_str(),
        Some("/assets/stable/manifest.json")
    );
    assert_eq!(health["urls"]["asset_base"].as_str(), Some("/assets/releases"));
    assert_eq!(
        health["assets"]["files"][0]["url"].as_str(),
        Some("/assets/releases/2030.0101.1/arm64-initrd.img")
    );
    assert!(
        health["updates"]["assets"]["files"].is_null(),
        "VM asset file inventory belongs under assets.files, not updates.assets.files"
    );
    assert_eq!(health["assets"]["compatibility"]["min_binary"].as_str(), Some("1.0.0"));
    assert_eq!(health["assets"]["requires_newer"]["binary"].as_bool(), Some(false));
    assert_eq!(health["asset_releases"][0]["date"].as_str(), Some("2030-01-01"));
    assert_eq!(
        health["evidence"]["vm_oboms"][0]["url"].as_str(),
        Some("/assets/releases/2030.0101.1/arm64-obom.cdx.json")
    );
    assert_eq!(
        health["evidence"]["host_sboms"][0]["name"].as_str(),
        Some("capsem-sbom.spdx.json")
    );
    assert_eq!(
        health["evidence"]["host_binary_files"][1]["name"].as_str(),
        Some("capsem-sbom.spdx.json")
    );
    assert_eq!(
        health["evidence"]["attestations"][0]["name"].as_str(),
        Some("github_attestations_host")
    );
    assert_eq!(
        health["evidence"]["attestations"][0]["predicate_type"].as_str(),
        Some("https://slsa.dev/provenance/v1")
    );
    assert_eq!(
        health["evidence"]["attestations"][0]["verify_command"].as_str(),
        Some("gh attestation verify <subject-url> --owner google")
    );
    assert_eq!(
        health["evidence"]["attestations"][1]["name"].as_str(),
        Some("github_attestations_host_sbom")
    );
    assert_eq!(
        health["evidence"]["attestations"][1]["predicate_type"].as_str(),
        Some("https://spdx.dev/Document/v2.3")
    );
    assert_eq!(
        health["evidence"]["attestations"][1]["predicate_url"].as_str(),
        Some("https://github.com/google/capsem/releases/download/v1.0.0/capsem-sbom.spdx.json")
    );
    assert_eq!(
        health["evidence"]["attestations"][1]["subjects"][0].as_str(),
        Some("https://github.com/google/capsem/releases/download/v1.0.0/capsem-1.0.0.pkg")
    );
    assert_eq!(
        health["evidence"]["attestations"][2]["name"].as_str(),
        Some("github_attestations_vm_assets")
    );
    assert_eq!(
        health["evidence"]["attestations"][2]["predicate_url"].as_str(),
        Some("/assets/releases/2030.0101.1/arm64-obom.cdx.json")
    );
    assert_eq!(
        health["evidence"]["attestations"][2]["subjects"][0].as_str(),
        Some("/assets/releases/2030.0101.1/arm64-initrd.img")
    );
    assert_eq!(
        health["updates"]["binary"]["latest"].as_str(),
        health["current"]["binary"].as_str()
    );
    assert_eq!(
        health["updates"]["binary"]["current"].as_str(),
        health["current"]["binary"].as_str()
    );
    assert_eq!(
        health["updates"]["binary"]["source"].as_str(),
        Some("manifest.binaries.current")
    );
    assert_eq!(health["updates"]["assets"]["latest"].as_str(), Some("2030.0101.1"));
    assert_eq!(health["updates"]["assets"]["current"].as_str(), Some("2030.0101.1"));
    assert_eq!(
        health["updates"]["assets"]["manifest"].as_str(),
        Some("/assets/stable/manifest.json")
    );
    assert_eq!(
        health["updates"]["assets"]["asset_base"].as_str(),
        Some("/assets/releases")
    );
    assert_eq!(
        health["updates"]["assets"]["compatibility"]["min_binary"].as_str(),
        Some("1.0.0")
    );
    assert_eq!(
        health["updates"]["assets"]["requires_newer"]["binary"].as_bool(),
        Some(false)
    );
    assert_eq!(
        health["profiles"]["revision"].as_str(),
        health["updates"]["profiles"]["latest"].as_str()
    );
    assert!(
        health["profiles"]["compatibility"].is_null(),
        "profiles must not publish channel compatibility"
    );
    assert_eq!(health["profiles"]["min_binary"].as_str(), Some("1.0.0"));
    assert!(
        health["updates"]["profiles"]["compatibility"].is_null(),
        "profile update metadata must not publish channel compatibility"
    );
    assert_eq!(health["updates"]["profiles"]["state"].as_str(), Some("current"));
    assert_eq!(health["profiles"]["source"].as_str(), Some("manifest.profiles"));
    assert!(health["profiles"]["hash"].is_null());
    assert_eq!(
        health["updates"]["profiles"]["source"].as_str(),
        Some("manifest.profiles")
    );
    assert!(health["updates"]["profiles"]["hash"].is_null());
    assert_eq!(health["updates"]["images"]["latest"].as_str(), None);
    assert!(
        health["updates"]["images"]["latest"].is_null(),
        "unpublished image latest should be explicit null"
    );
    assert_eq!(health["updates"]["images"]["state"].as_str(), Some("not_published"));
    assert_eq!(
        health["updates"]["images"]["source"].as_str(),
        Some("manifest.profiles.images")
    );

    let check = check_assets_channel(&out_dir, "stable").expect("asset channel checks");
    assert_eq!(check.channel, "stable");
    assert_eq!(check.manifest, channel_manifest.display().to_string());
}

#[test]
fn release_graph_manifest_version_is_independent_from_package_and_assets() {
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

    let channels: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("channels.json")).unwrap()).expect("channels json");
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("assets/stable/manifest.json")).unwrap())
            .expect("graph manifest json");

    assert_eq!(
        channels["channels"]["stable"]["manifests"][0]["version"].as_str(),
        Some("1.0.2")
    );
    assert_eq!(manifest["version"].as_str(), Some("1.0.2"));
    assert_eq!(manifest["packages"][0]["version"].as_str(), Some("1.0.0"));
    assert_eq!(
        manifest["profiles"]["code"]["architectures"][0]["image_revision"].as_str(),
        Some("2030.0101.1")
    );
}

#[test]
fn asset_attestation_predicate_uses_published_obom_url_shape() {
    let files = vec![
        AssetsChannelAssetFile {
            arch: "arm64".to_string(),
            logical_name: "initrd.img".to_string(),
            url: "/assets/releases/2030.0101.1/arm64-initrd.img".to_string(),
            hash: "1".repeat(64),
            size: 1,
        },
        AssetsChannelAssetFile {
            arch: "arm64".to_string(),
            logical_name: "arm64-obom.cdx.json".to_string(),
            url: "/assets/releases/2030.0101.1/arm64-obom.cdx.json".to_string(),
            hash: "2".repeat(64),
            size: 1,
        },
    ];

    let attestations = current_asset_attestations(&files);

    assert_eq!(attestations.len(), 1);
    assert_eq!(
        attestations[0].predicate_url.as_deref(),
        Some("/assets/releases/2030.0101.1/arm64-obom.cdx.json")
    );
}

#[test]
fn assets_channel_build_preserves_existing_channels_when_adding_nightly() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let manifest_url = file_url(&manifest_path);
    let assets_dir = temp.path().join("assets");
    let profiles_dir = repo_config_profiles_dir();
    let out_dir = temp.path().join("cache/target/distribution");

    build_assets_channel(
        &manifest_url,
        &assets_dir,
        &profiles_dir,
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("stable channel builds");
    let stable_channels: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("channels.json")).unwrap())
            .expect("stable channels json");
    let stable_manifest_url = stable_channels["channels"]["stable"]["manifests"][0]["url"]
        .as_str()
        .expect("stable manifest url")
        .to_string();

    build_assets_channel(
        &manifest_url,
        &assets_dir,
        &profiles_dir,
        "nightly",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect("nightly channel builds without erasing stable");

    let channels: serde_json::Value = serde_json::from_str(&fs::read_to_string(out_dir.join("channels.json")).unwrap())
        .expect("merged channels json");
    let channel_ids = channels["channels"]
        .as_object()
        .expect("channels object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(channel_ids, vec!["nightly".to_string(), "stable".to_string()]);
    assert_eq!(
        channels["channels"]["stable"]["manifests"][0]["url"].as_str(),
        Some(stable_manifest_url.as_str())
    );
    assert!(out_dir.join(stable_manifest_url.trim_start_matches('/')).is_file());
    assert!(out_dir.join("assets/stable/manifest.json").is_file());
    assert!(out_dir.join("assets/nightly/manifest.json").is_file());
    let nightly_manifest_url = channels["channels"]["nightly"]["manifests"][0]["url"]
        .as_str()
        .expect("nightly manifest url");
    assert!(out_dir.join(nightly_manifest_url.trim_start_matches('/')).is_file());

    check_assets_channel(&out_dir, "stable").expect("merged stable channel checks");
    fs::remove_file(out_dir.join("index.html")).expect("remove stable test index fixture");
    check_assets_channel(&out_dir, "nightly").expect("merged nightly channel checks");
}

#[test]
fn assets_channel_build_bootstraps_without_binary_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest")).expect("manifest json");
    manifest["binaries"]["releases"]["1.0.0"]
        .as_object_mut()
        .expect("binary release")
        .remove("files");
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
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
    .expect("first asset channel builds before binary evidence exists");

    let health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("health.json")).unwrap()).expect("health json parses");
    assert_eq!(health["evidence"]["host_binary_files"], serde_json::json!([]));
    assert_eq!(health["evidence"]["host_sboms"], serde_json::json!([]));
    assert!(health["evidence"]["attestations"]
        .as_array()
        .expect("attestations")
        .iter()
        .any(|item| item["name"] == "github_attestations_vm_assets"));

    check_assets_channel(&out_dir, "stable").expect("first asset channel checks before binary evidence exists");
}

#[test]
fn assets_channel_headers_split_mutable_and_immutable_paths() {
    let headers = render_assets_channel_headers("stable");

    assert!(headers.contains("/\n  Cache-Control: no-cache, must-revalidate"));
    assert!(headers.contains("/index.html\n  Cache-Control: no-cache, must-revalidate"));
    assert!(headers.contains("/404\n  Cache-Control: no-cache, must-revalidate"));
    assert!(headers.contains("/404.html\n  Cache-Control: no-cache, must-revalidate"));
    assert!(headers.contains("/health.json\n  Cache-Control: no-cache, must-revalidate"));
    assert!(headers.contains("/assets/stable/*\n  Cache-Control: no-cache, must-revalidate"));
    assert!(!headers.contains("/profiles/stable/*\n  Cache-Control: no-cache"));
    assert!(headers.contains("/assets/releases/*\n  Cache-Control: public, max-age=31536000, immutable"));
    assert!(headers.contains("/profiles/releases/*\n  Cache-Control: public, max-age=31536000, immutable"));
    assert!(!headers.contains("/assets/*\n  Cache-Control: no-cache"));
    assert!(!headers.contains("/profiles/*\n  Cache-Control: no-cache"));
}

#[test]
fn host_spdx_requires_sha256_file_checksums() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sbom_path = temp.path().join("capsem-sbom.spdx.json");
    let sha1_only = br#"{
          "spdxVersion": "SPDX-2.3",
          "files": [
            {
              "SPDXID": "SPDXRef-File-capsem-gateway",
              "checksums": [
                {
                  "algorithm": "SHA1",
                  "checksumValue": "2a2bebeee60f894f3599e06c755c91944f1c3cc8"
                }
              ]
            }
          ]
        }"#;

    let error =
        validate_host_spdx_sbom_bytes(sha1_only, &sbom_path).expect_err("SHA1-only SPDX file checksums rejected");

    assert!(format!("{error:#}").contains("missing SHA256 checksum"), "{error:#}");
}

#[test]
fn host_spdx_accepts_sha256_file_checksums() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sbom_path = temp.path().join("capsem-sbom.spdx.json");
    let with_sha256 = br#"{
          "spdxVersion": "SPDX-2.3",
          "files": [
            {
              "SPDXID": "SPDXRef-File-capsem-gateway",
              "checksums": [
                {
                  "algorithm": "SHA256",
                  "checksumValue": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                }
              ]
            }
          ]
        }"#;

    validate_host_spdx_sbom_bytes(with_sha256, &sbom_path).expect("SPDX file with SHA256 checksum validates");
}

#[test]
fn assets_channel_record_binary_rejects_legacy_manifest_without_package_provenance() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let pkg_path = artifacts_dir.join("Capsem-1.4.1234567890.pkg");
    let deb_path = artifacts_dir.join("Capsem_1.4.1234567890_arm64.deb");
    let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
    write_minimal_pkg_with_file(
        &pkg_path,
        "Applications/Capsem.app/Contents/MacOS/capsem-app",
        b"pkg executable bytes",
    );
    write_minimal_deb_with_file(
        &deb_path,
        "usr/bin/capsem-app",
        b"deb executable bytes",
        release_graph::PackageArchitecture::Arm64,
    );
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let error = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        &source_commit(),
        None,
        &[pkg_path, deb_path, sbom_path],
        "2030-02-03",
    )
    .expect_err("legacy manifests cannot record per-package source provenance");

    assert!(format!("{error:#}").contains("requires a release graph manifest"));
}

#[test]
fn assets_channel_record_binary_updates_graph_manifest_without_changing_profiles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_release_graph_manifest(temp.path());
    let original: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest")).expect("json");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let pkg_path = artifacts_dir.join("Capsem-1.4.1234567890.pkg");
    let deb_path = artifacts_dir.join("Capsem_1.4.1234567890_amd64.deb");
    let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
    write_minimal_pkg_with_file(
        &pkg_path,
        "Applications/Capsem.app/Contents/MacOS/capsem-app",
        b"pkg executable bytes",
    );
    write_minimal_deb_with_file(
        &deb_path,
        "usr/bin/capsem-app",
        b"deb executable bytes",
        release_graph::PackageArchitecture::Amd64,
    );
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let report = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        &source_commit(),
        None,
        &[pkg_path.clone(), deb_path, sbom_path],
        "2030-02-03",
    )
    .expect("record graph binary release");

    assert_eq!(report.version, "1.4.1234567890");
    assert_eq!(report.min_assets, "2030.0101.1");
    let updated: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest")).expect("json");
    assert_eq!(updated["profiles"], original["profiles"]);
    assert!(updated.get("assets").is_none());
    assert!(updated.get("binaries").is_none());
    assert_eq!(updated["packages"].as_array().expect("packages").len(), 2);
    assert_eq!(updated["packages"][0]["name"], "Capsem-1.4.1234567890.pkg");
    assert_eq!(updated["packages"][0]["version"], "1.4.1234567890");
    assert_eq!(updated["packages"][0]["source_commit"], source_commit().as_str());
    assert_eq!(updated["packages"][1]["source_commit"], source_commit().as_str());
    assert!(updated.get("source_commit").is_none());
    assert!(updated["packages"][0]["binaries"][0].get("source_commit").is_none());
    assert_eq!(updated["packages"][0]["status"], "current");
    assert_eq!(updated["packages"][0]["platform"], "macos");
    assert_eq!(updated["packages"][0]["architecture"], "arm64");
    assert_eq!(
        updated["packages"][0]["digest"]["sha256"],
        format!("{:x}", Sha256::digest(fs::read(&pkg_path).expect("pkg bytes")))
    );
    assert_eq!(
        updated["packages"][0]["binaries"][0]["installed_path"].as_str(),
        Some("/Applications/Capsem.app/Contents/MacOS/capsem-app")
    );
    assert_eq!(
        updated["packages"][0]["evidence"][0]["name"].as_str(),
        Some("capsem-sbom.spdx.json")
    );
    assert_eq!(
        updated["packages"][0]["evidence"][0]["url"].as_str(),
        Some("https://github.com/google/capsem/releases/download/v1.4.1234567890/capsem-sbom.spdx.json")
    );
    assert_eq!(
        updated["packages"][1]["binaries"][0]["installed_path"].as_str(),
        Some("/usr/bin/capsem-app")
    );
    assert_eq!(updated["packages"][1]["name"], "Capsem_1.4.1234567890_amd64.deb");
    assert_eq!(updated["packages"][1]["architecture"], "amd64");
}

#[test]
fn release_graph_health_uses_profile_obom_as_vm_attestation_predicate() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_release_graph_manifest(temp.path());
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest")).expect("json");
    let predicate_url = "/profiles/releases/stable/co-work/2030.0101.1/arm64/obom.cdx.json";
    manifest["profiles"]["co-work"]["architectures"][0]["evidence"] = serde_json::json!([
        {
            "kind": "obom",
            "url": predicate_url,
            "bytes": 811,
            "digest": {
                "sha256": "8888888888888888888888888888888888888888888888888888888888888888",
                "blake3": "9999999999999999999999999999999999999999999999999999999999999999",
            },
            "status": "current",
        }
    ]);
    let dist = temp.path().join("dist");

    build_assets_channel_from_graph(manifest, "stable", "1.0.2", &dist, "2030-02-03T04:05:06Z")
        .expect("build graph distribution");

    let health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dist.join("health.json")).expect("health")).expect("health json");
    let attestation = health["evidence"]["attestations"]
        .as_array()
        .expect("attestations")
        .iter()
        .find(|item| item["name"] == "github_attestations_vm_assets")
        .expect("VM asset attestation");
    assert_eq!(attestation["predicate_url"], predicate_url);
    assert_eq!(health["evidence"]["vm_oboms"][0]["url"], predicate_url);
}

#[test]
fn staged_profile_then_binary_activation_enforces_bounds_without_rebuilding_profile() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_release_graph_manifest(temp.path());
    let mut staged: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest")).expect("json");
    staged["profiles"]["co-work"]["version"] = serde_json::Value::String("2030.0203.1".to_string());
    staged["profiles"]["co-work"]["revision"] = serde_json::Value::String("2030.0203.1".to_string());
    staged["profiles"]["co-work"]["min_capsem_version"] = serde_json::Value::String("1.4.1234567890".to_string());
    staged["profiles"]["co-work"]["max_capsem_version"] = serde_json::Value::String("1.4.1234567890".to_string());
    fs::write(
        &manifest_path,
        format!("{}\n", serde_json::to_string_pretty(&staged).expect("staged manifest")),
    )
    .expect("write staged manifest");
    let staged_bytes = fs::read(&manifest_path).expect("staged bytes");
    let staged_profile = staged["profiles"]["co-work"].clone();

    assert!(
        !graph_profile_matches_current_binary(&staged_profile, &staged).expect("old binary compatibility"),
        "the staged profile must remain private while the public binary is too old"
    );
    let error = build_assets_channel_from_graph(
        staged.clone(),
        "stable",
        "1.0.2",
        &temp.path().join("incompatible-dist"),
        "2030-02-03T04:05:06Z",
    )
    .expect_err("incompatible staged source cannot become a public distribution");
    assert!(
        format!("{error:#}").contains("co-work"),
        "the rejection must identify the incompatible profile: {error:#}"
    );

    let too_new_dir = temp.path().join("too-new-binary");
    fs::create_dir_all(&too_new_dir).expect("too-new artifact dir");
    let too_new_pkg = too_new_dir.join("Capsem-2.0.0.pkg");
    let too_new_deb = too_new_dir.join("Capsem_2.0.0_amd64.deb");
    let too_new_sbom = too_new_dir.join("capsem-sbom.spdx.json");
    write_minimal_pkg_with_file(
        &too_new_pkg,
        "Applications/Capsem.app/Contents/MacOS/capsem-app",
        b"too new pkg executable bytes",
    );
    write_minimal_deb_with_file(
        &too_new_deb,
        "usr/bin/capsem-app",
        b"too new deb executable bytes",
        release_graph::PackageArchitecture::Amd64,
    );
    fs::write(&too_new_sbom, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("too-new SBOM");
    let error = record_binary_release_metadata(
        &manifest_path,
        "2.0.0",
        &source_commit(),
        None,
        &[too_new_pkg, too_new_deb, too_new_sbom],
        "2030-02-03",
    )
    .expect_err("binary newer than the staged profile maximum must be rejected");
    assert!(
        format!("{error:#}").contains("co-work"),
        "the rejection must identify the incompatible profile: {error:#}"
    );
    assert_eq!(
        fs::read(&manifest_path).expect("manifest after rejected binary"),
        staged_bytes,
        "a rejected binary must not mutate the staged source manifest"
    );

    let compatible_dir = temp.path().join("compatible-binary");
    fs::create_dir_all(&compatible_dir).expect("compatible artifact dir");
    let compatible_pkg = compatible_dir.join("Capsem-1.4.1234567890.pkg");
    let compatible_deb = compatible_dir.join("Capsem_1.4.1234567890_amd64.deb");
    let compatible_sbom = compatible_dir.join("capsem-sbom.spdx.json");
    write_minimal_pkg_with_file(
        &compatible_pkg,
        "Applications/Capsem.app/Contents/MacOS/capsem-app",
        b"compatible pkg executable bytes",
    );
    write_minimal_deb_with_file(
        &compatible_deb,
        "usr/bin/capsem-app",
        b"compatible deb executable bytes",
        release_graph::PackageArchitecture::Amd64,
    );
    fs::write(&compatible_sbom, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("compatible SBOM");
    record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        &source_commit(),
        None,
        &[compatible_pkg, compatible_deb, compatible_sbom],
        "2030-02-03",
    )
    .expect("compatible binary activates staged profile");

    let activated: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("activated manifest")).expect("activated json");
    assert_eq!(
        activated["profiles"]["co-work"], staged_profile,
        "binary activation must reuse the exact staged profile instead of rebuilding it"
    );
    assert!(
        graph_profile_matches_current_binary(&activated["profiles"]["co-work"], &activated)
            .expect("activated compatibility")
    );
    build_assets_channel_from_graph(
        activated,
        "stable",
        "1.0.2",
        &temp.path().join("activated-dist"),
        "2030-02-03T04:05:06Z",
    )
    .expect("compatible staged profile and binary can become public");
}

#[test]
fn assets_channel_record_binary_rejects_sbom_without_host_package() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let error = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        &source_commit(),
        None,
        &[sbom_path],
        "2030-02-03",
    )
    .expect_err("SBOM-only binary metadata rejected");

    assert!(
        format!("{error:#}").contains("binary release metadata must include a host package artifact"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_record_binary_rejects_non_package_host_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let readme_path = artifacts_dir.join("release-notes.txt");
    let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
    fs::write(&readme_path, b"not an installable package").expect("readme");
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let error = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        &source_commit(),
        None,
        &[readme_path, sbom_path],
        "2030-02-03",
    )
    .expect_err("non-package host artifact rejected");

    assert!(
        format!("{error:#}").contains("binary release metadata must include a .pkg or .deb artifact"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_record_binary_rejects_empty_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let pkg_path = artifacts_dir.join("Capsem-1.4.1234567890.pkg");
    let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
    fs::write(&pkg_path, []).expect("empty pkg");
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let error = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        &source_commit(),
        None,
        &[pkg_path, sbom_path],
        "2030-02-03",
    )
    .expect_err("empty binary artifact rejected");

    assert!(
        format!("{error:#}").contains("binary release artifact is empty"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_record_binary_rejects_package_version_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let pkg_path = artifacts_dir.join("Capsem-1.4.0000000000.pkg");
    let sbom_path = artifacts_dir.join("capsem-sbom.spdx.json");
    write_minimal_pkg_with_file(
        &pkg_path,
        "Applications/Capsem.app/Contents/MacOS/capsem-app",
        b"pkg executable bytes",
    );
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let error = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        &source_commit(),
        None,
        &[pkg_path, sbom_path],
        "2030-02-03",
    )
    .expect_err("mismatched package version rejected");

    assert!(
        format!("{error:#}").contains("binary release package artifact name must match version"),
        "{error:#}"
    );
}

#[test]
fn assets_channel_record_binary_rejects_noncanonical_sbom_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let artifacts_dir = temp.path().join("release-artifacts");
    fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
    let pkg_path = artifacts_dir.join("Capsem-1.4.1234567890.pkg");
    let sbom_path = artifacts_dir.join("host-sbom.spdx.json");
    write_minimal_pkg_with_file(
        &pkg_path,
        "Applications/Capsem.app/Contents/MacOS/capsem-app",
        b"pkg executable bytes",
    );
    fs::write(&sbom_path, br#"{"spdxVersion":"SPDX-2.3"}"#).expect("sbom");

    let error = record_binary_release_metadata(
        &manifest_path,
        "1.4.1234567890",
        &source_commit(),
        None,
        &[pkg_path, sbom_path],
        "2030-02-03",
    )
    .expect_err("noncanonical SBOM artifact rejected");

    assert!(format!("{error:#}").contains("capsem-sbom.spdx.json"), "{error:#}");
}
