#[test]
fn blake3_identity_streaming_derivation_and_constant_time_equality_remain_compatible() {
    let expected = "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85";
    assert_eq!(crate::blake3::hash(b"abc").to_hex().as_str(), expected);
    let mut streaming = crate::blake3::Hasher::new();
    streaming.update(b"a").update(b"bc");
    let digest = streaming.finalize();
    assert_eq!(digest.to_hex().as_str(), expected);
    assert_eq!(digest, crate::blake3::Hash::from(*digest.as_bytes()));
    let mut derived = crate::blake3::Hasher::new_derive_key("capsem managed session capability v1");
    derived.update(b"abc");
    assert_ne!(digest, derived.finalize());
}

#[cfg(feature = "sha256")]
#[test]
fn websocket_digest_matches_rfc_6455_vector() {
    assert_eq!(
        crate::websocket_accept_digest(b"dGhlIHNhbXBsZSBub25jZQ=="),
        [179, 122, 79, 44, 192, 98, 79, 22, 144, 246, 70, 6, 207, 56, 89, 69, 178, 190, 196, 234]
    );
}

#[cfg(feature = "random")]
#[test]
fn random_tokens_keep_their_shape_and_fallible_entropy_contract() {
    let first = crate::random::alphanumeric(64);
    assert_eq!(first.len(), 64);
    assert!(first.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    assert_ne!(first, crate::random::alphanumeric(64));
    let mut entropy = [0; 32];
    crate::random::fill(&mut entropy).unwrap();
    assert_ne!(entropy, [0; 32]);
    assert!((10_000..99_999).contains(&crate::random::range_u32(10_000..99_999)));
    assert_ne!(crate::random::u64(), crate::random::u64());
}

#[cfg(feature = "secrets")]
#[test]
fn secret_comparison_handles_length_and_every_mismatch_position() {
    let secret = crate::secrets::Zeroizing::new(vec![b'x'; 64]);
    assert!(crate::secrets::constant_time_eq(&secret, &secret));
    assert!(!crate::secrets::constant_time_eq(&secret, &secret[..63]));
    for position in 0..secret.len() {
        let mut wrong = secret.to_vec();
        wrong[position] ^= 1;
        assert!(!crate::secrets::constant_time_eq(&secret, &wrong));
    }
}
