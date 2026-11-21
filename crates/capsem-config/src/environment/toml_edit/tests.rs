use super::*;

fn source() -> String {
    format!(
        "schema_version=1\nid='00000000-0000-4000-8000-000000000001'\nrevision_id='00000000-0000-4000-8000-000000000002'\nname='work'\n[image]\nreference='work:one'\nresolved_digest='sha256:{}'\n[[skills.selected]]\nid='work'\nversion='1.0.0'\ncatalog_reference='registry.example/skills:stable'\nresolved_digest='sha256:{}'\n",
        "a".repeat(64), "b".repeat(64)
    )
}

#[test]
fn canonical_edit_keeps_selectors_and_checks_revision_identity() {
    let original = EnvironmentDocument::from_toml(&source()).unwrap();
    let canonical = original.to_toml().unwrap();
    let edited = canonical.replace("name = \"work\"", "name = \"next\"");
    let draft = original.draft_from_toml(&edited).unwrap();
    assert_eq!(draft.name, "next");
    assert_eq!(draft.image.unwrap().reference, "work:one");
    assert_eq!(draft.skills.selected[0].id, "work");
    assert_eq!(draft.skills.selected[0].version, "1.0.0");
    assert!(original
        .draft_from_toml(&edited.replace("000000000002", "000000000003"))
        .is_err());
}

#[test]
fn changed_source_selectors_require_removing_previous_resolution_metadata() {
    let original = EnvironmentDocument::from_toml(&source()).unwrap();
    let forged = source().replace(&"a".repeat(64), &"c".repeat(64));
    assert!(original
        .draft_from_toml(&forged)
        .unwrap_err()
        .contains("resolved_digest"));
    let edited = source().replace("work:one", "work:two");
    assert!(original.draft_from_toml(&edited).is_err());
    let without_image_pin = edited.replace(&format!("resolved_digest='sha256:{}'\n", "a".repeat(64)), "");
    let draft = original.draft_from_toml(&without_image_pin).unwrap();
    assert_eq!(draft.image.unwrap().reference, "work:two");
    assert!(original.draft_from_toml(&source().replace("1.0.0", "2.0.0")).is_err());
    assert!(original
        .draft_from_toml(&source().replace("name='work'", "name='work'\nunknown=true"))
        .is_err());
}
