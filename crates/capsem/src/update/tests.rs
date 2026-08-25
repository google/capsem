use super::*;

#[test]
fn preverified_package_manifest_defers_asset_hydration_to_the_service() {
    let payload = b"{}".to_vec();
    let package_handoff = ExplicitManifestInput {
        source: "file:///tmp/manifest.json",
        payload: Some(payload),
    };
    let ordinary_refresh = ExplicitManifestInput {
        source: "file:///tmp/manifest.json",
        payload: None,
    };

    assert!(!should_hydrate_assets(Some(&package_handoff)));
    assert!(should_hydrate_assets(Some(&ordinary_refresh)));
    assert!(should_hydrate_assets(None));
}

#[test]
fn is_newer_semver() {
    assert!(is_newer("0.17.0", "0.16.1"));
    assert!(is_newer("1.0.0", "0.99.99"));
    assert!(!is_newer("0.16.1", "0.16.1"));
    assert!(!is_newer("0.16.0", "0.16.1"));
}

#[test]
fn is_newer_rejects_garbage() {
    assert!(!is_newer("error", "0.16.1"));
    assert!(!is_newer("", "0.16.1"));
    assert!(!is_newer("not-a-version", "0.16.1"));
}

#[test]
fn is_newer_rejects_malformed_current() {
    assert!(!is_newer("0.17.0", "garbage"));
}

#[test]
fn is_newer_prerelease() {
    assert!(!is_newer("0.17.0-beta.1", "0.17.0"));
    assert!(is_newer("0.18.0-beta.1", "0.17.0"));
}

#[test]
fn update_check_roundtrip() {
    let check = UpdateCheck {
        checked_at: 1718444400,
        latest_version: Some("0.17.0".into()),
        update_available: true,
        binary_installer: Some(BinaryInstaller {
            name: "Capsem-0.17.0.pkg".into(),
            url: "https://github.com/google/capsem/releases/download/v0.17.0/Capsem-0.17.0.pkg"
                .into(),
            sha256: "abc123".into(),
            blake3: "def456".into(),
            size: 123,
            install_layout: "macos_pkg".into(),
        }),
        latest_assets: Some("2030.0101.1".into()),
        current_assets: Some("2030.0101.0".into()),
        assets_update_available: true,
        assets_state: Some("published".into()),
        assets_blocked_reason: None,
        latest_profiles: Some("profiles-2030.0101.1".into()),
        current_profiles: Some("profiles-2030.0101.0".into()),
        profiles_update_available: false,
        profiles_state: Some("published".into()),
        profiles_blocked_reason: Some("requires binary 1.4.0 or newer".into()),
        profile_catalog_source: Some(
            "/profiles/releases/profiles-2030.0101.1/catalog.json".into(),
        ),
        profile_catalog_hash: Some("b".repeat(64)),
        latest_images: None,
        images_update_available: false,
        images_state: Some("not_published".into()),
        images_blocked_reason: None,
        source: Some("https://release.capsem.org/assets/stable/manifest.json".into()),
        channel_hash: Some("a".repeat(64)),
        validation_status: Some("valid".into()),
        validation_error: None,
    };
    let json = serde_json::to_string(&check).unwrap();
    let rt: UpdateCheck = serde_json::from_str(&json).unwrap();
    assert_eq!(rt.latest_version, Some("0.17.0".into()));
    assert!(rt.update_available);
    assert_eq!(
        rt.binary_installer
            .as_ref()
            .map(|installer| installer.name.as_str()),
        Some("Capsem-0.17.0.pkg")
    );
    assert_eq!(rt.latest_assets, Some("2030.0101.1".into()));
    assert_eq!(rt.current_assets, Some("2030.0101.0".into()));
    assert!(rt.assets_update_available);
    assert_eq!(rt.assets_state, Some("published".into()));
    assert_eq!(rt.assets_blocked_reason, None);
    assert_eq!(rt.latest_profiles, Some("profiles-2030.0101.1".into()));
    assert_eq!(rt.current_profiles, Some("profiles-2030.0101.0".into()));
    assert!(!rt.profiles_update_available);
    assert_eq!(rt.profiles_state, Some("published".into()));
    assert_eq!(
        rt.profiles_blocked_reason,
        Some("requires binary 1.4.0 or newer".into())
    );
    assert_eq!(
        rt.profile_catalog_source,
        Some("/profiles/releases/profiles-2030.0101.1/catalog.json".into())
    );
    assert_eq!(rt.profile_catalog_hash, Some("b".repeat(64)));
    assert_eq!(rt.latest_images, None);
    assert!(!rt.images_update_available);
    assert_eq!(rt.images_state, Some("not_published".into()));
    assert_eq!(rt.images_blocked_reason, None);
    assert_eq!(
        rt.source,
        Some("https://release.capsem.org/assets/stable/manifest.json".into())
    );
    assert_eq!(rt.channel_hash, Some("a".repeat(64)));
    assert_eq!(rt.validation_status, Some("valid".into()));
    assert_eq!(rt.validation_error, None);
}

#[test]
fn update_check_old_cache_shape_defaults_new_release_channel_fields() {
    let rt: UpdateCheck = serde_json::from_str(
        r#"{"checked_at":1718444400,"latest_version":"0.17.0","update_available":true}"#,
    )
    .unwrap();

    assert_eq!(rt.latest_version, Some("0.17.0".into()));
    assert!(rt.update_available);
    assert_eq!(rt.binary_installer, None);
    assert_eq!(rt.latest_assets, None);
    assert_eq!(rt.current_assets, None);
    assert!(!rt.assets_update_available);
    assert_eq!(rt.assets_state, None);
    assert_eq!(rt.assets_blocked_reason, None);
    assert_eq!(rt.latest_profiles, None);
    assert_eq!(rt.current_profiles, None);
    assert!(!rt.profiles_update_available);
    assert_eq!(rt.profiles_state, None);
    assert_eq!(rt.profiles_blocked_reason, None);
    assert_eq!(rt.profile_catalog_source, None);
    assert_eq!(rt.profile_catalog_hash, None);
    assert_eq!(rt.latest_images, None);
    assert!(!rt.images_update_available);
    assert_eq!(rt.images_state, None);
    assert_eq!(rt.images_blocked_reason, None);
    assert_eq!(rt.source, None);
    assert_eq!(rt.channel_hash, None);
    assert_eq!(rt.validation_status, None);
    assert_eq!(rt.validation_error, None);
}

#[test]
fn cached_update_notice_reports_asset_only_updates() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
    let mut check = cached_notice_check();
    check.latest_assets = Some("2030.0101.1".into());
    check.current_assets = Some("2026.0627.1".into());
    check.assets_update_available = true;
    seed_manifest_metadata(&check);
    write_cache(&check).unwrap();

    assert_eq!(
        read_cached_update_notice().as_deref(),
        Some(
            "VM asset update available: 2030.0101.1. The installed service will apply it automatically."
        )
    );
}

#[test]
fn cached_update_notice_reports_profile_catalog_updates() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
    let mut check = cached_notice_check();
    check.latest_profiles = Some("profiles-2030.0101.1".into());
    check.current_profiles = Some("profiles-2030.0101.0".into());
    check.profiles_update_available = true;
    seed_manifest_metadata(&check);
    write_cache(&check).unwrap();

    assert_eq!(
        read_cached_update_notice().as_deref(),
        Some(
            "Profile catalog update available: profiles-2030.0101.1. The installed service will apply it automatically."
        )
    );
}

#[test]
fn cached_update_notice_reports_blocked_profile_catalog_updates() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
    let mut check = cached_notice_check();
    check.latest_profiles = Some("profiles-2030.0101.1".into());
    check.current_profiles = Some("profiles-2030.0101.0".into());
    check.profiles_blocked_reason = Some("requires binary 1.4.1 or newer".into());
    seed_manifest_metadata(&check);
    write_cache(&check).unwrap();

    assert_eq!(
        read_cached_update_notice().as_deref(),
        Some(
            "Profile catalog update blocked: requires binary 1.4.1 or newer. Run `capsem update --check` for details."
        )
    );
}
#[test]
fn update_check_merges_into_single_manifest_metadata_file() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
    let assets = home.path().join("assets");
    std::fs::create_dir_all(&assets).unwrap();
    let path = assets.join("manifest-metadata.json");
    assert_eq!(manifest_metadata_path().as_deref(), Some(path.as_path()));
    std::fs::write(
        &path,
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
            "installed_at": 100,
            "package_version": "1.5.0"
        })
        .to_string(),
    )
    .unwrap();

    let check = cached_notice_check();
    write_cache(&check).unwrap();

    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(metadata["schema"], "capsem.manifest_metadata.v1");
    assert_eq!(metadata["origin"], "package");
    assert_eq!(metadata["installed_at"], 100);
    assert_eq!(metadata["package_version"], "1.5.0");
    assert_eq!(
        metadata["checked_url"],
        check.source.unwrap(),
        "metadata={metadata}"
    );
    assert_eq!(metadata["checked_at"], check.checked_at);
}

#[test]
fn single_manifest_metadata_records_only_the_latest_channel_check() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");

    let mut stable = cached_notice_check();
    stable.source = Some("https://release.capsem.org/assets/stable/manifest.json".into());
    write_cache(&stable).unwrap();
    let mut nightly = cached_notice_check();
    nightly.source = Some("https://release.capsem.org/assets/nightly/manifest.json".into());
    write_cache(&nightly).unwrap();

    assert!(read_cache_for_source(stable.source.as_deref().unwrap()).is_err());
    assert_eq!(
        read_cache_for_source(nightly.source.as_deref().unwrap())
            .unwrap()
            .source,
        nightly.source
    );
}

#[test]
fn stable_to_nightly_manifest_switch_resolves_nightly_updates() {
    let stable_source = "https://release.capsem.org/assets/stable/manifest.json";
    let nightly_source = "https://release.capsem.org/assets/nightly/manifest.json";
    let stable = test_manifest("1.4.0", "2026.0627.8", "1.4.0", "2026.0627.8");
    let nightly = test_manifest(
        "1.5.0-nightly.20260702",
        "2026.0702.1-nightly",
        "1.4.0",
        "2026.0627.8",
    );

    let stable_check = update_check_from_release_manifest(
        &stable,
        100,
        "1.4.0",
        Some("2026.0627.8"),
        None,
        &InstallLayout::MacosPkg,
        stable_source,
        Some("stable-channel-hash".into()),
    )
    .expect("stable check");
    let nightly_check = update_check_from_release_manifest(
        &nightly,
        200,
        "1.4.0",
        Some("2026.0627.8"),
        None,
        &InstallLayout::MacosPkg,
        nightly_source,
        Some("nightly-channel-hash".into()),
    )
    .expect("nightly check");

    assert_eq!(stable_check.source.as_deref(), Some(stable_source));
    assert_eq!(stable_check.latest_version.as_deref(), Some("1.4.0"));
    assert!(!stable_check.update_available);
    assert!(!stable_check.assets_update_available);
    assert_eq!(
        stable_check.channel_hash.as_deref(),
        Some("stable-channel-hash")
    );

    assert_eq!(nightly_check.source.as_deref(), Some(nightly_source));
    assert_eq!(
        nightly_check.latest_version.as_deref(),
        Some("1.5.0-nightly.20260702")
    );
    assert!(nightly_check.update_available);
    assert_eq!(
        nightly_check.latest_assets.as_deref(),
        Some("2026.0702.1-nightly")
    );
    assert!(nightly_check.assets_update_available);
    assert_eq!(
        nightly_check.channel_hash.as_deref(),
        Some("nightly-channel-hash")
    );
    assert_eq!(
        nightly_check
            .binary_installer
            .as_ref()
            .map(|installer| installer.name.as_str()),
        Some("Capsem-1.5.0-nightly.20260702.pkg")
    );
}

