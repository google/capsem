//! Tests for `request_parser` (extracted from inline `mod tests`).

use super::*;

#[test]
fn test_extract_model_field() {
    let body = br#"{"model":"claude-3-opus-20240229","messages":[]}"#;
    assert_eq!(
        extract_model_field(body),
        Some("claude-3-opus-20240229".to_string())
    );

    let truncated = br#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "..."#;
    assert_eq!(extract_model_field(truncated), Some("gpt-4o".to_string()));

    let spaced = br#"{ "model" : "test-model" }"#;
    assert_eq!(extract_model_field(spaced), Some("test-model".to_string()));

    let none = br#"{"messages":[]}"#;
    assert_eq!(extract_model_field(none), None);
}

#[test]
fn test_truncated_json_fallback() {
    let truncated =
        br#"{"model": "claude-3-5-sonnet-20240620", "messages": [{"role": "user", "con"#;
    let meta = parse_request(ProviderKind::Anthropic, truncated);
    assert_eq!(meta.model.as_deref(), Some("claude-3-5-sonnet-20240620"));
    assert_eq!(meta.messages_count, 0); // parsing failed, but model was extracted
}

// ── Anthropic ───────────────────────────────────────────────────

#[test]
fn anthropic_basic_request() {
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "stream": true,
        "system": "You are a helpful assistant.",
        "messages": [
            {"role": "user", "content": "Hi"},
            {"role": "assistant", "content": "Hello!"},
            {"role": "user", "content": "How are you?"}
        ],
        "tools": [
            {"name": "get_weather"},
            {"name": "search"}
        ]
    }"#;

    let meta = parse_request(ProviderKind::Anthropic, body);
    assert_eq!(meta.model.as_deref(), Some("claude-sonnet-4-20250514"));
    assert!(meta.stream);
    assert_eq!(
        meta.system_prompt_preview.as_deref(),
        Some("You are a helpful assistant.")
    );
    assert_eq!(meta.messages_count, 3);
    assert_eq!(meta.tools_count, 2);
    assert!(meta.tool_results.is_empty());
}

#[test]
fn anthropic_system_as_blocks() {
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "system": [{"type": "text", "text": "Block system prompt."}],
        "messages": [{"role": "user", "content": "Hi"}]
    }"#;

    let meta = parse_request(ProviderKind::Anthropic, body);
    assert_eq!(
        meta.system_prompt_preview.as_deref(),
        Some("Block system prompt.")
    );
}

#[test]
fn anthropic_tool_results() {
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": "weather?"},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_01", "name": "get_weather", "input": {"city": "NYC"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_01", "content": "72F and sunny"}
            ]}
        ]
    }"#;

    let meta = parse_request(ProviderKind::Anthropic, body);
    assert_eq!(meta.messages_count, 3);
    assert_eq!(meta.tool_results.len(), 1);
    assert_eq!(meta.tool_results[0].call_id, "toolu_01");
    assert_eq!(meta.tool_results[0].content_preview, "72F and sunny");
    assert!(!meta.tool_results[0].is_error);
}

#[test]
fn anthropic_tool_result_error() {
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_err", "content": "connection timeout", "is_error": true}
            ]}
        ]
    }"#;

    let meta = parse_request(ProviderKind::Anthropic, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert!(meta.tool_results[0].is_error);
}

// ── OpenAI ──────────────────────────────────────────────────────

#[test]
fn openai_chat_completions_request() {
    let body = br#"{
        "model": "gpt-4o",
        "stream": true,
        "messages": [
            {"role": "system", "content": "You help with code."},
            {"role": "user", "content": "Write hello world"}
        ],
        "tools": [
            {"type": "function", "function": {"name": "run_code"}}
        ]
    }"#;

    let meta = parse_request(ProviderKind::OpenAi, body);
    assert_eq!(meta.model.as_deref(), Some("gpt-4o"));
    assert!(meta.stream);
    assert_eq!(
        meta.system_prompt_preview.as_deref(),
        Some("You help with code.")
    );
    assert_eq!(meta.messages_count, 2);
    assert_eq!(meta.tools_count, 1);
}

#[test]
fn openai_responses_api_request() {
    let body = br#"{
        "model": "gpt-4o",
        "instructions": "You are a coding assistant.",
        "input": [
            {"role": "user", "content": "Help me"}
        ]
    }"#;

    let meta = parse_request(ProviderKind::OpenAi, body);
    assert_eq!(
        meta.system_prompt_preview.as_deref(),
        Some("You are a coding assistant.")
    );
    assert_eq!(meta.messages_count, 1);
}

