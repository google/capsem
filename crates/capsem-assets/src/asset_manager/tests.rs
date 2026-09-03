use super::*;

mod hydration;
mod manifest_arch_keys;

const SAMPLE_V2_MANIFEST: &str = r#"{
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": "2026.0415.1",
            "releases": {
                "2026.0415.1": {
                    "date": "2026-04-15",
                    "deprecated": false,
                    "min_binary": "1.0.0",
                    "arches": {
                        "arm64": {
                            "vmlinuz": { "hash": "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c", "size": 7797248 },
                            "initrd.img": { "hash": "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456", "size": 2270154 },
                            "rootfs.erofs": { "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 454230016 }
                        }
                    }
                }
            }
        },
        "binaries": {
            "current": "1.0.1776269479",
            "releases": {
                "1.0.1776269479": {
                    "date": "2026-04-15",
                    "deprecated": false,
                    "min_assets": "2026.0415.1"
                }
            }
        }
    }"#;

#[test]
fn manifest_parse() {
    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    assert_eq!(m.format, 2);
    assert_eq!(m.refresh_policy, "24h");
    assert_eq!(m.assets.current, "2026.0415.1");
    assert_eq!(m.binaries.current, "1.0.1776269479");
    assert_eq!(m.assets.releases.len(), 1);
    assert_eq!(m.binaries.releases.len(), 1);
    let rel = &m.assets.releases["2026.0415.1"];
    assert!(!rel.deprecated);
    assert_eq!(rel.min_binary, "1.0.0");
    let arm64 = &rel.arches["arm64"];
    assert_eq!(arm64.len(), 3);
    assert_eq!(arm64["vmlinuz"].size, 7797248);
}