fn test_manifest(
    binary_version: &str,
    asset_version: &str,
    min_binary: &str,
    min_assets: &str,
) -> capsem_core::asset_manager::ManifestV2 {
    capsem_core::asset_manager::ManifestV2::from_json(&format!(
        r#"{{
                "format": 2,
                "refresh_policy": "24h",
                "asset_base": "https://github.com/google/capsem/releases/download/v{binary_version}/",
                "assets": {{
                    "current": "{asset_version}",
                    "releases": {{
                        "{asset_version}": {{
                            "date": "2026-07-02",
                            "deprecated": false,
                            "min_binary": "{min_binary}",
                            "arches": {{}}
                        }}
                    }}
                }},
                "binaries": {{
                    "current": "{binary_version}",
                    "releases": {{
                        "{binary_version}": {{
                            "date": "2026-07-02",
                            "deprecated": false,
                            "min_assets": "{min_assets}",
                            "version": "{binary_version}",
                            "files": [
                                {{
                                    "name": "Capsem-{binary_version}.pkg",
                                    "size": 42,
                                    "sha256": "{}",
                                    "blake3": "{}"
                                }}
                            ]
                        }}
                    }}
                }}
            }}"#,
        "a".repeat(64),
        "b".repeat(64)
    ))
    .expect("test manifest")
}

fn cached_notice_check() -> UpdateCheck {
    UpdateCheck {
        checked_at: now_secs(),
        latest_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        update_available: false,
        binary_installer: None,
        latest_assets: Some("2026.0627.1".into()),
        current_assets: Some("2026.0627.1".into()),
        assets_update_available: false,
        assets_state: Some("published".into()),
        assets_blocked_reason: None,
        latest_profiles: Some("profiles-2030.0101.1".into()),
        current_profiles: Some("profiles-2030.0101.0".into()),
        profiles_update_available: false,
        profiles_state: Some("published".into()),
        profiles_blocked_reason: Some("requires binary 1.4.1 or newer".into()),
        profile_catalog_source: Some(
            "/profiles/releases/profiles-2030.0101.1/catalog.json".into(),
        ),
        profile_catalog_hash: Some("b".repeat(64)),
        latest_images: None,
        images_update_available: false,
        images_state: Some("not_published".into()),
        images_blocked_reason: None,
        source: Some("https://release.capsem.org/assets/stable/manifest.json".into()),
        channel_hash: Some("a".repeat(64)),
        validation_status: Some("valid".into()),
        validation_error: None,
    }
}

fn seed_manifest_metadata(check: &UpdateCheck) {
    let path = manifest_metadata_path().expect("manifest metadata path");
    std::fs::create_dir_all(path.parent().expect("metadata parent")).unwrap();
    std::fs::write(
        path,
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "manifest_url": check.source.as_deref().expect("check source"),
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn update_channel_provenance_preserves_previous_cache_on_failure() {
    let previous = UpdateCheck {
        checked_at: 1000,
        latest_version: Some("99.99.99".into()),
        update_available: true,
        binary_installer: None,
        latest_assets: Some("2030.0101.1".into()),
        current_assets: Some("2030.0101.0".into()),
        assets_update_available: true,
        assets_state: Some("published".into()),
        assets_blocked_reason: None,
        latest_profiles: None,
        current_profiles: None,
        profiles_update_available: false,
        profiles_state: None,
        profiles_blocked_reason: None,
        profile_catalog_source: None,
        profile_catalog_hash: None,
        latest_images: None,
        images_update_available: false,
        images_state: None,
        images_blocked_reason: None,
        source: Some("https://release.capsem.org/assets/stable/manifest.json".into()),
        channel_hash: Some("f".repeat(64)),
        validation_status: Some("valid".into()),
        validation_error: None,
    };

    let check = failed_update_check_from_previous(
        Some(previous),
        1200,
        "https://release.capsem.org/assets/stable/manifest.json",
        "fetch_error",
        "connection refused".to_string(),
    );

    assert_eq!(check.checked_at, 1200);
    assert_eq!(check.latest_version, Some("99.99.99".into()));
    assert_eq!(check.latest_assets, Some("2030.0101.1".into()));
    assert_eq!(check.current_assets, Some("2030.0101.0".into()));
    assert_eq!(check.channel_hash, Some("f".repeat(64)));
    assert_eq!(check.validation_status, Some("fetch_error".into()));
    assert_eq!(check.validation_error, Some("connection refused".into()));
}

#[test]
fn cache_ttl_constant() {
    assert_eq!(CACHE_TTL_SECS, 86400);
}

#[test]
fn update_does_not_fetch_health_for_manifest_url() {
    assert_eq!(
        channel_manifest_url("https://release.capsem.org/assets/stable/manifest.json").unwrap(),
        "https://release.capsem.org/assets/stable/manifest.json"
    );
    assert_eq!(
        channel_manifest_url("https://corp.example/capsem/assets/internal/manifest.json").unwrap(),
        "https://corp.example/capsem/assets/internal/manifest.json"
    );
    assert!(channel_manifest_url("file:///tmp/assets/stable/manifest.json").is_err());
}

#[test]
fn installed_update_source_requires_manifest_metadata() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
    let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
    let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");

    let error = release_manifest_url_for_layout(&InstallLayout::UserDir)
        .expect_err("installed Capsem must not silently select stable");

    assert!(
        format!("{error:#}").contains("manifest-metadata.json"),
        "{error:#}"
    );
}

#[test]
fn installed_update_source_rejects_malformed_manifest_metadata() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
    let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
    let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");
    let assets = home.path().join("assets");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::write(assets.join("manifest-metadata.json"), b"not json\n").unwrap();

    let error = release_manifest_url_for_layout(&InstallLayout::MacosPkg)
        .expect_err("malformed installed metadata must fail closed");

    assert!(format!("{error:#}").contains("parse"), "{error:#}");
}

#[test]
fn installed_update_source_requires_manifest_url_field() {
    let _lock = crate::lock_test_env();
    let assets = tempfile::tempdir().unwrap();
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", assets.path().to_str().unwrap());
    let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
    let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");
    std::fs::write(
        assets.path().join("manifest-metadata.json"),
        br#"{"schema":"capsem.manifest_metadata.v1"}"#,
    )
    .unwrap();

    let error = release_manifest_url_for_layout(&InstallLayout::LinuxDeb)
        .expect_err("installed metadata without manifest_url must fail closed");

    assert!(format!("{error:#}").contains("manifest_url"), "{error:#}");
}

#[test]
fn installed_update_source_rejects_wrong_metadata_schema() {
    let _lock = crate::lock_test_env();
    let assets = tempfile::tempdir().unwrap();
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", assets.path().to_str().unwrap());
    let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
    let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");
    std::fs::write(
        assets.path().join("manifest-metadata.json"),
        br#"{"schema":"capsem.wrong.v1","manifest_url":"https://release.capsem.org/assets/nightly/manifest.json"}"#,
    )
    .unwrap();

    let error = release_manifest_url_for_layout(&InstallLayout::MacosPkg)
        .expect_err("wrong metadata schema must fail closed");

    assert!(
        format!("{error:#}").contains("capsem.manifest_metadata.v1"),
        "{error:#}"
    );
}

#[test]
fn installed_update_source_does_not_replace_file_manifest_with_stable() {
    let _lock = crate::lock_test_env();
    let assets = tempfile::tempdir().unwrap();
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", assets.path().to_str().unwrap());
    let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
    let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");
    std::fs::write(
        assets.path().join("manifest-metadata.json"),
        br#"{"schema":"capsem.manifest_metadata.v1","manifest_url":"file:///tmp/release/assets/nightly/manifest.json"}"#,
    )
    .unwrap();

    let error = release_manifest_url_for_layout(&InstallLayout::UserDir)
        .expect_err("local manifest provenance must not silently become stable");

    let message = format!("{error:#}");
    // This asserted on the literal string "http(s)", which was the wording of
    // a message that blamed the scheme of URLs whose scheme was fine. What it
    // is actually about is that a `file://` provenance must fail loudly
    // rather than quietly become the public stable channel.
    assert!(message.contains("file"), "the rejection must name the scheme: {message}");
    assert!(!message.contains(DEFAULT_RELEASE_MANIFEST_URL), "{message}");
}

#[test]
fn installed_update_source_uses_exact_metadata_url_and_assets_override() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", assets.path().to_str().unwrap());
    let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
    let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");
    let metadata_path = assets.path().join("manifest-metadata.json");
    let nightly = "https://release.capsem.org/assets/nightly/manifest.json";
    std::fs::write(
        &metadata_path,
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "manifest_url": nightly,
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(
        manifest_metadata_path().as_deref(),
        Some(metadata_path.as_path())
    );
    assert_eq!(
        release_manifest_url_for_layout(&InstallLayout::MacosPkg).unwrap(),
        nightly
    );
    assert!(!home.path().join("assets/manifest-metadata.json").exists());
}

#[test]
fn installed_update_source_rejects_environment_channel_bypass() {
    let _lock = crate::lock_test_env();
    let assets = tempfile::tempdir().unwrap();
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", assets.path().to_str().unwrap());
    let _manifest_override = EnvGuard::set(
        RELEASE_MANIFEST_URL_ENV,
        "https://release.capsem.org/assets/stable/manifest.json",
    );
    let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");
    std::fs::write(
        assets.path().join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "manifest_url": "https://corp.example/capsem/assets/internal/manifest.json",
        })
        .to_string(),
    )
    .unwrap();

    let error = release_manifest_url_for_layout(&InstallLayout::UserDir)
        .expect_err("installed environment must not bypass corporate provenance");

    let message = format!("{error:#}");
    assert!(message.contains(RELEASE_MANIFEST_URL_ENV), "{message}");
    assert!(message.contains("installed"), "{message}");
}

#[test]
fn development_update_source_accepts_explicit_environment_url() {
    let _lock = crate::lock_test_env();
    let assets = tempfile::tempdir().unwrap();
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", assets.path().to_str().unwrap());
    let nightly = "https://release.capsem.org/assets/nightly/manifest.json";
    let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, nightly);
    let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");

    assert_eq!(
        release_manifest_url_for_layout(&InstallLayout::Development).unwrap(),
        nightly
    );
}

