use super::*;

fn selected() -> String {
    format!("{BASE}\n[[skills.selected]]\nid='sandbox'\nversion='1.0.0'\ncatalog_reference='ghcr.io/google/capsem/skills:stable'\nresolved_digest='sha256:{}'\n", "a".repeat(64))
}

#[test]
fn environment_skill_pins_roundtrip_and_drafts_refuse_host_resolution_fields() {
    let source = selected();
    let doc = EnvironmentDocument::from_toml(&source).unwrap();
    let encoded = doc.to_toml().unwrap();
    assert!(EnvironmentDocument::from_toml(&encoded).unwrap() == doc);
    let draft = "name='development'\n[[skills.selected]]\nid='sandbox'\nversion='1.0.0'\n";
    let parsed: EnvironmentDraft = toml::from_str(draft).unwrap();
    parsed.validate().unwrap();
    assert!(!toml::to_string(&parsed).unwrap().contains("resolved_"));
    assert!(toml::from_str::<EnvironmentDraft>(&source[source.find("name =").unwrap()..]).is_err());
    for extra in ["resolved_digest='sha256:forged'", "catalog_reference='attacker/source'"] {
        assert!(toml::from_str::<EnvironmentDraft>(&format!("{draft}{extra}\n")).is_err());
    }
    assert!(
        EnvironmentDocument::from_toml(&format!("{BASE}\n[[skills.selected]]\nid='sandbox'\nversion='1.0.0'\n"))
            .is_err()
    );
}

#[test]
fn environment_skill_pins_refuse_duplicates_invalid_metadata_and_oversize() {
    let source = selected();
    for bad in [
        source.replace("id='sandbox'", "id='../sandbox'"),
        source.replace("version='1.0.0'", "version=''"),
        source.replace(
            "version='1.0.0'",
            &format!(
                "version='{}'",
                "v".repeat(capsem_proto::ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES + 1)
            ),
        ),
        source.replace(&"a".repeat(64), &"A".repeat(64)),
        source.replace(
            "ghcr.io/google/capsem/skills:stable",
            &"x".repeat(capsem_proto::ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES + 1),
        ),
        format!("{source}{}", &source[source.find("[[skills.selected]]").unwrap()..]),
    ] {
        assert!(EnvironmentDocument::from_toml(&bad).is_err());
    }
    let mut source = "name='development'\n".to_string();
    for index in 0..capsem_proto::ENVIRONMENT_MAX_CATALOG_SELECTIONS {
        source.push_str(&format!(
            "[[skills.selected]]\nid='skill-{index}'\nversion='selector-canary'\n"
        ));
    }
    let parsed: EnvironmentDraft = toml::from_str(&source).unwrap();
    parsed.validate().unwrap();
    source.push_str("[[skills.selected]]\nid='one-too-many'\nversion='selector-canary'\n");
    let parsed: EnvironmentDraft = toml::from_str(&source).unwrap();
    let error = parsed.validate().unwrap_err();
    assert!(!error.contains("selector-canary"));
    assert!(!format!("{parsed:?}").contains("selector-canary"));
}
