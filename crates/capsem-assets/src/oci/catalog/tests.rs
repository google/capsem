use super::*;
use serde_json::{json, Value};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const E: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn image(hex: &str) -> String {
    format!("ghcr.io/google/capsem/codex@sha256:{hex}")
}

fn version(hex: &str, platforms: &[&str], contract: u32) -> Value {
    json!({"image": image(hex), "platforms": platforms, "contract": contract})
}

/// Oldest to newest: A (both arches, contract 1), B (arm64 only, contract 1),
/// C (both arches, contract 2).
fn document() -> Value {
    json!({
        "schema_version": 1,
        "channel": "stable",
        "entries": {
            "codex": {
                "description": "OpenAI Codex CLI",
                "icon": "codex.svg",
                "versions": [
                    {"image": image(A), "platforms": ["linux/arm64", "linux/amd64"], "contract": 1,
                     "obom": {"arm64": format!("sha256:{E}"), "amd64": format!("sha256:{B}")},
                     "erofs": {"arm64": format!("sha256:{E}"), "amd64": format!("sha256:{E}")}},
                    version(B, &["linux/arm64"], 1),
                    version(C, &["linux/arm64", "linux/amd64"], 2),
                ]
            },
            "claude-code": {
                "description": "Claude Code",
                "versions": [{"image": format!("ghcr.io/google/capsem/claude-code@sha256:{E}"),
                              "platforms": ["linux/amd64"], "contract": 1}]
            }
        }
    })
}

fn parse(value: &Value) -> Result<Catalog> {
    Catalog::parse(&serde_json::to_vec(value).unwrap())
}

fn rejected(value: &Value, needle: &str) {
    let error = format!("{:#}", parse(value).expect_err("invalid catalog accepted"));
    assert!(error.contains(needle), "expected {needle:?} in {error:?}");
}

fn digest(hex: &str) -> Digest {
    Digest::parse(&format!("sha256:{hex}")).unwrap()
}

#[test]
fn valid_document_parses_into_typed_entries() {
    let catalog = parse(&document()).unwrap();
    assert_eq!(catalog.channel(), Channel::Stable);
    let codex = catalog.entry("codex").unwrap();
    assert_eq!(codex.description(), "OpenAI Codex CLI");
    assert_eq!(codex.icon(), Some("codex.svg"));
    let first = &codex.versions()[0];
    assert_eq!(first.image().digest(), &digest(A));
    assert_eq!(first.image().repository().to_string(), "ghcr.io/google/capsem/codex");
    assert_eq!(
        first.platforms(),
        [PackageArchitecture::Arm64, PackageArchitecture::Amd64]
    );
    assert_eq!(first.contract(), 1);
    assert_eq!(first.obom(PackageArchitecture::Arm64), Some(&digest(E)));
    assert_eq!(first.obom(PackageArchitecture::Amd64), Some(&digest(B)));
    assert_eq!(catalog.generated_at(), None);
    assert_eq!(first.erofs(PackageArchitecture::Amd64), Some(&digest(E)));
    assert_eq!(codex.versions()[1].erofs(PackageArchitecture::Arm64), None);
    assert_eq!(catalog.entry("claude-code").unwrap().icon(), None);
    assert!(catalog.entry("missing").is_none());
    let mut nightly = document();
    nightly["channel"] = json!("nightly");
    assert_eq!(parse(&nightly).unwrap().channel(), Channel::Nightly);
}

#[test]
fn unknown_fields_are_refused_at_every_level() {
    let mut top = document();
    top["signature"] = json!("x");
    rejected(&top, "unknown field");
    let mut entry = document();
    entry["entries"]["codex"]["homepage"] = json!("x");
    rejected(&entry, "unknown field");
    let mut version = document();
    version["entries"]["codex"]["versions"][0]["tag"] = json!("latest");
    rejected(&version, "unknown field");
    let mut erofs = document();
    erofs["entries"]["codex"]["versions"][0]["erofs"]["riscv64"] = json!(format!("sha256:{E}"));
    rejected(&erofs, "unknown variant");
}

#[test]
fn schema_version_and_channel_are_closed_sets() {
    let mut schema = document();
    schema["schema_version"] = json!(2);
    rejected(&schema, "schema_version");
    let mut channel = document();
    channel["channel"] = json!("beta");
    rejected(&channel, "unknown variant");
}

