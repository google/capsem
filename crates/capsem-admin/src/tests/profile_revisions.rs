use super::*;

#[test]
fn a_dated_profile_revision_is_rejected_at_release_time() {
    // The scheme being retired. It is URL-path safe, so path validation alone
    // waved it through -- which is how a June date shipped on a July build.
    // Each profile's own revision is what must be semver; the collapsed
    // release identifier may still be a `profiles-<hash>` when one release
    // spans profiles sitting at different versions, which independent
    // versioning makes normal rather than exceptional.
    let profiles = vec![profile_config_file("code", "2026.06.08.9")];

    // Formatted as the operator sees it: anyhow's alternate form prints the
    // whole chain, so the message names both the profile and the value.
    let error = format!(
        "{:#}",
        profile_release_revision(&profiles, ProfileRevisionPolicyArg::Strict).unwrap_err()
    );

    assert!(
        error.contains("2026.06.08.9") && error.contains("code"),
        "rejection must name both the profile and the offending revision: {error}"
    );
}

#[test]
fn a_semver_profile_revision_is_accepted_at_release_time() {
    let profiles = vec![profile_config_file("code", "0.6.0")];

    assert_eq!(
        profile_release_revision(&profiles, ProfileRevisionPolicyArg::Strict).unwrap(),
        "0.6.0"
    );
}

#[test]
fn selected_input_policy_imports_a_legacy_published_revision() {
    let profiles = vec![profile_config_file("co-work", "2026.06.08.7")];

    assert_eq!(
        profile_release_revision(&profiles, ProfileRevisionPolicyArg::SelectedInput).unwrap(),
        "2026.06.08.7"
    );
}

#[test]
fn profiles_at_different_semver_revisions_collapse_to_a_hash_identifier() {
    // Independent versioning means a multi-profile release has no single
    // revision to name. That identifier is not itself semver, and must not be
    // held to it.
    let profiles = vec![
        profile_config_file("code", "0.6.0"),
        profile_config_file("co-work", "0.3.2"),
    ];

    let revision = profile_release_revision(&profiles, ProfileRevisionPolicyArg::Strict).unwrap();

    assert!(
        revision.starts_with("profiles-"),
        "differing revisions must collapse to a content identifier: {revision}"
    );
    assert!(validate_profile_revision_path(&revision).is_ok());
}

#[test]
fn profile_revision_validation_still_rejects_unsafe_paths() {
    // Semver enforcement must not displace the path check it joins.
    assert!(validate_profile_revision_path("../etc/passwd").is_err());
    assert!(validate_profile_revision_path("0.6.0/../..").is_err());

    let profiles = vec![profile_config_file("code", "../etc/passwd")];
    assert!(profile_release_revision(&profiles, ProfileRevisionPolicyArg::SelectedInput).is_err());
}

/// A minimal profile carrying just an id and a revision.
///
/// Built through serde so the fixture tracks the schema: a field gaining a
/// default here should not need a test edit, and a field losing one should
/// fail loudly rather than silently defaulting.
fn profile_config_file(id: &str, revision: &str) -> ProfileConfigFile {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "name": id,
        "description": id,
        "revision": revision,
        "refresh_policy": "manual",
        "assets": { "format": "erofs", "refresh_policy": "manual", "arch": {} }
    }))
    .expect("minimal profile fixture must match ProfileConfigFile")
}
