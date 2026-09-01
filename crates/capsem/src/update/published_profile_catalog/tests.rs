/// A `file://` channel's root is its dist directory, not the filesystem root.
///
/// The generated release channel is a website: the manifest sits at
/// `<dist>/assets/<channel>/manifest.json` and its artifacts are recorded
/// site-root-relative, as `/profiles/releases/...`. Served over HTTP that is
/// exactly right -- the site root is the origin.
///
/// Resolved against a `file://` manifest, `set_path` replaced the whole path
/// and produced `file:///profiles/releases/...`, which is the filesystem root.
/// The gate's install proof hands the postinst a locally built channel, so
/// every hydration of it failed with ENOENT -- and the `apt-get install -f`
/// retry then fell back to the public channel, where the real error was
/// reported against a URL nobody had asked for.
#[test]
fn root_relative_artifacts_resolve_against_a_file_channels_dist_root() {
    let manifest = "file:///src/cache/target/distribution/install-proof/assets/local/manifest.json";

    let resolved = super::resolve_release_channel_artifact_url(
        manifest,
        "/profiles/releases/local/co-work/0.6.0/arm64/initrd.img",
    )
    .expect("resolve");

    assert_eq!(
        resolved,
        "file:///src/cache/target/distribution/install-proof/profiles/releases/local/co-work/0.6.0/arm64/initrd.img"
    );
}

/// And an http channel still resolves against its origin, where the site root
/// and the filesystem root are the same thing.
#[test]
fn root_relative_artifacts_resolve_against_an_http_origin() {
    let resolved = super::resolve_release_channel_artifact_url(
        "https://release.capsem.org/assets/stable/manifest.json",
        "/profiles/releases/stable/code/0.6.0/arm64/initrd.img",
    )
    .expect("resolve");

    assert_eq!(
        resolved,
        "https://release.capsem.org/profiles/releases/stable/code/0.6.0/arm64/initrd.img"
    );
}

/// A relative reference keeps resolving against the manifest itself, and an
/// absolute URL is still taken as given.
#[test]
fn relative_and_absolute_artifact_references_are_unchanged() {
    let manifest = "file:///src/cache/target/distribution/install-proof/assets/local/manifest.json";

    assert_eq!(
        super::resolve_release_channel_artifact_url(manifest, "health.json").expect("relative"),
        "file:///src/cache/target/distribution/install-proof/assets/local/health.json"
    );
    assert_eq!(
        super::resolve_release_channel_artifact_url(manifest, "https://example.test/a.img").expect("absolute"),
        "https://example.test/a.img"
    );
}
