use super::*;

fn receipt() -> CacheReceipt {
    let pin = format!("sha256:{}", "a".repeat(64));
    let native = format!("sha256:{}", "b".repeat(64));
    let identity = CacheIdentity::new(&format!("registry.example/team/image@{pin}"), "amd64", 1).unwrap();
    let origin = image_reference(&identity.image().to_string()).unwrap();
    CacheReceipt::new(
        identity,
        &origin,
        Digest::parse(&native).unwrap(),
        vec![
            BlobRef::new(&pin, 100, BlobKind::Metadata).unwrap(),
            BlobRef::new(&native, 100, BlobKind::Metadata).unwrap(),
            BlobRef::new(&format!("sha256:{}", "c".repeat(64)), 100, BlobKind::Private).unwrap(),
        ],
    )
    .unwrap()
}

#[test]
fn receipt_codec_preserves_facts_but_has_no_ready_or_authority_field() {
    let original = receipt();
    let bytes = original.encode().unwrap();
    let loaded = CacheReceipt::decode(&bytes, &original.key()).unwrap();
    assert_eq!(loaded.identity(), original.identity());
    assert_eq!(loaded.native_digest(), original.native_digest());
    assert_eq!(loaded.generation().unwrap(), original.generation().unwrap());
    let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(doc.get("ready").is_none() && doc.get("allowed").is_none());
}

#[test]
fn malformed_foreign_or_oversized_receipts_are_not_accepted_as_owner_metadata() {
    let original = receipt();
    let doc: serde_json::Value = serde_json::from_slice(&original.encode().unwrap()).unwrap();
    for case in 0..7 {
        let mut changed = doc.clone();
        match case {
            0 => changed["schema_version"] = 999.into(),
            1 => changed["key"] = "../../outside".into(),
            2 => {
                changed["origin"] =
                    format!("another.example/team/image@{}", original.identity().image().digest()).into()
            }
            3 => changed["verified_at_unix_ns"] = 0.into(),
            4 => changed["blobs"][0]["digest"] = "sha256:../../outside".into(),
            5 => changed["blobs"] = serde_json::json!([]),
            6 => changed["ready"] = true.into(),
            _ => unreachable!(),
        }
        assert!(
            CacheReceipt::decode(&serde_json::to_vec(&changed).unwrap(), &original.key()).is_err(),
            "accepted case {case}"
        );
    }
    assert!(CacheReceipt::decode(&vec![b'x'; METADATA_LIMIT + 1], &original.key()).is_err());
}
