use super::*;

#[test]
fn trace_state_new_trace_on_no_match() {
    let state = TraceState::new();
    assert!(state.lookup(&["call_1".to_string()]).is_none());
    assert!(state.lookup(&[]).is_none());
}

#[test]
fn trace_state_register_and_lookup() {
    let mut state = TraceState::new();
    state.register_tool_calls("trace_A", &["call_1".to_string(), "call_2".to_string()]);

    assert_eq!(
        state.lookup(&["call_1".to_string()]).as_deref(),
        Some("trace_A")
    );
    assert_eq!(
        state.lookup(&["call_2".to_string()]).as_deref(),
        Some("trace_A")
    );
    assert!(state.lookup(&["call_3".to_string()]).is_none());
}

#[test]
fn trace_state_complete_cleans_up() {
    let mut state = TraceState::new();
    state.register_tool_calls("trace_A", &["call_1".to_string()]);
    assert!(state.lookup(&["call_1".to_string()]).is_some());

    state.complete_trace("trace_A");
    assert!(state.lookup(&["call_1".to_string()]).is_none());
}

#[test]
fn trace_state_concurrent_traces_isolated() {
    let mut state = TraceState::new();
    state.register_tool_calls("trace_A", &["call_A1".to_string()]);
    state.register_tool_calls("trace_B", &["call_B1".to_string()]);

    assert_eq!(
        state.lookup(&["call_A1".to_string()]).as_deref(),
        Some("trace_A")
    );
    assert_eq!(
        state.lookup(&["call_B1".to_string()]).as_deref(),
        Some("trace_B")
    );

    // Complete trace_A, trace_B remains.
    state.complete_trace("trace_A");
    assert!(state.lookup(&["call_A1".to_string()]).is_none());
    assert_eq!(
        state.lookup(&["call_B1".to_string()]).as_deref(),
        Some("trace_B")
    );
}

#[test]
fn trace_state_multiple_tool_calls_same_trace() {
    let mut state = TraceState::new();
    let calls: Vec<String> = (0..3).map(|i| format!("call_{i}")).collect();
    state.register_tool_calls("trace_X", &calls);

    for call in &calls {
        assert_eq!(
            state.lookup(std::slice::from_ref(call)).as_deref(),
            Some("trace_X"),
        );
    }
}

#[test]
fn trace_state_registers_workspace_file_hints_from_tool_arguments() {
    let mut state = TraceState::new();
    state.register_tool_file_hints(
        "trace_file",
        [
            r#"{"cmd":"printf '%s\n' abc > /root/openai-two-123.txt","file_path":"/root/direct.txt"}"#,
        ],
    );

    assert_eq!(
        state.lookup_file_path("openai-two-123.txt").as_deref(),
        Some("trace_file")
    );
    assert_eq!(
        state.lookup_file_path("/root/direct.txt").as_deref(),
        Some("trace_file")
    );
    assert_eq!(
        state.lookup_file_path("/workspace/direct.txt").as_deref(),
        Some("trace_file")
    );
    assert!(state.lookup_file_path("../escape.txt").is_none());
}

#[test]
fn trace_state_keeps_file_hints_after_tool_trace_completes() {
    let mut state = TraceState::new();
    state.register_tool_calls("trace_file", &["call_1".to_string()]);
    state.register_tool_file_hints(
        "trace_file",
        [r#"{"cmd":"printf '%s\n' abc > /root/later.txt"}"#],
    );

    state.complete_trace("trace_file");

    assert!(state.lookup(&["call_1".to_string()]).is_none());
    assert_eq!(
        state.lookup_file_path("later.txt").as_deref(),
        Some("trace_file")
    );
}

#[test]
fn trace_state_keeps_trace_credentials_for_late_file_events() {
    let mut state = TraceState::new();
    state.register_trace_credential(
        "trace_credential",
        Some("credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    );
    state.complete_trace("trace_credential");

    assert_eq!(
        state.lookup_trace_credential("trace_credential").as_deref(),
        Some("credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
}

#[test]
fn trace_state_preserves_first_credential_for_async_file_attribution() {
    let mut state = TraceState::new();
    state.register_trace_credential(
        "trace_credential",
        Some("credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    );
    state.register_trace_credential(
        "trace_credential",
        Some("credential:blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
    );

    assert_eq!(
        state.lookup_trace_credential("trace_credential").as_deref(),
        Some("credential:blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
}