#[test]
fn development_update_source_may_default_to_stable_without_metadata() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let _assets_override = EnvGuard::set("CAPSEM_ASSETS_DIR", "");
    let _manifest_override = EnvGuard::set(RELEASE_MANIFEST_URL_ENV, "");
    let _legacy_override = EnvGuard::set(LEGACY_RELEASE_HEALTH_URL_ENV, "");

    assert_eq!(
        release_manifest_url_for_layout(&InstallLayout::Development).unwrap(),
        DEFAULT_RELEASE_MANIFEST_URL
    );
}

fn channel_catalog_fixture() -> ReleaseChannelsCatalog {
    serde_json::from_value(serde_json::json!({
        "version": 1,
        "channels": {
            "nightly": {
                "manifests": [
                    {
                        "version": "1.5.0-nightly.20260702",
                        "status": "current",
                        "url": "/assets/nightly/manifest.json",
                        "digest": {
                            "sha256": "a".repeat(64),
                            "blake3": "b".repeat(64),
                            "hmac": "nightly-current-hmac"
                        },
                        "min_capsem_version": "1.5.0"
                    },
                    {
                        "version": "1.4.9-nightly.20260701",
                        "status": "supported",
                        "url": "/assets/nightly/1.4/manifest.json",
                        "digest": {
                            "sha256": "c".repeat(64),
                            "blake3": "d".repeat(64),
                            "hmac": "nightly-supported-hmac"
                        },
                        "min_capsem_version": "1.4.0",
                        "max_capsem_version": "1.4.99"
                    },
                    {
                        "version": "1.3.0-nightly.revoked",
                        "status": "revoked",
                        "url": "/assets/nightly/revoked/manifest.json",
                        "digest": {
                            "sha256": "e".repeat(64),
                            "blake3": "f".repeat(64),
                            "hmac": "nightly-revoked-hmac"
                        }
                    }
                ]
            }
        }
    }))
    .expect("channel catalog fixture")
}

#[test]
fn channel_manifest_resolution_never_selects_revoked_manifest() {
    let catalog = channel_catalog_fixture();

    let selected =
        select_channel_manifest_url(&catalog, "nightly", "1.4.12").expect("selection");

    assert_ne!(selected, "/assets/nightly/revoked/manifest.json");
    assert_eq!(selected, "/assets/nightly/1.4/manifest.json");
}

#[test]
fn channel_manifest_resolution_old_capsem_selects_compatible_supported_manifest() {
    let catalog = channel_catalog_fixture();

    let selected =
        select_channel_manifest_url(&catalog, "nightly", "1.4.12").expect("selection");

    assert_eq!(selected, "/assets/nightly/1.4/manifest.json");
}

#[test]
fn channel_manifest_resolution_requires_digest_shape() {
    let catalog: ReleaseChannelsCatalog = serde_json::from_value(serde_json::json!({
        "version": 1,
        "channels": {
            "stable": {
                "manifests": [
                    {
                        "version": "1.4.0",
                        "status": "current",
                        "url": "/assets/stable/manifest.json",
                        "digest": {
                            "sha256": "abc123",
                            "blake3": "b".repeat(64)
                        }
                    }
                ]
            }
        }
    }))
    .expect("bad catalog parses before validation");

    let error = select_channel_manifest_url(&catalog, "stable", "1.4.0")
        .expect_err("bad digest shape rejected");

    assert!(format!("{error:#}").contains("sha256"), "{error:#}");
}

