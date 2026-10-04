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
        serde_json::json!({"version": "1", "packages": []}),
        serde_json::json!({
            "version": "1",
            "channel": "stable",
            "packages": [],
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
            "url": format!("https://release.capsem.org/runtime/releases/stable/2030.0101.1/{arch}/{name}"),
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
        "runtime": {
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

fn update_plan_check(binary: bool, assets: bool, images: bool) -> UpdateCheck {
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
        latest_assets: Some(if assets { "runtime-2" } else { "runtime-1" }.to_string()),
        current_assets: Some("runtime-1".to_string()),
        assets_update_available: assets,
        assets_state: Some("current".to_string()),
        assets_blocked_reason: None,
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
            "url": format!("https://release.capsem.org/runtime/releases/stable/runtime-2/{arch}/{name}"),
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
        "runtime": {
            "revision": "runtime-2",
            "status": "current",
            "min_capsem_version": min_capsem_version,
            "architectures": [{
                "architecture": arch,
                "image_revision": "runtime-2",
                "images": [
                    image("kernel", "vmlinuz"),
                    image("initrd", "initrd.img"),
                    image("rootfs", "rootfs.erofs"),
                ]
            }]
        }
    }))
    .unwrap()
}

fn plan_test_update(mut check: UpdateCheck, body: &[u8], installed_binary: &str) -> Result<VerifiedUpdatePlan> {
    check.channel_hash = Some(channel_payload_hash(body));
    plan_verified_update(&check, body, installed_binary)
}

#[test]
fn complete_update_plan_keeps_binary_and_runtime_orthogonal() {
    let binary_body = update_plan_graph("1.0.0", "2.0.0");
    let binary = plan_test_update(update_plan_check(true, false, false), &binary_body, "1.0.0").unwrap();
    assert_eq!(binary.steps, vec![UpdatePlanStep::Binary]);

    let runtime_body = update_plan_graph("1.0.0", "1.0.0");
    let runtime = plan_test_update(update_plan_check(false, true, true), &runtime_body, "1.0.0").unwrap();
    assert_eq!(runtime.steps, vec![UpdatePlanStep::Runtime]);
}

#[test]
fn complete_update_plan_orders_binary_before_runtime() {
    let body = update_plan_graph("2.0.0", "2.0.0");
    let plan = plan_test_update(update_plan_check(true, true, true), &body, "1.0.0").unwrap();

    assert_eq!(plan.steps, vec![UpdatePlanStep::Binary, UpdatePlanStep::Runtime]);
    assert_eq!(plan.installed_binary, "1.0.0");
    assert_eq!(plan.selected_binary, "2.0.0");
}

#[test]
fn complete_update_plan_rejects_a_runtime_incompatible_with_the_selected_binary() {
    let body = update_plan_graph("3.0.0", "2.0.0");
    let error = plan_test_update(update_plan_check(true, true, true), &body, "1.0.0")
        .expect_err("the selected binary must satisfy the selected runtime");

    assert!(
        format!("{error:#}").contains("runtime runtime-2 requires Capsem 3.0.0 or newer"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn complete_update_plan_requires_a_verified_installer_for_binary_change() {
    let mut check = update_plan_check(true, false, false);
    check.binary_installer = None;
    let body = update_plan_graph("1.0.0", "2.0.0");
    let error = plan_test_update(check, &body, "1.0.0")
        .expect_err("a binary update without a matching native package must fail closed");

    assert!(
        format!("{error:#}").contains("no verified installer"),
        "unexpected error: {error:#}"
    );
}

fn staged_runtime_fixture(release_dir: &Path, corrupt_rootfs: bool) -> (Vec<u8>, String, Vec<u8>) {
    std::fs::create_dir_all(release_dir).unwrap();
    let kernel = b"verified-kernel".to_vec();
    let initrd = b"verified-initrd".to_vec();
    let rootfs = b"verified-rootfs".to_vec();
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
        "runtime": {
            "revision": "runtime-2",
            "status": "current",
            "min_capsem_version": env!("CARGO_PKG_VERSION"),
            "architectures": [{
                "architecture": arch,
                "image_revision": "runtime-2",
                "images": [
                    artifact("kernel", "vmlinuz", &kernel),
                    artifact("initrd", "initrd.img", &initrd),
                    artifact("rootfs", "rootfs.erofs", &rootfs),
                ]
            }]
        }
    }))
    .unwrap();
    let manifest_path = release_dir.join("manifest.json");
    std::fs::write(&manifest_path, &body).unwrap();
    let source = reqwest::Url::from_file_path(&manifest_path).unwrap().to_string();
    (body, source, kernel)
}