#[test]
fn public_release_graph_parses_to_runtime_view_without_rewriting_document() {
    let raw = serde_json::json!({
        "channel": "stable",
        "version": "1.0.142",
        "status": "current",
        "packages": [{
            "name": "Capsem-1.5.1783857731.pkg",
            "version": "1.5.1783857731",
            "status": "current"
        }],
        "profiles": {
            "co-work": {
                "name": "Co-work",
                "description": "Shared profile for collaborative agent sessions.",
                "revision": "2026.0703.2",
                "status": "current",
                "min_capsem_version": "1.5.0",
                "architectures": [{
                    "architecture": "arm64",
                    "image_revision": "2026.0714.18",
                    "images": [
                        {"kind":"kernel","name":"vmlinuz","bytes":10,"status":"current","digest":{"blake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sha256":"1111111111111111111111111111111111111111111111111111111111111111"}},
                        {"kind":"initrd","name":"initrd.img","bytes":20,"status":"current","digest":{"blake3":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","sha256":"2222222222222222222222222222222222222222222222222222222222222222"}},
                        {"kind":"rootfs","name":"rootfs.erofs","bytes":30,"status":"current","digest":{"blake3":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","sha256":"3333333333333333333333333333333333333333333333333333333333333333"}}
                    ]
                }]
            }
        }
    });
    let raw_text = serde_json::to_string(&raw).unwrap();

    let runtime = ManifestV2::from_json(&raw_text).unwrap();

    assert_eq!(runtime.assets.current, "2026.0714.18");
    assert_eq!(runtime.binaries.current, "1.5.1783857731");
    assert_eq!(
        runtime.assets.releases["2026.0714.18"].arches["arm64"]["rootfs.erofs"].size,
        30
    );
    let unchanged: serde_json::Value = serde_json::from_str(&raw_text).unwrap();
    assert_eq!(
        unchanged["profiles"]["co-work"]["description"],
        raw["profiles"]["co-work"]["description"]
    );
    assert_eq!(unchanged["packages"], raw["packages"]);
}

#[test]
fn public_release_graph_retains_every_profile_state_identity() {
    let images = |seed: char| {
        serde_json::json!([
            {"kind":"kernel","name":"vmlinuz","bytes":10,"status":"current","digest":{"blake3":seed.to_string().repeat(64),"sha256":"1".repeat(64)}},
            {"kind":"initrd","name":"initrd.img","bytes":20,"status":"current","digest":{"blake3":"b".repeat(64),"sha256":"2".repeat(64)}},
            {"kind":"rootfs","name":"rootfs.erofs","bytes":30,"status":"current","digest":{"blake3":"c".repeat(64),"sha256":"3".repeat(64)}}
        ])
    };
    let graph = serde_json::json!({
        "profiles": {
            "co-work": {
                "revision": "2030.0101.1",
                "status": "current",
                "architectures": [{
                    "architecture": "arm64",
                    "image_revision": "2030.0101.10",
                    "config": [{"kind":"profile","path":"profiles/co-work/profile.toml","bytes":1,"digest":{"blake3":"d".repeat(64),"sha256":"4".repeat(64)}}],
                    "images": images('a'),
                    "evidence": [{"kind":"obom","bytes":1,"digest":{"blake3":"e".repeat(64),"sha256":"5".repeat(64)}}]
                }]
            },
            "code": {
                "revision": "2030.0101.2",
                "status": "current",
                "architectures": [{
                    "architecture": "arm64",
                    "image_revision": "2030.0101.20",
                    "config": [{"kind":"profile","path":"profiles/code/profile.toml","bytes":1,"digest":{"blake3":"f".repeat(64),"sha256":"6".repeat(64)}}],
                    "images": images('9'),
                    "evidence": [{"kind":"obom","bytes":1,"digest":{"blake3":"8".repeat(64),"sha256":"7".repeat(64)}}]
                }]
            }
        }
    });

    let state = release_graph_profile_state(&graph).unwrap();

    assert_eq!(state.profiles.keys().cloned().collect::<Vec<_>>(), ["co-work", "code"]);
    assert_eq!(state.profiles["co-work"].revision, "2030.0101.1");
    assert_eq!(state.profiles["code"].revision, "2030.0101.2");
    assert!(state.catalog_revision.starts_with("catalog-"));
    assert!(state.images_revision.starts_with("images-"));

    let mut config_changed = graph.clone();
    config_changed["profiles"]["code"]["architectures"][0]["config"][0]["digest"]["blake3"] =
        serde_json::json!("0".repeat(64));
    let config_state = release_graph_profile_state(&config_changed).unwrap();
    assert_ne!(config_state.catalog_revision, state.catalog_revision);
    assert_eq!(config_state.images_revision, state.images_revision);

    let mut revision_changed = graph.clone();
    revision_changed["profiles"]["code"]["revision"] = serde_json::json!("2030.0101.3");
    let revision_state = release_graph_profile_state(&revision_changed).unwrap();
    assert_ne!(revision_state.catalog_revision, state.catalog_revision);
    assert_eq!(revision_state.images_revision, state.images_revision);

    let mut evidence_changed = graph.clone();
    evidence_changed["profiles"]["code"]["architectures"][0]["evidence"][0]["digest"]["blake3"] =
        serde_json::json!("1".repeat(64));
    let evidence_state = release_graph_profile_state(&evidence_changed).unwrap();
    assert_ne!(evidence_state.catalog_revision, state.catalog_revision);
    assert_eq!(evidence_state.images_revision, state.images_revision);

    let mut image_changed = graph;
    image_changed["profiles"]["code"]["architectures"][0]["images"][0]["digest"]["blake3"] =
        serde_json::json!("2".repeat(64));
    let image_state = release_graph_profile_state(&image_changed).unwrap();
    assert_ne!(image_state.catalog_revision, state.catalog_revision);
    assert_ne!(image_state.images_revision, state.images_revision);
}

#[test]
fn public_release_graph_rejects_an_incomplete_sibling_profile() {
    let complete_images = serde_json::json!([
        {"kind":"kernel","name":"vmlinuz","bytes":10,"status":"current","digest":{"blake3":"a".repeat(64),"sha256":"1".repeat(64)}},
        {"kind":"initrd","name":"initrd.img","bytes":20,"status":"current","digest":{"blake3":"b".repeat(64),"sha256":"2".repeat(64)}},
        {"kind":"rootfs","name":"rootfs.erofs","bytes":30,"status":"current","digest":{"blake3":"c".repeat(64),"sha256":"3".repeat(64)}}
    ]);
    let graph = serde_json::json!({
        "packages": [{"version": "1.5.0", "status": "current"}],
        "profiles": {
            "default": {
                "revision": "2030.0101.1",
                "status": "current",
                "architectures": [{"architecture":"arm64","image_revision":"2030.0101.1","images":complete_images}]
            },
            "code": {
                "revision": "2030.0101.2",
                "status": "current",
                "architectures": [{"architecture":"arm64","image_revision":"2030.0101.2","images":[
                    {"kind":"kernel","name":"vmlinuz","bytes":10,"status":"current","digest":{"blake3":"a".repeat(64),"sha256":"1".repeat(64)}},
                    {"kind":"initrd","name":"initrd.img","bytes":20,"status":"current","digest":{"blake3":"b".repeat(64),"sha256":"2".repeat(64)}}
                ]}]
            }
        }
    });

    let error = ManifestV2::from_json(&graph.to_string()).unwrap_err();

    assert!(format!("{error:#}").contains("profile code"));
    assert!(format!("{error:#}").contains("rootfs"));
}

#[test]
fn public_release_graph_requires_one_exact_image_revision_for_every_architecture() {
    let base_image = serde_json::json!({
        "kind": "kernel",
        "name": "vmlinuz",
        "bytes": 10,
        "status": "current",
        "digest": {"blake3": "a".repeat(64), "sha256": "1".repeat(64)}
    });
    let images = vec![
        base_image,
        serde_json::json!({
            "kind": "initrd", "name": "initrd.img", "bytes": 20, "status": "current",
            "digest": {"blake3": "b".repeat(64), "sha256": "2".repeat(64)}
        }),
        serde_json::json!({
            "kind": "rootfs", "name": "rootfs.erofs", "bytes": 30, "status": "current",
            "digest": {"blake3": "c".repeat(64), "sha256": "3".repeat(64)}
        }),
    ];
    let graph = |architectures: serde_json::Value| {
        serde_json::json!({
            "packages": [{"version": "1.5.0", "status": "current"}],
            "profiles": {"code": {
                "revision": "profile-revision-is-not-an-image-version",
                "status": "current",
                "architectures": architectures
            }}
        })
    };

    let missing = graph(serde_json::json!([{
        "architecture": "arm64",
        "images": images
    }]));
    let error = ManifestV2::from_json(&missing.to_string()).unwrap_err();
    assert!(format!("{error:#}").contains("missing image_revision"));

    let disagreeing = graph(serde_json::json!([
        {"architecture": "arm64", "image_revision": "2026.0714.18", "images": images},
        {"architecture": "x86_64", "image_revision": "2026.0714.19", "images": images}
    ]));
    let error = ManifestV2::from_json(&disagreeing.to_string()).unwrap_err();
    assert!(format!("{error:#}").contains("image revisions disagree"));
}

#[test]
fn manifest_requires_refresh_policy() {
    let json = SAMPLE_V2_MANIFEST.replace(r#""refresh_policy": "24h","#, "");
    let err = ManifestV2::from_json(&json).unwrap_err();
    let error_chain = format!("{err:#}");
    assert!(
        error_chain.contains("refresh_policy"),
        "missing refresh policy must fail closed, got: {error_chain}"
    );
}

#[test]
fn manifest_resolve() {
    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();
    assert_eq!(resolved.asset_version, "2026.0415.1");
    assert!(resolved.kernel.to_str().unwrap().contains("vmlinuz-a65f925ebe0b0cc7"));
    assert!(resolved
        .initrd
        .to_str()
        .unwrap()
        .contains("initrd-cba052ee1e3fc7de.img"));
    assert!(resolved
        .rootfs
        .to_str()
        .unwrap()
        .contains("rootfs-b8199dc4a83069b9.erofs"));
}

#[test]
fn manifest_resolve_unknown_binary_uses_current_assets() {
    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let resolved = m.resolve("1.0.9999999999", "arm64", dir.path()).unwrap();
    assert_eq!(resolved.asset_version, "2026.0415.1");
}

#[test]
fn manifest_resolve_rejects_current_assets_that_require_newer_binary() {
    let mut m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let future_version = "2030.0101.1".to_string();
    let mut future_release = m.assets.releases["2026.0415.1"].clone();
    future_release.min_binary = "2.0.0".to_string();
    m.assets.releases.insert(future_version.clone(), future_release);
    m.assets.current = future_version;

    let dir = tempfile::tempdir().unwrap();
    let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();

    assert_eq!(
        resolved.asset_version, "2026.0415.1",
        "older binaries must keep using the newest asset release whose min_binary allows them"
    );
}

#[test]
fn manifest_resolve_avoids_deprecated_asset_releases() {
    let mut m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let deprecated_version = "2026.0416.1".to_string();
    let mut deprecated_release = m.assets.releases["2026.0415.1"].clone();
    deprecated_release.deprecated = true;
    deprecated_release.deprecated_date = Some("2026-04-17".to_string());
    m.assets.releases.insert(deprecated_version.clone(), deprecated_release);
    m.assets.current = deprecated_version;

    let dir = tempfile::tempdir().unwrap();
    let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();

    assert_eq!(
        resolved.asset_version, "2026.0415.1",
        "new sessions must avoid deprecated asset releases when a compatible release remains"
    );
}

#[test]
fn manifest_resolve_fails_when_only_compatible_assets_are_deprecated() {
    let mut m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    m.assets.releases.get_mut("2026.0415.1").unwrap().deprecated = true;

    let dir = tempfile::tempdir().unwrap();
    let err = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap_err();

    assert!(
        format!("{err:#}").contains("no compatible asset release for binary 1.0.1776269479"),
        "{err:#}"
    );
}

#[test]
fn manifest_resolve_fails_when_no_asset_release_supports_binary() {
    let mut m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    m.assets.releases.get_mut("2026.0415.1").unwrap().min_binary = "2.0.0".to_string();

    let dir = tempfile::tempdir().unwrap();
    let err = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap_err();

    assert!(
        format!("{err:#}").contains("no compatible asset release for binary 1.0.1776269479"),
        "{err:#}"
    );
}

#[test]
fn numeric_version_comparison_handles_multi_digit_components() {
    assert!(version_at_least("10.0.0", "9.9.9"));
    assert!(version_at_least("2026.1001.1", "2026.0630.9"));
    assert!(!version_at_least("1.9.9", "1.10.0"));
}

#[test]
fn hash_filename_cases() {
    assert_eq!(
        hash_filename(
            "vmlinuz",
            "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c"
        ),
        "vmlinuz-a65f925ebe0b0cc7"
    );
    assert_eq!(
        hash_filename(
            "initrd.img",
            "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456"
        ),
        "initrd-cba052ee1e3fc7de.img"
    );
    assert_eq!(
        hash_filename(
            "rootfs.erofs",
            "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee"
        ),
        "rootfs-b8199dc4a83069b9.erofs"
    );
}

#[test]
fn manifest_rejects_wrong_format() {
    let json = SAMPLE_V2_MANIFEST.replace("\"format\": 2", "\"format\": 99");
    assert!(ManifestV2::from_json(&json).is_err());
}

#[test]
fn expected_hashes_current_returns_arch_hashes() {
    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let h = m.expected_hashes_current("arm64").unwrap();
    assert_eq!(
        h.kernel,
        "a65f925ebe0b0cc76afe0fe4945431473cb1a32c4f47a9e9b1592e92c46c829c"
    );
    assert_eq!(
        h.initrd,
        "cba052ee1e3fc7de5bb1af0da9f4a6472622b24788051f0e4d4ae6eabb0c3456"
    );
    assert_eq!(
        h.rootfs,
        "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee"
    );
}

#[test]
fn expected_hashes_current_returns_none_for_unknown_arch() {
    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    assert!(m.expected_hashes_current("riscv64").is_none());
}

#[test]
fn expected_hashes_current_returns_none_when_canonical_asset_missing() {
    // Manifest with arm64 present but missing any known rootfs entry.
    let json = SAMPLE_V2_MANIFEST.replace(
        r#""rootfs.erofs": { "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 454230016 }"#,
        r#""rootfs.placeholder": { "hash": "b8199dc4a83069b99f41e1eb3829992d12777d09e2ce8295276f9d3a1abb1eee", "size": 454230016 }"#,
    );
    let m = ManifestV2::from_json(&json).unwrap();
    assert!(m.expected_hashes_current("arm64").is_none());
}

#[test]
fn expected_hashes_current_rejects_squashfs_manifest() {
    let json = SAMPLE_V2_MANIFEST.replace("rootfs.erofs", "rootfs.squashfs");
    let m = ManifestV2::from_json(&json).unwrap();
    assert!(m.expected_hashes_current("arm64").is_none());
}

#[test]
fn host_manifest_arch_maps_aarch64_to_arm64() {
    // Static check: the function maps the rustc arch name (aarch64) to the
    // manifest arch key (arm64). On an aarch64 host this yields "arm64";
    // on x86_64 it yields "x86_64". We can only test the arm's value if
    // we run on that arch, so pin the full mapping table instead.
    assert_eq!(map_rustc_arch_to_manifest("aarch64"), "arm64");
    assert_eq!(map_rustc_arch_to_manifest("x86_64"), "x86_64");
    // Unknown arches pass through (leaves the caller to fail resolution).
    assert_eq!(map_rustc_arch_to_manifest("riscv64"), "riscv64");
}

#[test]
fn load_manifest_for_assets_reads_flat_adjacent_layout() {
    // ~/.capsem/assets/ style: manifest.json lives in the assets dir.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("manifest.json"), SAMPLE_V2_MANIFEST).unwrap();
    let m = load_manifest_for_assets(dir.path()).unwrap();
    assert_eq!(m.assets.current, "2026.0415.1");
}

#[test]
fn load_manifest_for_assets_reads_per_arch_layout() {
    // Dev-tree style: assets passed in is assets/arm64/, manifest.json
    // lives at assets/manifest.json (one level up).
    let dir = tempfile::tempdir().unwrap();
    let arm64 = dir.path().join("arm64");
    std::fs::create_dir(&arm64).unwrap();
    std::fs::write(dir.path().join("manifest.json"), SAMPLE_V2_MANIFEST).unwrap();
    let m = load_manifest_for_assets(&arm64).unwrap();
    assert_eq!(m.assets.current, "2026.0415.1");
}

#[test]
fn load_manifest_for_assets_returns_none_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_manifest_for_assets(dir.path()).is_none());
}

#[test]
fn load_manifest_for_assets_returns_none_on_malformed_json() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("manifest.json"), "not json").unwrap();
    assert!(load_manifest_for_assets(dir.path()).is_none());
}

#[test]
fn manifest_merge() {
    let mut m1 = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let json2 = SAMPLE_V2_MANIFEST
        .replace("2026.0415.1", "2026.0416.1")
        .replace("1.0.1776269479", "1.0.1776300000");
    let m2 = ManifestV2::from_json(&json2).unwrap();
    m1.merge(&m2);
    assert_eq!(m1.assets.releases.len(), 2);
    assert_eq!(m1.binaries.releases.len(), 2);
    assert_eq!(m1.assets.current, "2026.0416.1");
    assert_eq!(m1.binaries.current, "1.0.1776300000");
}

#[test]
fn manifest_merge_compares_numeric_version_components() {
    let older = SAMPLE_V2_MANIFEST
        .replace("2026.0415.1", "2026.0415.2")
        .replace("1.0.1776269479", "1.0.2");
    let newer = SAMPLE_V2_MANIFEST
        .replace("2026.0415.1", "2026.0415.10")
        .replace("1.0.1776269479", "1.0.10");
    let mut merged = ManifestV2::from_json(&older).unwrap();

    merged.merge(&ManifestV2::from_json(&newer).unwrap());

    assert_eq!(merged.assets.current, "2026.0415.10");
    assert_eq!(merged.binaries.current, "1.0.10");
}

#[test]
fn manifest_resolve_finds_files_in_arch_subdir() {
    // Simulates installed/dev layout: base_dir/arm64/vmlinuz-{hash}
    let dir = tempfile::tempdir().unwrap();
    let arm64 = dir.path().join("arm64");
    std::fs::create_dir(&arm64).unwrap();
    std::fs::write(arm64.join("vmlinuz-a65f925ebe0b0cc7"), b"k").unwrap();
    std::fs::write(arm64.join("initrd-cba052ee1e3fc7de.img"), b"i").unwrap();
    std::fs::write(arm64.join("rootfs-b8199dc4a83069b9.erofs"), b"r").unwrap();

    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();
    assert!(resolved.kernel.exists(), "kernel not found: {:?}", resolved.kernel);
    assert!(resolved.initrd.exists(), "initrd not found: {:?}", resolved.initrd);
    assert!(resolved.rootfs.exists(), "rootfs not found: {:?}", resolved.rootfs);
    // Must resolve to the arch subdir, not the flat path
    assert!(resolved.kernel.to_str().unwrap().contains("arm64/"));
}

#[test]
fn manifest_resolve_finds_files_flat() {
    // Simulates flat layout: base_dir/vmlinuz-{hash}
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vmlinuz-a65f925ebe0b0cc7"), b"k").unwrap();
    std::fs::write(dir.path().join("initrd-cba052ee1e3fc7de.img"), b"i").unwrap();
    std::fs::write(dir.path().join("rootfs-b8199dc4a83069b9.erofs"), b"r").unwrap();

    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let resolved = m.resolve("1.0.1776269479", "arm64", dir.path()).unwrap();
    assert!(resolved.kernel.exists());
    assert!(resolved.initrd.exists());
    assert!(resolved.rootfs.exists());
}

#[test]
fn version_traversal_rejected() {
    assert!(validate_version("../etc").is_err());
    assert!(validate_version("foo/bar").is_err());
    assert!(validate_version("").is_err());
    assert!(validate_version("0.9.0").is_ok());
}

#[test]
fn filename_traversal_rejected() {
    assert!(validate_filename("../../x").is_err());
    assert!(validate_filename("foo/bar").is_err());
    assert!(validate_filename("").is_err());
    assert!(validate_filename("vmlinuz").is_ok());
}

#[test]
fn hash_file_known_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test");
    std::fs::write(&path, b"hello world").unwrap();
    let h = hash_file(&path).unwrap();
    assert_eq!(h.len(), 64);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn hash_file_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty");
    std::fs::write(&path, b"").unwrap();
    let h = hash_file(&path).unwrap();
    assert_eq!(h.len(), 64);
}

