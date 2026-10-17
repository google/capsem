use super::*;

fn pin(repository: &str) -> String {
    format!("{repository}@sha256:{}", "a".repeat(64))
}

#[test]
fn canonical_cache_identity_has_a_stable_opaque_key() {
    let a = CacheIdentity::new(&pin("docker.io/redis"), "amd64", 1).unwrap();
    let b = CacheIdentity::new(&pin("registry-1.docker.io/library/redis"), "amd64", 1).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.image().to_string(), pin("docker.io/library/redis"));
    assert_eq!(a.architecture(), "amd64");
    assert_eq!(a.runtime_contract(), 1);
    assert_eq!(a.key(), b.key());
    assert_eq!(CacheKey::parse(a.key().as_str()).unwrap(), a.key());
    assert!(!a.key().as_str().contains("redis"));
}

#[test]
fn cache_identity_isolates_repository_pin_architecture_and_runtime_contract() {
    let base = CacheIdentity::new(&pin("ghcr.io/team/image"), "amd64", 1).unwrap();
    let alternatives = [
        CacheIdentity::new(&pin("registry.example/team/image"), "amd64", 1).unwrap(),
        CacheIdentity::new(&pin("ghcr.io/other/image"), "amd64", 1).unwrap(),
        CacheIdentity::new(
            &pin("ghcr.io/team/image").replace(&"a".repeat(64), &"b".repeat(64)),
            "amd64",
            1,
        )
        .unwrap(),
        CacheIdentity::new(&pin("ghcr.io/team/image"), "arm64", 1).unwrap(),
        CacheIdentity::new(&pin("ghcr.io/team/image"), "amd64", 2).unwrap(),
    ];
    for other in alternatives {
        assert_ne!(base.key(), other.key());
    }
}

#[test]
fn cache_identity_refuses_unresolved_or_malformed_inputs() {
    for reference in [
        "ghcr.io/team/image:latest",
        "../../image",
        "ghcr.io/team/../image",
        "ghcr.io/team/image@sha256:bad",
    ] {
        assert!(CacheIdentity::new(reference, "amd64", 1).is_err());
    }
    for arch in ["x86_64", "aarch64", "amd64/../../outside", "", "linux/amd64"] {
        assert!(CacheIdentity::new(&pin("ghcr.io/team/image"), arch, 1).is_err());
    }
    assert!(CacheIdentity::new(&pin("ghcr.io/team/image"), "amd64", 0).is_err());
}

#[test]
fn caller_cache_keys_cannot_be_filesystem_locators() {
    for key in [
        "",
        "/tmp/image",
        "../image",
        "oci-../../image",
        "oci-bad",
        "oci-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        assert!(CacheKey::parse(key).is_err());
    }
}
