use super::*;

#[test]
fn security_hydration_reads_sparse_msgpack_and_legacy_json() {
    let event = capsem_proto::forensic::SecurityForensicEvent::from_json(
        r#"{"event_type":"http.request","credential_observations":[],"http":{"host":"api.openai.com","body":null}}"#,
        "http.request",
    )
    .unwrap();
    let encoded = event.encode().unwrap();
    let json = forensic_payload_json(Some("application/vnd.capsem.security+msgpack"), &encoded).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["http"]["host"], "api.openai.com");
    assert!(value.get("credential_observations").is_none());
    assert!(value["http"].get("body").is_none());

    let legacy = br#"{"http":{"host":"old.example"}}"#;
    assert_eq!(
        forensic_payload_json(Some("application/json"), legacy).as_deref(),
        std::str::from_utf8(legacy).ok()
    );
    assert!(forensic_payload_json(Some("application/vnd.capsem.security+msgpack"), b"broken").is_none());
}
