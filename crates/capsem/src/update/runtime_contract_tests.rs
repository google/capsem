use super::*;

#[test]
fn package_preactivation_preserves_the_channel_declared_by_the_candidate_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let assets_dir = temp.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "https://release.capsem.org/assets/nightly/manifest.json",
            "channel": "nightly",
            "channel_kind": "public",
            "channel_locked": false,
        }))
        .unwrap(),
    )
    .unwrap();
    let candidate = serde_json::to_vec(&serde_json::json!({
        "version": "1.6.1785192352",
        "channel": "nightly",
        "status": "current",
        "packages": [],
        "profiles": {},
    }))
    .unwrap();

    let transition = channel_transition_for_explicit_manifest_payload(
        &assets_dir,
        "file:///tmp/binary-channel/nightly/manifest.json",
        &candidate,
    )
    .unwrap();

    assert_eq!(transition, ChannelTransition::Preserve);
}

#[test]
fn preserving_a_candidate_channel_leaves_metadata_the_release_gate_accepts() {
    // The published release gate reads manifest-metadata.json back and fails on
    // `channel is 'corp', expected 'nightly'`. Returning Preserve is only half
    // the contract; persisting it must leave the packaged public channel
    // untouched, or every candidate install re-brands itself as corporate.
    let temp = tempfile::tempdir().unwrap();
    let assets_dir = temp.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "https://release.capsem.org/assets/nightly/manifest.json",
            "channel": "nightly",
            "channel_kind": "public",
            "channel_locked": false,
        }))
        .unwrap(),
    )
    .unwrap();
    let candidate = serde_json::to_vec(&serde_json::json!({
        "version": "1.6.1785192352",
        "channel": "nightly",
        "status": "current",
        "packages": [],
        "profiles": {},
    }))
    .unwrap();

    let transition = channel_transition_for_explicit_manifest_payload(
        &assets_dir,
        "file:///tmp/binary-channel/nightly/manifest.json",
        &candidate,
    )
    .unwrap();
    assert_eq!(transition, ChannelTransition::Preserve);

    persist_channel_transition(&assets_dir, &transition).unwrap();

    let metadata = installed_manifest_metadata(&assets_dir).unwrap().unwrap();
    assert_eq!(metadata["channel"], "nightly");
    assert_eq!(metadata["channel_kind"], "public");
    assert_eq!(metadata["channel_locked"], false);
}

#[test]
fn explicit_manifest_without_the_packaged_public_channel_remains_corporate() {
    let temp = tempfile::tempdir().unwrap();
    let assets_dir = temp.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "https://release.capsem.org/assets/nightly/manifest.json",
            "channel": "nightly",
            "channel_kind": "public",
            "channel_locked": false,
        }))
        .unwrap(),
    )
    .unwrap();

    for candidate in [
        serde_json::json!({"version": "1", "packages": [], "profiles": {}}),
        serde_json::json!({
            "version": "1",
            "channel": "stable",
            "packages": [],
            "profiles": {},
        }),
    ] {
        let transition = channel_transition_for_explicit_manifest_payload(
            &assets_dir,
            "file:///tmp/corporate/manifest.json",
            &serde_json::to_vec(&candidate).unwrap(),
        )
        .unwrap();
        assert_eq!(transition, ChannelTransition::Corporate);
    }
}

#[test]
fn explicit_manifest_rejects_a_non_string_declared_channel() {
    let temp = tempfile::tempdir().unwrap();
    let assets_dir = temp.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let candidate = serde_json::to_vec(&serde_json::json!({
        "version": "1",
        "channel": ["nightly"],
        "packages": [],
        "profiles": {},
    }))
    .unwrap();

    let error = channel_transition_for_explicit_manifest_payload(
        &assets_dir,
        "file:///tmp/binary-channel/nightly/manifest.json",
        &candidate,
    )
    .expect_err("a malformed manifest channel must fail closed");

    assert!(
        format!("{error:#}").contains("release manifest channel must be a string"),
        "{error:#}"
    );
}