#[test]
fn hash_file_nonexistent() {
    assert!(hash_file(Path::new("/nonexistent/file")).is_err());
}

#[test]
fn default_assets_dir_under_home() {
    // With CAPSEM_HOME / CAPSEM_ASSETS_DIR overrides the path won't contain
    // ".capsem/assets" -- it's whatever the user pointed at. Only assert
    // the substring when we're on the default layout.
    let overridden = std::env::var("CAPSEM_ASSETS_DIR").is_ok() || std::env::var("CAPSEM_HOME").is_ok();
    if let Some(dir) = default_assets_dir() {
        if overridden {
            assert!(dir.to_str().is_some());
        } else {
            assert!(dir.to_str().unwrap().contains(".capsem/assets"));
        }
    }
}

#[test]
fn release_url_format() {
    assert_eq!(
        release_url("1.0.1776269479"),
        "https://github.com/google/capsem/releases/download/v1.0.1776269479"
    );
}

/// Pin the exact URL `download_missing_assets` constructs. Assets are
/// deployed by asset version under release.capsem.org; the channel manifest
/// can move without breaking older installed manifests.
#[test]
fn asset_download_url_uses_asset_version_channel_base_and_arch_prefix() {
    assert_eq!(
        asset_download_url("2026.0627.1", "arm64", "vmlinuz"),
        "https://release.capsem.org/assets/releases/2026.0627.1/arm64-vmlinuz",
    );
    assert_eq!(
        asset_download_url("2026.0627.1", "x86_64", "rootfs.erofs"),
        "https://release.capsem.org/assets/releases/2026.0627.1/x86_64-rootfs.erofs",
    );
    let url = asset_download_url("2026.0627.1", "arm64", "initrd.img");
    assert!(!url.contains("1.0."), "binary version leaked into asset URL: {url}");
    assert_eq!(
        asset_download_url_with_base(
            "https://github.com/google/capsem/releases/download/assets-v{asset_version}",
            "2026.0627.1",
            "arm64",
            "rootfs.erofs",
        ),
        "https://github.com/google/capsem/releases/download/assets-v2026.0627.1/arm64-rootfs.erofs",
    );
}

