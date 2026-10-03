use super::*;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn selector(value: &str) -> ImageSelector {
    value
        .parse()
        .unwrap_or_else(|error| panic!("rejected selector {value:?}: {error:#}"))
}

fn reference(value: &str) -> ImageReference {
    value
        .parse()
        .unwrap_or_else(|error| panic!("rejected reference {value:?}: {error:#}"))
}

/// An image as admission sees it: a repository and the digest it resolved to.
fn image(repository: &str, hex: &str) -> ResolvedImage {
    reference(repository).resolve(digest(hex)).unwrap()
}

fn digest(hex: &str) -> Digest {
    Digest::parse(&format!("sha256:{hex}")).unwrap()
}

fn admitted(selector_value: &str, repository: &str) -> bool {
    selector(selector_value).matches(&image(repository, A))
}

#[test]
fn selectors_parse_into_their_four_scopes() {
    assert_eq!(
        selector("registry.company.com"),
        ImageSelector::Registry("registry.company.com".into())
    );
    assert_eq!(
        selector("registry.company.com:5000"),
        ImageSelector::Registry("registry.company.com:5000".into())
    );
    assert_eq!(selector("localhost"), ImageSelector::Registry("localhost".into()));
    assert!(matches!(
        selector("ghcr.io/company/agents/**"),
        ImageSelector::Subtree(_)
    ));
    assert!(matches!(
        selector("ghcr.io/company/agents/codex"),
        ImageSelector::Repository(_)
    ));
    assert!(matches!(
        selector(&format!("ghcr.io/company/agents/codex@sha256:{A}")),
        ImageSelector::Image(_)
    ));
}

#[test]
fn selectors_display_in_canonical_form() {
    for (input, canonical) in [
        ("REGISTRY.Company.com", "registry.company.com"),
        ("ghcr.io/company/agents/**", "ghcr.io/company/agents/**"),
        ("redis", "docker.io/library/redis"),
        ("index.docker.io/company/**", "docker.io/company/**"),
        ("ghcr.io/**", "ghcr.io"),
    ] {
        assert_eq!(selector(input).to_string(), canonical, "{input}");
        assert_eq!(selector(canonical), selector(input), "{input} does not round-trip");
    }
    let pinned = format!("ghcr.io/a/b@sha256:{A}");
    assert_eq!(selector(&format!("ghcr.io/a/b:v1@sha256:{A}")).to_string(), pinned);
}

#[test]
fn registry_selector_matches_every_repository_on_that_registry_only() {
    assert!(admitted("registry.company.com", "registry.company.com/x"));
    assert!(admitted("registry.company.com", "registry.company.com/team/deep/image"));
    assert!(!admitted("registry.company.com", "ghcr.io/x/y"));
}

#[test]
fn registry_selector_is_not_a_suffix_or_prefix_match() {
    assert!(!admitted("registry.company.com", "registry.company.com.evil.example/x"));
    assert!(!admitted("registry.company.com", "evilregistry.company.com/x"));
    assert!(!admitted("registry.company.com", "evil.registry.company.com/x"));
    assert!(!admitted("company.com", "registry.company.com/x"));
}

#[test]
fn an_explicit_port_is_part_of_the_authority() {
    assert!(!admitted("registry.company.com", "registry.company.com:5000/x"));
    assert!(!admitted("registry.company.com:5000", "registry.company.com/x"));
    assert!(!admitted("registry.company.com:5000", "registry.company.com:5001/x"));
    assert!(admitted("registry.company.com:5000", "registry.company.com:5000/x"));
    assert!(!admitted("docker.io", "docker.io:443/library/redis"));
}

#[test]
fn authority_case_folds() {
    assert!(admitted("ghcr.io", "GHCR.io/google/x"));
    assert!(admitted("GHCR.IO", "ghcr.io/google/x"));
    assert!(admitted("ghcr.io/google/x", "GHCR.io/google/x"));
    assert!(admitted("GHCR.io/google/**", "ghcr.io/google/x"));
}

