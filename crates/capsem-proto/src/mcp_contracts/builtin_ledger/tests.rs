use serde_json::json;

use super::*;

fn http_record() -> BuiltinLedgerRecord {
    BuiltinLedgerRecord::HttpRequest(HttpRequestRecord {
        timestamp_unix_ms: 1_700_000_000_000,
        domain: "example.com".to_string(),
        method: "GET".to_string(),
        path: "/about".to_string(),
        decision: HttpDecision::Allowed,
        status_code: Some(200),
        bytes_sent: 0,
        bytes_received: 42,
        duration_ms: 7,
        policy_action: "allow".to_string(),
        policy_rule: None,
        policy_reason: None,
    })
}

#[test]
fn records_round_trip_through_the_meta_value() {
    let records = vec![http_record(), http_record()];
    assert_eq!(decode(encode(&records)).unwrap(), records);
}

/// The wire shape is a contract between two binaries that ship together but
/// are built from different crates; pin it so a rename is a visible change.
#[test]
fn the_wire_shape_is_tagged_by_kind() {
    let value = encode(&[http_record()]);
    assert_eq!(value[0]["kind"], "http_request");
    assert_eq!(value[0]["decision"], "allowed");
}

#[test]
fn take_removes_the_key_and_an_emptied_meta() {
    let mut result = json!({
        "content": [{"type": "text", "text": "ok"}],
        "_meta": {BUILTIN_LEDGER_META_KEY: encode(&[http_record()])},
    });
    let taken = take(&mut result).expect("the key was there");
    assert_eq!(decode(taken).unwrap(), vec![http_record()]);
    assert_eq!(result, json!({"content": [{"type": "text", "text": "ok"}]}));
}

#[test]
fn take_leaves_the_rest_of_meta_alone() {
    let mut result = json!({
        "content": [],
        "_meta": {BUILTIN_LEDGER_META_KEY: [], "other": 1},
    });
    assert!(take(&mut result).is_some());
    assert_eq!(result, json!({"content": [], "_meta": {"other": 1}}));
}

#[test]
fn take_is_none_when_nothing_is_reserved() {
    for mut result in [
        json!({"content": []}),
        json!({"content": [], "_meta": {"other": 1}}),
        json!({"content": [], "_meta": "not an object"}),
        json!("not an object"),
    ] {
        let before = result.clone();
        assert!(take(&mut result).is_none());
        assert_eq!(result, before, "a result without the key is left untouched");
    }
}

#[test]
fn an_unknown_kind_does_not_decode() {
    assert!(decode(json!([{"kind": "exec", "command": "rm -rf /"}])).is_err());
}