#[test]
fn images_must_be_pinned_by_a_valid_digest() {
    for (bad, needle) in [
        ("ghcr.io/google/capsem/codex:latest", "pinned"),
        ("ghcr.io/google/capsem/codex", "pinned"),
        ("ghcr.io/google/capsem/codex@sha256:abc", "codex"),
        ("ghcr.io/google/capsem/codex@sha512:abc", "codex"),
        ("ghcr.io/Google/capsem/codex@sha256:{A}", "codex"),
    ] {
        let mut value = document();
        value["entries"]["codex"]["versions"][0]["image"] = json!(bad.replace("{A}", A));
        rejected(&value, needle);
    }
    for field in ["obom", "erofs"] {
        let mut value = document();
        value["entries"]["codex"]["versions"][0][field] = json!({"arm64": "sha256:XYZ"});
        rejected(&value, &format!("invalid {field} digest for arm64"));
        // Each architecture has its own artifact: a bare digest is refused.
        value["entries"]["codex"]["versions"][0][field] = json!(format!("sha256:{E}"));
        rejected(&value, "invalid type");
    }
}

/// The exact shape `images/ci/catalog.py` publishes: `generated_at`, sorted
/// keys, platforms sorted by architecture, no `icon`, and per-architecture
/// `obom` and `erofs` maps on every version.
#[test]
fn parses_the_document_the_publishing_workflow_writes() {
    let published = format!(
        r#"{{
  "channel": "nightly",
  "entries": {{
    "claude-code": {{
      "description": "Claude Code, Anthropic's coding agent",
      "versions": [
        {{
          "contract": 1,
          "erofs": {{
            "amd64": "sha256:{A}",
            "arm64": "sha256:{B}"
          }},
          "image": "ghcr.io/google/capsem/claude-code@sha256:{C}",
          "obom": {{
            "amd64": "sha256:{E}",
            "arm64": "sha256:{A}"
          }},
          "platforms": [
            "linux/amd64",
            "linux/arm64"
          ]
        }}
      ]
    }}
  }},
  "generated_at": "2026-10-03T12:00:00Z",
  "schema_version": 1
}}
"#
    );
    let catalog = Catalog::parse(published.as_bytes()).unwrap();
    assert_eq!(catalog.channel(), Channel::Nightly);
    assert_eq!(catalog.generated_at(), Some("2026-10-03T12:00:00Z"));
    let version = &catalog.entry("claude-code").unwrap().versions()[0];
    assert_eq!(version.obom(PackageArchitecture::Amd64), Some(&digest(E)));
    assert_eq!(version.obom(PackageArchitecture::Arm64), Some(&digest(A)));
    assert_eq!(version.erofs(PackageArchitecture::Arm64), Some(&digest(B)));
    assert_eq!(
        catalog
            .resolve("claude-code", PackageArchitecture::Arm64, RUNTIME_CONTRACT)
            .unwrap()
            .to_string(),
        format!("ghcr.io/google/capsem/claude-code@sha256:{C}")
    );
}

#[test]
fn entry_names_are_short_lowercase_slugs() {
    let long = "a".repeat(64);
    for bad in [
        "",
        "-codex",
        "Codex",
        "codex_cli",
        "codex/cli",
        "codex.cli",
        long.as_str(),
    ] {
        let mut value = document();
        let entry = value["entries"].as_object_mut().unwrap().remove("codex").unwrap();
        value["entries"][bad] = entry;
        rejected(&value, "entry name");
    }
    let mut value = document();
    let entry = value["entries"].as_object_mut().unwrap().remove("codex").unwrap();
    value["entries"]["a".repeat(63)] = entry.clone();
    value["entries"]["0-x"] = entry;
    parse(&value).unwrap();
}