#[test]
fn path_case_never_folds() {
    // OCI repository paths are lowercase by grammar. An uppercase path is
    // refused rather than folded, so `Google` can never stand for `google`.
    for value in ["ghcr.io/Google/x", "GHCR.io/Google/x", "Google/x", "ghcr.io/google/X"] {
        assert!(value.parse::<ImageReference>().is_err(), "accepted reference {value:?}");
        assert!(value.parse::<ImageSelector>().is_err(), "accepted selector {value:?}");
    }
    assert!("ghcr.io/Google/**".parse::<ImageSelector>().is_err());
}

#[test]
fn docker_hub_spellings_are_one_identity() {
    for spelling in [
        "redis",
        "library/redis",
        "docker.io/redis",
        "docker.io/library/redis",
        "index.docker.io/library/redis",
        "registry-1.docker.io/library/redis",
        "registry-1.docker.io/redis",
        "DOCKER.IO/library/redis",
    ] {
        let parsed = reference(spelling);
        assert_eq!(parsed.repository().authority(), "docker.io", "{spelling}");
        assert_eq!(parsed.repository().path(), "library/redis", "{spelling}");
        for scope in ["redis", "docker.io/library/redis", "index.docker.io/redis", "docker.io"] {
            assert!(admitted(scope, spelling), "{scope} did not match {spelling}");
        }
    }
    for registry in ["docker.io", "index.docker.io", "registry-1.docker.io", "Docker.IO"] {
        assert_eq!(selector(registry), ImageSelector::Registry("docker.io".into()));
    }
}

#[test]
fn library_prefix_is_added_only_for_single_component_hub_repositories() {
    assert_eq!(reference("company/app").repository().path(), "company/app");
    assert!(!admitted("redis", "company/redis"));
    assert!(!admitted("docker.io/library/redis", "ghcr.io/redis"));
    // A non-Hub registry never gains `library/`.
    assert_eq!(reference("ghcr.io/redis").repository().path(), "redis");
}

#[test]
fn subtree_matches_descendants_at_any_depth() {
    assert!(admitted("ghcr.io/company/agents/**", "ghcr.io/company/agents/x"));
    assert!(admitted("ghcr.io/company/agents/**", "ghcr.io/company/agents/x/y"));
}

/// A subtree means "strictly beneath". `agents/**` does not admit the
/// repository `agents` itself; a policy that wants both names both.
#[test]
fn subtree_does_not_match_its_own_root() {
    assert!(!admitted("ghcr.io/company/agents/**", "ghcr.io/company/agents"));
}

#[test]
fn subtree_does_not_accept_a_sibling_or_a_parent() {
    let subtree = "ghcr.io/company/agents/**";
    assert!(!admitted(subtree, "ghcr.io/company/agents-evil/x"));
    assert!(!admitted(subtree, "ghcr.io/company/agentsx"));
    assert!(!admitted(subtree, "ghcr.io/company/agents.evil/x"));
    assert!(!admitted(subtree, "ghcr.io/company/x"));
    assert!(!admitted(subtree, "ghcr.io/company"));
    assert!(!admitted(subtree, "ghcr.io.evil.example/company/agents/x"));
    assert!(!admitted(subtree, "registry.company.com/company/agents/x"));
}

#[test]
fn hub_subtree_prefix_is_a_namespace_not_a_repository() {
    // `docker.io/library/**` is every official image; it must not become
    // `library/library/**` the way a single-component repository would.
    assert!(admitted("docker.io/library/**", "redis"));
    assert!(admitted("docker.io/company/**", "company/app"));
    assert!(!admitted("docker.io/company/**", "redis"));
}

#[test]
fn registry_root_subtree_is_the_registry() {
    assert_eq!(selector("ghcr.io/**"), ImageSelector::Registry("ghcr.io".into()));
    assert_eq!(
        selector("index.docker.io/**"),
        ImageSelector::Registry("docker.io".into())
    );
}