#[test]
fn remote_asset_release_base_preserves_asset_version_template() {
    let dir = tempfile::tempdir().unwrap();
    let mut manifest = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let asset_base = "https://github.com/google/capsem/releases/download/assets-v{asset_version}";
    manifest.asset_base = Some(asset_base.to_string());

    let resolved_base = remote_asset_release_base_url(&manifest, dir.path()).unwrap();

    assert_eq!(resolved_base, asset_base);
    assert_eq!(
        asset_download_url_with_base(&resolved_base, "2026.0415.1", "arm64", "vmlinuz"),
        "https://github.com/google/capsem/releases/download/assets-v2026.0415.1/arm64-vmlinuz",
    );
}

#[test]
fn asset_release_base_derives_from_channel_manifest_url() {
    assert_eq!(
        asset_release_base_url_from_manifest_url("https://release.capsem.org/assets/stable/manifest.json").as_deref(),
        Some("https://release.capsem.org/assets/releases")
    );
    assert_eq!(
        asset_release_base_url_from_manifest_url("https://corp.example/capsem/assets/internal/manifest.json")
            .as_deref(),
        Some("https://corp.example/capsem/assets/releases")
    );
    assert_eq!(
        asset_release_base_url_from_manifest_url("file:///tmp/assets/stable/manifest.json"),
        None
    );
}