#[test]
fn selected_channel_manifest_verification_rejects_payload_substitution() {
    let bytes = br#"{"channel":"stable"}"#;
    let selection = ResolvedReleaseChannelManifest {
        channel: "stable".to_string(),
        url: "https://release.capsem.org/assets/stable/manifest.json".to_string(),
        sha256: sha256_hex(bytes),
        blake3: blake3::hash(bytes).to_hex().to_string(),
    };

    verify_selected_channel_manifest(&selection, bytes).expect("matching payload");
    let error = verify_selected_channel_manifest(&selection, br#"{"channel":"nightly"}"#)
        .expect_err("substituted payload must fail closed");
    assert!(format!("{error:#}").contains("SHA-256 mismatch"));
}

#[test]
fn release_manifest_url_env_rejects_bare_paths() {
    let err =
        validate_release_manifest_url("/tmp/release/assets/stable/manifest.json").unwrap_err();
    assert!(
        format!("{err:#}").contains("CAPSEM_RELEASE_MANIFEST_URL must be a URL"),
        "{err:#}"
    );
}

#[test]
fn update_source_url_flags_are_url_only() {
    for flag in ["--manifest", "--corp"] {
        for source in [
            "https://release.capsem.org/assets/stable/manifest.json",
            "http://127.0.0.1:8080/assets/stable/manifest.json",
            "file:///tmp/capsem/assets/stable/manifest.json",
        ] {
            assert_eq!(
                validate_source_url_arg(flag, source),
                Ok(source.to_string()),
                "{flag} should accept {source}"
            );
        }

        for source in [
            "/tmp/capsem/assets/stable/manifest.json",
            "assets/stable/manifest.json",
            "file:assets/stable/manifest.json",
            "file://relative/manifest.json",
            "ssh://updates.example/assets/stable/manifest.json",
            "https:release.capsem.org/assets/stable/manifest.json",
        ] {
            let err =
                validate_source_url_arg(flag, source).expect_err("source should be rejected");
            assert!(
                err.contains(flag),
                "error for {source} should mention {flag}: {err}"
            );
        }
    }
}

#[test]
fn release_graph_update_check_selects_linux_deb_package() {
    let package_name = format!("Capsem_2.0.0_{}.deb", deb_arch());
    let other_package_architecture = match package_architecture() {
        PackageArchitecture::Amd64 => PackageArchitecture::Arm64,
        PackageArchitecture::Arm64 => PackageArchitecture::Amd64,
    };
    let graph: ReleaseGraphManifest = serde_json::from_value(serde_json::json!({
        "version": "1.0.0",
        "channel": "nightly",
        "packages": [
            {
                "name": "Capsem_2.0.0_wrong.deb",
                "url": "/releases/download/v2.0.0/Capsem_2.0.0_wrong.deb",
                "version": "2.0.0",
                "kind": "debian_package",
                "platform": "linux",
                "architecture": other_package_architecture,
                "status": "current",
                "bytes": 111,
                "digest": {"sha256": "1".repeat(64), "blake3": "a".repeat(64)}
            },
            {
                "name": package_name,
                "url": format!("/releases/download/v2.0.0/{package_name}"),
                "version": "2.0.0",
                "kind": "debian_package",
                "platform": "linux",
                "architecture": package_architecture(),
                "status": "current",
                "bytes": 222,
                "digest": {"sha256": "2".repeat(64), "blake3": "b".repeat(64)}
            },
            {
                "name": "Capsem-2.0.0.pkg",
                "url": "https://github.com/google/capsem/releases/download/v2.0.0/Capsem-2.0.0.pkg",
                "version": "2.0.0",
                "kind": "macos_pkg",
                "platform": "macos",
                "architecture": "arm64",
                "status": "current",
                "bytes": 333,
                "digest": {"sha256": "3".repeat(64), "blake3": "c".repeat(64)}
            }
        ],
        "profiles": {}
    }))
    .unwrap();

    let check = update_check_from_release_graph_manifest(
        &graph,
        1718444400,
        "1.5.0",
        Some("2026.0709.6"),
        Some("profiles-2026.0709.6"),
        &InstallLayout::LinuxDeb,
        "http://127.0.0.1:33773/assets/nightly/manifest.json",
        Some("f".repeat(64)),
    )
    .unwrap();

    assert_eq!(check.latest_version, Some("2.0.0".to_string()));
    assert!(check.update_available);
    let installer = check.binary_installer.as_ref().unwrap();
    assert_eq!(installer.name, package_name);
    assert_eq!(
        installer.url,
        format!("http://127.0.0.1:33773/releases/download/v2.0.0/{package_name}")
    );
    assert_eq!(installer.sha256, "2".repeat(64));
    assert_eq!(installer.size, 222);
    assert_eq!(installer.install_layout, "linux_deb");
    assert_eq!(check.latest_assets, None);
    assert!(!check.assets_update_available);
    assert_eq!(check.latest_profiles, None);
    assert_eq!(check.channel_hash, Some("f".repeat(64)));
    assert_eq!(check.validation_status, Some("valid".to_string()));
}

#[test]
fn release_graph_reads_exact_legacy_x86_64_amd64_package_row() {
    let graph = serde_json::from_value::<ReleaseGraphManifest>(serde_json::json!({
        "packages": [{
            "name": "Capsem_2.0.0_amd64.deb",
            "url": "/releases/download/v2.0.0/Capsem_2.0.0_amd64.deb",
            "version": "2.0.0",
            "kind": "debian_package",
            "platform": "linux",
            "architecture": "x86_64",
            "status": "current",
            "bytes": 222,
            "digest": {"sha256": "2".repeat(64), "blake3": "b".repeat(64)}
        }],
        "profiles": {}
    }))
    .expect("the immutable legacy package identity must remain readable");

    assert_eq!(graph.packages[0].architecture, PackageArchitecture::Amd64);
    assert!(
        graph_linux_package_matches_architecture(
            &graph.packages[0],
            PackageArchitecture::Amd64
        ),
        "the exact x86_64 + _amd64.deb legacy row must select the existing package"
    );
}

#[test]
fn release_graph_rejects_non_exact_legacy_package_architecture_aliases() {
    for (label, name, url, kind, platform, architecture) in [
        (
            "wrong filename",
            "Capsem_2.0.0_arm64.deb",
            "/releases/download/v2.0.0/Capsem_2.0.0_arm64.deb",
            "debian_package",
            "linux",
            "x86_64",
        ),
        (
            "wrong URL filename",
            "Capsem_2.0.0_amd64.deb",
            "/releases/download/v2.0.0/not-the-package.deb",
            "debian_package",
            "linux",
            "x86_64",
        ),
        (
            "wrong package kind",
            "Capsem_2.0.0_amd64.deb",
            "/releases/download/v2.0.0/Capsem_2.0.0_amd64.deb",
            "archive",
            "linux",
            "x86_64",
        ),
        (
            "wrong platform",
            "Capsem_2.0.0_amd64.deb",
            "/releases/download/v2.0.0/Capsem_2.0.0_amd64.deb",
            "debian_package",
            "macos",
            "x86_64",
        ),
        (
            "unrecognized alias",
            "Capsem_2.0.0_amd64.deb",
            "/releases/download/v2.0.0/Capsem_2.0.0_amd64.deb",
            "debian_package",
            "linux",
            "x64",
        ),
    ] {
        let error = serde_json::from_value::<ReleaseGraphManifest>(serde_json::json!({
            "packages": [{
                "name": name,
                "url": url,
                "version": "2.0.0",
                "kind": kind,
                "platform": platform,
                "architecture": architecture,
                "status": "current",
                "bytes": 222,
                "digest": {"sha256": "2".repeat(64), "blake3": "b".repeat(64)}
            }],
            "profiles": {}
        }))
        .expect_err(label);

        assert!(
            error
                .to_string()
                .contains("unsupported package architecture"),
            "{label}: {error}"
        );
    }
}

#[test]
fn release_graph_update_check_selects_macos_pkg_package() {
    let graph: ReleaseGraphManifest = serde_json::from_value(serde_json::json!({
        "version": "1.0.0",
        "channel": "stable",
        "packages": [
            {
                "name": "Capsem-2.0.0.pkg",
                "url": "https://github.com/google/capsem/releases/download/v2.0.0/Capsem-2.0.0.pkg",
                "version": "2.0.0",
                "kind": "macos_pkg",
                "platform": "macos",
                "architecture": "arm64",
                "status": "current",
                "bytes": 333,
                "digest": {"sha256": "3".repeat(64), "blake3": "c".repeat(64)}
            },
            {
                "name": format!("Capsem_2.0.0_{}.deb", deb_arch()),
                "url": format!(
                    "https://github.com/google/capsem/releases/download/v2.0.0/Capsem_2.0.0_{}.deb",
                    deb_arch()
                ),
                "version": "2.0.0",
                "kind": "debian_package",
                "platform": "linux",
                "architecture": package_architecture(),
                "status": "current",
                "bytes": 222,
                "digest": {"sha256": "2".repeat(64), "blake3": "b".repeat(64)}
            }
        ],
        "profiles": {}
    }))
    .unwrap();

    let check = update_check_from_release_graph_manifest(
        &graph,
        1718444400,
        "1.5.0",
        None,
        None,
        &InstallLayout::MacosPkg,
        "https://release.capsem.org/assets/stable/manifest.json",
        None,
    )
    .unwrap();

    let installer = check.binary_installer.as_ref().unwrap();
    assert_eq!(installer.name, "Capsem-2.0.0.pkg");
    assert_eq!(installer.install_layout, "macos_pkg");
    assert_eq!(installer.sha256, "3".repeat(64));
}

#[test]
fn release_graph_update_check_does_not_select_installer_when_current() {
    let package_name = format!("Capsem_1.5.0_{}.deb", deb_arch());
    let graph: ReleaseGraphManifest = serde_json::from_value(serde_json::json!({
        "packages": [
            {
                "name": package_name,
                "url": format!("https://github.com/google/capsem/releases/download/v1.5.0/{package_name}"),
                "version": "1.5.0",
                "kind": "debian_package",
                "platform": "linux",
                "architecture": package_architecture(),
                "status": "current",
                "bytes": 222,
                "digest": {"sha256": "2".repeat(64), "blake3": "b".repeat(64)}
            }
        ],
        "profiles": {}
    }))
    .unwrap();

    let check = update_check_from_release_graph_manifest(
        &graph,
        1718444400,
        "1.5.0",
        None,
        None,
        &InstallLayout::LinuxDeb,
        "https://release.capsem.org/assets/stable/manifest.json",
        None,
    )
    .unwrap();

    assert_eq!(check.latest_version, Some("1.5.0".to_string()));
    assert!(!check.update_available);
    assert_eq!(check.binary_installer, None);
}

#[test]
fn shared_release_payload_parser_accepts_public_release_graphs() {
    let body = serde_json::to_vec(&serde_json::json!({
        "version": "1.0.142",
        "channel": "stable",
        "status": "current",
        "packages": [{
            "name": "Capsem-99.99.99.pkg",
            "url": "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg",
            "version": "99.99.99",
            "kind": "macos_pkg",
            "platform": "macos",
            "architecture": "arm64",
            "status": "current",
            "bytes": 123,
            "digest": {"sha256": "3".repeat(64), "blake3": "b".repeat(64)}
        }],
        "profiles": {}
    }))
    .unwrap();

    let check = update_check_from_release_payload(
        &body,
        &InstallLayout::MacosPkg,
        "https://release.capsem.org/assets/stable/manifest.json",
        Some("f".repeat(64)),
    )
    .expect("public graph payload");

    assert_eq!(check.latest_version.as_deref(), Some("99.99.99"));
    assert_eq!(
        check.source.as_deref(),
        Some("https://release.capsem.org/assets/stable/manifest.json")
    );
    assert_eq!(check.channel_hash, Some("f".repeat(64)));
}

#[test]
fn shared_release_payload_parser_accepts_profiles_only_release_graphs() {
    let body = serde_json::to_vec(&serde_json::json!({
        "version": "1.0.142",
        "channel": "stable",
        "status": "current",
        "packages": [],
        "profiles": {
            "default": {
                "revision": "2030.0101.2",
                "status": "current",
                "architectures": [{
                    "architecture": machine_architecture(),
                    "image_revision": "2030.0101.7",
                    "images": [
                        {
                            "kind": "kernel",
                            "name": "vmlinuz",
                            "url": "https://release.capsem.org/assets/releases/2030.0101.7/vmlinuz",
                            "bytes": 1,
                            "status": "current",
                            "digest": {
                                "sha256": "1".repeat(64),
                                "blake3": "a".repeat(64)
                            }
                        },
                        {
                            "kind": "initrd",
                            "name": "initrd.img",
                            "url": "https://release.capsem.org/assets/releases/2030.0101.7/initrd.img",
                            "bytes": 1,
                            "status": "current",
                            "digest": {
                                "sha256": "2".repeat(64),
                                "blake3": "b".repeat(64)
                            }
                        },
                        {
                            "kind": "rootfs",
                            "name": "rootfs.erofs",
                            "url": "https://release.capsem.org/assets/releases/2030.0101.7/rootfs.erofs",
                            "bytes": 1,
                            "status": "current",
                            "digest": {
                                "sha256": "3".repeat(64),
                                "blake3": "c".repeat(64)
                            }
                        }
                    ]
                }]
            }
        }
    }))
    .unwrap();

    let check = update_check_from_release_payload(
        &body,
        &InstallLayout::LinuxDeb,
        "https://release.capsem.org/assets/stable/manifest.json",
        Some("f".repeat(64)),
    )
    .expect("profiles-only public graph payload");

    assert_eq!(check.latest_version, None);
    assert!(check
        .latest_assets
        .as_deref()
        .is_some_and(|revision| revision.starts_with("images-")));
    assert!(check
        .latest_profiles
        .as_deref()
        .is_some_and(|revision| revision.starts_with("catalog-")));
    assert_eq!(
        binary_installer_from_release_payload(
            &body,
            &InstallLayout::LinuxDeb,
            "https://release.capsem.org/assets/stable/manifest.json",
        )
        .unwrap(),
        None
    );
}

#[test]
fn release_graph_materializes_installed_profile_pins_from_manifest() {
    let source = r#"
[assets]
format = "profile-assets.v1"
refresh_policy = "on_profile_refresh"

[assets.arch.x86_64.kernel]
name = "vmlinuz"
url = "https://old.example/vmlinuz"

[assets.arch.x86_64.initrd]
name = "initrd.img"
url = "https://old.example/initrd.img"

[assets.arch.x86_64.rootfs]
name = "rootfs.erofs"
url = "https://old.example/rootfs.erofs"
"#;
    let pins = [
        ("kernel", "vmlinuz", "a", 11_u64),
        ("initrd", "initrd.img", "b", 22_u64),
        ("rootfs", "rootfs.erofs", "c", 33_u64),
    ]
    .into_iter()
    .map(
        |(kind, name, digest_seed, size)| ReleaseChannelProfileRuntimePin {
            profile_id: "code".to_string(),
            arch: "x86_64".to_string(),
            kind: kind.to_string(),
            name: name.to_string(),
            url: format!("/profiles/releases/test/code/x86_64/{name}"),
            size,
            blake3: digest_seed.repeat(64),
        },
    )
    .collect::<Vec<_>>();

    let materialized = materialize_release_channel_profile_toml(
        source,
        "code",
        "https://release.example/assets/nightly/manifest.json",
        &pins,
    )
    .unwrap();
    let document: toml::Value = toml::from_str(&materialized).unwrap();
    let assets = &document["assets"]["arch"]["x86_64"];

    for (kind, name, digest_seed, size) in [
        ("kernel", "vmlinuz", "a", 11_i64),
        ("initrd", "initrd.img", "b", 22_i64),
        ("rootfs", "rootfs.erofs", "c", 33_i64),
    ] {
        assert_eq!(assets[kind]["name"].as_str(), Some(name));
        assert_eq!(
            assets[kind]["url"].as_str(),
            Some(
                format!("https://release.example/profiles/releases/test/code/x86_64/{name}")
                    .as_str()
            )
        );
        assert_eq!(
            assets[kind]["hash"].as_str(),
            Some(format!("blake3:{}", digest_seed.repeat(64)).as_str())
        );
        assert_eq!(assets[kind]["size"].as_integer(), Some(size));
    }
}

#[test]
fn release_graph_profile_materialization_rejects_incomplete_manifest_pins() {
    let source = r#"
[assets]
format = "profile-assets.v1"
refresh_policy = "on_profile_refresh"

[assets.arch.x86_64.kernel]
name = "vmlinuz"
url = "https://old.example/vmlinuz"

[assets.arch.x86_64.initrd]
name = "initrd.img"
url = "https://old.example/initrd.img"

[assets.arch.x86_64.rootfs]
name = "rootfs.erofs"
url = "https://old.example/rootfs.erofs"
"#;
    let pins = vec![ReleaseChannelProfileRuntimePin {
        profile_id: "code".to_string(),
        arch: "x86_64".to_string(),
        kind: "kernel".to_string(),
        name: "vmlinuz".to_string(),
        url: "https://release.example/vmlinuz".to_string(),
        size: 11,
        blake3: "a".repeat(64),
    }];

    let error = materialize_release_channel_profile_toml(
        source,
        "code",
        "https://release.example/assets/nightly/manifest.json",
        &pins,
    )
    .unwrap_err();

    assert!(
        format!("{error:#}").contains("missing manifest runtime pins"),
        "{error:#}"
    );
}

#[test]
fn release_graph_update_compares_independent_multi_profile_state() {
    let profile = |revision: &str, image_revision: &str, seed: char| {
        serde_json::json!({
            "revision": revision,
            "status": "current",
            "architectures": [{
                "architecture": machine_architecture(),
                "image_revision": image_revision,
                "config": [{
                    "kind": "profile",
                    "path": format!("profiles/{revision}/profile.toml"),
                    "url": format!("https://release.example/{revision}/profile.toml"),
                    "bytes": 1,
                    "digest": {"sha256": "1".repeat(64), "blake3": seed.to_string().repeat(64)}
                }],
                "images": [
                    {"kind":"kernel","name":"vmlinuz","url":"https://release.example/vmlinuz","bytes":1,"status":"current","digest":{"sha256":"2".repeat(64),"blake3":"b".repeat(64)}},
                    {"kind":"initrd","name":"initrd.img","url":"https://release.example/initrd.img","bytes":1,"status":"current","digest":{"sha256":"3".repeat(64),"blake3":"c".repeat(64)}},
                    {"kind":"rootfs","name":"rootfs.erofs","url":"https://release.example/rootfs.erofs","bytes":1,"status":"current","digest":{"sha256":"4".repeat(64),"blake3":"d".repeat(64)}}
                ],
                "evidence": [{
                    "kind": "obom",
                    "url": format!("https://release.example/{revision}/obom.cdx.json"),
                    "bytes": 1,
                    "digest": {"sha256": "5".repeat(64), "blake3": "e".repeat(64)}
                }]
            }]
        })
    };
    let graph_value = serde_json::json!({
        "packages": [],
        "profiles": {
            "co-work": profile("2030.0101.1", "2030.0101.10", 'a'),
            "code": profile("2030.0101.2", "2030.0101.20", 'f')
        }
    });
    let installed_state =
        capsem_core::asset_manager::release_graph_profile_state(&graph_value).unwrap();
    let graph: ReleaseGraphManifest = serde_json::from_value(graph_value.clone()).unwrap();
    assert_eq!(
        serde_json::json!({"profiles": &graph.profiles})["profiles"],
        graph_value["profiles"]
    );

    let first = update_check_from_release_graph_manifest(
        &graph,
        1718444400,
        "1.5.0",
        None,
        None,
        &InstallLayout::LinuxDeb,
        "https://release.capsem.org/assets/nightly/manifest.json",
        None,
    )
    .unwrap();

    let latest_profiles = first.latest_profiles.as_deref().unwrap();
    let latest_images = first.latest_images.as_deref().unwrap();
    assert_eq!(latest_profiles, installed_state.catalog_revision);
    assert_eq!(latest_images, installed_state.images_revision);
    assert!(latest_profiles.starts_with("catalog-"));
    assert!(latest_images.starts_with("images-"));
    assert_eq!(first.latest_assets.as_deref(), Some(latest_images));

    let unchanged = update_check_from_release_graph_manifest(
        &graph,
        1718444401,
        "1.5.0",
        Some(latest_images),
        Some(latest_profiles),
        &InstallLayout::LinuxDeb,
        "https://release.capsem.org/assets/nightly/manifest.json",
        None,
    )
    .unwrap();

    assert!(!unchanged.assets_update_available);
    assert!(!unchanged.profiles_update_available);
    assert!(!unchanged.images_update_available);
}

#[test]
fn release_graph_update_check_does_not_downgrade_lower_nightly_package() {
    let package_name = format!("Capsem_1.5.99_{}.deb", deb_arch());
    let graph: ReleaseGraphManifest = serde_json::from_value(serde_json::json!({
        "version": "1.0.0",
        "channel": "nightly",
        "packages": [
            {
                "name": package_name,
                "url": format!("https://github.com/google/capsem/releases/download/v1.5.99/{package_name}"),
                "version": "1.5.99",
                "kind": "debian_package",
                "platform": "linux",
                "architecture": package_architecture(),
                "status": "current",
                "bytes": 222,
                "digest": {"sha256": "2".repeat(64), "blake3": "b".repeat(64)}
            }
        ],
        "profiles": {}
    }))
    .unwrap();

    let check = update_check_from_release_graph_manifest(
        &graph,
        1718444400,
        "1.5.100",
        None,
        None,
        &InstallLayout::LinuxDeb,
        "https://release.capsem.org/assets/nightly/manifest.json",
        None,
    )
    .unwrap();

    assert_eq!(check.latest_version, Some("1.5.99".to_string()));
    assert!(!check.update_available);
    assert_eq!(check.binary_installer, None);
}

#[test]
fn release_graph_update_check_does_not_update_non_comparable_nightly_package() {
    let package_name = format!("Capsem_nightly-20260710_{}.deb", deb_arch());
    let graph: ReleaseGraphManifest = serde_json::from_value(serde_json::json!({
        "version": "1.0.0",
        "channel": "nightly",
        "packages": [
            {
                "name": package_name,
                "url": format!("https://github.com/google/capsem/releases/download/nightly-20260710/{package_name}"),
                "version": "nightly-20260710",
                "kind": "debian_package",
                "platform": "linux",
                "architecture": package_architecture(),
                "status": "current",
                "bytes": 222,
                "digest": {"sha256": "2".repeat(64), "blake3": "b".repeat(64)}
            }
        ],
        "profiles": {}
    }))
    .unwrap();

    let check = update_check_from_release_graph_manifest(
        &graph,
        1718444400,
        "1.5.100",
        None,
        None,
        &InstallLayout::LinuxDeb,
        "https://release.capsem.org/assets/nightly/manifest.json",
        None,
    )
    .unwrap();

    assert_eq!(check.latest_version, Some("nightly-20260710".to_string()));
    assert!(!check.update_available);
    assert_eq!(check.binary_installer, None);
}

#[test]
fn release_health_update_check_uses_updates_block() {
    let pkg_sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let deb_sha = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    let pkg_blake3 = "1111111111111111111111111111111111111111111111111111111111111111";
    let deb_blake3 = "2222222222222222222222222222222222222222222222222222222222222222";
    let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
        "schema": "capsem.assets_channel.legacy.v1",
        "updates": {
            "binary": {
                "latest": "99.99.99",
                "current": "99.99.98",
                "files": [
                    {
                        "name": "Capsem-99.99.99.pkg",
                        "url": "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg",
                        "sha256": pkg_sha,
                        "blake3": pkg_blake3,
                        "size": 123
                    },
                    {
                        "name": format!("Capsem_99.99.99_{}.deb", deb_arch()),
                        "url": format!("https://github.com/google/capsem/releases/download/v99.99.99/Capsem_99.99.99_{}.deb", deb_arch()),
                        "sha256": deb_sha,
                        "blake3": deb_blake3,
                        "size": 456
                    }
                ]
            },
            "assets": {
                "latest": "2030.0101.1",
                "current": "2030.0101.0",
                "state": "published",
                "compatibility": {
                    "min_binary": "1.0.0"
                }
            },
            "profiles": {
                "latest": "profiles-2030.0101.1",
                "state": "published",
                "source": "/profiles/releases/profiles-2030.0101.1/catalog.json",
                "hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "requires_newer": {
                    "binary": false,
                    "assets": false
                }
            },
            "images": {"latest": null, "state": "not_published"}
        }
    }))
    .unwrap();

    let check = update_check_from_release_health(
        &health,
        1718444400,
        "1.3.1782582155",
        Some("2026.0627.1"),
        Some("profiles-2030.0101.0"),
        &InstallLayout::MacosPkg,
        "https://release.capsem.org/assets/stable/manifest.json",
        Some("f".repeat(64)),
    )
    .unwrap();

    assert_eq!(check.latest_version, Some("99.99.99".to_string()));
    assert!(check.update_available);
    let installer = check.binary_installer.as_ref().unwrap();
    assert_eq!(installer.name, "Capsem-99.99.99.pkg");
    assert_eq!(installer.sha256, pkg_sha);
    assert_eq!(installer.size, 123);
    assert_eq!(installer.install_layout, "macos_pkg");
    assert_eq!(check.latest_assets, Some("2030.0101.1".to_string()));
    assert!(check.assets_update_available);
    assert_eq!(check.assets_state, Some("published".to_string()));
    assert_eq!(check.assets_blocked_reason, None);
    assert_eq!(
        check.current_profiles,
        Some("profiles-2030.0101.0".to_string())
    );
    assert_eq!(
        check.latest_profiles,
        Some("profiles-2030.0101.1".to_string())
    );
    assert!(check.profiles_update_available);
    assert_eq!(check.profiles_state, Some("published".to_string()));
    assert_eq!(check.profiles_blocked_reason, None);
    assert_eq!(
        check.profile_catalog_source,
        Some("/profiles/releases/profiles-2030.0101.1/catalog.json".to_string())
    );
    assert_eq!(check.profile_catalog_hash, Some("b".repeat(64)));
    assert_eq!(check.latest_images, None);
    assert!(!check.images_update_available);
    assert_eq!(check.images_state, Some("not_published".to_string()));
    assert_eq!(check.images_blocked_reason, None);
    assert_eq!(
        check.source,
        Some("https://release.capsem.org/assets/stable/manifest.json".to_string())
    );
    assert_eq!(check.channel_hash, Some("f".repeat(64)));
    assert_eq!(check.validation_status, Some("valid".to_string()));
    assert_eq!(check.validation_error, None);
}