#[test]
fn exact_repository_matches_only_itself() {
    let exact = "ghcr.io/company/agents/codex";
    assert!(admitted(exact, "ghcr.io/company/agents/codex"));
    assert!(!admitted(exact, "ghcr.io/company/agents/codex/sub"));
    assert!(!admitted(exact, "ghcr.io/company/agents/codex-evil"));
    assert!(!admitted(exact, "ghcr.io/company/agents"));
}

#[test]
fn exact_repository_admits_any_digest_of_it() {
    let exact = selector("ghcr.io/company/agents/codex");
    assert!(exact.matches(&image("ghcr.io/company/agents/codex", A)));
    assert!(exact.matches(&image("ghcr.io/company/agents/codex", B)));
}

#[test]
fn digest_selector_matches_only_that_digest() {
    let pinned = selector(&format!("ghcr.io/company/agents/codex@sha256:{A}"));
    assert!(pinned.matches(&image("ghcr.io/company/agents/codex", A)));
    assert!(!pinned.matches(&image("ghcr.io/company/agents/codex", B)));
    assert!(!pinned.matches(&image("ghcr.io/company/agents/other", A)));
    assert!(!pinned.matches(&image("mirror.company.com/company/agents/codex", A)));
}

#[test]
fn digest_wins_over_tag_in_references_and_selectors() {
    let tagged = reference(&format!("ghcr.io/a/b:v1@sha256:{A}"));
    assert_eq!(tagged, reference(&format!("ghcr.io/a/b@sha256:{A}")));
    assert_eq!(tagged.digest(), Some(&digest(A)));
    assert_eq!(
        selector(&format!("ghcr.io/a/b:anything@sha256:{A}")),
        selector(&format!("ghcr.io/a/b@sha256:{A}"))
    );
}

#[test]
fn tag_only_reference_never_matches_a_digest_selector_as_a_source() {
    let pinned = selector(&format!("ghcr.io/a/b@sha256:{A}"));
    assert!(!pinned.matches_source(&reference("ghcr.io/a/b:v1")));
    assert!(!pinned.matches_source(&reference("ghcr.io/a/b")));
    assert!(!pinned.matches_source(&reference(&format!("ghcr.io/a/b@sha256:{B}"))));
    assert!(pinned.matches_source(&reference(&format!("ghcr.io/a/b:v1@sha256:{A}"))));
}

#[test]
fn source_checks_use_the_same_scopes_before_resolution() {
    let tag_only = reference("registry.company.com/team/app:v1");
    assert!(selector("registry.company.com").matches_source(&tag_only));
    assert!(selector("registry.company.com/team/**").matches_source(&tag_only));
    assert!(selector("registry.company.com/team/app").matches_source(&tag_only));
    assert!(!selector("registry.company.com.evil.example").matches_source(&tag_only));
}

#[test]
fn a_reference_resolves_only_to_the_digest_it_pins() {
    let pinned = reference(&format!("ghcr.io/a/b@sha256:{A}"));
    assert!(pinned.clone().resolve(digest(A)).is_ok());
    assert!(
        pinned.resolve(digest(B)).is_err(),
        "a pinned reference resolved to another digest"
    );
    let resolved = reference("ghcr.io/a/b:v1").resolve(digest(B)).unwrap();
    assert_eq!(resolved.digest(), &digest(B));
    assert_eq!(resolved.repository(), reference("ghcr.io/a/b").repository());
}

#[test]
fn tags_are_not_selectors() {
    for value in ["ghcr.io/a/b:v1", "ghcr.io/a/b:latest", "registry.company.com:5000/a:v1"] {
        assert!(
            value.parse::<ImageSelector>().is_err(),
            "accepted tag selector {value:?}"
        );
    }
}

