use super::*;
use capsem_proto::catalogs::{CatalogKind, CatalogSourceDescriptor, CatalogSourceOrigin, SelectedCatalogSource};

fn selected(reference: &str) -> CatalogSourceDescriptor {
    CatalogSourceDescriptor {
        reference: reference.into(),
        schema_version: 1,
    }
}

#[test]
fn catalog_source_normalization_preserves_origin_tag_and_digest_identity() {
    let descriptor = selected("ghcr.io/google/capsem/skills:stable");
    let source = ValidatedCatalogSource::new(
        CatalogKind::Skills,
        SelectedCatalogSource {
            source: &descriptor,
            origin: CatalogSourceOrigin::Corporate,
        },
    )
    .unwrap();
    assert_eq!(source.kind(), CatalogKind::Skills);
    assert_eq!(source.origin(), CatalogSourceOrigin::Corporate);
    assert_eq!(source.schema_version(), 1);
    assert_eq!(source.reference().tag(), Some("stable"));
    assert_eq!(
        source.identity().repository().to_string(),
        "ghcr.io/google/capsem/skills"
    );
    let descriptor = selected(&format!("ghcr.io/google/capsem/mcp@sha256:{}", "a".repeat(64)));
    let pinned = ValidatedCatalogSource::new(
        CatalogKind::Mcp,
        SelectedCatalogSource {
            source: &descriptor,
            origin: CatalogSourceOrigin::Manifest,
        },
    )
    .unwrap();
    assert_eq!(
        pinned.identity().digest().unwrap().as_str(),
        format!("sha256:{}", "a".repeat(64))
    );
    assert_eq!(
        pinned.reference().digest(),
        Some(pinned.identity().digest().unwrap().as_str())
    );
}

#[test]
fn catalog_source_normalization_reuses_docker_alias_identity_and_refuses_bad_sources() {
    let first = selected("index.docker.io/library/catalog:approved");
    let second = selected("docker.io/library/catalog:approved");
    let validated = |source: &CatalogSourceDescriptor| {
        ValidatedCatalogSource::new(
            CatalogKind::Rules,
            SelectedCatalogSource {
                source,
                origin: CatalogSourceOrigin::Manifest,
            },
        )
        .unwrap()
    };
    assert_eq!(validated(&first).identity(), validated(&second).identity());
    for reference in [
        "unqualified-canary",
        "https://private-canary@example.org/catalog",
        "registry.example/UPPER-canary:tag",
        "registry.example/project@sha256:private-canary",
        "registry.example/a/../private-canary:tag",
    ] {
        let bad = selected(reference);
        let error = ValidatedCatalogSource::new(
            CatalogKind::Rules,
            SelectedCatalogSource {
                source: &bad,
                origin: CatalogSourceOrigin::Corporate,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(!error.contains("canary"));
    }
    let future = CatalogSourceDescriptor {
        schema_version: 2,
        ..first
    };
    assert!(ValidatedCatalogSource::new(
        CatalogKind::Skills,
        SelectedCatalogSource {
            source: &future,
            origin: CatalogSourceOrigin::Corporate
        }
    )
    .is_err());
}
