use super::*;

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