#[test]
fn cleanup_removes_unreferenced_files() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();

    // Create a referenced hash-named file
    std::fs::write(base.join("vmlinuz-a65f925ebe0b0cc7"), b"kernel").unwrap();
    // Create an unreferenced hash-named file
    std::fs::write(base.join("vmlinuz-deadbeef12345678"), b"old").unwrap();
    // Create manifest.json (should be preserved)
    std::fs::write(base.join("manifest.json"), b"{}").unwrap();

    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let removed = cleanup_unused_assets(base, &m).unwrap();

    assert_eq!(removed.len(), 1);
    assert!(base.join("vmlinuz-a65f925ebe0b0cc7").exists());
    assert!(!base.join("vmlinuz-deadbeef12345678").exists());
    assert!(base.join("manifest.json").exists());
}

#[test]
fn cleanup_preserves_hyphenated_canonical_asset_names() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();

    std::fs::write(base.join("software-inventory.json"), b"inventory").unwrap();
    std::fs::write(base.join("software-inventory-deadbeef12345678.json"), b"old").unwrap();

    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let removed = cleanup_unused_assets(base, &m).unwrap();

    assert_eq!(removed, vec![base.join("software-inventory-deadbeef12345678.json")]);
    assert!(base.join("software-inventory.json").exists());
}

