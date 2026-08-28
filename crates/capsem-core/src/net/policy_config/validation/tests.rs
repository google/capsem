use super::*;

#[test]
fn identifiers_share_one_length_and_character_contract() {
    assert!(validate_identifier("fixture id", "alpha-2_beta").is_ok());
    assert!(validate_identifier("fixture id", &"a".repeat(64)).is_ok());

    for invalid in ["", " ", "Upper", "dot.id", &"a".repeat(65)] {
        let error = validate_identifier("fixture id", invalid).unwrap_err();
        assert!(error.contains("fixture id"), "{error}");
    }
}

#[test]
fn profile_targets_share_one_traversal_and_padding_contract() {
    assert!(validate_profile_target("fixture target", "server/tool.name").is_ok());
    assert!(validate_profile_target("fixture target", &"a".repeat(128)).is_ok());

    for invalid in [
        "",
        " ",
        "../tool",
        "server\\tool",
        " padded",
        &"a".repeat(129),
    ] {
        let error = validate_profile_target("fixture target", invalid).unwrap_err();
        assert!(error.contains("fixture target"), "{error}");
    }
}
