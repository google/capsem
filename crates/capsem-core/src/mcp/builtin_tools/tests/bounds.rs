//! Pagination and grep parameters come straight from the tool call. They are
//! clamped at parse time and combined with saturating arithmetic, so a
//! hostile `max_length: 18446744073709551615` neither panics (debug) nor
//! slices out of range (release).

use super::*;

fn loopback_allowed_rules() -> SecurityRuleSet {
    crate::net::policy_config::SecurityRuleProfile::parse_toml(
        r#"
            [profiles.rules.allow_local_fixture]
            name = "allow_local_fixture"
            action = "allow"
            reason = "local test fixture"
            match = 'http.host == "127.0.0.1"'
            "#,
    )
    .and_then(|profile| SecurityRuleSet::compile_profile(&profile, crate::net::policy_config::SecurityRuleSource::User))
    .expect("test security rules compile")
}

async fn call(tool: &str, args: Value) -> JsonRpcResponse {
    let fixture = spawn_builtin_http_fixture().await;
    let mut args = args;
    args["url"] = Value::String(format!("{}/about", fixture.base_url));
    call_builtin_tool(
        tool,
        &args,
        &test_client(),
        &loopback_allowed_rules(),
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await
}

const HOSTILE_SIZES: &[u64] = &[u64::MAX, u64::MAX - 1, u64::MAX / 2 + 1, 1 << 40];

#[test]
fn paginate_saturates_instead_of_overflowing() {
    let (chunk, total, more) = paginate("héllo wörld", 1, usize::MAX);
    assert_eq!(total, "héllo wörld".len());
    assert_eq!(chunk, &"héllo wörld"[1..]);
    assert!(!more);
    let (chunk, _, more) = paginate("abc", usize::MAX, usize::MAX);
    assert!(chunk.is_empty() && !more);
    let (chunk, _, more) = paginate("abc", usize::MAX - 1, 1);
    assert!(chunk.is_empty() && !more);
}

#[test]
fn grep_context_saturates_instead_of_overflowing() {
    let re = regex::Regex::new("b").unwrap();
    let lines = ["a", "b", "c"];
    let (blocks, total) = collect_grep_matches(&lines, &re, usize::MAX, usize::MAX);
    assert_eq!(total, 1);
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].contains(">>> 2: b"));
    assert!(blocks[0].contains("    1: a") && blocks[0].contains("    3: c"));
}

#[test]
fn tool_params_are_clamped_at_parse_time() {
    let args = serde_json::json!({"max_length": u64::MAX, "start_index": u64::MAX, "context_lines": u64::MAX, "max_matches": u64::MAX});
    assert_eq!(
        bounded_param(&args, "max_length", DEFAULT_MAX_LENGTH, MAX_PAGE_LENGTH),
        MAX_PAGE_LENGTH
    );
    assert_eq!(bounded_param(&args, "start_index", 0, MAX_START_INDEX), MAX_START_INDEX);
    assert_eq!(
        bounded_param(&args, "context_lines", DEFAULT_CONTEXT_LINES, MAX_CONTEXT_LINES),
        MAX_CONTEXT_LINES
    );
    assert_eq!(
        bounded_param(&args, "max_matches", DEFAULT_MAX_MATCHES, MAX_GREP_MATCHES),
        MAX_GREP_MATCHES
    );
    let (start, max) = pagination_params(&args);
    assert_eq!((start, max), (MAX_START_INDEX, MAX_PAGE_LENGTH));

    let defaults = serde_json::json!({"max_length": -1, "start_index": "9", "context_lines": 1.5, "max_matches": null});
    assert_eq!(pagination_params(&defaults), (0, DEFAULT_MAX_LENGTH as usize));
    assert_eq!(bounded_param(&defaults, "context_lines", 7, 10), 7);
    assert_eq!(bounded_param(&defaults, "max_matches", 7, 10), 7);
    assert_eq!(
        bounded_param(&serde_json::json!({"max_matches": 3}), "max_matches", 7, 10),
        3
    );
}

#[tokio::test]
async fn fetch_http_survives_hostile_pagination_params() {
    for &max_length in HOSTILE_SIZES {
        let resp = call(
            "fetch_http",
            serde_json::json!({"start_index": 1, "max_length": max_length}),
        )
        .await;
        assert!(!is_tool_error(&resp), "max_length={max_length}: {resp:?}");
        assert!(
            extract_tool_text(&resp).contains("Elie"),
            "content must still be served"
        );
    }
    for &start_index in HOSTILE_SIZES {
        let resp = call(
            "fetch_http",
            serde_json::json!({"start_index": start_index, "max_length": u64::MAX}),
        )
        .await;
        assert!(!is_tool_error(&resp), "start_index={start_index}: {resp:?}");
        let text = extract_tool_text(&resp);
        assert!(text.contains("Content length:"), "{text}");
        assert!(
            !text.contains("Remaining:"),
            "past the end there is nothing left: {text}"
        );
    }
}

#[tokio::test]
async fn grep_http_survives_hostile_grep_and_pagination_params() {
    for &value in HOSTILE_SIZES {
        let resp = call(
            "grep_http",
            serde_json::json!({"pattern": "Elie", "context_lines": value, "max_matches": value, "max_length": value, "start_index": 1}),
        )
        .await;
        assert!(!is_tool_error(&resp), "value={value}: {resp:?}");
        assert!(extract_tool_text(&resp).contains("Matches found:"));
    }
    let resp = call(
        "grep_http",
        serde_json::json!({"pattern": "Elie", "start_index": u64::MAX, "max_length": u64::MAX}),
    )
    .await;
    assert!(!is_tool_error(&resp));
    assert!(extract_tool_text(&resp).is_empty(), "past the end yields an empty page");
}

#[tokio::test]
async fn http_headers_survives_hostile_pagination_params() {
    for &value in HOSTILE_SIZES {
        let resp = call(
            "http_headers",
            serde_json::json!({"start_index": 1, "max_length": value}),
        )
        .await;
        assert!(!is_tool_error(&resp), "max_length={value}: {resp:?}");
        assert!(extract_tool_text(&resp).contains("Status: 200"));
        let resp = call(
            "http_headers",
            serde_json::json!({"start_index": value, "max_length": 10}),
        )
        .await;
        assert!(!is_tool_error(&resp), "start_index={value}: {resp:?}");
        assert!(extract_tool_text(&resp).is_empty());
    }
}
