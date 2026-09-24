use super::*;

#[test]
fn an_empty_ledger_encodes_as_an_empty_map() {
    // One byte: a fixmap of length zero. A default written as zeros would
    // make every idle session's snapshot pay for every field it never used.
    assert_eq!(LedgerCounters::default().encode().unwrap(), vec![0x80]);
    assert_eq!(LedgerCounters::decode(&[0x80]).unwrap(), LedgerCounters::default());
}

#[test]
fn a_populated_snapshot_roundtrips_with_named_fields() {
    let mut counters = LedgerCounters::default();
    counters.net.total = 3;
    counters.net.denied = 1;
    counters.model.total.cost_micro_usd = 1_250;
    counters.model.usage_details.insert("thinking".into(), 42);
    bounded_entry(bounded_entry(&mut counters.model.by_model, "anthropic"), "claude").calls = 2;
    counters.security.open_asks.insert("ask-1".into());
    counters.credentials.insert(
        "capsem-cred://a".into(),
        CredentialCounters {
            provider: Some("github".into()),
            injections: 1,
            ..CredentialCounters::default()
        },
    );

    let encoded = counters.encode().unwrap();
    assert_eq!(LedgerCounters::decode(&encoded).unwrap(), counters);
    // Named, not positional: a reader keyed on field names survives a field
    // being added in front of the one it wants.
    // Untouched sections are omitted, not written as zeros.
    let as_value: serde_json::Value = rmp_serde::from_slice(&encoded).unwrap();
    let keys: BTreeSet<&str> = as_value.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, BTreeSet::from(["net", "model", "security", "credentials"]));
    assert_eq!(as_value["net"], serde_json::json!({"total": 3, "denied": 1}));
}

#[test]
fn a_map_past_its_bound_folds_new_keys_into_overflow() {
    let mut map = BTreeMap::<String, u64>::new();
    for index in 0..MAX_KEYS_PER_MAP {
        *bounded_entry(&mut map, &format!("tool-{index}")) += 1;
    }
    *bounded_entry(&mut map, "one-too-many") += 1;
    *bounded_entry(&mut map, "and-another") += 1;
    // A key already attributed keeps its own entry after the bound.
    *bounded_entry(&mut map, "tool-0") += 1;

    assert_eq!(map.len(), MAX_KEYS_PER_MAP + 1);
    assert_eq!(map[OVERFLOW_KEY], 2);
    assert_eq!(map["tool-0"], 2);
    assert!(!map.contains_key("one-too-many"));
    assert_eq!(map.values().sum::<u64>(), MAX_KEYS_PER_MAP as u64 + 3);
}

#[test]
fn an_oversized_snapshot_is_refused_on_decode() {
    let oversized = vec![0u8; MAX_ENCODED_EVENT_BYTES + 1];
    assert!(LedgerCounters::decode(&oversized).is_err());
}
