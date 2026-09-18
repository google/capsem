//! Catalog-level behaviour: what the installed set of profiles answers.
use super::*;

#[test]
fn profile_catalog_loads_directory_profiles_and_rejects_id_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("code")).unwrap();
    std::fs::write(
        dir.path().join("code/profile.toml"),
        include_str!("../../../../../../config/profiles/code/profile.toml"),
    )
    .unwrap();

    let catalog = ProfileCatalog::load_from_dir(dir.path()).expect("catalog loads");
    let profile = catalog.get("code").expect("code profile exists");
    assert_eq!(profile.name, "Code");
    assert_eq!(catalog.profiles().count(), 1);

    std::fs::write(
        dir.path().join("legacy-flat.toml"),
        include_str!("../../../../../../config/profiles/code/profile.toml"),
    )
    .unwrap();
    let catalog = ProfileCatalog::load_from_dir(dir.path()).expect("flat files are ignored");
    assert_eq!(catalog.profiles().count(), 1);

    std::fs::create_dir(dir.path().join("wrong")).unwrap();
    std::fs::write(
        dir.path().join("wrong/profile.toml"),
        include_str!("../../../../../../config/profiles/code/profile.toml"),
    )
    .unwrap();
    let error = ProfileCatalog::load_from_dir(dir.path()).unwrap_err();
    assert!(error.contains("id mismatch"), "{error}");
}

#[test]
fn profile_catalog_rejects_flat_only_profile_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("code.toml"),
        include_str!("../../../../../../config/profiles/code/profile.toml"),
    )
    .unwrap();

    let error = ProfileCatalog::load_from_dir(dir.path()).unwrap_err();

    assert!(
        error.contains("contains no profile directories with profile.toml"),
        "{error}"
    );
}

/// Which profile a client gets when it names none is the catalog's answer,
/// not a string compiled into every SDK, and it is asked per runtime: one
/// profile may claim the VM default and another the container default.
#[test]
fn the_catalog_names_one_default_profile_per_runtime() {
    let builtin = ProfileCatalog::builtin();
    assert_eq!(builtin.default_profile_id(ProfileRuntime::Vm), Some("code"));
    assert_eq!(builtin.default_profile_id(ProfileRuntime::Container), Some("code"));

    let dir = tempfile::tempdir().unwrap();
    let code = include_str!("../../../../../../config/profiles/code/profile.toml");
    let cowork = include_str!("../../../../../../config/profiles/co-work/profile.toml");
    std::fs::create_dir(dir.path().join("code")).unwrap();
    std::fs::write(dir.path().join("code/profile.toml"), code).unwrap();
    let catalog = ProfileCatalog::load_from_dir(dir.path()).unwrap();
    assert_eq!(catalog.default_profile_id(ProfileRuntime::Vm), Some("code"));
    assert_eq!(catalog.default_profile_id(ProfileRuntime::Container), Some("code"));

    // A second claimant for one runtime is refused, and the message names the
    // runtime: two profiles may legitimately hold the two claims.
    std::fs::create_dir(dir.path().join("co-work")).unwrap();
    std::fs::write(
        dir.path().join("co-work/profile.toml"),
        cowork.replace("id = \"co-work\"", "id = \"co-work\"\ndefault_for = [\"container\"]"),
    )
    .unwrap();
    let error = ProfileCatalog::load_from_dir(dir.path()).unwrap_err();
    assert!(error.contains("more than one default container profile"), "{error}");
    assert!(
        !error.contains("default vm profile"),
        "only the contested runtime is named: {error}"
    );

    // The two claims may part: the VM keeps code, the container takes co-work.
    std::fs::write(
        dir.path().join("code/profile.toml"),
        code.replace("default_for = [\"vm\", \"container\"]", "default_for = [\"vm\"]"),
    )
    .unwrap();
    let catalog = ProfileCatalog::load_from_dir(dir.path()).expect("one claimant per runtime loads");
    assert_eq!(catalog.default_profile_id(ProfileRuntime::Vm), Some("code"));
    assert_eq!(catalog.default_profile_id(ProfileRuntime::Container), Some("co-work"));
    // What the status route publishes: both claims, never just the VM's.
    assert_eq!(
        catalog.default_profile_ids(),
        [("vm", "code"), ("container", "co-work")].into_iter().collect()
    );

    std::fs::write(dir.path().join("co-work/profile.toml"), cowork).unwrap();
    std::fs::write(
        dir.path().join("code/profile.toml"),
        code.replace("default_for = [\"vm\", \"container\"]\n", ""),
    )
    .unwrap();
    let catalog = ProfileCatalog::load_from_dir(dir.path()).expect("a catalog without a default still loads");
    assert!(catalog.default_profile_ids().is_empty());
    for runtime in ProfileRuntime::ALL {
        assert_eq!(
            catalog.default_profile_id(runtime),
            None,
            "no profile claims {}",
            runtime.as_str()
        );
    }
}

