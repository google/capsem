use super::*;
use capsem_proto::catalogs::{CatalogKind, CatalogSourceDescriptor, CatalogSourceOrigin, SelectedCatalogSource};

fn selected(reference: &str) -> CatalogSourceDescriptor {
    CatalogSourceDescriptor {
        reference: reference.into(),
        schema_version: 1,
    }
}

#[test]
fn catalog_artifact_debug_is_bounded_metadata_without_content_bytes() {
    let bytes = b"catalog-content-canary".repeat(4096);
    let digest = crate::oci::Digest::parse(&format!("sha256:{}", "a".repeat(64))).unwrap();
    let artifact = CatalogArtifact::new(CatalogKind::Rules, digest.clone(), bytes.clone());
    let debug = format!("{artifact:?}");
    assert!(debug.len() < 256, "diagnostics must not copy the catalog byte vector");
    assert!(!debug.contains("bytes: ["));
    assert!(!debug.contains("catalog-content-canary"));
    assert!(debug.contains("Rules"));
    assert!(debug.contains(&digest.to_string()));
    assert!(debug.contains(&bytes.len().to_string()));
    assert_eq!(artifact.into_bytes(), bytes);
}

#[test]
fn manifest_catalog_source_projection_is_independent_of_runtime_and_release_shape() {
    let source = serde_json::json!({"skills":{"reference":"ghcr.io/google/capsem/skills:stable","schema_version":1}});
    for mut manifest in [
        serde_json::json!({"version":"0.7.3","channel":"stable","packages":[]}),
        serde_json::json!({"format":2,"refresh_policy":"24h","assets":{},"binaries":{}}),
    ] {
        assert_eq!(
            sources_from_manifest(&manifest.to_string()).unwrap(),
            capsem_proto::catalogs::CatalogSources::default()
        );
        manifest["catalogs"] = source.clone();
        let sources = sources_from_manifest(&manifest.to_string()).unwrap();
        assert_eq!(sources.skills, Some(selected("ghcr.io/google/capsem/skills:stable")));
        assert!(sources.rules.is_none());
        assert!(sources.mcp.is_none());
        let validated = select_source(&sources, &Default::default(), CatalogKind::Skills)
            .unwrap()
            .unwrap();
        assert_eq!(validated.origin(), CatalogSourceOrigin::Manifest);
    }
    assert!(sources_from_manifest("{\"catalogs\":null}").unwrap().skills.is_none());
    for malformed in [
        "{",
        "[]",
        r#"{"catalogs":{"unknown":{}}}"#,
        r#"{"catalogs":{"skills":{"reference":"private-canary","schema_version":1,"extra":true}}}"#,
    ] {
        let error = sources_from_manifest(malformed).unwrap_err().to_string();
        assert!(!error.contains("canary"));
    }
}

#[test]
fn effective_catalog_source_validation_never_falls_back_from_corporate_choice() {
    use capsem_proto::catalogs::CatalogSources;
    let manifest = CatalogSources {
        skills: Some(selected("ghcr.io/google/capsem/skills:stable")),
        ..Default::default()
    };
    let corporate = CatalogSources {
        skills: Some(selected("private-canary")),
        ..Default::default()
    };
    let error = select_source(&manifest, &corporate, CatalogKind::Skills)
        .unwrap_err()
        .to_string();
    assert!(!error.contains("canary"));
    assert!(select_source(&manifest, &corporate, CatalogKind::Mcp)
        .unwrap()
        .is_none());
    let manifest = CatalogSources {
        skills: Some(selected("private-canary")),
        ..Default::default()
    };
    let corporate = CatalogSources {
        skills: Some(selected("registry.example/corporate/skills:approved")),
        ..Default::default()
    };
    let source = select_source(&manifest, &corporate, CatalogKind::Skills)
        .unwrap()
        .unwrap();
    assert_eq!(source.origin(), CatalogSourceOrigin::Corporate);
    assert_eq!(source.reference().tag(), Some("approved"));
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
fn mcp_source_requires_an_exact_digest_from_the_effective_authority() {
    let reference = format!("ghcr.io/google/capsem/mcp@sha256:{}", "a".repeat(64));
    let manifest = CatalogSources {
        mcp: Some(selected(&reference)),
        ..Default::default()
    };
    let pinned = select_source(&manifest, &Default::default(), CatalogKind::Mcp)
        .unwrap()
        .unwrap();
    assert_eq!(pinned.origin(), CatalogSourceOrigin::Manifest);
    assert_eq!(pinned.reference().to_string(), reference);

    let mut corporate = CatalogSources::default();
    for unpinned in ["registry.example/company/mcp", "registry.example/company/mcp:approved"] {
        corporate.mcp = Some(selected(unpinned));
        assert!(select_source(&corporate, &Default::default(), CatalogKind::Mcp).is_err());
        assert!(select_source(&manifest, &corporate, CatalogKind::Mcp).is_err());
    }
    let replacement = format!("registry.example/company/mcp@sha256:{}", "b".repeat(64));
    corporate.mcp = Some(selected(&replacement));
    let pinned = select_source(&manifest, &corporate, CatalogKind::Mcp).unwrap().unwrap();
    assert_eq!(pinned.origin(), CatalogSourceOrigin::Corporate);
    assert_eq!(pinned.reference().to_string(), replacement);
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