#[test]
fn shared_release_payload_parser_rejects_missing_runtime_image_revision() {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    let image = |kind: &str, name: &str| {
        serde_json::json!({
            "kind": kind,
            "name": name,
            "url": format!("https://release.capsem.org/assets/releases/2030.0101.1/{arch}-{name}"),
            "bytes": 1,
            "digest": {
                "sha256": "1".repeat(64),
                "blake3": "2".repeat(64),
            },
            "status": "current",
        })
    };
    let body = serde_json::to_vec(&serde_json::json!({
        "version": "1.0.143",
        "channel": "stable",
        "status": "current",
        "packages": [],
        "profiles": {
            "code": {
                "revision": "2030.0101.1",
                "status": "current",
                "architectures": [{
                    "architecture": arch,
                    "images": [
                        image("kernel", "vmlinuz"),
                        image("initrd", "initrd.img"),
                        image("rootfs", "rootfs.erofs"),
                    ]
                }]
            }
        }
    }))
    .unwrap();

    let error = update_check_from_release_payload(
        &body,
        &InstallLayout::UserDir,
        "https://release.capsem.org/assets/stable/manifest.json",
        None,
    )
    .expect_err("an update-checkable graph must also be bootable by the runtime parser");

    assert!(
        format!("{error:#}").contains("missing image_revision"),
        "unexpected error: {error:#}"
    );
}

fn update_plan_check(binary: bool, profiles: bool, assets: bool, images: bool) -> UpdateCheck {
    UpdateCheck {
        checked_at: 1,
        latest_version: Some(if binary { "2.0.0" } else { "1.0.0" }.to_string()),
        update_available: binary,
        binary_installer: binary.then(|| BinaryInstaller {
            name: "Capsem_2.0.0_amd64.deb".to_string(),
            url: "https://release.capsem.org/Capsem_2.0.0_amd64.deb".to_string(),
            sha256: "1".repeat(64),
            blake3: "2".repeat(64),
            size: 1,
            install_layout: "linux_deb".to_string(),
        }),
        latest_assets: Some(if assets { "images-2" } else { "images-1" }.to_string()),
        current_assets: Some("images-1".to_string()),
        assets_update_available: assets,
        assets_state: Some("current".to_string()),
        assets_blocked_reason: None,
        latest_profiles: Some(if profiles { "profiles-2" } else { "profiles-1" }.to_string()),
        current_profiles: Some("profiles-1".to_string()),
        profiles_update_available: profiles,
        profiles_state: Some("current".to_string()),
        profiles_blocked_reason: None,
        profile_catalog_source: None,
        profile_catalog_hash: None,
        latest_images: Some(if images { "images-2" } else { "images-1" }.to_string()),
        images_update_available: images,
        images_state: Some("current".to_string()),
        images_blocked_reason: None,
        source: Some("https://release.capsem.org/assets/stable/manifest.json".to_string()),
        channel_hash: Some("2".repeat(64)),
        validation_status: Some("valid".to_string()),
        validation_error: None,
    }
}

fn update_plan_graph(min_capsem_version: &str, package_version: &str) -> Vec<u8> {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    let package_arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    let image = |kind: &str, name: &str| {
        serde_json::json!({
            "kind": kind,
            "name": name,
            "url": format!("https://release.capsem.org/assets/releases/images-2/{arch}-{name}"),
            "bytes": 1,
            "digest": {
                "sha256": "3".repeat(64),
                "blake3": "4".repeat(64),
            },
            "status": "current",
        })
    };
    serde_json::to_vec(&serde_json::json!({
        "version": "1.0.0",
        "channel": "stable",
        "status": "current",
        "packages": [{
            "name": format!("Capsem_{package_version}_{package_arch}.deb"),
            "url": format!("https://release.capsem.org/Capsem_{package_version}_{package_arch}.deb"),
            "version": package_version,
            "kind": "debian_package",
            "platform": "linux",
            "architecture": package_arch,
            "status": "current",
            "bytes": 1,
            "digest": {
                "sha256": "1".repeat(64),
                "blake3": "2".repeat(64),
            }
        }],
        "profiles": {
            "code": {
                "revision": "profiles-2",
                "status": "current",
                "min_capsem_version": min_capsem_version,
                "architectures": [{
                    "architecture": arch,
                    "image_revision": "images-2",
                    "images": [
                        image("kernel", "vmlinuz"),
                        image("initrd", "initrd.img"),
                        image("rootfs", "rootfs.erofs"),
                    ]
                }]
            }
        }
    }))
    .unwrap()
}

