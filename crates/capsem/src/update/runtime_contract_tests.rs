use super::*;

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

fn plan_test_update(
    mut check: UpdateCheck,
    body: &[u8],
    installed_binary: &str,
) -> Result<VerifiedUpdatePlan> {
    check.channel_hash = Some(channel_payload_hash(body));
    plan_verified_update(&check, body, installed_binary)
}

#[test]
fn complete_update_plan_keeps_binary_and_profiles_orthogonal() {
    let binary_body = update_plan_graph("1.0.0", "2.0.0");
    let binary = plan_test_update(
        update_plan_check(true, false, false, false),
        &binary_body,
        "1.0.0",
    )
    .unwrap();
    assert_eq!(binary.steps, vec![UpdatePlanStep::Binary]);

    let profile_body = update_plan_graph("1.0.0", "1.0.0");
    let profile = plan_test_update(
        update_plan_check(false, true, true, true),
        &profile_body,
        "1.0.0",
    )
    .unwrap();
    assert_eq!(profile.steps, vec![UpdatePlanStep::Profiles]);
}

#[test]
fn complete_update_plan_orders_binary_before_profiles() {
    let body = update_plan_graph("2.0.0", "2.0.0");
    let plan = plan_test_update(update_plan_check(true, true, true, true), &body, "1.0.0").unwrap();

    assert_eq!(
        plan.steps,
        vec![UpdatePlanStep::Binary, UpdatePlanStep::Profiles]
    );
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
