use capsem_ui_catalog::editable_state::{expected_audit_json, run_editable_state_audit};

#[test]
fn loro_audit_fixture_round_trips_edits_and_topology() {
    let audit = run_editable_state_audit().expect("loro audit fixture");

    assert_eq!(audit.updated_json, expected_audit_json());
    assert_eq!(audit.restored_json, audit.updated_json);
    assert!(audit.base_snapshot_bytes > 0);
    assert!(audit.update_bytes > 0);
    assert!(audit.updated_snapshot_bytes >= audit.base_snapshot_bytes);
    assert_eq!(audit.update_blake3.len(), 64);
    assert_eq!(audit.record.payload.engine, "loro");
    assert_eq!(audit.record.payload.update_blake3, audit.update_blake3);
    assert_eq!(audit.record.payload.update_bytes, audit.update_bytes);

    for id in [
        "alert:security-warning",
        "card:model",
        "deck:realms-of-code",
        "field:email",
        "form:signup",
        "page:welcome",
        "sheet:houses",
        "slide:lannister",
        "slide:overview",
        "spreadsheet:finance",
        "website:onboarding",
    ] {
        assert!(
            audit.topology_ids.iter().any(|candidate| candidate == id),
            "missing stable topology id {id}"
        );
    }

    assert_eq!(
        audit.updated_json["workspace"]["components"][0]["id"],
        "alert:security-warning"
    );
    assert_eq!(
        audit.updated_json["workspace"]["slideDeck"]["slides"][0]["id"],
        "slide:lannister"
    );
    assert_eq!(
        audit.updated_json["workspace"]["spreadsheet"]["sheets"][0]["cells"]["B2"],
        "A Lannister always pays his debts"
    );
    assert_eq!(
        audit.updated_json["workspace"]["website"]["pages"][0]["form"]["fields"][0]["label"],
        "Work email"
    );
}
