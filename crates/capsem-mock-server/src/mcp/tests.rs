use super::*;

fn call(arguments: Value) -> Value {
    response(json!({"jsonrpc": "2.0", "id": 17, "method": "tools/call",
        "params": {"name": "fixture_lookup", "arguments": arguments}}))
}

#[test]
fn ordinary_mcp_results_keep_the_original_content_and_id() {
    let result = call(json!({"query": "unchanged"}));
    assert_eq!(result["id"], 17);
    assert_eq!(
        result["result"]["content"][0]["text"],
        "capsem-mock-server:mcp:fixture_lookup"
    );
    assert_eq!(result["result"]["isError"], false);
}

#[test]
fn requested_fixture_size_is_exact_and_bounded() {
    for size in [0, 128, 2 * 1024 * 1024] {
        let result = call(json!({"response_bytes": size}));
        let text = result["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(text.len(), size);
        assert_eq!(result["result"]["isError"], false);
        if size >= 128 {
            assert!(text.starts_with("capsem-mock-server:mcp:fixture_lookup"));
        }
    }
}

#[test]
fn an_invalid_size_is_an_explicit_error_without_a_large_allocation() {
    for value in [json!(-1), json!(u64::MAX), json!("large")] {
        let result = call(json!({"response_bytes": value}));
        assert_eq!(result["result"]["isError"], true);
        assert!(result["result"]["content"][0]["text"].as_str().unwrap().len() < 128);
    }
}
