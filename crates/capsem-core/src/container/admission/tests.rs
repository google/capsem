use super::*;

const A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn images(sources: &[&str], admit: &[&str]) -> SettingsFile {
    SettingsFile {
        images: Some(ImagePolicyConfig {
            sources: sources.iter().map(|s| s.to_string()).collect(),
            admit: admit.iter().map(|s| s.to_string()).collect(),
        }),
        ..SettingsFile::default()
    }
}

fn reference(value: &str) -> ImageReference {
    value.parse().unwrap()
}

fn resolved(value: &str, digest: &str) -> ResolvedImage {
    reference(value)
        .resolve(capsem_assets::oci::Digest::parse(digest).unwrap())
        .unwrap()
}

#[test]
fn with_no_policy_exactly_the_catalog_runs() {
    let codex = resolved("ghcr.io/google/capsem/codex-cli", A);
    let policy =
        ImagePolicy::from_files(&SettingsFile::default(), &SettingsFile::default(), vec![codex.clone()]).unwrap();
    policy
        .check_source(&reference("ghcr.io/google/capsem/codex-cli:latest"))
        .unwrap();
    policy.admit(&codex).unwrap();
    // Same repository, a digest the catalog does not list: refused.
    assert!(policy.admit(&resolved("ghcr.io/google/capsem/codex-cli", B)).is_err());
    // The debug image lives beside the catalog's but is not in it.
    assert!(policy
        .check_source(&reference("ghcr.io/google/capsem/capsem-debug:1"))
        .is_err());
    assert!(policy
        .admit(&resolved("ghcr.io/google/capsem/capsem-debug", B))
        .is_err());
    assert!(policy.check_source(&reference("docker.io/library/redis:7")).is_err());
}

#[test]
fn a_mirrored_catalog_digest_is_admitted_wherever_it_came_from() {
    let policy = ImagePolicy::from_files(
        &images(&["mirror.company.com"], &[]),
        &SettingsFile::default(),
        vec![resolved("ghcr.io/google/capsem/claude-code", A)],
    )
    .unwrap();
    policy
        .check_source(&reference("mirror.company.com/capsem/claude-code:x"))
        .unwrap();
    // Admission is by digest: the mirror's copy of the catalog digest runs,
    // anything else from the mirror does not.
    policy
        .admit(&resolved("mirror.company.com/capsem/claude-code", A))
        .unwrap();
    assert!(policy
        .admit(&resolved("mirror.company.com/capsem/claude-code", B))
        .is_err());
}

#[test]
fn granted_registries_are_sources_and_admitted_by_selector() {
    let policy = ImagePolicy::from_files(
        &images(&["registry.company.com"], &["registry.company.com/team/**"]),
        &SettingsFile::default(),
        Vec::new(),
    )
    .unwrap();
    policy
        .check_source(&reference("registry.company.com/team/dev:latest"))
        .unwrap();
    policy.admit(&resolved("registry.company.com/team/dev", A)).unwrap();
    assert!(policy.admit(&resolved("registry.company.com/other/dev", A)).is_err());
    assert!(policy
        .check_source(&reference("registry.company.com.evil.example/team/dev:1"))
        .is_err());
}

#[test]
fn corp_images_replace_the_users() {
    let user = images(&["registry.user.example"], &["registry.user.example"]);
    let corp = images(&["registry.corp.example"], &["registry.corp.example"]);
    let policy = ImagePolicy::from_files(&user, &corp, Vec::new()).unwrap();
    assert!(policy.check_source(&reference("registry.user.example/a:1")).is_err());
    policy.check_source(&reference("registry.corp.example/a:1")).unwrap();
}

#[test]
fn a_selector_that_does_not_parse_fails_the_policy() {
    for bad in ["ghcr.io/a/../b", "ghcr.io/a/b:tag", "GHCR.io/Google/x", ""] {
        let error = ImagePolicy::from_files(&images(&[bad], &[]), &SettingsFile::default(), Vec::new())
            .expect_err(bad)
            .to_string();
        assert!(error.contains("[images] sources"), "{bad}: {error}");
    }
}