#[test]
fn release_health_update_check_accepts_legacy_current_targets() {
    let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
        "schema": "capsem.assets_channel.legacy.v1",
        "updates": {
            "binary": {"current": "99.99.99"},
            "assets": {"current": "2030.0101.1"}
        }
    }))
    .unwrap();

    let check = update_check_from_release_health(
        &health,
        1718444400,
        "1.3.1782582155",
        Some("2026.0627.1"),
        None,
        &InstallLayout::MacosPkg,
        "https://release.capsem.org/assets/stable/manifest.json",
        None,
    )
    .unwrap();

    assert_eq!(check.latest_version, Some("99.99.99".to_string()));
    assert_eq!(check.latest_assets, Some("2030.0101.1".to_string()));
}

#[test]
fn release_health_asset_update_reports_blocked_compatibility() {
    let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
        "schema": "capsem.assets_channel.legacy.v1",
        "updates": {
            "binary": {"current": "1.3.1782582155"},
            "assets": {
                "latest": "2030.0101.1",
                "current": "2030.0101.1",
                "state": "published",
                "compatibility": {
                    "min_binary": "99.99.99"
                }
            }
        }
    }))
    .unwrap();

    let check = update_check_from_release_health(
        &health,
        1718444400,
        "1.3.1782582155",
        Some("2026.0627.1"),
        None,
        &InstallLayout::MacosPkg,
        "https://release.capsem.org/assets/stable/manifest.json",
        None,
    )
    .unwrap();

    assert_eq!(check.latest_assets, Some("2030.0101.1".to_string()));
    assert_eq!(check.current_assets, Some("2026.0627.1".to_string()));
    assert!(!check.assets_update_available);
    assert_eq!(check.assets_state, Some("published".to_string()));
    assert_eq!(
        check.assets_blocked_reason.as_deref(),
        Some("requires binary 99.99.99 or newer")
    );
}

#[test]
fn release_health_deprecated_asset_update_is_blocked() {
    let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
        "schema": "capsem.assets_channel.legacy.v1",
        "updates": {
            "binary": {"current": "1.3.1782582155"},
            "assets": {
                "latest": "2030.0101.1",
                "current": "2030.0101.1",
                "state": "deprecated"
            }
        }
    }))
    .unwrap();

    let check = update_check_from_release_health(
        &health,
        1718444400,
        "1.3.1782582155",
        Some("2026.0627.1"),
        None,
        &InstallLayout::MacosPkg,
        "https://release.capsem.org/assets/stable/manifest.json",
        None,
    )
    .unwrap();

    assert!(!check.assets_update_available);
    assert_eq!(
        check.assets_blocked_reason.as_deref(),
        Some("latest VM asset release is deprecated")
    );
}

