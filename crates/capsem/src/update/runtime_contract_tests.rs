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