fn runtime_stage_plan() -> VerifiedUpdatePlan {
    VerifiedUpdatePlan {
        installed_binary: env!("CARGO_PKG_VERSION").to_string(),
        selected_binary: env!("CARGO_PKG_VERSION").to_string(),
        steps: vec![UpdatePlanStep::Runtime],
    }
}

fn staged_runtime_check(source: String, body: &[u8]) -> UpdateCheck {
    let mut check = update_plan_check(false, true, true);
    check.latest_version = Some(env!("CARGO_PKG_VERSION").to_string());
    check.source = Some(source);
    check.channel_hash = Some(channel_payload_hash(body));
    check
}

#[tokio::test]
async fn stage_verified_update_downloads_every_runtime_image_without_mutating_install() {
    let temp = tempfile::tempdir().unwrap();
    let capsem_home = temp.path().join("home");
    let release_dir = temp.path().join("release");
    let (body, source, kernel) = staged_runtime_fixture(&release_dir, false);
    let installed_manifest = capsem_home.join("assets/manifest.json");
    std::fs::create_dir_all(installed_manifest.parent().unwrap()).unwrap();
    std::fs::write(&installed_manifest, b"installed-manifest").unwrap();

    let check = staged_runtime_check(source, &body);
    let staged = stage_verified_update_at(&capsem_home, &runtime_stage_plan(), &check, &body)
        .await
        .unwrap();

    assert_eq!(std::fs::read(&installed_manifest).unwrap(), b"installed-manifest");
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
    assert!(staged.installer_path.is_none());
}

#[tokio::test]
async fn stage_verified_update_rejects_corruption_before_candidate_or_install_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let capsem_home = temp.path().join("home");
    let release_dir = temp.path().join("release");
    let (body, source, _) = staged_runtime_fixture(&release_dir, true);
    let installed_manifest = capsem_home.join("assets/manifest.json");
    std::fs::create_dir_all(installed_manifest.parent().unwrap()).unwrap();
    std::fs::write(&installed_manifest, b"installed-manifest").unwrap();

    let check = staged_runtime_check(source, &body);
    let error = stage_verified_update_at(&capsem_home, &runtime_stage_plan(), &check, &body)
        .await
        .expect_err("corrupt runtime bytes must fail before activation");

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
async fn activate_staged_update_switches_runtime_assets_and_manifest_together() {
    let temp = tempfile::tempdir().unwrap();
    let capsem_home = temp.path().join("home");
    let release_dir = temp.path().join("release");
    let (body, source, kernel) = staged_runtime_fixture(&release_dir, false);
    let installed_assets = capsem_home.join("assets");
    let installed_manifest = installed_assets.join("manifest.json");
    std::fs::create_dir_all(&installed_assets).unwrap();
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

    let check = staged_runtime_check(source.clone(), &body);
    let staged = stage_verified_update_at(&capsem_home, &runtime_stage_plan(), &check, &body)
        .await
        .unwrap();

    activate_staged_update_at(&installed_assets, &staged, &check, &ChannelTransition::Preserve).unwrap();

    assert_eq!(std::fs::read(&installed_manifest).unwrap(), body);
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
    let (body, source, kernel) = staged_runtime_fixture(&release_dir, false);
    let installed_assets = capsem_home.join("assets");
    let installed_manifest = installed_assets.join("manifest.json");
    let installed_metadata = installed_assets.join("manifest-metadata.json");
    std::fs::create_dir_all(&installed_assets).unwrap();
    std::fs::write(&installed_manifest, b"installed-manifest").unwrap();
    std::fs::write(&installed_metadata, b"installed-metadata").unwrap();

    let check = staged_runtime_check(source, &body);
    let staged = stage_verified_update_at(&capsem_home, &runtime_stage_plan(), &check, &body)
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

    let error = activate_staged_update_at(&installed_assets, &staged, &check, &ChannelTransition::Preserve)
        .expect_err("manifest activation failure must roll the runtime transaction back");

    assert!(format!("{error:#}").contains("manifest.tmp"), "{error:#}");
    assert_eq!(std::fs::read(&installed_manifest).unwrap(), b"installed-manifest");
    assert_eq!(std::fs::read(&installed_metadata).unwrap(), b"installed-metadata");
    assert!(
        !installed_kernel.exists(),
        "new content-addressed assets must be removed on rollback"
    );
}

