use super::*;
mod compilation;
use capsem_config::catalogs::{CatalogKind, CatalogSourceDescriptor, CatalogSourceOrigin, SelectedCatalogSource};

fn source(kind: CatalogKind) -> ValidatedCatalogSource {
    let descriptor = CatalogSourceDescriptor {
        reference: "registry.example/catalog:stable".into(),
        schema_version: 1,
    };
    ValidatedCatalogSource::new(
        kind,
        SelectedCatalogSource {
            source: &descriptor,
            origin: CatalogSourceOrigin::Manifest,
        },
    )
    .unwrap()
}
fn digest() -> Digest {
    Digest::parse(&format!("sha256:{}", "a".repeat(64))).unwrap()
}
fn bytes() -> Vec<u8> {
    let main = "---\nname: sandbox\ndescription: Sandbox instructions\n---\nRead every file.\n";
    let support = "Whole support text.\r\n";
    serde_json::to_vec(&serde_json::json!({"schema_version":1,"kind":"skills","entries":[{
        "metadata":{"id":"sandbox","version":"1.0.0","name":"Sandbox","publisher":"Example","provenance":"registry.example/skills","license":"Apache-2.0","status":"active","compatibility":{"min_runtime_contract":1}},
        "payload":{"files":{
            "SKILL.md":{"content":main,"sha256":format!("sha256:{:x}", capsem_crypto::Sha256::digest(main))},
            "references/proof.md":{"content":support,"sha256":format!("sha256:{:x}", capsem_crypto::Sha256::digest(support))}
        }}
    }]})).unwrap()
}

#[test]
fn catalog_contents_verify_every_complete_skill_file_before_pinning() {
    let catalog = SkillCatalogContents::decode(source(CatalogKind::Skills), digest(), bytes()).unwrap();
    assert_eq!(catalog.entries().len(), 1);
    let pin = catalog.pin("sandbox", "1.0.0").unwrap();
    assert_eq!(pin.catalog_reference, "registry.example/catalog:stable");
    assert_eq!(pin.resolved_digest, digest().to_string());
    let selected = catalog.entry_for_pin(&pin).unwrap();
    assert_eq!(
        selected.payload.as_ref().unwrap().files["references/proof.md"].content,
        "Whole support text.\r\n"
    );
    for path in ["SKILL.md", "references/proof.md"] {
        let mut value: serde_json::Value = serde_json::from_slice(&bytes()).unwrap();
        value["entries"][0]["payload"]["files"][path]["content"] = serde_json::json!(format!(
            "{}changed",
            value["entries"][0]["payload"]["files"][path]["content"]
                .as_str()
                .unwrap()
        ));
        assert_eq!(
            SkillCatalogContents::decode(
                source(CatalogKind::Skills),
                digest(),
                serde_json::to_vec(&value).unwrap()
            )
            .unwrap_err(),
            CatalogContentError::IntegrityMismatch
        );
    }
}

