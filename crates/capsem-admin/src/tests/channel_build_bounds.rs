use super::*;

/// Every published asset path is `release_dir/<arch>-<name>` and every source
/// path is `assets_dir/<arch>/<name>`. Both are built from manifest keys, and
/// `auditfs::stage` creates parents and unlinks the destination before it
/// copies, so a key that is not a single path component is a write and a
/// delete outside the release root.
const ESCAPING_KEYS: &[&str] = &[
    "../../../../etc/pwn",
    "..",
    ".",
    "/tmp/pwn",
    "/",
    "arm64/evil",
    "arm64/../../pwn",
    "arm64\0",
    "",
];

#[test]
fn release_asset_destination_accepts_a_plain_component_pair() {
    let release_dir = Path::new("/release/2030.0101.1");
    let destination = release_asset_destination(release_dir, "arm64", "vmlinuz").expect("plain keys");
    assert_eq!(destination, release_dir.join("arm64-vmlinuz"));
    let source = source_asset_path(Path::new("/assets"), "arm64", "vmlinuz").expect("plain keys");
    assert_eq!(source, Path::new("/assets/arm64/vmlinuz"));
}

#[test]
fn release_asset_destination_refuses_keys_that_leave_the_release_dir() {
    let release_dir = Path::new("/release/2030.0101.1");
    for arch in ESCAPING_KEYS {
        let error = release_asset_destination(release_dir, arch, "vmlinuz")
            .err()
            .unwrap_or_else(|| panic!("arch {arch:?} must be refused"));
        assert!(format!("{error:#}").contains("architecture"), "{arch:?}: {error:#}");
        let error = source_asset_path(Path::new("/assets"), arch, "vmlinuz")
            .err()
            .unwrap_or_else(|| panic!("source arch {arch:?} must be refused"));
        assert!(format!("{error:#}").contains("architecture"), "{arch:?}: {error:#}");
    }
    for name in ESCAPING_KEYS {
        assert!(
            release_asset_destination(release_dir, "arm64", name).is_err(),
            "asset name {name:?} must be refused"
        );
        assert!(
            source_asset_path(Path::new("/assets"), "arm64", name).is_err(),
            "asset name {name:?} must be refused as a source"
        );
    }
}

#[test]
fn copying_release_assets_refuses_a_traversal_arch_before_touching_the_filesystem() {
    // Bypass the manifest parser on purpose: this is the last line, and it must
    // hold even for a release built in memory.
    let temp = tempfile::tempdir().expect("tempdir");
    let assets_dir = temp.path().join("assets");
    let release_dir = temp.path().join("out/assets/releases/2030.0101.1");
    fs::create_dir_all(&release_dir).unwrap();
    let victim = temp.path().join("victim");
    fs::write(&victim, b"do not touch").unwrap();
    let escaping_arch = "../../../../victim";
    // With the traversal, the destination `release_dir/<arch>-<name>` would be
    // `temp/victim-...`; make the exact escaping path exist so a delete shows.
    let escaped_destination = temp.path().join("victim-vmlinuz");
    fs::write(&escaped_destination, b"do not touch either").unwrap();

    let mut release = capsem_assets::asset_manager::AssetRelease {
        date: "2030-01-01".into(),
        deprecated: false,
        deprecated_date: None,
        min_binary: "1.0.0".into(),
        arches: HashMap::from([(
            escaping_arch.to_string(),
            HashMap::from([(
                "vmlinuz".to_string(),
                capsem_assets::asset_manager::AssetEntry {
                    hash: "0".repeat(64),
                    sha256: String::new(),
                    size: 0,
                },
            )]),
        )]),
    };
    let mut cache = AssetDigestCache::new();
    let error = copy_assets_channel_release_assets(&assets_dir, &release_dir, &mut release, &mut cache)
        .expect_err("traversal arch must be refused");
    assert!(format!("{error:#}").contains("architecture"), "{error:#}");
    assert_eq!(fs::read(&escaped_destination).unwrap(), b"do not touch either");
    assert_eq!(fs::read(&victim).unwrap(), b"do not touch");
    assert!(
        fs::read_dir(&release_dir).unwrap().next().is_none(),
        "nothing may be staged for a refused release"
    );

    let mut manifest =
        ManifestV2::from_json(&fs::read_to_string(write_test_assets_manifest(temp.path(), "arm64")).unwrap()).unwrap();
    let current = manifest.assets.current.clone();
    let hydrated = manifest.assets.releases.get_mut(&current).unwrap();
    let entry = hydrated.arches.remove("arm64").unwrap();
    hydrated.arches.insert(escaping_arch.to_string(), entry);
    let error = hydrate_current_asset_entry_sha256(&mut manifest, &assets_dir, &mut cache)
        .expect_err("traversal arch must be refused for source reads too");
    assert!(format!("{error:#}").contains("architecture"), "{error:#}");
}

#[test]
fn assets_channel_build_refuses_a_manifest_whose_arch_key_escapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    assert!(manifest.contains("\"arm64\": {"), "fixture drifted");
    fs::write(
        &manifest_path,
        manifest.replace("\"arm64\": {", "\"../../../../escape\": {"),
    )
    .unwrap();
    let out_dir = temp.path().join("cache/target/release/distribution");

    let error = build_assets_channel(
        &file_url(&manifest_path),
        &temp.path().join("assets"),
        &repo_config_profiles_dir(),
        "stable",
        "1.0.2",
        &out_dir,
        "2030-01-01T00:00:00Z",
        None,
    )
    .expect_err("an escaping arch key must fail the build");
    assert!(format!("{error:#}").contains("architecture"), "{error:#}");
    assert!(!temp.path().join("escape-vmlinuz").exists());
    assert!(
        !out_dir.join("assets").join("releases").exists(),
        "no release dir for a refused manifest"
    );
}
