use super::*;
use serde_json::json;

fn donor(channel: &str) -> Value {
    json!({
        "version": "1.0.143",
        "channel": channel,
        "status": "current",
        "packages": [
            {
                "id": "capsem-1-5-0-deb-arm64",
                "kind": "debian_package",
                "name": "Capsem_1.5.0_arm64.deb",
                "version": "1.5.0",
                "platform": "linux",
                "architecture": "arm64",
                "bytes": 123,
                "url": "https://github.com/google/capsem/releases/download/v1.5.0/Capsem_1.5.0_arm64.deb",
                "digest": {
                    "sha256": "a".repeat(64),
                    "blake3": "b".repeat(64)
                },
                "status": "current",
                "binaries": [
                    {
                        "name": "capsem",
                        "description": "Capsem command-line executable",
                        "version": "1.5.0",
                        "installed_path": "/usr/local/bin/capsem",
                        "platform": "linux",
                        "architecture": "arm64",
                        "bytes": 7,
                        "digest": {
                            "sha256": "c".repeat(64),
                            "blake3": "d".repeat(64)
                        },
                        "status": "current",
                        "sbom_component_ref": "SPDXRef-File-capsem"
                    }
                ]
            }
        ],
        "runtime": {
            "revision": "stable-only"
        }
    })
}

/// A 0.6 public graph: the shape a retired-graph bootstrap replaces.
fn retired_profile_graph(channel: &str) -> Value {
    let mut graph = donor(channel);
    let graph_fields = graph.as_object_mut().expect("graph object");
    graph_fields.remove("runtime");
    graph_fields.insert("profiles".to_string(), json!({"code": {"revision": "0.6.2"}}));
    graph
}

#[test]
fn missing_channel_bootstrap_copies_only_official_packages() {
    let donor = donor("stable");
    let before = donor.clone();

    let bootstrapped = bootstrap_first_party_channel_source("nightly", &donor).expect("bootstrap nightly");

    assert_eq!(bootstrapped["channel"], "nightly");
    assert_eq!(bootstrapped["version"], donor["version"]);
    assert_eq!(bootstrapped["status"], donor["status"]);
    assert_eq!(bootstrapped["packages"], donor["packages"]);
    assert!(
        bootstrapped.get("runtime").is_none(),
        "the donor's runtime belongs to the donor channel"
    );
    assert!(bootstrapped.get("profiles").is_none());
    assert_eq!(donor, before, "the donor channel must remain byte-for-byte unchanged");
}

#[test]
fn missing_channel_bootstrap_rejects_unsupported_channels() {
    let stable = donor("stable");

    for channel in ["stable", "corp", "experimental"] {
        assert!(
            bootstrap_first_party_channel_source(channel, &stable).is_err(),
            "{channel} must not be bootstrapped from stable"
        );
    }
}

#[test]
fn missing_channel_bootstrap_rejects_unofficial_or_empty_package_cohorts() {
    let mut unofficial = donor("stable");
    unofficial["packages"][0]["url"] = json!("https://example.com/releases/download/v1.5.0/Capsem.deb");
    assert!(bootstrap_first_party_channel_source("nightly", &unofficial).is_err());

    let mut empty = donor("stable");
    empty["packages"] = json!([]);
    assert!(bootstrap_first_party_channel_source("nightly", &empty).is_err());
}

#[test]
fn missing_channel_bootstrap_source_allows_explicit_empty_membership() {
    let bootstrapped = bootstrap_first_party_channel_source("nightly", &donor("stable")).expect("bootstrap");

    crate::validate_assets_channel_graph_manifest(&bootstrapped, "nightly")
        .expect("a channel publishes no runtime before its first runtime release");
}

#[test]
fn exact_retired_channel_bootstrap_removes_both_dead_families() {
    let retired = retired_profile_graph("stable");
    let before = retired.clone();

    let bootstrapped =
        bootstrap_retired_first_party_channel_source("stable", &retired).expect("retire exact stable source");

    assert_eq!(bootstrapped["channel"], "stable");
    assert_eq!(bootstrapped["version"], retired["version"]);
    assert_eq!(bootstrapped["status"], "current");
    assert_eq!(bootstrapped["packages"], json!([]));
    assert!(
        bootstrapped.get("profiles").is_none(),
        "the retired profile family is dropped"
    );
    assert!(bootstrapped.get("runtime").is_none());
    crate::validate_assets_channel_graph_manifest(&bootstrapped, "stable")
        .expect("a retired channel restarts as a valid empty runtime graph");
    assert_eq!(retired, before, "retirement must not mutate its input");
}

