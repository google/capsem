use super::*;

/// The arch key is joined into filesystem paths by every consumer: the asset
/// root cleanup, the runtime resolver, and the release channel builder, which
/// creates parents and unlinks the destination before staging. A manifest is
/// downloaded data, so the key is validated where the manifest is parsed.
fn manifest_with_arch_key(arch: &str) -> String {
    let escaped = arch.replace('\\', "\\\\").replace('\0', "\\u0000");
    assert!(SAMPLE_V2_MANIFEST.contains("\"arm64\": {"), "fixture drifted");
    SAMPLE_V2_MANIFEST.replace("\"arm64\": {", &format!("\"{escaped}\": {{"))
}

#[test]
fn a_well_formed_arch_key_still_parses() {
    let manifest = ManifestV2::from_json(&manifest_with_arch_key("x86_64")).expect("plain arch key parses");
    assert!(manifest.assets.releases["2026.0415.1"].arches.contains_key("x86_64"));
}

#[test]
fn arch_keys_that_escape_the_asset_root_are_rejected_at_parse() {
    for arch in [
        "../../../../etc/pwn",
        "..",
        "/tmp/pwn",
        "/",
        "arm64/../../pwn",
        "arm64/evil",
        "arm64\\evil",
        "arm64\0",
        "\0",
        "",
    ] {
        let error = ManifestV2::from_json(&manifest_with_arch_key(arch))
            .err()
            .unwrap_or_else(|| panic!("arch key {arch:?} must be refused"));
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("architecture"),
            "arch key {arch:?} must be refused as an architecture key, got: {rendered}"
        );
    }
}

#[test]
fn a_release_graph_manifest_gets_the_same_arch_key_check() {
    // The release-graph shape reaches `from_json` through a conversion; the
    // validation runs on the converted manifest, so it must cover that path.
    let digest = |fill: char| {
        serde_json::json!({
            "blake3": fill.to_string().repeat(64),
            "sha256": fill.to_string().repeat(64),
        })
    };
    let graph = serde_json::json!({
        "channel": "stable",
        "version": "1.0.142",
        "status": "current",
        "packages": [{"name": "Capsem-1.5.1.pkg", "version": "1.5.1", "status": "current"}],
        "profiles": {
            "co-work": {
                "name": "Co-work",
                "description": "Shared profile.",
                "revision": "2026.0703.2",
                "status": "current",
                "min_capsem_version": "1.5.0",
                "architectures": [{
                    "architecture": "../../../../etc/pwn",
                    "image_revision": "2026.0714.18",
                    "images": [
                        {"kind": "kernel", "name": "vmlinuz", "bytes": 10, "status": "current", "digest": digest('a')},
                        {"kind": "initrd", "name": "initrd.img", "bytes": 20, "status": "current", "digest": digest('b')},
                        {"kind": "rootfs", "name": "rootfs.erofs", "bytes": 30, "status": "current", "digest": digest('c')}
                    ]
                }]
            }
        }
    });
    let error = ManifestV2::from_json(&graph.to_string()).expect_err("traversal architecture must be refused");
    assert!(format!("{error:#}").contains("architecture"), "{error:#}");
}
