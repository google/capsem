use super::*;

#[test]
fn generation_parser_requires_canonical_nonzero_hex() {
    assert_eq!(
        parse_generation("07070707070707070707070707070707").unwrap().as_bytes(),
        [7; 16]
    );
    for invalid in [
        "",
        "00000000000000000000000000000000",
        "0707070707070707070707070707070",
        "070707070707070707070707070707070",
        "0707070707070707070707070707070G",
        "0707070707070707070707070707070A",
    ] {
        assert!(parse_generation(invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn failed_policy_update_preserves_the_running_engine_revision() {
    let active_policy = br#"
[network]
[user_rules.profiles.rules.worker_http]
name = "worker_http"
action = "allow"
match = 'http.host == "worker.example"'
[corp_rules]
"#;
    let mut state = ProxyRuntimeState::default();
    let digest = state.apply(active_policy).unwrap();

    assert!(state.apply(b"[network]\nunknown = true").is_err());
    assert_eq!(state.engine.as_ref().unwrap().policy().snapshot().digest(), digest);
}