#[test]
fn release_health_profile_update_reports_blocked_compatibility() {
    let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
        "schema": "capsem.assets_channel.legacy.v1",
        "updates": {
            "binary": {"current": "1.3.1782582155"},
            "assets": {"current": "2026.0627.1"},
            "profiles": {
                "latest": "profiles-2030.0101.1",
                "state": "published",
                "requires_newer": {
                    "binary": true,
                    "assets": false
                },
                "compatibility": {
                    "min_binary": "1.4.0",
                    "min_assets": "2026.0627.1"
                }
            }
        }
    }))
    .unwrap();

    let check = update_check_from_release_health(
        &health,
        1718444400,
        "1.3.1782582155",
        Some("2026.0627.1"),
        Some("profiles-2030.0101.0"),
        &InstallLayout::MacosPkg,
        "https://release.capsem.org/assets/stable/manifest.json",
        None,
    )
    .unwrap();

    assert_eq!(
        check.current_profiles,
        Some("profiles-2030.0101.0".to_string())
    );
    assert_eq!(
        check.latest_profiles,
        Some("profiles-2030.0101.1".to_string())
    );
    assert!(!check.profiles_update_available);
    assert_eq!(
        check.profiles_blocked_reason.as_deref(),
        Some("requires binary 1.4.0 or newer")
    );
}

#[test]
fn binary_installer_for_layout_selects_matching_deb_arch() {
    let files = vec![
        ReleaseChannelBinaryFile {
            name: "Capsem_99.99.99_wrong.deb".to_string(),
            url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem_99.99.99_wrong.deb".to_string(),
            sha256: "1".repeat(64),
            blake3: "a".repeat(64),
            size: 10,
        },
        ReleaseChannelBinaryFile {
            name: format!("Capsem_99.99.99_{}.deb", deb_arch()),
            url: format!(
                "https://github.com/google/capsem/releases/download/v99.99.99/Capsem_99.99.99_{}.deb",
                deb_arch()
            ),
            sha256: "2".repeat(64),
            blake3: "b".repeat(64),
            size: 20,
        },
        ReleaseChannelBinaryFile {
            name: "Capsem-99.99.99.pkg".to_string(),
            url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg".to_string(),
            sha256: "3".repeat(64),
            blake3: "c".repeat(64),
            size: 30,
        },
    ];

    let installer = binary_installer_for_layout(&files, &InstallLayout::LinuxDeb).unwrap();

    assert_eq!(
        installer.name,
        format!("Capsem_99.99.99_{}.deb", deb_arch())
    );
    assert_eq!(installer.sha256, "2".repeat(64));
    assert_eq!(installer.blake3, "b".repeat(64));
    assert_eq!(installer.size, 20);
    assert_eq!(installer.install_layout, "linux_deb");
}

#[test]
fn binary_installer_for_layout_rejects_non_http_urls() {
    let files = vec![ReleaseChannelBinaryFile {
        name: "Capsem-99.99.99.pkg".to_string(),
        url: "file:///tmp/Capsem-99.99.99.pkg".to_string(),
        sha256: "local".to_string(),
        blake3: "local".to_string(),
        size: 10,
    }];

    assert_eq!(
        binary_installer_for_layout(&files, &InstallLayout::MacosPkg),
        None
    );
}

#[test]
fn verify_binary_installer_bytes_accepts_matching_sha256_and_size() {
    let bytes = b"verified installer payload";
    let installer = BinaryInstaller {
        name: "Capsem-99.99.99.pkg".to_string(),
        url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
            .to_string(),
        sha256: test_sha256(bytes),
        blake3: test_blake3(bytes),
        size: bytes.len() as u64,
        install_layout: "macos_pkg".to_string(),
    };

    verify_binary_installer_bytes(bytes, &installer).unwrap();
}

#[test]
fn verify_binary_installer_bytes_rejects_size_mismatch() {
    let bytes = b"verified installer payload";
    let installer = BinaryInstaller {
        name: "Capsem-99.99.99.pkg".to_string(),
        url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
            .to_string(),
        sha256: test_sha256(bytes),
        blake3: test_blake3(bytes),
        size: bytes.len() as u64 + 1,
        install_layout: "macos_pkg".to_string(),
    };

    let err = verify_binary_installer_bytes(bytes, &installer).unwrap_err();

    assert!(
        format!("{err:#}").contains("binary installer size mismatch"),
        "{err:#}"
    );
}

#[test]
fn verify_binary_installer_bytes_rejects_sha256_mismatch() {
    let bytes = b"verified installer payload";
    let installer = BinaryInstaller {
        name: "Capsem-99.99.99.pkg".to_string(),
        url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
            .to_string(),
        sha256: "0".repeat(64),
        blake3: test_blake3(bytes),
        size: bytes.len() as u64,
        install_layout: "macos_pkg".to_string(),
    };

    let err = verify_binary_installer_bytes(bytes, &installer).unwrap_err();

    assert!(
        format!("{err:#}").contains("binary installer sha256 mismatch"),
        "{err:#}"
    );
}

#[test]
fn verify_binary_installer_bytes_rejects_blake3_mismatch() {
    let bytes = b"verified installer payload";
    let installer = BinaryInstaller {
        name: "Capsem-99.99.99.pkg".to_string(),
        url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
            .to_string(),
        sha256: test_sha256(bytes),
        blake3: "0".repeat(64),
        size: bytes.len() as u64,
        install_layout: "macos_pkg".to_string(),
    };

    let err = verify_binary_installer_bytes(bytes, &installer).unwrap_err();

    assert!(
        format!("{err:#}").contains("binary installer blake3 mismatch"),
        "{err:#}"
    );
}

#[test]
fn binary_installer_metadata_rejects_path_names() {
    let installer = BinaryInstaller {
        name: "../Capsem-99.99.99.pkg".to_string(),
        url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
            .to_string(),
        sha256: "0".repeat(64),
        blake3: "0".repeat(64),
        size: 10,
        install_layout: "macos_pkg".to_string(),
    };

    let err = validate_binary_installer_metadata(&installer).unwrap_err();

    assert!(
        format!("{err:#}").contains("binary installer name must be a plain filename"),
        "{err:#}"
    );
}

#[test]
fn binary_installer_apply_plan_uses_macos_pkg_installer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Capsem 99.99.99.pkg");
    std::fs::write(&path, b"pkg").unwrap();
    let installer = BinaryInstaller {
        name: "Capsem-99.99.99.pkg".to_string(),
        url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
            .to_string(),
        sha256: "0".repeat(64),
        blake3: "0".repeat(64),
        size: 3,
        install_layout: "macos_pkg".to_string(),
    };

    let plan = binary_installer_apply_plan(&installer, &path).unwrap();

    assert_eq!(
        plan.commands,
        vec![BinaryInstallerApplyCommand {
            program: "sudo".to_string(),
            args: vec![
                "/usr/sbin/installer".to_string(),
                "-pkg".to_string(),
                path.display().to_string(),
                "-target".to_string(),
                "/".to_string(),
            ],
        }]
    );
    assert_eq!(
        plan.command_lines(),
        vec![format!(
            "sudo /usr/sbin/installer -pkg '{}' -target /",
            path.display()
        )]
    );
}

#[test]
fn binary_installer_apply_plan_uses_apt_for_linux_deb() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Capsem_99.99.99_arm64.deb");
    std::fs::write(&path, b"deb").unwrap();
    let installer = BinaryInstaller {
        name: "Capsem_99.99.99_arm64.deb".to_string(),
        url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem_99.99.99_arm64.deb"
            .to_string(),
        sha256: "0".repeat(64),
        blake3: "0".repeat(64),
        size: 3,
        install_layout: "linux_deb".to_string(),
    };

    let plan = binary_installer_apply_plan(&installer, &path).unwrap();

    assert_eq!(
        plan.commands,
        vec![BinaryInstallerApplyCommand {
            program: "sudo".to_string(),
            args: vec![
                "apt-get".to_string(),
                "install".to_string(),
                "--yes".to_string(),
                "--allow-downgrades".to_string(),
                path.display().to_string(),
            ],
        }]
    );
    assert_eq!(
        plan.command_lines(),
        vec![format!(
            "sudo apt-get install --yes --allow-downgrades {}",
            path.display()
        )]
    );
}

#[test]
fn binary_installer_apply_plan_rejects_unknown_layout() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Capsem-99.99.99.pkg");
    std::fs::write(&path, b"pkg").unwrap();
    let installer = BinaryInstaller {
        name: "Capsem-99.99.99.pkg".to_string(),
        url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg"
            .to_string(),
        sha256: "0".repeat(64),
        blake3: "0".repeat(64),
        size: 3,
        install_layout: "portable_zip".to_string(),
    };

    let err = binary_installer_apply_plan(&installer, &path).unwrap_err();

    assert!(
        format!("{err:#}").contains("unsupported binary installer layout portable_zip"),
        "{err:#}"
    );
}

#[tokio::test(flavor = "current_thread")]
#[allow(clippy::await_holding_lock)]
async fn download_binary_installer_fetches_verifies_and_caches() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let payload = b"downloaded installer payload".to_vec();
    let response_payload = payload.clone();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
        let _ = std::io::Read::read(&mut stream, &mut request);
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            response_payload.len()
        );
        std::io::Write::write_all(&mut stream, header.as_bytes()).unwrap();
        std::io::Write::write_all(&mut stream, &response_payload).unwrap();
    });
    let installer = BinaryInstaller {
        name: "Capsem-99.99.99.pkg".to_string(),
        url: format!("http://{addr}/Capsem-99.99.99.pkg"),
        sha256: test_sha256(&payload),
        blake3: test_blake3(&payload),
        size: payload.len() as u64,
        install_layout: "macos_pkg".to_string(),
    };

    let path = download_binary_installer(&installer).await.unwrap();
    server.join().unwrap();

    assert_eq!(
        path,
        home.path()
            .join("updates/installers/sha256")
            .join(test_sha256(&payload))
            .join("Capsem-99.99.99.pkg")
    );
    assert_eq!(std::fs::read(path).unwrap(), payload);
}

#[test]
fn binary_installer_cache_is_manifest_digest_addressed_and_channel_independent() {
    let home = tempfile::tempdir().unwrap();
    let payload = b"one immutable installer";
    let mut nightly = BinaryInstaller {
        name: "Capsem.pkg".to_string(),
        url: "https://release.example/assets/nightly/Capsem.pkg".to_string(),
        sha256: test_sha256(payload),
        blake3: test_blake3(payload),
        size: payload.len() as u64,
        install_layout: "macos_pkg".to_string(),
    };
    let nightly_path = binary_installer_cache_path_at(home.path(), &nightly).unwrap();

    nightly.url = "https://corp.example/releases/Capsem.pkg".to_string();
    let corporate_path = binary_installer_cache_path_at(home.path(), &nightly).unwrap();

    let mut replacement = nightly.clone();
    replacement.sha256 = "f".repeat(64);
    replacement.blake3 = "e".repeat(64);
    let replacement_path = binary_installer_cache_path_at(home.path(), &replacement).unwrap();

    assert_eq!(nightly_path, corporate_path);
    assert_ne!(nightly_path, replacement_path);
    assert_eq!(
        nightly_path,
        home.path()
            .join("updates/installers/sha256")
            .join(test_sha256(payload))
            .join("Capsem.pkg")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn corrupt_cached_binary_installer_is_discarded_and_refetched() {
    let home = tempfile::tempdir().unwrap();
    let payload = b"replacement installer payload".to_vec();
    let response_payload = payload.clone();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut request = [0_u8; 1024];
                    let _ = std::io::Read::read(&mut stream, &mut request);
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                        response_payload.len()
                    );
                    std::io::Write::write_all(&mut stream, header.as_bytes()).unwrap();
                    std::io::Write::write_all(&mut stream, &response_payload).unwrap();
                    return true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("accept installer request: {error}"),
            }
        }
    });
    let installer = BinaryInstaller {
        name: "Capsem.pkg".to_string(),
        url: format!("http://{addr}/Capsem.pkg"),
        sha256: test_sha256(&payload),
        blake3: test_blake3(&payload),
        size: payload.len() as u64,
        install_layout: "macos_pkg".to_string(),
    };
    let target = home
        .path()
        .join("updates/installers/sha256")
        .join(&installer.sha256)
        .join(&installer.name);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"corrupt").unwrap();

    let result = download_binary_installer_at(home.path(), &installer).await;
    let fetched = server.join().unwrap();
    let path = result.unwrap();

    assert!(
        fetched,
        "corrupt cache entry must trigger an artifact fetch"
    );
    assert_eq!(path, target);
    assert_eq!(std::fs::read(path).unwrap(), payload);
}

