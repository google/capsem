use super::*;

#[test]
fn jwt_provider_accepts_only_rs256_public_components_and_never_signs() {
    install().unwrap();
    install().unwrap();
    let key = DecodingKey::from_secret(b"secret");
    assert!((PROVIDER.verifier_factory)(&Algorithm::HS256, &key).is_err());
    assert!((PROVIDER.verifier_factory)(&Algorithm::RS256, &key).is_err());
    assert!((PROVIDER.signer_factory)(&Algorithm::RS256, &jsonwebtoken::EncodingKey::from_secret(b"secret")).is_err());
    let utilities = &PROVIDER.key_utils;
    assert!((utilities.rsa_pub_components_from_private_key)(b"key").is_err());
    assert!((utilities.rsa_pub_components_from_public_key)(b"key").is_err());
    assert!(decode::<serde_json::Value>("invalid", &key, &Validation::new(Algorithm::RS256)).is_err());
}

#[cfg(feature = "test-fixtures")]
#[test]
fn owned_rsa_verification_rejects_corrupted_signatures() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/oauth/identity-jwks-test-only.json"
    ))
    .unwrap();
    let key = DecodingKey::from_rsa_components(
        fixture["keys"][0]["n"].as_str().unwrap(),
        fixture["keys"][0]["e"].as_str().unwrap(),
    )
    .unwrap();
    let verifier = (PROVIDER.verifier_factory)(&Algorithm::RS256, &key).unwrap();
    assert_eq!(verifier.algorithm(), Algorithm::RS256);
    let message = b"header.claims";
    let signature = sign_rs256_fixture(
        include_bytes!("../../../../tests/fixtures/oauth/identity-signing-test-only.der"),
        message,
    )
    .unwrap();
    verifier.verify(message, &signature).unwrap();
    assert!(verifier.verify(b"header.modified", &signature).is_err());
    assert!(sign_rs256_fixture(b"invalid key", message).is_err());
}