#[tokio::test]
async fn a_binary_only_graph_installs_its_manifest_and_no_assets() {
    let temp = tempfile::tempdir().unwrap();
    let assets_dir = temp.path().join("assets");
    let body = serde_json::to_vec(&serde_json::json!({
        "version": "1.0.0",
        "channel": "stable",
        "status": "current",
        "packages": [],
    }))
    .unwrap();

    install_manifest_bytes(
        &assets_dir,
        "https://release.capsem.org/assets/stable/manifest.json",
        &body,
        ManifestMetadataPolicy::RecordSource,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(assets_dir.join("manifest.json")).unwrap(), body);
    assert!(!assets_dir
        .join(capsem_assets::asset_manager::host_manifest_arch())
        .exists());
    hydrate_assets_for_binary(&assets_dir, env!("CARGO_PKG_VERSION"))
        .await
        .expect("a channel without a runtime has nothing to hydrate");
}

#[test]
fn update_removes_the_retired_profile_catalog_without_following_links() {
    let temp = tempfile::tempdir().unwrap();
    let capsem_home = temp.path().join("home");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(capsem_home.join("profiles/code/root/root")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep"), b"keep").unwrap();
    std::fs::write(capsem_home.join("profiles/code/profile.toml"), b"id = \"code\"").unwrap();
    std::os::unix::fs::symlink(&outside, capsem_home.join("profiles/code/root/root/escape")).unwrap();
    std::fs::create_dir_all(capsem_home.join("assets")).unwrap();

    assert!(remove_retired_profile_catalog(&capsem_home).unwrap());

    assert!(std::fs::symlink_metadata(capsem_home.join("profiles")).is_err());
    assert!(
        capsem_home.join("assets").is_dir(),
        "only the retired catalog is removed"
    );
    assert_eq!(std::fs::read(outside.join("keep")).unwrap(), b"keep");
    assert!(
        !remove_retired_profile_catalog(&capsem_home).unwrap(),
        "an absent catalog is not an error"
    );
}

#[test]
fn a_retired_profile_catalog_symlink_is_removed_as_the_link() {
    let temp = tempfile::tempdir().unwrap();
    let capsem_home = temp.path().join("home");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&capsem_home).unwrap();
    std::fs::create_dir_all(outside.join("code")).unwrap();
    std::fs::write(outside.join("code/profile.toml"), b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, capsem_home.join("profiles")).unwrap();

    assert!(remove_retired_profile_catalog(&capsem_home).unwrap());

    assert!(std::fs::symlink_metadata(capsem_home.join("profiles")).is_err());
    assert_eq!(std::fs::read(outside.join("code/profile.toml")).unwrap(), b"keep");
    assert!(
        !remove_retired_profile_catalog(&temp.path().join("never-installed")).unwrap(),
        "a missing Capsem home has no catalog to remove"
    );
}
