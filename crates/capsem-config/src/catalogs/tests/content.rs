use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    metadata: CatalogEntryMetadata,
    payload: String,
}
impl CatalogEntry for Entry {
    const KIND: CatalogKind = CatalogKind::Rules;
    fn metadata(&self) -> &CatalogEntryMetadata {
        &self.metadata
    }
    fn validate_content(&self) -> Result<(), &'static str> {
        if self.payload == "valid" {
            Ok(())
        } else {
            Err("invalid fixture content")
        }
    }
}

fn metadata() -> CatalogEntryMetadata {
    toml::from_str("id='preset'\nversion='1.0.0'\nname='Preset'\ndescription='A preset'\ntags=['network']\npublisher='Example publisher'\nprovenance='registry.example/publisher/rules'\nlicense='Apache-2.0'\nstatus='active'\n[compatibility]\nmin_runtime_contract=1").unwrap()
}
fn document() -> CatalogDocument<Entry> {
    CatalogDocument {
        schema_version: ENVIRONMENT_CATALOG_SCHEMA_VERSION,
        kind: CatalogKind::Rules,
        entries: vec![Entry {
            metadata: metadata(),
            payload: "valid".into(),
        }],
    }
}

#[test]
fn catalog_content_contract_roundtrips_empty_and_multi_version_documents() {
    let mut document = document();
    let mut next = document.entries[0].clone();
    next.metadata.version = "2.0.0".into();
    document.entries.push(next);
    document.validate().unwrap();
    let bytes = serde_json::to_vec(&document).unwrap();
    let restored: CatalogDocument<Entry> = serde_json::from_slice(&bytes).unwrap();
    assert!(restored == document);
    assert!(restored.entries[0].metadata.compatibility.is_supported());
    document.entries.clear();
    document.validate().unwrap();
    document.entries.push(Entry {
        metadata: metadata(),
        payload: "invalid".into(),
    });
    assert!(document.validate().is_err());
}

#[test]
fn catalog_content_contract_refuses_wrong_kind_schema_unknown_fields_and_duplicate_identity() {
    let mut invalid = document();
    invalid.kind = CatalogKind::Mcp;
    assert!(invalid.validate().is_err());
    invalid = document();
    invalid.schema_version += 1;
    assert!(invalid.validate().is_err());
    invalid = document();
    invalid.entries.push(invalid.entries[0].clone());
    invalid.entries[1].metadata.publisher = "Different claimed publisher".into();
    assert!(invalid.validate().is_err());
    invalid = document();
    invalid.entries = vec![invalid.entries[0].clone(); capsem_proto::CATALOG_MAX_ENTRIES + 1];
    assert!(invalid.validate().is_err());
    for path in [
        vec![],
        vec!["entries", "0"],
        vec!["entries", "0", "metadata"],
        vec!["entries", "0", "metadata", "compatibility"],
    ] {
        let mut value = serde_json::to_value(document()).unwrap();
        let mut target = &mut value;
        for part in path {
            target = if part == "0" { &mut target[0] } else { &mut target[part] };
        }
        target["unknown"] = serde_json::json!(true);
        assert!(serde_json::from_value::<CatalogDocument<Entry>>(value).is_err());
    }
}

#[test]
fn catalog_content_contract_bounds_metadata_without_conferring_publisher_or_compatibility_authority() {
    let cases: &[fn(&mut CatalogEntryMetadata)] = &[
        |m| m.id = "UPPER".into(),
        |m| m.id = "../preset".into(),
        |m| m.id = "a".repeat(capsem_proto::CATALOG_MAX_IDENTIFIER_BYTES + 1),
        |m| m.version = "".into(),
        |m| m.version = "1 2".into(),
        |m| m.version = "a".repeat(capsem_proto::CATALOG_MAX_VERSION_BYTES + 1),
        |m| m.name = " ".into(),
        |m| m.name = "\0".into(),
        |m| m.name = "é".repeat(capsem_proto::CATALOG_MAX_NAME_BYTES / 2 + 1),
        |m| m.description = "x".repeat(capsem_proto::CATALOG_MAX_DESCRIPTION_BYTES + 1),
        |m| m.description = "\0".into(),
        |m| m.tags = vec!["network".into(), "network".into()],
        |m| {
            m.tags = (0..=capsem_proto::CATALOG_MAX_TAGS)
                .map(|n| format!("tag{n}"))
                .collect()
        },
        |m| m.tags = vec!["x".repeat(capsem_proto::CATALOG_MAX_TAG_BYTES + 1)],
        |m| m.publisher = "".into(),
        |m| m.provenance = "x".repeat(capsem_proto::CATALOG_MAX_PROVENANCE_BYTES + 1),
        |m| m.license = "".into(),
        |m| m.compatibility.min_runtime_contract = 0,
        |m| m.status = CatalogEntryStatus::Revoked,
        |m| m.status = CatalogEntryStatus::Deprecated,
        |m| m.status_reason = Some("unexpected active reason".into()),
    ];
    for change in cases {
        let mut metadata = metadata();
        change(&mut metadata);
        assert!(metadata.validate().is_err());
    }
    let mut metadata = metadata();
    metadata.compatibility.min_runtime_contract = CATALOG_RUNTIME_CONTRACT + 1;
    metadata.validate().unwrap();
    assert!(!metadata.compatibility.is_supported());
    metadata.status = CatalogEntryStatus::Revoked;
    metadata.status_reason = Some("Publisher withdrawal".into());
    metadata.validate().unwrap();
    assert!(!metadata.is_selectable());
    metadata.status = CatalogEntryStatus::Deprecated;
    assert!(!metadata.is_selectable(), "incompatible deprecation stays unavailable");
    metadata.compatibility.min_runtime_contract = CATALOG_RUNTIME_CONTRACT;
    assert!(metadata.is_selectable());
}
