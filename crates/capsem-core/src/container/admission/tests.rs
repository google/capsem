use super::*;

const A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn images(sources: &[&str], admit: &[&str]) -> SettingsFile {
    SettingsFile {
        images: Some(ImagePolicyConfig {
            sources: sources.iter().map(|s| s.to_string()).collect(),
            admit: admit.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
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
    let policy = ImagePolicy::from_files(&SettingsFile::default(), &SettingsFile::default())
        .unwrap()
        .with_catalog([codex.clone()]);
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
    let policy = ImagePolicy::from_files(&images(&["mirror.company.com"], &[]), &SettingsFile::default())
        .unwrap()
        .with_catalog([resolved("ghcr.io/google/capsem/claude-code", A)]);
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
    let policy = ImagePolicy::from_files(&user, &corp).unwrap();
    assert!(policy.check_source(&reference("registry.user.example/a:1")).is_err());
    policy.check_source(&reference("registry.corp.example/a:1")).unwrap();
}

#[test]
fn a_selector_that_does_not_parse_fails_the_policy() {
    for bad in ["ghcr.io/a/../b", "ghcr.io/a/b:tag", "GHCR.io/Google/x", ""] {
        let error = ImagePolicy::from_files(&images(&[bad], &[]), &SettingsFile::default())
            .expect_err(bad)
            .to_string();
        assert!(error.contains("[images] sources"), "{bad}: {error}");
    }
}

fn with_catalog(catalog: CatalogSetting, ca: Option<&str>) -> SettingsFile {
    let mut file = images(&[], &[]);
    let config = file.images.as_mut().unwrap();
    config.catalog = Some(catalog);
    config.catalog_ca = ca.map(str::to_owned);
    file
}

#[test]
fn the_official_catalog_is_read_unless_a_mirror_is_named_or_it_is_turned_off() {
    let source = |settings: &SettingsFile| {
        ImagePolicy::from_files(settings, &SettingsFile::default())
            .unwrap()
            .catalog_source()
            .cloned()
    };
    let official = Some(CatalogSource {
        reference: DEFAULT_CATALOG.into(),
        ca: None,
    });
    assert_eq!(source(&SettingsFile::default()), official);
    assert_eq!(source(&with_catalog(CatalogSetting::Enabled(true), None)), official);
    assert_eq!(source(&with_catalog(CatalogSetting::Enabled(false), None)), None);
    let mirror = with_catalog(
        CatalogSetting::Reference("mirror.company.com/capsem/catalog:stable".into()),
        Some("/etc/capsem/mirror.pem"),
    );
    assert_eq!(
        source(&mirror),
        Some(CatalogSource {
            reference: "mirror.company.com/capsem/catalog:stable".into(),
            ca: Some("/etc/capsem/mirror.pem".into()),
        })
    );
}

#[test]
fn corp_owns_the_catalog_setting_too() {
    let user = with_catalog(CatalogSetting::Reference("user.example/catalog:stable".into()), None);
    let corp = with_catalog(CatalogSetting::Enabled(false), None);
    let policy = ImagePolicy::from_files(&user, &corp).unwrap();
    assert_eq!(
        policy.catalog_source(),
        None,
        "a user cannot turn corp's catalog back on"
    );
    // A corp [images] without `catalog` reads the official one, not the user's.
    let policy = ImagePolicy::from_files(&user, &images(&["registry.corp.example"], &[])).unwrap();
    assert_eq!(policy.catalog_source().unwrap().reference, DEFAULT_CATALOG);
}

#[test]
fn a_catalog_setting_that_cannot_be_used_fails_the_policy() {
    for (catalog, ca, needle) in [
        (
            CatalogSetting::Reference("catalog:stable".into()),
            None,
            "registry-qualified",
        ),
        (CatalogSetting::Reference("".into()), None, "registry-qualified"),
        (
            CatalogSetting::Reference("https://ghcr.io/x/catalog".into()),
            None,
            "registry-qualified",
        ),
        (CatalogSetting::Enabled(true), Some("relative/ca.pem"), "absolute"),
        (CatalogSetting::Enabled(false), Some("/etc/ca.pem"), "turned off"),
    ] {
        let error = ImagePolicy::from_files(&with_catalog(catalog.clone(), ca), &SettingsFile::default())
            .expect_err(&format!("{catalog:?} {ca:?}"));
        let error = format!("{error:#}");
        assert!(
            error.contains(needle) && error.contains("[images]"),
            "{catalog:?}: {error}"
        );
    }
}

/// The explicit grants answer without the catalog, so a caller that gets
/// `true` never needs to fetch it; the catalog only ever widens the answer.
#[test]
fn explicit_grants_decide_without_the_catalog() {
    let policy = ImagePolicy::from_files(
        &images(&["registry.company.com"], &["registry.company.com/team/**"]),
        &SettingsFile::default(),
    )
    .unwrap();
    assert!(policy.grants_source(&reference("registry.company.com/team/dev:1")));
    assert!(policy.grants(&resolved("registry.company.com/team/dev", A)));
    let codex = resolved("ghcr.io/google/capsem/codex-cli", A);
    assert!(!policy.grants_source(&reference("ghcr.io/google/capsem/codex-cli:1")));
    assert!(!policy.grants(&codex));
    let policy = policy.with_catalog([codex.clone()]);
    assert!(
        !policy.grants(&codex),
        "grants are the explicit policy only, catalog or not"
    );
    policy.admit(&codex).unwrap();
}