#[test]
fn catalog_contents_refuse_malformed_unavailable_or_changed_pin_identity() {
    let catalog = SkillCatalogContents::decode(source(CatalogKind::Skills), digest(), bytes()).unwrap();
    let pin = catalog.pin("sandbox", "1.0.0").unwrap();
    for change in [0, 1, 2, 3, 4] {
        let mut stale = pin.clone();
        match change {
            0 => stale.id = "unknown".into(),
            1 => stale.version = "2.0.0".into(),
            2 => stale.catalog_reference = "registry.example/other:stable".into(),
            3 => stale.resolved_digest = format!("sha256:{}", "b".repeat(64)),
            _ => stale.catalog_reference = "registry.example/catalog:other".into(),
        }
        assert!(catalog.entry_for_pin(&stale).is_err());
    }
    assert_eq!(
        catalog.pin("unknown", "1").err(),
        Some(CatalogContentError::UnknownEntry)
    );
    for status in ["revoked", "deprecated"] {
        let mut value: serde_json::Value = serde_json::from_slice(&bytes()).unwrap();
        value["entries"][0]["metadata"]["status"] = serde_json::json!(status);
        value["entries"][0]["metadata"]["status_reason"] = serde_json::json!("Publisher notice");
        let current = SkillCatalogContents::decode(
            source(CatalogKind::Skills),
            digest(),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        assert_eq!(current.pin("sandbox", "1.0.0").is_ok(), status == "deprecated");
    }
    let mut value: serde_json::Value = serde_json::from_slice(&bytes()).unwrap();
    value["entries"][0]["metadata"]["compatibility"]["min_runtime_contract"] = serde_json::json!(2);
    let current = SkillCatalogContents::decode(
        source(CatalogKind::Skills),
        digest(),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert_eq!(
        current.pin("sandbox", "1.0.0").err(),
        Some(CatalogContentError::UnusableEntry)
    );
}

#[test]
fn catalog_contents_require_bounded_strict_objects_and_matching_kind() {
    let mut boundary = bytes();
    boundary.resize(capsem_proto::CATALOG_MAX_DOCUMENT_BYTES, b' ');
    SkillCatalogContents::decode(source(CatalogKind::Skills), digest(), boundary).unwrap();
    for bytes in [
        b"[]".to_vec(),
        b"[1,\"skills\",[]]".to_vec(),
        b"private-canary".to_vec(),
    ] {
        let error = SkillCatalogContents::decode(source(CatalogKind::Skills), digest(), bytes).unwrap_err();
        assert_eq!(error, CatalogContentError::InvalidDocument);
        assert!(!format!("{error:?} {error}").contains("canary"));
    }
    assert_eq!(
        SkillCatalogContents::decode(
            source(CatalogKind::Skills),
            digest(),
            vec![b' '; capsem_proto::CATALOG_MAX_DOCUMENT_BYTES + 1]
        )
        .unwrap_err(),
        CatalogContentError::TooLarge
    );
    assert_eq!(
        SkillCatalogContents::decode(source(CatalogKind::Rules), digest(), bytes()).unwrap_err(),
        CatalogContentError::WrongKind
    );
    for field in ["schema_version", "kind", "unknown"] {
        let mut value: serde_json::Value = serde_json::from_slice(&bytes()).unwrap();
        value[field] = match field {
            "schema_version" => serde_json::json!(2),
            "kind" => serde_json::json!("rules"),
            _ => serde_json::json!(true),
        };
        assert!(SkillCatalogContents::decode(
            source(CatalogKind::Skills),
            digest(),
            serde_json::to_vec(&value).unwrap()
        )
        .is_err());
    }
}

#[test]
fn catalog_contents_validate_rule_presets_without_claiming_publisher_authority() {
    let mut value: serde_json::Value = serde_json::from_slice(&bytes()).unwrap();
    value["kind"] = serde_json::json!("rules");
    value["entries"][0].as_object_mut().unwrap().remove("payload");
    value["entries"][0]["definition"] =
        serde_json::json!({"kind":"access","rule":{"name":"Host","action":"ask","match":"http.host == {{host}}"}});
    value["entries"][0]["parameters"] =
        serde_json::json!({"host":{"type":"text","default":"api.example","max_bytes":128}});
    let catalog = RuleCatalogContents::decode(
        source(CatalogKind::Rules),
        digest(),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert_eq!(catalog.entries().len(), 1);
    let pin = catalog.pin("sandbox", "1.0.0").unwrap();
    assert!(catalog
        .entry_for_pin(&pin)
        .unwrap()
        .resolve(&Default::default())
        .is_ok());
    assert!(!format!("{catalog:?}").contains("api.example"));
    value["entries"][0]["definition"]["rule"]["match"] = serde_json::json!("http.misspelled == {{host}}");
    assert!(RuleCatalogContents::decode(
        source(CatalogKind::Rules),
        digest(),
        serde_json::to_vec(&value).unwrap()
    )
    .is_err());
    value["entries"][0]["metadata"]["compatibility"]["min_runtime_contract"] = serde_json::json!(2);
    let future = RuleCatalogContents::decode(
        source(CatalogKind::Rules),
        digest(),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert_eq!(future.entries().len(), 1);
    assert_eq!(
        future.pin("sandbox", "1.0.0").err(),
        Some(CatalogContentError::UnusableEntry)
    );
}

#[test]
fn catalog_contents_cached_requests_bind_source_and_digest_before_io() {
    let catalog = SkillCatalogContents::decode(source(CatalogKind::Skills), digest(), bytes()).unwrap();
    let pin = catalog.pin("sandbox", "1.0.0").unwrap();
    assert_eq!(
        SkillCatalogContents::cached_request(catalog.source(), &pin).unwrap(),
        digest()
    );
    assert_eq!(
        SkillCatalogContents::cached_request(&source(CatalogKind::Rules), &pin).err(),
        Some(CatalogContentError::WrongKind)
    );
    let mut changed = pin.clone();
    changed.catalog_reference = "registry.example/other:stable".into();
    assert_eq!(
        SkillCatalogContents::cached_request(catalog.source(), &changed).err(),
        Some(CatalogContentError::PinMismatch)
    );
    changed = pin.clone();
    changed.resolved_digest = "private-canary".into();
    let error = SkillCatalogContents::cached_request(catalog.source(), &changed).unwrap_err();
    assert_eq!(error, CatalogContentError::PinMismatch);
    assert!(!format!("{error:?} {error}").contains("canary"));
    let descriptor = CatalogSourceDescriptor {
        reference: format!("registry.example/catalog@{}", digest()),
        schema_version: 1,
    };
    let pinned_source = ValidatedCatalogSource::new(
        CatalogKind::Skills,
        SelectedCatalogSource {
            source: &descriptor,
            origin: CatalogSourceOrigin::Corporate,
        },
    )
    .unwrap();
    changed = pin;
    changed.catalog_reference = pinned_source.reference().to_string();
    assert!(SkillCatalogContents::cached_request(&pinned_source, &changed).is_ok());
    changed.resolved_digest = format!("sha256:{}", "b".repeat(64));
    assert_eq!(
        SkillCatalogContents::cached_request(&pinned_source, &changed).err(),
        Some(CatalogContentError::PinMismatch)
    );
}
