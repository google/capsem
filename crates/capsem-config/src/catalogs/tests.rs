use super::*;
mod content;

#[test]
fn corporate_catalog_source_configuration_roundtrips_typed_optional_descriptors() {
    let source = "[catalogs.skills]\nreference='registry.example/corp/skills:approved'\nschema_version=1\n[catalogs.mcp]\nreference='registry.example/corp/mcp:approved'\nschema_version=1\n";
    let configuration: crate::SettingsFile = toml::from_str(source).unwrap();
    configuration.validate_metadata_contract().unwrap();
    let catalogs = configuration.catalogs.as_ref().unwrap();
    assert!(catalogs.rules.is_none());
    assert_eq!(
        catalogs.skills.as_ref().unwrap().reference,
        "registry.example/corp/skills:approved"
    );
    let restored: crate::SettingsFile = toml::from_str(&toml::to_string(&configuration).unwrap()).unwrap();
    assert_eq!(restored, configuration);
    assert!(!toml::to_string(&crate::SettingsFile::default())
        .unwrap()
        .contains("catalogs"));
}

#[test]
fn corporate_catalog_source_configuration_rejects_invalid_shape_and_schema() {
    for source in [
        "[catalogs.unknown]\nreference='registry.example/catalog:stable'\nschema_version=1",
        "[catalogs.skills]\nreference='registry.example/catalog:stable'\nschema_version=1\nextra=true",
    ] {
        assert!(toml::from_str::<crate::SettingsFile>(source).is_err());
    }
    for (reference, schema) in [
        ("registry.example/catalog:stable".into(), 2),
        ("https://private-canary".into(), 1),
        ("a".repeat(capsem_proto::ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES + 1), 1),
    ] {
        let configuration = crate::SettingsFile {
            catalogs: Some(CatalogSources {
                skills: Some(CatalogSourceDescriptor {
                    reference,
                    schema_version: schema,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let error = configuration.validate_metadata_contract().unwrap_err();
        assert!(!error.contains("canary"));
    }
}

#[test]
fn catalog_source_config_and_assets_share_one_proto_contract() {
    let source: CatalogSourceDescriptor = capsem_proto::catalogs::CatalogSourceDescriptor {
        reference: "ghcr.io/google/capsem/skills:stable".into(),
        schema_version: 1,
    };
    source.validate().unwrap();
    let kind: CatalogKind = capsem_proto::catalogs::CatalogKind::Skills;
    let origin: CatalogSourceOrigin = capsem_proto::catalogs::CatalogSourceOrigin::Corporate;
    assert_eq!(kind, CatalogKind::Skills);
    assert_eq!(origin, CatalogSourceOrigin::Corporate);
}