#[test]
fn retired_channel_bootstrap_rejects_relabeling_or_non_first_party_sources() {
    let stable = donor("stable");
    assert!(bootstrap_retired_first_party_channel_source("nightly", &stable).is_err());

    let mut corp = donor("stable");
    corp["channel"] = json!("corp");
    assert!(bootstrap_retired_first_party_channel_source("corp", &corp).is_err());
}

#[test]
fn retired_graph_digest_is_a_canonical_lowercase_sha256() {
    assert!("a".repeat(64).parse::<RetiredGraphSha256>().is_ok());
    for malformed in ["A".repeat(64), "a".repeat(63), "main".to_string()] {
        assert!(
            malformed.parse::<RetiredGraphSha256>().is_err(),
            "{malformed} must not cross the digest boundary"
        );
    }
}

fn legacy_source(channel: &str) -> Value {
    let mut graph = retired_profile_graph(channel);
    let package = &mut graph["packages"][0];
    package["id"] = json!("capsem-0-6-9-deb-arm64");
    package["name"] = json!("Capsem_0.6.9_arm64.deb");
    package["version"] = json!("0.6.9");
    package["url"] = json!("https://github.com/google/capsem/releases/download/v0.6.9/Capsem_0.6.9_arm64.deb");
    package["binaries"][0]["version"] = json!("0.6.9");
    graph
}

#[test]
fn legacy_runtime_source_preserves_every_package_field_and_channel() {
    for channel in ["stable", "nightly"] {
        let legacy = legacy_source(channel);
        let migrated = prepare_runtime_release_base(legacy.clone(), channel).expect("migrate source");
        let mut expected = legacy;
        expected.as_object_mut().expect("source object").remove("profiles");
        assert_eq!(migrated, expected, "only the legacy runtime unit may leave the source");
        assert!(!migrated["packages"].as_array().expect("packages").is_empty());
        assert!(migrated.get("runtime").is_none());
    }
}

#[test]
fn legacy_source_migration_refuses_foreign_or_relabelled_channels() {
    for channel in ["nightly", "corp"] {
        assert!(prepare_runtime_release_base(legacy_source("stable"), channel).is_err());
    }
}

#[test]
fn legacy_source_migration_refuses_mixed_or_malformed_state() {
    let legacy = legacy_source("stable");
    for (key, value) in [
        ("runtime", json!(null)),
        ("runtime", json!({"revision": "already-staged"})),
        ("profiles", json!({})),
        ("profiles", json!([])),
        ("packages", json!([])),
        ("status", json!("deprecated")),
        ("unexpected_metadata", json!(true)),
    ] {
        let mut invalid = legacy.clone();
        invalid[key] = value;
        assert!(
            prepare_runtime_release_base(invalid, "stable").is_err(),
            "{key} must not be discarded"
        );
    }
}

#[test]
fn legacy_source_migration_refuses_other_package_lines_or_publishers() {
    for version in ["0.5.9", "0.7.0", "1.0.0", "not-semver"] {
        let mut invalid = legacy_source("stable");
        invalid["packages"][0]["version"] = json!(version);
        assert!(prepare_runtime_release_base(invalid, "stable").is_err(), "{version}");
    }
    let mut foreign = legacy_source("stable");
    foreign["packages"][0]["url"] = json!("https://example.com/Capsem.deb");
    assert!(prepare_runtime_release_base(foreign, "stable").is_err());
}

#[test]
fn ordinary_runtime_sources_keep_strict_validation() {
    let modern = bootstrap_first_party_channel_source("nightly", &donor("stable")).expect("modern source");
    assert_eq!(
        prepare_runtime_release_base(modern.clone(), "nightly").expect("valid"),
        modern
    );
    let mut invalid = modern;
    invalid["unexpected_metadata"] = json!(true);
    assert!(prepare_runtime_release_base(invalid, "nightly").is_err());
}