#[tokio::test(flavor = "current_thread")]
async fn update_check_rejects_mutating_options_programmatically() {
    for result in [
        run_update(true, true, false, None, None, false, None).await,
        run_update(false, true, true, None, None, false, None).await,
        run_update(
            false,
            true,
            false,
            None,
            Some("https://release.capsem.org/assets/stable/manifest.json"),
            false,
            None,
        )
        .await,
        run_update(
            false,
            true,
            false,
            None,
            None,
            false,
            Some("https://corp.example/capsem/corp.json"),
        )
        .await,
    ] {
        let err = result.expect_err("check-only update must reject mutating options");
        assert!(
            format!("{err:#}").contains("--check cannot be combined"),
            "{err:#}"
        );
    }
}

#[test]
fn mutating_updates_fail_closed_when_release_check_fails() {
    assert!(release_check_failure_is_fatal(true, false, false));
    assert!(release_check_failure_is_fatal(false, true, false));
    assert!(release_check_failure_is_fatal(false, false, true));
    assert!(!release_check_failure_is_fatal(false, false, false));
}

#[test]
fn mutating_updates_defer_status_cache_until_activation() {
    assert!(!should_write_preflight_cache(true));
    assert!(should_write_preflight_cache(false));
}

#[tokio::test(flavor = "current_thread")]
async fn update_assets_rejects_corp_policy_source_programmatically() {
    let result = run_update(
        false,
        false,
        true,
        None,
        None,
        false,
        Some("https://corp.example/capsem/corp.toml"),
    )
    .await;
    let err = result.expect_err("--assets must not accept a corp policy source");
    let message = format!("{err:#}");
    assert!(
        message.contains("--assets cannot be combined with --corp"),
        "{message}"
    );
    assert!(
        message.contains("--manifest for corporate asset channels"),
        "{message}"
    );
}

fn test_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn test_blake3(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn release_health_update_check_rejects_wrong_schema() {
    let health: ReleaseChannelHealth = serde_json::from_value(serde_json::json!({
        "schema": "capsem.bad_legacy.v1",
        "updates": {
            "binary": {"current": "99.99.99"},
            "assets": {"current": "2030.0101.1"}
        }
    }))
    .unwrap();

    let err = update_check_from_release_health(
        &health,
        1718444400,
        "1.3.1782582155",
        Some("2026.0627.1"),
        None,
        &InstallLayout::MacosPkg,
        "https://release.capsem.org/assets/stable/manifest.json",
        None,
    )
    .unwrap_err();

    assert!(
        format!("{err:#}").contains("release channel legacy schema mismatch"),
        "{err:#}"
    );
}

#[test]
fn write_manifest_metadata_preserves_package_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("installed-assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
            "package_version": "1.5.1783554373",
            "packaged_at": "2026-07-10T07:20:51Z"
        })
        .to_string(),
    )
    .unwrap();

    write_manifest_metadata(
        &assets_dir,
        "https://release.capsem.org/assets/nightly/manifest.json",
    )
    .unwrap();

    let origin: serde_json::Value = serde_json::from_slice(
        &std::fs::read(assets_dir.join("manifest-metadata.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(origin["schema"], "capsem.manifest_metadata.v1");
    assert_eq!(origin["origin"], "update");
    assert_eq!(
        origin["manifest_url"],
        "https://release.capsem.org/assets/nightly/manifest.json"
    );
    assert_eq!(origin["package_version"], "1.5.1783554373");
    assert_eq!(origin["packaged_at"], "2026-07-10T07:20:51Z");
}

#[test]
fn installed_manifest_metadata_replaces_the_previous_channel_check() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let assets_dir = home.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let _assets = EnvGuard::set("CAPSEM_ASSETS_DIR", assets_dir.to_str().unwrap());
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "update",
            "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
            "checked_url": "https://release.capsem.org/assets/stable/manifest.json",
            "checked_at": 100,
            "latest_assets": "stale-assets",
            "assets_update_available": true,
            "package_version": env!("CARGO_PKG_VERSION")
        })
        .to_string(),
    )
    .unwrap();
    let manifest = test_manifest(
        env!("CARGO_PKG_VERSION"),
        "2026.0714.18",
        env!("CARGO_PKG_VERSION"),
        "2026.0714.18",
    );
    let bytes = serde_json::to_vec(&manifest).unwrap();
    std::fs::write(assets_dir.join("manifest.json"), &bytes).unwrap();
    let corp_source = "https://corp.example/capsem/manifest.json";

    write_installed_manifest_metadata(&assets_dir, corp_source, &bytes).unwrap();

    let metadata: serde_json::Value = serde_json::from_slice(
        &std::fs::read(assets_dir.join("manifest-metadata.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(metadata["manifest_url"], corp_source);
    assert_eq!(metadata["checked_url"], corp_source);
    assert_eq!(metadata["latest_assets"], "2026.0714.18");
    assert_eq!(metadata["current_assets"], "2026.0714.18");
    assert_eq!(metadata["assets_update_available"], false);
    assert_eq!(metadata["validation_status"], "valid");
    assert!(metadata["checked_at"].as_u64().unwrap() > 100);
}

#[test]
fn public_channel_switch_is_allowed_in_both_directions_and_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("installed-assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "update",
            "manifest_url": "https://release.capsem.org/assets/nightly/manifest.json",
            "channel": "nightly",
            "channel_kind": "public",
            "channel_locked": false
        })
        .to_string(),
    )
    .unwrap();

    let transition = channel_transition_for_request(&assets_dir, Some("stable"), None).unwrap();
    assert_eq!(transition, ChannelTransition::Public("stable".to_string()));
    persist_channel_transition(&assets_dir, &transition).unwrap();

    let origin = installed_manifest_metadata(&assets_dir).unwrap().unwrap();
    assert_eq!(origin["channel"], "stable");
    assert_eq!(origin["channel_kind"], "public");
    assert_eq!(origin["channel_locked"], false);
}

#[tokio::test(flavor = "current_thread")]
async fn preverified_install_payload_updates_check_state_without_rebranding_package() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("installed-assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let packaged_source = "https://release.capsem.org/assets/stable/manifest.json";
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": packaged_source,
            "channel": "stable",
            "channel_kind": "public",
            "channel_locked": false,
            "package_version": env!("CARGO_PKG_VERSION"),
            "packaged_at": "2026-08-11T00:00:00Z"
        })
        .to_string(),
    )
    .unwrap();
    let payload = serde_json::to_vec(&test_manifest(
        env!("CARGO_PKG_VERSION"),
        "2026.0811.1",
        env!("CARGO_PKG_VERSION"),
        "2026.0811.1",
    ))
    .unwrap();
    let selected_source = "http://127.0.0.1:43123/assets/nightly/manifest.json";

    let transition =
        channel_transition_for_preverified_install_payload(&assets_dir, selected_source, &payload)
            .unwrap();
    assert_eq!(transition, ChannelTransition::PreservePackageOrigin);
    install_manifest_bytes(
        &assets_dir,
        selected_source,
        &payload,
        transition.manifest_metadata_policy(),
    )
    .await
    .unwrap();

    assert_eq!(
        std::fs::read(assets_dir.join("manifest.json")).unwrap(),
        payload
    );
    let metadata = installed_manifest_metadata(&assets_dir).unwrap().unwrap();
    assert_eq!(metadata["origin"], "package");
    assert_eq!(metadata["manifest_url"], packaged_source);
    assert_eq!(metadata["channel"], "stable");
    assert_eq!(metadata["channel_kind"], "public");
    assert_eq!(metadata["channel_locked"], false);
    assert_eq!(metadata["package_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(metadata["checked_url"], selected_source);
    assert_eq!(metadata["validation_status"], "valid");
    assert_eq!(metadata["channel_hash"], channel_payload_hash(&payload));
    assert!(metadata["installed_at"].as_u64().unwrap() > 0);
    assert!(metadata["refreshed_at"].as_u64().unwrap() > 0);
}

#[test]
fn preverified_install_payload_rejects_a_different_package_version() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("installed-assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
            "package_version": env!("CARGO_PKG_VERSION")
        })
        .to_string(),
    )
    .unwrap();
    let payload = serde_json::to_vec(&test_manifest(
        "9.9.9",
        "2026.0811.1",
        "9.9.9",
        "2026.0811.1",
    ))
    .unwrap();

    let error = channel_transition_for_preverified_install_payload(
        &assets_dir,
        "http://127.0.0.1:43123/assets/nightly/manifest.json",
        &payload,
    )
    .unwrap_err();

    assert!(format!("{error:#}").contains("installed package metadata selects"));
    assert!(!assets_dir.join("manifest.json").exists());
}