fn plan_test_update(mut check: UpdateCheck, body: &[u8], installed_binary: &str) -> Result<VerifiedUpdatePlan> {
    check.channel_hash = Some(channel_payload_hash(body));
    plan_verified_update(&check, body, installed_binary)
}

#[test]
fn complete_update_plan_keeps_binary_and_profiles_orthogonal() {
    let binary_body = update_plan_graph("1.0.0", "2.0.0");
    let binary = plan_test_update(update_plan_check(true, false, false, false), &binary_body, "1.0.0").unwrap();
    assert_eq!(binary.steps, vec![UpdatePlanStep::Binary]);

    let profile_body = update_plan_graph("1.0.0", "1.0.0");
    let profile = plan_test_update(update_plan_check(false, true, true, true), &profile_body, "1.0.0").unwrap();
    assert_eq!(profile.steps, vec![UpdatePlanStep::Profiles]);
}

#[test]
fn complete_update_plan_orders_binary_before_profiles() {
    let body = update_plan_graph("2.0.0", "2.0.0");
    let plan = plan_test_update(update_plan_check(true, true, true, true), &body, "1.0.0").unwrap();

    assert_eq!(plan.steps, vec![UpdatePlanStep::Binary, UpdatePlanStep::Profiles]);
    assert_eq!(plan.installed_binary, "1.0.0");
    assert_eq!(plan.selected_binary, "2.0.0");
}