#[test]
fn cleanup_preserves_manifest_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();

    std::fs::write(base.join("manifest.json"), SAMPLE_V2_MANIFEST).unwrap();
    std::fs::write(
        base.join("manifest-metadata.json"),
        br#"{"schema":"capsem.manifest_metadata.v1","origin":"package"}"#,
    )
    .unwrap();
    std::fs::write(base.join("rootfs-deadbeef12345678.erofs"), b"stale").unwrap();

    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let removed = cleanup_unused_assets(base, &m).unwrap();

    assert_eq!(removed, vec![base.join("rootfs-deadbeef12345678.erofs")]);
    assert!(base.join("manifest.json").exists());
    assert!(base.join("manifest-metadata.json").exists());
}

#[test]
fn cleanup_preserves_explicit_retention_filenames() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();

    std::fs::write(base.join("vmlinuz-deadbeef12345678"), b"profile kernel").unwrap();
    std::fs::write(base.join("rootfs-feedface87654321.erofs"), b"profile rootfs").unwrap();
    std::fs::write(base.join("rootfs-1111111111111111.erofs"), b"old rootfs").unwrap();

    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let removed =
        cleanup_unused_assets_preserving(base, &m, ["vmlinuz-deadbeef12345678", "rootfs-feedface87654321.erofs"])
            .unwrap();

    assert_eq!(removed, vec![base.join("rootfs-1111111111111111.erofs")]);
    assert!(base.join("vmlinuz-deadbeef12345678").exists());
    assert!(base.join("rootfs-feedface87654321.erofs").exists());
}