#[test]
fn staged_channel_switch_records_correlated_asset_audit() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let assets_dir = home.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let previous_source = "https://release.capsem.org/assets/stable/manifest.json";
    let next_source = "https://release.capsem.org/assets/nightly/manifest.json";
    let manifest = serde_json::to_vec(&test_manifest(
        env!("CARGO_PKG_VERSION"),
        "2026.0725.1",
        env!("CARGO_PKG_VERSION"),
        "2026.0725.1",
    ))
    .unwrap();
    std::fs::write(assets_dir.join("manifest.json"), &manifest).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "update",
            "manifest_url": previous_source,
            "channel": "stable",
            "channel_kind": "public",
            "channel_locked": false,
            "package_version": env!("CARGO_PKG_VERSION")
        })
        .to_string(),
    )
    .unwrap();
    let staged_root = home.path().join("updates/candidates/nightly");
    std::fs::create_dir_all(&staged_root).unwrap();
    std::fs::write(staged_root.join("manifest.json"), &manifest).unwrap();
    let staged = StagedUpdate {
        manifest_path: staged_root.join("manifest.json"),
        installer_path: None,
        assets_dir: None,
        profiles_dir: None,
    };
    let check = UpdateCheck {
        checked_at: now_secs(),
        latest_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        update_available: false,
        binary_installer: None,
        latest_assets: Some("2026.0725.1".into()),
        current_assets: Some("2026.0725.1".into()),
        assets_update_available: false,
        assets_state: Some("published".into()),
        assets_blocked_reason: None,
        latest_profiles: None,
        current_profiles: None,
        profiles_update_available: false,
        profiles_state: None,
        profiles_blocked_reason: None,
        profile_catalog_source: None,
        profile_catalog_hash: None,
        latest_images: None,
        images_update_available: false,
        images_state: None,
        images_blocked_reason: None,
        source: Some(next_source.into()),
        channel_hash: Some(channel_payload_hash(&manifest)),
        validation_status: Some("valid".into()),
        validation_error: None,
    };

    activate_staged_update_with_asset_audit(
        home.path(),
        &assets_dir,
        &staged,
        &check,
        &ChannelTransition::Public("nightly".into()),
    )
    .unwrap();

    let rows: Vec<serde_json::Value> =
        std::fs::read_to_string(home.path().join("logs/update.log"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    assert_eq!(
        rows.iter()
            .map(|row| row["event"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["asset_update_start", "asset_update_complete"]
    );
    let complete = rows.last().unwrap();
    assert_eq!(complete["source"], next_source);
    assert_eq!(
        complete["candidate_manifest_sha256"],
        check.channel_hash.unwrap()
    );
    assert_eq!(complete["channel"], "nightly");
    assert_eq!(complete["previous"]["source"], previous_source);
    assert_eq!(complete["current"]["source"], next_source);
    assert_eq!(complete["current"]["channel"], "nightly");
}

#[test]
fn failed_staged_channel_switch_never_records_asset_completion() {
    let _lock = crate::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("CAPSEM_HOME", home.path().to_str().unwrap());
    let assets_dir = home.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let source = "https://release.capsem.org/assets/nightly/manifest.json";
    let manifest = serde_json::to_vec(&test_manifest(
        env!("CARGO_PKG_VERSION"),
        "2026.0725.1",
        env!("CARGO_PKG_VERSION"),
        "2026.0725.1",
    ))
    .unwrap();
    let staged_root = home.path().join("updates/candidates/nightly");
    std::fs::create_dir_all(&staged_root).unwrap();
    std::fs::write(staged_root.join("manifest.json"), &manifest).unwrap();
    let staged = StagedUpdate {
        manifest_path: staged_root.join("manifest.json"),
        installer_path: None,
        assets_dir: None,
        profiles_dir: None,
    };
    let mut check = cached_notice_check();
    check.source = Some(source.into());
    check.channel_hash = Some("f".repeat(64));

    let error = activate_staged_update_with_asset_audit(
        home.path(),
        &assets_dir,
        &staged,
        &check,
        &ChannelTransition::Public("nightly".into()),
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("staged manifest SHA-256 mismatch"),
        "{error:#}"
    );

    let rows: Vec<serde_json::Value> =
        std::fs::read_to_string(home.path().join("logs/update.log"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    assert_eq!(
        rows.iter()
            .map(|row| row["event"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["asset_update_start", "asset_update_failed"]
    );
    assert!(rows
        .iter()
        .all(|row| row["event"] != "asset_update_complete"));
    assert_eq!(rows.last().unwrap()["source"], source);
    assert_eq!(
        rows.last().unwrap()["candidate_manifest_sha256"],
        "f".repeat(64)
    );
}

#[test]
fn explicit_corporate_manifest_locks_channel_one_way() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("installed-assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "update",
            "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
            "channel": "stable",
            "channel_kind": "public",
            "channel_locked": false
        })
        .to_string(),
    )
    .unwrap();

    let corp_source = "https://corp.example/capsem/manifest.json";
    let transition =
        channel_transition_for_request(&assets_dir, None, Some(corp_source)).unwrap();
    assert_eq!(transition, ChannelTransition::Corporate);
    write_manifest_metadata(&assets_dir, corp_source).unwrap();
    persist_channel_transition(&assets_dir, &transition).unwrap();

    let origin = installed_manifest_metadata(&assets_dir).unwrap().unwrap();
    assert_eq!(origin["channel"], "corp");
    assert_eq!(origin["channel_kind"], "corporate");
    assert_eq!(origin["channel_locked"], true);

    let error = channel_transition_for_request(&assets_dir, Some("stable"), None).unwrap_err();
    assert!(
        format!("{error:#}").contains("corporate channel is locked"),
        "{error:#}"
    );
    assert_eq!(
        channel_transition_for_request(&assets_dir, None, Some(corp_source)).unwrap(),
        ChannelTransition::Preserve
    );
    let error = channel_transition_for_request(
        &assets_dir,
        None,
        Some("https://other-corp.example/capsem/manifest.json"),
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("corporate channel is locked to"),
        "{error:#}"
    );
}

#[test]
fn local_manifest_asset_source_uses_manifest_metadata_parent() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("installed-assets");
    let source_dir = dir.path().join("source-assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::create_dir_all(&source_dir).unwrap();
    let manifest = source_dir.join("manifest.json");
    std::fs::write(&manifest, "{}").unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": format!("file://{}", manifest.display())
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(
        local_manifest_asset_source(&assets_dir).unwrap(),
        Some(source_dir)
    );
}

#[test]
fn local_manifest_asset_source_ignores_remote_origin() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("installed-assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "https://example.invalid/manifest.json"
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(local_manifest_asset_source(&assets_dir).unwrap(), None);
}

#[test]
fn remote_manifest_asset_source_uses_remote_origin() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("installed-assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "https://release.capsem.org/assets/stable/manifest.json"
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(
        remote_manifest_asset_source(&assets_dir).unwrap(),
        Some("https://release.capsem.org/assets/stable/manifest.json".to_string())
    );
}

#[test]
fn remote_manifest_asset_source_ignores_file_origin() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("installed-assets");
    let source_dir = dir.path().join("source-assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::create_dir_all(&source_dir).unwrap();
    let manifest = source_dir.join("manifest.json");
    std::fs::write(&manifest, "{}").unwrap();
    let source = format!("file://{}", manifest.display());
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": source
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(remote_manifest_asset_source(&assets_dir).unwrap(), None);
}

#[test]
fn local_manifest_asset_source_rejects_bare_paths() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("installed-assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "/tmp/corp/assets/stable/manifest.json"
        })
        .to_string(),
    )
    .unwrap();

    let err = local_manifest_asset_source(&assets_dir).unwrap_err();
    assert!(
        format!("{err:#}").contains("asset manifest metadata source must be a URL"),
        "{err:#}"
    );
}

#[test]
fn local_manifest_asset_source_rejects_file_url_shorthand_paths() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("installed-assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "file:assets/stable/manifest.json"
        })
        .to_string(),
    )
    .unwrap();

    let err = local_manifest_asset_source(&assets_dir).unwrap_err();
    assert!(
        format!("{err:#}").contains("asset manifest metadata file URL must start with file://"),
        "{err:#}"
    );
}

/// A `file://` channel's root is its dist directory, not the filesystem root.
///
/// The generated release channel is a website: the manifest sits at
/// `<dist>/assets/<channel>/manifest.json` and its artifacts are recorded
/// site-root-relative, as `/profiles/releases/...`. Served over HTTP that is
/// exactly right -- the site root is the origin.
///
/// Resolved against a `file://` manifest, `set_path` replaced the whole path
/// and produced `file:///profiles/releases/...`, which is the filesystem root.
/// The gate's install proof hands the postinst a locally built channel, so
/// every hydration of it failed with ENOENT -- and the `apt-get install -f`
/// retry then fell back to the public channel, where the real error was
/// reported against a URL nobody had asked for.
#[test]
fn root_relative_artifacts_resolve_against_a_file_channels_dist_root() {
    let manifest = "file:///src/target/install-test-channel/assets/local/manifest.json";

    let resolved = super::resolve_release_channel_artifact_url(
        manifest,
        "/profiles/releases/local/co-work/0.6.0/arm64/initrd.img",
    )
    .expect("resolve");

    assert_eq!(
        resolved,
        "file:///src/target/install-test-channel/profiles/releases/local/co-work/0.6.0/arm64/initrd.img"
    );
}

/// And an http channel still resolves against its origin, where the site root
/// and the filesystem root are the same thing.
#[test]
fn root_relative_artifacts_resolve_against_an_http_origin() {
    let resolved = super::resolve_release_channel_artifact_url(
        "https://release.capsem.org/assets/stable/manifest.json",
        "/profiles/releases/stable/code/0.6.0/arm64/initrd.img",
    )
    .expect("resolve");

    assert_eq!(
        resolved,
        "https://release.capsem.org/profiles/releases/stable/code/0.6.0/arm64/initrd.img"
    );
}

/// A relative reference keeps resolving against the manifest itself, and an
/// absolute URL is still taken as given.
#[test]
fn relative_and_absolute_artifact_references_are_unchanged() {
    let manifest = "file:///src/target/install-test-channel/assets/local/manifest.json";

    assert_eq!(
        super::resolve_release_channel_artifact_url(manifest, "health.json").expect("relative"),
        "file:///src/target/install-test-channel/assets/local/health.json"
    );
    assert_eq!(
        super::resolve_release_channel_artifact_url(manifest, "https://example.test/a.img")
            .expect("absolute"),
        "https://example.test/a.img"
    );
}

// ---------------------------------------------------------------------------
// What a rejected manifest URL says.
//
// A release glow-up served `http://127.0.0.1:33029/transitions/current/manifest.json`
// and the service reported
//
//     manifest_url must be an http(s) channel manifest URL, got http://...
//
// of a URL that is plainly http. The requirement it actually enforces -- a
// path with an `assets` segment, a channel after it, ending in
// `manifest.json` -- appeared nowhere, so diagnosing it cost a full CI cycle
// and a read of this file. A rejection that does not name what it wanted is a
// rejection that has to be reverse-engineered.
// ---------------------------------------------------------------------------

#[test]
fn a_url_that_is_not_channel_shaped_says_what_was_expected() {
    let error = channel_manifest_url("http://127.0.0.1:33029/transitions/current/manifest.json")
        .expect_err("a path with no assets segment is not a channel manifest");
    let message = error.to_string();
    assert!(
        message.contains("assets"),
        "the rejection must name the segment it wanted: {message}"
    );
    assert!(
        !message.contains("http(s)"),
        "the URL is http; blaming the scheme sends the reader to the wrong \
         end of the problem: {message}"
    );
}

#[test]
fn a_url_with_the_wrong_scheme_blames_the_scheme() {
    let error = channel_manifest_url("file:///tmp/assets/stable/manifest.json")
        .expect_err("file:// is not fetchable");
    assert!(error.to_string().contains("file"), "{error}");
}

#[test]
fn a_url_not_ending_in_the_manifest_says_so() {
    let error = channel_manifest_url("https://release.capsem.org/assets/stable/")
        .expect_err("a directory is not a manifest");
    assert!(error.to_string().contains("manifest.json"), "{error}");
}

#[test]
fn a_channel_shaped_url_is_accepted_unchanged() {
    for url in [
        "https://release.capsem.org/assets/stable/manifest.json",
        "https://corp.example/capsem/assets/internal/manifest.json",
        "http://127.0.0.1:33029/assets/current/manifest.json",
    ] {
        assert_eq!(channel_manifest_url(url).expect("channel shaped"), url);
    }
}
