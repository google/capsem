use super::*;

fn sources() -> CatalogSources {
    toml::from_str("[skills]\nreference='ghcr.io/google/capsem/skills:stable'\nschema_version=1\n[rules]\nreference='ghcr.io/google/capsem/rules:stable'\nschema_version=1\n[mcp]\nreference='ghcr.io/google/capsem/mcp:stable'\nschema_version=1\n").unwrap()
}

#[test]
fn catalog_sources_roundtrip_all_kinds_without_inventing_missing_sources() {
    let manifest = sources();
    for kind in CatalogKind::ALL {
        manifest.source(kind).unwrap().validate().unwrap();
    }
    let restored: CatalogSources = toml::from_str(&toml::to_string(&manifest).unwrap()).unwrap();
    assert_eq!(restored, manifest);
    let missing = CatalogSources::default();
    for kind in CatalogKind::ALL {
        assert!(missing.effective_source(&CatalogSources::default(), kind).is_none());
    }
    assert!(toml::from_str::<CatalogSources>("[unknown]\nreference='invented'\nschema_version=1").is_err());
    assert!(toml::from_str::<CatalogSources>(
        "[skills]\nreference='ghcr.io/google/capsem/skills:stable'\nschema_version=1\nextra=true"
    )
    .is_err());
}

#[test]
fn catalog_corporate_override_is_selected_before_validation_without_fallback() {
    let manifest = sources();
    let mut corporate = CatalogSources {
        mcp: Some(CatalogSourceDescriptor {
            reference: "registry.corp.example/mcp:approved".into(),
            schema_version: 1,
        }),
        ..Default::default()
    };
    let chosen = manifest.effective_source(&corporate, CatalogKind::Mcp).unwrap();
    assert_eq!(chosen.origin, CatalogSourceOrigin::Corporate);
    assert!(std::ptr::eq(chosen.source, corporate.mcp.as_ref().unwrap()));
    assert_eq!(
        manifest
            .effective_source(&corporate, CatalogKind::Skills)
            .unwrap()
            .origin,
        CatalogSourceOrigin::Manifest
    );
    corporate.mcp.as_mut().unwrap().schema_version = 2;
    let chosen = manifest.effective_source(&corporate, CatalogKind::Mcp).unwrap();
    assert_eq!(chosen.origin, CatalogSourceOrigin::Corporate);
    assert!(chosen.source.validate().is_err());
    assert!(manifest.source(CatalogKind::Mcp).unwrap().validate().is_ok());
}

#[test]
fn catalog_source_descriptors_bound_references_and_sanitize_errors() {
    let valid = sources().skills.unwrap();
    for reference in [
        String::new(),
        "x".repeat(capsem_proto::ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES + 1),
        "bad reference-canary".into(),
        "https://user:secret-canary@example.org/catalog".into(),
        "bad\0reference-canary".into(),
    ] {
        let invalid = CatalogSourceDescriptor {
            reference,
            schema_version: 1,
        };
        let error = invalid.validate().unwrap_err();
        assert!(!error.contains("canary"));
    }
    let oversized = CatalogSourceDescriptor {
        reference: "x".repeat(capsem_proto::ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES),
        ..valid
    };
    oversized.validate().unwrap();
    assert!(CatalogSourceDescriptor {
        schema_version: 0,
        ..valid.clone()
    }
    .validate()
    .is_err());
    assert!(CatalogSourceDescriptor {
        schema_version: 2,
        ..valid
    }
    .validate()
    .is_err());
}