#[test]
fn openai_tool_results() {
    let body = br#"{
        "model": "gpt-4o",
        "messages": [
            {"role": "user", "content": "weather?"},
            {"role": "assistant", "content": null},
            {"role": "tool", "tool_call_id": "call_abc", "content": "72F sunny"}
        ]
    }"#;

    let meta = parse_request(ProviderKind::OpenAi, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert_eq!(meta.tool_results[0].call_id, "call_abc");
    assert_eq!(meta.tool_results[0].content_preview, "72F sunny");
}

// ── Google ──────────────────────────────────────────────────────

#[test]
fn google_basic_request() {
    let body = br#"{
        "contents": [
            {"parts": [{"text": "Hi"}], "role": "user"},
            {"parts": [{"text": "Hello!"}], "role": "model"}
        ],
        "tools": [
            {"functionDeclarations": [{"name": "search"}, {"name": "calc"}]}
        ],
        "systemInstruction": {
            "parts": [{"text": "Be helpful."}]
        }
    }"#;

    let meta = parse_request(ProviderKind::Google, body);
    assert!(meta.model.is_none()); // model is in URL for Google
    assert!(!meta.stream); // streaming detected from URL path, not body
    assert_eq!(meta.system_prompt_preview.as_deref(), Some("Be helpful."));
    assert_eq!(meta.messages_count, 2);
    assert_eq!(meta.tools_count, 2);
}

#[test]
fn google_function_response() {
    let body = br#"{
        "contents": [
            {"parts": [{"text": "weather?"}], "role": "user"},
            {"parts": [{"functionCall": {"name": "get_weather", "args": {"city": "NYC"}}}], "role": "model"},
            {"parts": [{"functionResponse": {"name": "get_weather", "response": {"temp": "72F"}}}], "role": "function"}
        ]
    }"#;

    let meta = parse_request(ProviderKind::Google, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert!(meta.tool_results[0]
        .call_id
        .starts_with("gemini_get_weather_"));
    assert!(meta.tool_results[0].content_preview.contains("72F"));
}

#[test]
fn google_function_response_preserves_bytes_verbatim() {
    // response is stored as RawValue, so content_preview holds the exact
    // byte slice from the wire -- whitespace, key order, and all.
    // A serde_json::Value would have re-serialized to canonical compact form.
    let body = br#"{"contents":[{"parts":[{"functionResponse":{"name":"get_weather","response":{"temp" : "72F" , "humidity":  "50%"}}}],"role":"function"}]}"#;

    let meta = parse_request(ProviderKind::Google, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert_eq!(
        meta.tool_results[0].content_preview,
        r#"{"temp" : "72F" , "humidity":  "50%"}"#
    );
}

// ── Adversarial ─────────────────────────────────────────────────

#[test]
fn empty_body() {
    let meta = parse_request(ProviderKind::Anthropic, b"");
    assert!(meta.model.is_none());
    assert_eq!(meta.messages_count, 0);
}

#[test]
fn invalid_json() {
    let meta = parse_request(ProviderKind::OpenAi, b"not json");
    assert!(meta.model.is_none());
    assert_eq!(meta.messages_count, 0);
}

#[test]
fn non_json_content_type() {
    let meta = parse_request(ProviderKind::Google, b"<html>not json</html>");
    assert!(meta.model.is_none());
}

#[test]
fn long_system_prompt_passes_through_untruncated() {
    let long_prompt = "x".repeat(500);
    let body = format!(
        r#"{{"model":"claude-sonnet-4-20250514","system":"{}","messages":[]}}"#,
        long_prompt
    );
    let meta = parse_request(ProviderKind::Anthropic, body.as_bytes());
    let preview = meta.system_prompt_preview.unwrap();
    assert_eq!(preview.len(), 500);
    assert_eq!(preview, long_prompt);
}

#[test]
fn request_without_stream_field_defaults_false() {
    let body =
        br#"{"model":"claude-sonnet-4-20250514","messages":[{"role":"user","content":"hi"}]}"#;
    let meta = parse_request(ProviderKind::Anthropic, body);
    assert!(!meta.stream);
}

#[test]
fn corrupt_utf8_in_body() {
    // JSON with invalid UTF-8 bytes in the model value.
    // from_utf8_lossy replaces \xFF with the Unicode replacement char,
    // so the regex-based fallback still extracts *something* (with the
    // replacement char). Verify we don't panic.
    let mut body = br#"{"model":"test","messages":[]}"#.to_vec();
    body[10] = 0xFF;
    let meta = parse_request(ProviderKind::Anthropic, &body);
    // The regex extracts "te\u{FFFD}t" via lossy conversion -- that's fine,
    // it won't match any real model for pricing. The key invariant is no panic.
    assert!(meta.model.is_some());
}

// ── Multi-turn dedup tests (Bug 1) ──────────────────────────────

#[test]
fn google_multi_turn_only_extracts_latest_tool_results() {
    // 3-turn conversation: turn 1 has a functionResponse, turn 3 re-sends
    // turn 1's history AND adds a new functionResponse. Only turn 3's
    // new result should be extracted.
    let body = br#"{
        "contents": [
            {"parts": [{"text": "weather?"}], "role": "user"},
            {"parts": [{"functionCall": {"name": "get_weather", "args": {"city": "NYC"}}}], "role": "model"},
            {"parts": [{"functionResponse": {"name": "get_weather", "response": {"temp": "72F"}}}], "role": "function"},
            {"parts": [{"text": "Looking up..."}], "role": "model"},
            {"parts": [{"text": "also check Paris"}], "role": "user"},
            {"parts": [{"functionCall": {"name": "get_weather", "args": {"city": "Paris"}}}], "role": "model"},
            {"parts": [{"functionResponse": {"name": "get_weather", "response": {"temp": "18C"}}}], "role": "function"}
        ]
    }"#;

    let meta = parse_request(ProviderKind::Google, body);
    // Only the trailing function message (Paris) should be extracted.
    assert_eq!(meta.tool_results.len(), 1);
    assert!(meta.tool_results[0].content_preview.contains("18C"));
}