#[test]
fn installed_release_graph_overlays_profile_bootstrap_assets_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join("manifest.json");
    let manifest = serde_json::json!({
        "version": "1.0.0",
        "profiles": {
            "code": {
                "revision": "2030.01.02.3",
                "architectures": [{
                "architecture": "arm64",
                "config": [
                    {"kind": "enforcement", "path": "profiles/code/enforcement.toml", "url": "https://release.example/enforcement.toml", "bytes": 44, "digest": {"blake3": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd", "sha256": "4444444444444444444444444444444444444444444444444444444444444444"}}
                ],
                "images": [
                        {"kind": "kernel", "name": "vmlinuz", "url": "https://release.example/arm64-vmlinuz", "bytes": 11, "digest": {"blake3": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "sha256": "1111111111111111111111111111111111111111111111111111111111111111"}},
                        {"kind": "initrd", "name": "initrd.img", "url": "https://release.example/arm64-initrd.img", "bytes": 22, "digest": {"blake3": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "sha256": "2222222222222222222222222222222222222222222222222222222222222222"}},
                        {"kind": "rootfs", "name": "rootfs.erofs", "url": "https://release.example/arm64-rootfs.erofs", "bytes": 33, "digest": {"blake3": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc", "sha256": "3333333333333333333333333333333333333333333333333333333333333333"}}
                    ]
                }]
            }
        }
    });
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let mut catalog = ProfileCatalog::builtin();

    overlay_release_manifest_assets(&mut catalog, &manifest_path).unwrap();

    let code = catalog.get("code").unwrap();
    let arm64 = code.assets.arch.get("arm64").unwrap();
    assert_eq!(code.revision, "2030.01.02.3");
    assert_eq!(arm64.kernel.url, "https://release.example/arm64-vmlinuz");
    assert_eq!(
        arm64.initrd.hash.as_deref(),
        Some("blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    assert_eq!(arm64.rootfs.size, Some(33));
    let enforcement = code.files.enforcement.as_ref().unwrap();
    assert_eq!(enforcement.path, "profiles/code/enforcement.toml");
    assert_eq!(
        enforcement.hash.as_deref(),
        Some("blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd")
    );
    assert_eq!(enforcement.size, Some(44));
}

#[test]
fn installed_release_graph_resolves_relative_image_urls_to_hydrated_assets() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join("manifest.json");
    let manifest = serde_json::json!({
        "version": "1.0.0",
        "profiles": {
            "code": {
                "revision": "2030.01.02.3",
                "architectures": [{
                    "architecture": "arm64",
                    "config": [],
                    "images": [
                        {"kind": "kernel", "name": "vmlinuz", "url": "/profiles/releases/nightly/code/2030.01.02.3/arm64/vmlinuz", "bytes": 11, "digest": {"blake3": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "sha256": "1111111111111111111111111111111111111111111111111111111111111111"}},
                        {"kind": "initrd", "name": "initrd.img", "url": "/profiles/releases/nightly/code/2030.01.02.3/arm64/initrd.img", "bytes": 22, "digest": {"blake3": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "sha256": "2222222222222222222222222222222222222222222222222222222222222222"}},
                        {"kind": "rootfs", "name": "rootfs.erofs", "url": "/profiles/releases/nightly/code/2030.01.02.3/arm64/rootfs.erofs", "bytes": 33, "digest": {"blake3": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc", "sha256": "3333333333333333333333333333333333333333333333333333333333333333"}}
                    ]
                }]
            }
        }
    });
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let mut catalog = ProfileCatalog::builtin();

    overlay_release_manifest_assets(&mut catalog, &manifest_path).unwrap();

    let code = catalog.get("code").unwrap();
    let arm64 = code.assets.arch.get("arm64").unwrap();
    assert_eq!(
        arm64.kernel.url,
        format!("file://{}", dir.path().join("arm64/vmlinuz").display())
    );
    assert_eq!(
        arm64.initrd.url,
        format!("file://{}", dir.path().join("arm64/initrd.img").display())
    );
    assert_eq!(
        arm64.rootfs.url,
        format!("file://{}", dir.path().join("arm64/rootfs.erofs").display())
    );
    code.validate().expect("installed profile overlay must remain valid");
}