#[test]
fn channel_cache_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let capsem_home = dir.path();
    let stable_manifest = capsem_home.join("channels/stable/manifest.json");
    let nightly_manifest = capsem_home.join("channels/nightly/manifest.json");
    std::fs::create_dir_all(stable_manifest.parent().unwrap()).unwrap();
    std::fs::create_dir_all(nightly_manifest.parent().unwrap()).unwrap();
    std::fs::write(&stable_manifest, br#"{"channel":"stable"}"#).unwrap();
    std::fs::write(&nightly_manifest, br#"{"channel":"nightly"}"#).unwrap();

    let asset_dir = capsem_home.join("assets/arm64");
    std::fs::create_dir_all(&asset_dir).unwrap();
    let stable_rootfs_hash = "1111111111111111111111111111111111111111111111111111111111111111";
    let nightly_rootfs_hash = "2222222222222222222222222222222222222222222222222222222222222222";
    let stable_rootfs = asset_dir.join(hash_filename("rootfs.erofs", stable_rootfs_hash));
    let nightly_rootfs = asset_dir.join(hash_filename("rootfs.erofs", nightly_rootfs_hash));
    std::fs::write(&stable_rootfs, b"stable profile rootfs").unwrap();
    std::fs::write(&nightly_rootfs, b"nightly profile rootfs").unwrap();

    assert_ne!(stable_manifest, nightly_manifest);
    assert_ne!(stable_rootfs, nightly_rootfs);
    assert!(stable_manifest.is_file());
    assert!(nightly_manifest.is_file());
    assert!(stable_rootfs.is_file());
    assert!(nightly_rootfs.is_file());
    assert_eq!(
        asset_release_base_url_from_manifest_url("https://release.capsem.org/assets/stable/manifest.json"),
        Some("https://release.capsem.org/assets/releases".to_string())
    );
    assert_eq!(
        asset_release_base_url_from_manifest_url("https://release.capsem.org/assets/nightly/manifest.json"),
        Some("https://release.capsem.org/assets/releases".to_string())
    );
}

#[test]
fn cleanup_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let removed = cleanup_unused_assets(dir.path(), &m).unwrap();
    assert!(removed.is_empty());
}

#[test]
fn cleanup_nonexistent_dir() {
    let m = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let removed = cleanup_unused_assets(Path::new("/nonexistent"), &m).unwrap();
    assert!(removed.is_empty());
}

#[test]
fn cleanup_rejects_unsafe_architecture_directory_before_removing_files() {
    let dir = tempfile::tempdir().unwrap();
    let orphan = dir.path().join("vmlinuz-deadbeef12345678");
    std::fs::write(&orphan, b"old").unwrap();
    let mut manifest = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    let release = manifest.assets.releases.get_mut("2026.0415.1").unwrap();
    let assets = release.arches.remove("arm64").unwrap();
    release.arches.insert("../outside".into(), assets);

    let error = cleanup_unused_assets(dir.path(), &manifest).unwrap_err();

    assert!(format!("{error:#}").contains("invalid asset architecture directory"));
    assert!(orphan.exists(), "validation must finish before cleanup starts");
}