#[test]
fn google_duplicate_function_name_unique_call_ids() {
    // Two calls to same function in trailing position.
    let body = br#"{
        "contents": [
            {"parts": [{"text": "weather?"}], "role": "user"},
            {"parts": [{"functionCall": {"name": "get_weather", "args": {}}}], "role": "model"},
            {"parts": [
                {"functionResponse": {"name": "get_weather", "response": {"temp": "72F"}}},
                {"functionResponse": {"name": "get_weather", "response": {"temp": "18C"}}}
            ], "role": "function"}
        ]
    }"#;

    let meta = parse_request(ProviderKind::Google, body);
    assert_eq!(meta.tool_results.len(), 2);
    // call_ids must be distinct
    assert_ne!(meta.tool_results[0].call_id, meta.tool_results[1].call_id);
    assert!(meta.tool_results[0]
        .call_id
        .starts_with("gemini_get_weather_"));
    assert!(meta.tool_results[1]
        .call_id
        .starts_with("gemini_get_weather_"));
}

#[test]
fn google_single_turn_tool_result_still_works() {
    // Regression: single-turn with one function response still extracts it.
    let body = br#"{
        "contents": [
            {"parts": [{"text": "weather?"}], "role": "user"},
            {"parts": [{"functionCall": {"name": "get_weather", "args": {}}}], "role": "model"},
            {"parts": [{"functionResponse": {"name": "get_weather", "response": {"temp": "72F"}}}], "role": "function"}
        ]
    }"#;

    let meta = parse_request(ProviderKind::Google, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert!(meta.tool_results[0].content_preview.contains("72F"));
}

#[test]
fn anthropic_multi_turn_only_extracts_latest_tool_results() {
    // Multi-turn: turn 1 has tool_result, turn 3 re-sends it AND adds new one.
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": "weather?"},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_01", "name": "get_weather", "input": {"city": "NYC"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_01", "content": "72F sunny"}
            ]},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_02", "name": "get_weather", "input": {"city": "Paris"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_02", "content": "18C cloudy"}
            ]}
        ]
    }"#;

    let meta = parse_request(ProviderKind::Anthropic, body);
    // Only the trailing user message (toolu_02) should be extracted.
    assert_eq!(meta.tool_results.len(), 1);
    assert_eq!(meta.tool_results[0].call_id, "toolu_02");
    assert_eq!(meta.tool_results[0].content_preview, "18C cloudy");
}

#[test]
fn openai_multi_turn_only_extracts_latest_tool_results() {
    // Multi-turn: tool results from turn 1 re-sent, new tool result in turn 3.
    let body = br#"{
        "model": "gpt-4o",
        "messages": [
            {"role": "user", "content": "weather?"},
            {"role": "assistant", "content": null},
            {"role": "tool", "tool_call_id": "call_01", "content": "72F sunny"},
            {"role": "assistant", "content": "Got NYC weather."},
            {"role": "user", "content": "also Paris?"},
            {"role": "assistant", "content": null},
            {"role": "tool", "tool_call_id": "call_02", "content": "18C cloudy"}
        ]
    }"#;

    let meta = parse_request(ProviderKind::OpenAi, body);
    // Only the trailing tool message (call_02) should be extracted.
    assert_eq!(meta.tool_results.len(), 1);
    assert_eq!(meta.tool_results[0].call_id, "call_02");
    assert_eq!(meta.tool_results[0].content_preview, "18C cloudy");
}

