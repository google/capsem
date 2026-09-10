use super::*;

#[test]
fn explicit_images_work_across_registries() {
    for (input, registry, repository) in [
        ("docker.io/library/redis:7-alpine", "docker.io", "library/redis"),
        ("ghcr.io/org/image:v1", "ghcr.io", "org/image"),
        ("quay.io/org/image:v1", "quay.io", "org/image"),
        (
            "registry.example:5443/team/image:latest",
            "registry.example:5443",
            "team/image",
        ),
        ("docker://redis:7-alpine", "docker.io", "library/redis"),
    ] {
        let reference = image_reference(input).unwrap();
        assert_eq!(reference.registry(), registry);
        assert_eq!(reference.repository(), repository);
    }
}

#[test]
fn commands_credentials_and_path_traversal_are_not_images() {
    for input in [
        "",
        "redis",
        "echo hello",
        "https://ghcr.io/a/b",
        "user:secret@ghcr.io/a/b",
        "../image",
        "ghcr.io/../image",
        "ghcr.io/a/b\n",
        "ghcr.io/a/b?token=secret",
    ] {
        assert!(image_reference(input).is_err(), "accepted {input:?}");
    }
}

#[test]
fn digests_cannot_escape_the_blob_directory() {
    let valid = format!("sha256:{}", "a".repeat(64));
    assert_eq!(digest_hex(&valid).unwrap(), "a".repeat(64));
    for input in ["sha256:../../outside", "sha256:ab", "sha512:abcd", "", "sha256:"] {
        assert!(digest_hex(input).is_err(), "accepted {input:?}");
    }
    assert!(digest_hex(&format!("sha256:{}", "G".repeat(64))).is_err());
}

#[test]
fn pinned_single_platform_images_still_require_native_linux() {
    let config = br#"{"architecture":"arm64","os":"linux"}"#;
    verify_platform(config, "arm64").unwrap();
    assert!(verify_platform(config, "amd64").is_err());
    assert!(verify_platform(br#"{"architecture":"arm64","os":"darwin"}"#, "arm64").is_err());
    assert!(verify_platform(b"{}", "arm64").is_err());
}