#[test]
fn complete_update_plan_rejects_profile_incompatible_with_selected_binary() {
    let body = update_plan_graph("3.0.0", "2.0.0");
    let error = plan_test_update(update_plan_check(true, true, true, true), &body, "1.0.0")
        .expect_err("the selected binary must satisfy every selected profile");

    assert!(
        format!("{error:#}").contains("profile code requires Capsem 3.0.0 or newer"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn complete_update_plan_requires_a_verified_installer_for_binary_change() {
    let mut check = update_plan_check(true, false, false, false);
    check.binary_installer = None;
    let body = update_plan_graph("1.0.0", "2.0.0");
    let error = plan_test_update(check, &body, "1.0.0")
        .expect_err("a binary update without a matching native package must fail closed");

    assert!(
        format!("{error:#}").contains("no verified installer"),
        "unexpected error: {error:#}"
    );
}

fn staged_profile_fixture(release_dir: &Path, corrupt_rootfs: bool) -> (Vec<u8>, String, Vec<u8>) {
    std::fs::create_dir_all(release_dir).unwrap();
    let profile = br#"id = "code"
name = "Code"
description = "Staged code profile"
revision = "profiles-2"
refresh_policy = "manual"

[assets]
format = "profile-assets.v1"
refresh_policy = "manual"

[assets.arch.arm64.kernel]
name = "vmlinuz"
url = "https://release.capsem.org/assets/releases/images-2/arm64-vmlinuz"

[assets.arch.arm64.initrd]
name = "initrd.img"
url = "https://release.capsem.org/assets/releases/images-2/arm64-initrd.img"

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "https://release.capsem.org/assets/releases/images-2/arm64-rootfs.erofs"

[assets.arch.x86_64.kernel]
name = "vmlinuz"
url = "https://release.capsem.org/assets/releases/images-2/x86_64-vmlinuz"

[assets.arch.x86_64.initrd]
name = "initrd.img"
url = "https://release.capsem.org/assets/releases/images-2/x86_64-initrd.img"

[assets.arch.x86_64.rootfs]
name = "rootfs.erofs"
url = "https://release.capsem.org/assets/releases/images-2/x86_64-rootfs.erofs"
"#
    .to_vec();
    let kernel = b"verified-kernel".to_vec();
    let initrd = b"verified-initrd".to_vec();
    let rootfs = b"verified-rootfs".to_vec();
    std::fs::write(release_dir.join("profile.toml"), &profile).unwrap();
    std::fs::write(release_dir.join("vmlinuz"), &kernel).unwrap();
    std::fs::write(release_dir.join("initrd.img"), &initrd).unwrap();
    std::fs::write(
        release_dir.join("rootfs.erofs"),
        if corrupt_rootfs {
            b"corrupt-rootfs"
        } else {
            rootfs.as_slice()
        },
    )
    .unwrap();

    let digest = |bytes: &[u8]| {
        serde_json::json!({
            "sha256": sha256_hex(bytes),
            "blake3": blake3::hash(bytes).to_hex().to_string(),
        })
    };
    let artifact = |kind: &str, name: &str, bytes: &[u8]| {
        serde_json::json!({
            "kind": kind,
            "name": name,
            "url": name,
            "bytes": bytes.len(),
            "digest": digest(bytes),
            "status": "current",
        })
    };
    let arch = capsem_assets::asset_manager::host_manifest_arch();
    let body = serde_json::to_vec(&serde_json::json!({
        "version": "1.0.0",
        "channel": "stable",
        "status": "current",
        "packages": [],
        "profiles": {
            "code": {
                "revision": "profiles-2",
                "status": "current",
                "min_capsem_version": env!("CARGO_PKG_VERSION"),
                "architectures": [{
                    "architecture": arch,
                    "image_revision": "images-2",
                    "config": [{
                        "kind": "profile",
                        "path": "profiles/code/profile.toml",
                        "url": "profile.toml",
                        "bytes": profile.len(),
                        "digest": digest(&profile),
                        "status": "current",
                    }],
                    "images": [
                        artifact("kernel", "vmlinuz", &kernel),
                        artifact("initrd", "initrd.img", &initrd),
                        artifact("rootfs", "rootfs.erofs", &rootfs),
                    ]
                }]
            }
        }
    }))
    .unwrap();
    let manifest_path = release_dir.join("manifest.json");
    std::fs::write(&manifest_path, &body).unwrap();
    let source = reqwest::Url::from_file_path(&manifest_path).unwrap().to_string();
    (body, source, kernel)
}

fn profile_stage_plan() -> VerifiedUpdatePlan {
    VerifiedUpdatePlan {
        installed_binary: env!("CARGO_PKG_VERSION").to_string(),
        selected_binary: env!("CARGO_PKG_VERSION").to_string(),
        steps: vec![UpdatePlanStep::Profiles],
    }
}

fn assert_profile_uses_release_manifest_pins(profile_path: &Path, release_dir: &Path) {
    let profile: toml::Value = toml::from_str(&std::fs::read_to_string(profile_path).unwrap()).unwrap();
    let arch = capsem_assets::asset_manager::host_manifest_arch();
    let assets = &profile["assets"]["arch"][arch];
    for (kind, name) in [
        ("kernel", "vmlinuz"),
        ("initrd", "initrd.img"),
        ("rootfs", "rootfs.erofs"),
    ] {
        let bytes = std::fs::read(release_dir.join(name)).unwrap();
        assert_eq!(assets[kind]["name"].as_str(), Some(name));
        assert_eq!(
            assets[kind]["url"].as_str(),
            Some(
                reqwest::Url::from_file_path(release_dir.join(name))
                    .unwrap()
                    .to_string()
                    .as_str()
            )
        );
        assert_eq!(
            assets[kind]["hash"].as_str(),
            Some(format!("blake3:{}", blake3::hash(&bytes).to_hex()).as_str())
        );
        assert_eq!(
            assets[kind]["size"].as_integer(),
            Some(i64::try_from(bytes.len()).unwrap())
        );
    }
}

#[tokio::test]
async fn stage_verified_update_downloads_every_profile_artifact_without_mutating_install() {
    let temp = tempfile::tempdir().unwrap();
    let capsem_home = temp.path().join("home");
    let release_dir = temp.path().join("release");
    let (body, source, kernel) = staged_profile_fixture(&release_dir, false);
    let installed_manifest = capsem_home.join("assets/manifest.json");
    let installed_profile = capsem_home.join("profiles/code/profile.toml");
    std::fs::create_dir_all(installed_manifest.parent().unwrap()).unwrap();
    std::fs::create_dir_all(installed_profile.parent().unwrap()).unwrap();
    std::fs::write(&installed_manifest, b"installed-manifest").unwrap();
    std::fs::write(&installed_profile, b"installed-profile").unwrap();

    let mut check = update_plan_check(false, true, true, true);
    check.latest_version = Some(env!("CARGO_PKG_VERSION").to_string());
    check.source = Some(source);
    check.channel_hash = Some(channel_payload_hash(&body));
    let staged = stage_verified_update_at(&capsem_home, &profile_stage_plan(), &check, &body)
        .await
        .unwrap();

    assert_eq!(std::fs::read(&installed_manifest).unwrap(), b"installed-manifest");
    assert_eq!(std::fs::read(&installed_profile).unwrap(), b"installed-profile");
    assert_eq!(std::fs::read(&staged.manifest_path).unwrap(), body);
    assert_eq!(
        std::fs::read(
            staged
                .assets_dir
                .as_ref()
                .unwrap()
                .join(capsem_assets::asset_manager::host_manifest_arch())
                .join(capsem_assets::asset_manager::hash_filename(
                    "vmlinuz",
                    blake3::hash(&kernel).to_hex().as_ref(),
                )),
        )
        .unwrap(),
        kernel
    );
    assert_profile_uses_release_manifest_pins(
        &staged.profiles_dir.as_ref().unwrap().join("code/profile.toml"),
        &release_dir,
    );
    assert!(staged.installer_path.is_none());
}

#[tokio::test]
async fn stage_verified_update_rejects_corruption_before_candidate_or_install_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let capsem_home = temp.path().join("home");
    let release_dir = temp.path().join("release");
    let (body, source, _) = staged_profile_fixture(&release_dir, true);
    let installed_manifest = capsem_home.join("assets/manifest.json");
    std::fs::create_dir_all(installed_manifest.parent().unwrap()).unwrap();
    std::fs::write(&installed_manifest, b"installed-manifest").unwrap();

    let mut check = update_plan_check(false, true, true, true);
    check.latest_version = Some(env!("CARGO_PKG_VERSION").to_string());
    check.source = Some(source);
    check.channel_hash = Some(channel_payload_hash(&body));
    let error = stage_verified_update_at(&capsem_home, &profile_stage_plan(), &check, &body)
        .await
        .expect_err("corrupt profile bytes must fail before activation");

    assert!(format!("{error:#}").contains("mismatch"), "{error:#}");
    assert_eq!(std::fs::read(&installed_manifest).unwrap(), b"installed-manifest");
    assert!(
        !capsem_home
            .join("updates/candidates")
            .join(channel_payload_hash(&body))
            .exists(),
        "a failed stage must not leave a complete candidate identity"
    );
}

#[tokio::test]
async fn activate_staged_update_switches_profiles_assets_and_manifest_together() {
    let temp = tempfile::tempdir().unwrap();
    let capsem_home = temp.path().join("home");
    let release_dir = temp.path().join("release");
    let (body, source, kernel) = staged_profile_fixture(&release_dir, false);
    let installed_assets = capsem_home.join("assets");
    let installed_manifest = installed_assets.join("manifest.json");
    let installed_profile = capsem_home.join("profiles/code/profile.toml");
    std::fs::create_dir_all(&installed_assets).unwrap();
    std::fs::create_dir_all(installed_profile.parent().unwrap()).unwrap();
    std::fs::write(&installed_manifest, b"installed-manifest").unwrap();
    std::fs::write(
        installed_assets.join("manifest-metadata.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "manifest_url": "https://release.capsem.org/assets/stable/old.json",
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(&installed_profile, b"installed-profile").unwrap();

    let mut check = update_plan_check(false, true, true, true);
    check.latest_version = Some(env!("CARGO_PKG_VERSION").to_string());
    check.source = Some(source.clone());
    check.channel_hash = Some(channel_payload_hash(&body));
    let staged = stage_verified_update_at(&capsem_home, &profile_stage_plan(), &check, &body)
        .await
        .unwrap();

    activate_staged_update_at(
        &capsem_home,
        &installed_assets,
        &staged,
        &check,
        &ChannelTransition::Preserve,
    )
    .unwrap();

    assert_eq!(std::fs::read(&installed_manifest).unwrap(), body);
    assert_profile_uses_release_manifest_pins(&installed_profile, &release_dir);
    assert_eq!(
        std::fs::read(
            installed_assets
                .join(capsem_assets::asset_manager::host_manifest_arch())
                .join(capsem_assets::asset_manager::hash_filename(
                    "vmlinuz",
                    blake3::hash(&kernel).to_hex().as_ref(),
                )),
        )
        .unwrap(),
        kernel
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(installed_assets.join("manifest-metadata.json")).unwrap()).unwrap();
    assert_eq!(metadata["manifest_url"], source);
    assert_eq!(metadata["validation_status"], "valid");
}

#[tokio::test]
async fn activate_staged_update_rolls_back_every_selected_path_on_manifest_failure() {
    let temp = tempfile::tempdir().unwrap();
    let capsem_home = temp.path().join("home");
    let release_dir = temp.path().join("release");
    let (body, source, kernel) = staged_profile_fixture(&release_dir, false);
    let installed_assets = capsem_home.join("assets");
    let installed_manifest = installed_assets.join("manifest.json");
    let installed_metadata = installed_assets.join("manifest-metadata.json");
    let installed_profile = capsem_home.join("profiles/code/profile.toml");
    std::fs::create_dir_all(&installed_assets).unwrap();
    std::fs::create_dir_all(installed_profile.parent().unwrap()).unwrap();
    std::fs::write(&installed_manifest, b"installed-manifest").unwrap();
    std::fs::write(&installed_metadata, b"installed-metadata").unwrap();
    std::fs::write(&installed_profile, b"installed-profile").unwrap();

    let mut check = update_plan_check(false, true, true, true);
    check.latest_version = Some(env!("CARGO_PKG_VERSION").to_string());
    check.source = Some(source);
    check.channel_hash = Some(channel_payload_hash(&body));
    let staged = stage_verified_update_at(&capsem_home, &profile_stage_plan(), &check, &body)
        .await
        .unwrap();
    let staged_kernel = staged
        .assets_dir
        .as_ref()
        .unwrap()
        .join(capsem_assets::asset_manager::host_manifest_arch())
        .join(capsem_assets::asset_manager::hash_filename(
            "vmlinuz",
            blake3::hash(&kernel).to_hex().as_ref(),
        ));
    let installed_kernel =
        installed_assets.join(staged_kernel.strip_prefix(staged.assets_dir.as_ref().unwrap()).unwrap());
    std::fs::create_dir(installed_assets.join("manifest.tmp")).unwrap();

    let error = activate_staged_update_at(
        &capsem_home,
        &installed_assets,
        &staged,
        &check,
        &ChannelTransition::Preserve,
    )
    .expect_err("manifest activation failure must roll the profile transaction back");

    assert!(format!("{error:#}").contains("manifest.tmp"), "{error:#}");
    assert_eq!(std::fs::read(&installed_manifest).unwrap(), b"installed-manifest");
    assert_eq!(std::fs::read(&installed_metadata).unwrap(), b"installed-metadata");
    assert_eq!(std::fs::read(&installed_profile).unwrap(), b"installed-profile");
    assert!(
        !installed_kernel.exists(),
        "new content-addressed assets must be removed on rollback"
    );
}
