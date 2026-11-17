use super::*;
use std::collections::BTreeMap;

fn skill() -> SkillCatalogEntry {
    let metadata: CatalogEntryMetadata = toml::from_str("id='sandbox'\nversion='1'\nname='Sandbox'\npublisher='Example'\nprovenance='registry.example/skills'\nlicense='Apache-2.0'\nstatus='active'\n[compatibility]\nmin_runtime_contract=1").unwrap();
    SkillCatalogEntry { metadata, payload: Some(SkillPayload { files: [
        ("SKILL.md".into(), SkillFile { sha256: format!("sha256:{}", "a".repeat(64)), content: "---\nname: sandbox\ndescription: Work in the sandbox\nallowed-tools: example_tool\n---\nInspect before acting.\n".into() }),
        ("references/example.md".into(), SkillFile { sha256: format!("sha256:{}", "b".repeat(64)), content: "Complete support text.\r\n".into() }),
    ].into() }) }
}

#[test]
fn skill_catalog_contract_preserves_complete_utf8_files_and_advisory_metadata() {
    let entry = skill();
    entry.validate_content().unwrap();
    let document = CatalogDocument {
        schema_version: ENVIRONMENT_CATALOG_SCHEMA_VERSION,
        kind: CatalogKind::Skills,
        entries: vec![entry],
    };
    document.validate().unwrap();
    let restored: CatalogDocument<SkillCatalogEntry> =
        serde_json::from_slice(&serde_json::to_vec(&document).unwrap()).unwrap();
    assert!(restored == document);
    let files = &restored.entries[0].payload.as_ref().unwrap().files;
    assert_eq!(files.len(), 2);
    assert!(
        files["SKILL.md"].content.contains("allowed-tools: example_tool"),
        "advisory text is retained without granting a capability"
    );
    assert_eq!(files["references/example.md"].content, "Complete support text.\r\n");
}

#[test]
fn skill_catalog_contract_refuses_unsafe_paths_missing_payload_and_malformed_frontmatter() {
    for path in [
        "/tmp/file",
        "../file",
        "a/../file",
        "./file",
        "a//file",
        "a/",
        "C:/file",
        "a\\file",
        "a\0file",
    ] {
        let mut entry = skill();
        let payload = entry.payload.as_mut().unwrap();
        payload
            .files
            .insert(path.into(), payload.files["references/example.md"].clone());
        assert!(entry.validate_content().is_err());
    }
    for content in [
        "No frontmatter",
        "---\nname: sandbox\n",
        "---\nname: other\ndescription: Wrong identity\n---\nText",
        "---\nname: sandbox\ndescription: ''\n---\nText",
        "---\nname: sandbox\ndescription: Valid\nunknown: true\n---\nText",
        "---\nname: sandbox\ndescription: Valid\n---\n   ",
        "---\nname: sandbox\ndescription: Valid\n---\nBinary\0text",
    ] {
        let mut entry = skill();
        entry
            .payload
            .as_mut()
            .unwrap()
            .files
            .get_mut("SKILL.md")
            .unwrap()
            .content = content.into();
        assert!(entry.validate_content().is_err());
    }
    let mut entry = skill();
    entry.payload.as_mut().unwrap().files.remove("SKILL.md");
    assert!(entry.validate_content().is_err());
    entry.payload = None;
    assert!(entry.validate_content().is_err());
    entry.metadata.status = CatalogEntryStatus::Revoked;
    entry.metadata.status_reason = Some("Withdrawn".into());
    entry.validate_content().unwrap();
}

#[test]
fn skill_catalog_contract_checks_declared_digest_shape_and_complete_content_budgets() {
    for digest in [
        "sha256:forged".into(),
        format!("sha256:{}", "A".repeat(64)),
        format!("blake3:{}", "a".repeat(64)),
    ] {
        let mut entry = skill();
        entry
            .payload
            .as_mut()
            .unwrap()
            .files
            .get_mut("SKILL.md")
            .unwrap()
            .sha256 = digest;
        assert!(entry.validate_content().is_err());
    }
    let mut entry = skill();
    let payload = entry.payload.as_mut().unwrap();
    payload.files.get_mut("references/example.md").unwrap().content =
        "é".repeat(capsem_proto::CATALOG_MAX_SKILL_FILE_BYTES / 2);
    entry.validate_content().unwrap();
    entry
        .payload
        .as_mut()
        .unwrap()
        .files
        .get_mut("references/example.md")
        .unwrap()
        .content
        .push('a');
    assert!(entry.validate_content().is_err());
    let mut entry = skill();
    let payload = entry.payload.as_mut().unwrap();
    let file = payload.files["references/example.md"].clone();
    for n in 0..capsem_proto::CATALOG_MAX_SKILL_FILES - 2 {
        payload.files.insert(format!("file{n}"), file.clone());
    }
    entry.validate_content().unwrap();
    entry
        .payload
        .as_mut()
        .unwrap()
        .files
        .insert("one-too-many.md".into(), file);
    assert!(entry.validate_content().is_err());
    let mut entry = skill();
    let payload = entry.payload.as_mut().unwrap();
    let mut file = payload.files["references/example.md"].clone();
    let main_bytes = payload.files["SKILL.md"].content.len();
    file.content = "a".repeat(capsem_proto::CATALOG_MAX_SKILL_FILE_BYTES);
    payload.files.insert("references/example.md".into(), file.clone());
    file.content = "b"
        .repeat(capsem_proto::CATALOG_MAX_SKILL_TOTAL_BYTES - capsem_proto::CATALOG_MAX_SKILL_FILE_BYTES - main_bytes);
    payload.files.insert("other.md".into(), file);
    entry.validate_content().unwrap();
    entry
        .payload
        .as_mut()
        .unwrap()
        .files
        .get_mut("other.md")
        .unwrap()
        .content
        .push('b');
    assert!(entry.validate_content().is_err());
}

#[test]
fn skill_catalog_contract_counts_utf8_path_bytes_and_preserves_crlf_instructions() {
    let mut entry = skill();
    let payload = entry.payload.as_mut().unwrap();
    let file = payload.files["references/example.md"].clone();
    let name = "é".repeat(capsem_proto::CATALOG_MAX_SKILL_PATH_BYTES / 2);
    payload.files.insert(name.clone(), file.clone());
    payload.files.get_mut("SKILL.md").unwrap().content = payload.files["SKILL.md"].content.replace('\n', "\r\n");
    entry.validate_content().unwrap();
    assert!(entry.payload.as_ref().unwrap().files["SKILL.md"]
        .content
        .starts_with("---\r\n"));
    entry.payload.as_mut().unwrap().files.insert(format!("{name}a"), file);
    assert!(entry.validate_content().is_err());
}

#[test]
fn skill_catalog_contract_rejects_duplicate_file_and_rule_parameter_map_keys() {
    let file = serde_json::to_string(&skill().payload.unwrap().files["SKILL.md"]).unwrap();
    let bytes = format!("{{\"files\":{{\"SKILL.md\":{file},\"SKILL.md\":{file}}}}}");
    assert!(serde_json::from_str::<SkillPayload>(&bytes).is_err());
    let parameters = r#"{"host":{"type":"boolean","default":true},"host":{"type":"boolean","default":false}}"#;
    let entry = format!(
        "{{\"metadata\":{},\"parameters\":{parameters},\"definition\":null}}",
        serde_json::to_string(&skill().metadata).unwrap()
    );
    assert!(serde_json::from_str::<RuleCatalogEntry>(&entry).is_err());
    let empty: SkillPayload = serde_json::from_str("{\"files\":{}}").unwrap();
    assert!(empty.files == BTreeMap::new());
}