// ── Anthropic non-text content blocks (Phase 1) ─────────────────

#[test]
fn anthropic_tool_result_with_tool_reference_blocks() {
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_ref", "content": [
                    {"type": "tool_reference", "tool_name": "fetch_http"},
                    {"type": "tool_reference", "tool_name": "http_headers"}
                ]}
            ]}
        ]
    }"#;
    let meta = parse_request(ProviderKind::Anthropic, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert!(
        !meta.tool_results[0].content_preview.is_empty(),
        "content_preview should not be empty for tool_reference blocks"
    );
    assert!(
        meta.tool_results[0].content_preview.contains("fetch_http"),
        "content_preview should mention fetch_http, got: {}",
        meta.tool_results[0].content_preview
    );
}

#[test]
fn anthropic_tool_result_mixed_text_and_non_text_blocks() {
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_mix", "content": [
                    {"type": "text", "text": "Loaded 2 tools"},
                    {"type": "tool_reference", "tool_name": "fetch_http"}
                ]}
            ]}
        ]
    }"#;
    let meta = parse_request(ProviderKind::Anthropic, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert!(
        meta.tool_results[0]
            .content_preview
            .contains("Loaded 2 tools"),
        "text blocks take priority, got: {}",
        meta.tool_results[0].content_preview
    );
}

#[test]
fn anthropic_tool_result_empty_content_array() {
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_empty", "content": []}
            ]}
        ]
    }"#;
    let meta = parse_request(ProviderKind::Anthropic, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert_eq!(meta.tool_results[0].content_preview, "");
}

#[test]
fn anthropic_tool_result_null_content() {
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_null"}
            ]}
        ]
    }"#;
    let meta = parse_request(ProviderKind::Anthropic, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert_eq!(meta.tool_results[0].content_preview, "");
}

#[test]
fn anthropic_tool_result_image_block_only() {
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_img", "content": [
                    {"type": "image", "source": {"type": "base64", "data": "aWdub3Jl"}}
                ]}
            ]}
        ]
    }"#;
    let meta = parse_request(ProviderKind::Anthropic, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert!(
        !meta.tool_results[0].content_preview.is_empty(),
        "image block should produce a fallback like [image]"
    );
}

#[test]
fn anthropic_tool_result_blocks_with_text_none() {
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_notext", "content": [
                    {"type": "text"}
                ]}
            ]}
        ]
    }"#;
    let meta = parse_request(ProviderKind::Anthropic, body);
    assert_eq!(meta.tool_results.len(), 1);
    // Should not crash
}

#[test]
fn anthropic_multiple_tool_results_in_single_message() {
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_a", "content": "result a"},
                {"type": "tool_result", "tool_use_id": "toolu_b", "content": "result b"},
                {"type": "tool_result", "tool_use_id": "toolu_c", "content": "result c"}
            ]}
        ]
    }"#;
    let meta = parse_request(ProviderKind::Anthropic, body);
    assert_eq!(meta.tool_results.len(), 3);
    assert_eq!(meta.tool_results[0].call_id, "toolu_a");
    assert_eq!(meta.tool_results[1].call_id, "toolu_b");
    assert_eq!(meta.tool_results[2].call_id, "toolu_c");
}

#[test]
fn anthropic_tool_result_large_content() {
    let big = "x".repeat(100_000);
    let body = format!(
        r#"{{"model":"claude-sonnet-4-20250514","messages":[
            {{"role":"user","content":[
                {{"type":"tool_result","tool_use_id":"toolu_big","content":"{big}"}}
            ]}}
        ]}}"#
    );
    let meta = parse_request(ProviderKind::Anthropic, body.as_bytes());
    assert_eq!(meta.tool_results.len(), 1);
    assert!(!meta.tool_results[0].content_preview.is_empty());
}

#[test]
fn anthropic_tool_result_content_as_blocks_with_text() {
    let body = br#"{
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_multi", "content": [
                    {"type": "text", "text": "line1"},
                    {"type": "text", "text": "line2"}
                ]}
            ]}
        ]
    }"#;
    let meta = parse_request(ProviderKind::Anthropic, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert_eq!(meta.tool_results[0].content_preview, "line1\nline2");
}

