use super::*;

#[test]
fn references_are_stable_and_provider_scoped() {
    let raw = "not-a-real-secret";
    let openai = credential_reference("openai", raw);

    assert_eq!(openai, credential_reference("openai", raw));
    assert_ne!(openai, credential_reference("github", raw));
    assert!(is_credential_reference(&openai));
    assert!(!openai.contains(raw));
}

#[test]
fn reference_shape_rejects_raw_and_malformed_values() {
    assert!(!is_credential_reference("not-a-real-secret"));
    assert!(!is_credential_reference("credential:blake3:xyz"));
    assert!(!is_credential_reference(&format!(
        "{CREDENTIAL_REF_PREFIX}{}",
        "g".repeat(64)
    )));
}