/// The predicate every entrypoint uses to decide whether a request names a
/// catalog entry. Anything with a registry, tag or digest separator is a
/// reference; a bare Docker Hub short name has a catalog name's shape.
#[test]
fn catalog_names_never_contain_reference_separators() {
    for name in ["codex-cli", "claude-code", "agy", "dev", "redis", "0-x"] {
        assert!(is_catalog_name(name), "{name}");
    }
    let long = "a".repeat(64);
    for reference in [
        "",
        "-codex",
        "Codex",
        "codex:latest",
        "library/redis",
        "ghcr.io/google/capsem/codex-cli",
        "codex@sha256:aaaa",
        "docker://redis",
        "codex.cli",
        long.as_str(),
    ] {
        assert!(!is_catalog_name(reference), "{reference}");
    }
}

#[test]
fn platforms_are_known_linux_architectures_listed_once() {
    for (platforms, needle) in [
        (json!(["linux/riscv64"]), "platform"),
        (json!(["darwin/arm64"]), "platform"),
        (json!(["arm64"]), "platform"),
        (json!([]), "platform"),
        (json!(["linux/arm64", "linux/arm64"]), "platform"),
    ] {
        let mut value = document();
        value["entries"]["codex"]["versions"][1]["platforms"] = platforms;
        rejected(&value, needle);
    }
    let mut value = document();
    value["entries"]["codex"]["versions"][1]["erofs"] = json!({"amd64": format!("sha256:{E}")});
    rejected(&value, "erofs");
}

#[test]
fn an_entry_lists_each_digest_once_and_at_least_one_version() {
    let mut duplicate = document();
    duplicate["entries"]["codex"]["versions"][2]["image"] = json!(format!("ghcr.io/google/capsem/other@sha256:{A}"));
    rejected(&duplicate, "duplicate");
    let mut empty = document();
    empty["entries"]["codex"]["versions"] = json!([]);
    rejected(&empty, "no versions");
    // The same digest under two entries is two listings, not a duplicate.
    let mut shared = document();
    shared["entries"]["claude-code"]["versions"][0]["image"] = json!(image(A));
    parse(&shared).unwrap();
}

#[test]
fn resolution_picks_the_newest_compatible_version() {
    let catalog = parse(&document()).unwrap();
    let resolve = |arch, contract| {
        catalog
            .resolve("codex", arch, contract)
            .map(|image| image.digest().clone())
    };
    assert_eq!(resolve(PackageArchitecture::Arm64, 2).unwrap(), digest(C));
    assert_eq!(resolve(PackageArchitecture::Amd64, 5).unwrap(), digest(C));
    // Contract filtering: C needs contract 2.
    assert_eq!(resolve(PackageArchitecture::Arm64, 1).unwrap(), digest(B));
    // Platform filtering: B is arm64 only.
    assert_eq!(resolve(PackageArchitecture::Amd64, 1).unwrap(), digest(A));
    let error = format!("{:#}", resolve(PackageArchitecture::Amd64, 0).unwrap_err());
    assert!(error.contains("codex") && error.contains("contract 0"), "{error}");
    let error = format!(
        "{:#}",
        catalog
            .resolve("claude-code", PackageArchitecture::Arm64, RUNTIME_CONTRACT)
            .unwrap_err()
    );
    assert!(error.contains("linux/arm64"), "{error}");
    let error = format!(
        "{:#}",
        catalog
            .resolve("gemini", PackageArchitecture::Arm64, RUNTIME_CONTRACT)
            .unwrap_err()
    );
    assert!(
        error.contains("gemini") && error.contains("not in the stable catalog"),
        "{error}"
    );
}

#[test]
fn supported_lists_every_digest_and_revocation_is_omission() {
    let catalog = parse(&document()).unwrap();
    let mut supported: Vec<_> = catalog.supported().map(|image| image.digest().clone()).collect();
    supported.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    assert_eq!(supported, [digest(A), digest(B), digest(C), digest(E)]);
    let revoked = image(B).parse::<ImageReference>().unwrap().resolve(digest(B)).unwrap();
    assert!(catalog.supported().any(|image| *image == revoked));

    let mut next = document();
    next["entries"]["codex"]["versions"].as_array_mut().unwrap().remove(1);
    let next = parse(&next).unwrap();
    assert!(
        !next.supported().any(|image| *image == revoked),
        "a dropped digest is revoked"
    );
    assert_eq!(next.supported().count(), 3);
    assert_eq!(
        next.resolve("codex", PackageArchitecture::Arm64, 1).unwrap().digest(),
        &digest(A)
    );
}