// ── OpenAI edge cases (Phase 1) ─────────────────────────────────

#[test]
fn openai_tool_result_empty_content() {
    let body = br#"{
        "model": "gpt-4o",
        "messages": [
            {"role": "tool", "tool_call_id": "call_empty", "content": ""}
        ]
    }"#;
    let meta = parse_request(ProviderKind::OpenAi, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert_eq!(meta.tool_results[0].content_preview, "");
}

#[test]
fn openai_tool_result_null_content() {
    let body = br#"{
        "model": "gpt-4o",
        "messages": [
            {"role": "tool", "tool_call_id": "call_null", "content": null}
        ]
    }"#;
    let meta = parse_request(ProviderKind::OpenAi, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert_eq!(meta.tool_results[0].content_preview, "");
}

#[test]
fn openai_tool_result_multipart_content() {
    let body = br#"{
        "model": "gpt-4o",
        "messages": [
            {"role": "tool", "tool_call_id": "call_parts", "content": [
                {"type": "text", "text": "result here"}
            ]}
        ]
    }"#;
    let meta = parse_request(ProviderKind::OpenAi, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert!(
        meta.tool_results[0].content_preview.contains("result here"),
        "multipart content should extract text, got: {}",
        meta.tool_results[0].content_preview
    );
}

#[test]
fn openai_multiple_tool_results_trailing() {
    let body = br#"{
        "model": "gpt-4o",
        "messages": [
            {"role": "assistant", "content": null},
            {"role": "tool", "tool_call_id": "call_1", "content": "r1"},
            {"role": "tool", "tool_call_id": "call_2", "content": "r2"},
            {"role": "tool", "tool_call_id": "call_3", "content": "r3"}
        ]
    }"#;
    let meta = parse_request(ProviderKind::OpenAi, body);
    assert_eq!(meta.tool_results.len(), 3);
}

#[test]
fn openai_tool_result_large_content() {
    let big = "x".repeat(100_000);
    let body = format!(
        r#"{{"model":"gpt-4o","messages":[
            {{"role":"tool","tool_call_id":"call_big","content":"{big}"}}
        ]}}"#
    );
    let meta = parse_request(ProviderKind::OpenAi, body.as_bytes());
    assert_eq!(meta.tool_results.len(), 1);
    assert!(!meta.tool_results[0].content_preview.is_empty());
}

// ── Google/Gemini edge cases (Phase 1) ──────────────────────────

#[test]
fn google_function_response_null_response() {
    let body = br#"{
        "contents": [
            {"parts": [{"functionResponse": {"name": "get_weather", "response": null}}], "role": "function"}
        ]
    }"#;
    let meta = parse_request(ProviderKind::Google, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert_eq!(meta.tool_results[0].content_preview, "");
}

#[test]
fn google_function_response_empty_object() {
    let body = br#"{
        "contents": [
            {"parts": [{"functionResponse": {"name": "get_weather", "response": {}}}], "role": "function"}
        ]
    }"#;
    let meta = parse_request(ProviderKind::Google, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert_eq!(meta.tool_results[0].content_preview, "{}");
}

#[test]
fn google_function_response_nested_response() {
    let body = br#"{
        "contents": [
            {"parts": [{"functionResponse": {"name": "list_items", "response": {"data": {"items": [1,2,3]}}}}], "role": "function"}
        ]
    }"#;
    let meta = parse_request(ProviderKind::Google, body);
    assert_eq!(meta.tool_results.len(), 1);
    assert!(
        meta.tool_results[0].content_preview.contains("items"),
        "nested response should contain 'items', got: {}",
        meta.tool_results[0].content_preview
    );
}

#[test]
fn google_multiple_function_responses_in_single_part() {
    let body = br#"{
        "contents": [
            {"parts": [
                {"functionResponse": {"name": "fn_a", "response": {"a": 1}}},
                {"functionResponse": {"name": "fn_b", "response": {"b": 2}}},
                {"functionResponse": {"name": "fn_c", "response": {"c": 3}}}
            ], "role": "function"}
        ]
    }"#;
    let meta = parse_request(ProviderKind::Google, body);
    assert_eq!(meta.tool_results.len(), 3);
    // All should have unique call_ids
    let ids: std::collections::HashSet<_> = meta.tool_results.iter().map(|r| &r.call_id).collect();
    assert_eq!(
        ids.len(),
        3,
        "all 3 function responses should have unique call_ids"
    );
}