#[test]
fn malformed_names_are_rejected_for_selectors_and_references() {
    for value in [
        "",
        "/",
        "ghcr.io/",
        "ghcr.io/a/",
        "ghcr.io/a/b/",
        "ghcr.io//a",
        "ghcr.io/a//b",
        "ghcr.io/./a",
        "ghcr.io/a/.",
        "ghcr.io/../a",
        "ghcr.io/a/../b",
        "ghcr.io/a/..",
        "../a",
        "./a",
        "ghcr.io/a b",
        "ghcr.io/a\n",
        "https://ghcr.io/a/b",
        "docker://redis",
        "user:secret@ghcr.io/a/b",
        "ghcr.io/a/b?token=secret",
        "ghcr.io/a/-b",
        "ghcr.io/a/b-",
        "ghcr.io/caf\u{e9}",
        "ghcr.io:99999/a",
        "ghcr.io:0/a",
        "ghcr..io/a",
        ".ghcr.io/a",
        "-ghcr.io/a",
        "ghcr.io./a",
    ] {
        assert!(value.parse::<ImageReference>().is_err(), "accepted reference {value:?}");
        assert!(value.parse::<ImageSelector>().is_err(), "accepted selector {value:?}");
    }
}

#[test]
fn malformed_subtree_and_registry_selectors_are_rejected() {
    for value in [
        "**",
        "/**",
        "ghcr.io/**/x",
        "ghcr.io/a/**/",
        "ghcr.io/a/*",
        "ghcr.io/a*/**",
        "ghcr.io/a/**/**",
        "ghcr.io/a//**",
        "ghcr.io/./**",
        "ghcr.io/../**",
        "ghcr.io/a:v1/**",
        &format!("ghcr.io/a@sha256:{A}/**"),
        "ghcr.io.",
        "ghcr.io:",
        "ghcr.io:http",
        "registry.company.com.:5000",
    ] {
        assert!(value.parse::<ImageSelector>().is_err(), "accepted selector {value:?}");
    }
}

#[test]
fn malformed_digests_are_rejected_everywhere() {
    for bad in [
        format!("sha256:{}", &A[..63]),
        format!("sha256:{A}a"),
        format!("sha256:{}", "A".repeat(64)),
        format!("sha256:{}g", &A[..63]),
        format!("sha512:{A}{A}"),
        format!("sha384:{}", "a".repeat(96)),
        format!("md5:{}", &A[..32]),
        format!("SHA256:{A}"),
        A.to_string(),
        "sha256:".into(),
        String::new(),
    ] {
        assert!(Digest::parse(&bad).is_err(), "accepted digest {bad:?}");
        assert!(
            format!("ghcr.io/a/b@{bad}").parse::<ImageReference>().is_err(),
            "accepted reference digest {bad:?}"
        );
        assert!(
            format!("ghcr.io/a/b@{bad}").parse::<ImageSelector>().is_err(),
            "accepted selector digest {bad:?}"
        );
    }
}

#[test]
fn references_convert_from_the_pull_reference() {
    let pull = crate::oci::image_reference(&format!("docker://redis:7@sha256:{A}")).unwrap();
    let converted = ImageReference::try_from(&pull).unwrap();
    assert_eq!(converted, reference(&format!("docker.io/library/redis@sha256:{A}")));
    let pull = crate::oci::image_reference("registry-1.docker.io/redis:7").unwrap();
    assert_eq!(ImageReference::try_from(&pull).unwrap(), reference("redis"));
}

#[test]
fn an_empty_policy_admits_nothing() {
    let everything = image("ghcr.io/a/b", A);
    assert!(!admits(&[], &everything));
    assert!(!allows_source(&[], &reference("ghcr.io/a/b")));
}

#[test]
fn a_policy_admits_when_any_selector_matches() {
    let policy = [
        selector("registry.company.com"),
        selector(&format!("ghcr.io/a/b@sha256:{A}")),
    ];
    assert!(admits(&policy, &image("registry.company.com/x", B)));
    assert!(admits(&policy, &image("ghcr.io/a/b", A)));
    assert!(!admits(&policy, &image("ghcr.io/a/b", B)));
    assert!(!admits(&policy, &image("ghcr.io/a/c", A)));
    assert!(allows_source(&policy, &reference("registry.company.com/x:v1")));
    assert!(!allows_source(&policy, &reference("ghcr.io/a/b:v1")));
}
