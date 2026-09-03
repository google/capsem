use super::*;

mod bounds;
mod fixtures;
mod ip_literals;
mod markdown;

fn test_db() -> Arc<DbWriter> {
    Arc::new(DbWriter::open_in_memory(64).unwrap())
}

/// Create a reqwest Client with proper User-Agent (matches production config).
/// Sites like Wikipedia return 403 without one.
fn test_client() -> Client {
    Client::builder()
        .user_agent("capsem-mcp/0.8")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest client")
}

async fn spawn_builtin_http_fixture() -> crate::test_support::http::LocalHttpRecorder {
    crate::test_support::http::spawn_static_http_recorder(vec![
        (
            "/",
            crate::test_support::http::RecordedHttpResponse::html(
                r#"
                    <!doctype html>
                    <html>
                      <head><title>Local Capsem HTTP Fixture</title></head>
                      <body>
                        <h1>Local Elie Test Page</h1>
                        <p>elie local deterministic page for builtin HTTP tests.</p>
                        <p>aaaaab proves regex safety without remote dependencies.</p>
                      </body>
                    </html>
                    "#,
            )
            .with_header("x-capsem-fixture", "home"),
        ),
        (
            "/about",
            crate::test_support::http::RecordedHttpResponse::html(about_fixture_html()),
        ),
        (
            "/wiki/Alan_Turing",
            crate::test_support::http::RecordedHttpResponse::html(
                "<html><body><h1>Alan Turing</h1><p>Turing proved useful local content.</p></body></html>",
            ),
        ),
        (
            "/wiki/Rust_(programming_language)",
            crate::test_support::http::RecordedHttpResponse::html(
                "<html><body><h1>Rust</h1><p>Mozilla sponsored early Rust work.</p></body></html>",
            ),
        ),
        (
            "/wiki/Unicode",
            crate::test_support::http::RecordedHttpResponse::html(
                "<html><body><h1>Unicode</h1><p>Unicode keeps café, 東京, and emoji safe.</p></body></html>",
            ),
        ),
    ])
    .await
    .expect("local HTTP fixture should start")
}

fn about_fixture_html() -> String {
    let repeated = "<p>Elie Bursztein works on Google security research, AI safety, and abuse prevention. <a href=\"/papers\">Read more</a>.</p>\n".repeat(80);
    format!(
        r#"<!doctype html>
            <html>
              <head>
                <title>Elie Bursztein - Local Fixture</title>
                <script>window.secret = "not content";</script>
              </head>
              <body>
                <main>
                  <h1>Elie Bursztein</h1>
                  <h2>About</h2>
                  {repeated}
                  <div>Google DeepMind AI Cybersecurity local fixture.</div>
                </main>
              </body>
            </html>"#
    )
}

fn default_dev_security_rules() -> SecurityRuleSet {
    crate::net::policy_config::SecurityRuleProfile::parse_toml(
        r#"
            [profiles.rules.block_evil_unknown_domain]
            name = "block_evil_unknown_domain"
            action = "block"
            reason = "test domain blocked"
            match = 'http.host == "evil-unknown-domain.xyz"'
            "#,
    )
    .and_then(|profile| SecurityRuleSet::compile_profile(&profile, crate::net::policy_config::SecurityRuleSource::User))
    .expect("test security rules compile")
}

#[test]
fn builtin_tool_defs_returns_three_tools() {
    let defs = builtin_tool_defs();
    assert_eq!(defs.len(), 3);
    assert!(defs.iter().all(|d| d.server_name == "builtin"));
    let names: Vec<&str> = defs.iter().map(|d| d.namespaced_name.as_str()).collect();
    assert!(names.contains(&"fetch_http"));
    assert!(names.contains(&"grep_http"));
    assert!(names.contains(&"http_headers"));
    // Names must NOT have the builtin__ prefix
    assert!(!names.iter().any(|n| n.starts_with("builtin__")));
}

#[test]
fn builtin_tool_annotations_all_present() {
    let defs = builtin_tool_defs();
    for def in &defs {
        assert!(
            def.annotations.is_some(),
            "tool '{}' missing annotations",
            def.namespaced_name
        );
    }
}

#[test]
fn fetch_http_annotations_correct() {
    let defs = builtin_tool_defs();
    let fetch = defs.iter().find(|d| d.namespaced_name == "fetch_http").unwrap();
    let ann = fetch.annotations.as_ref().unwrap();
    assert!(ann.read_only_hint, "fetch_http should be read-only");
    assert!(!ann.destructive_hint, "fetch_http should not be destructive");
    assert!(ann.idempotent_hint, "fetch_http should be idempotent");
    assert!(ann.open_world_hint, "fetch_http should be open-world");
}

#[test]
fn grep_http_annotations_correct() {
    let defs = builtin_tool_defs();
    let grep = defs.iter().find(|d| d.namespaced_name == "grep_http").unwrap();
    let ann = grep.annotations.as_ref().unwrap();
    assert!(ann.read_only_hint, "grep_http should be read-only");
    assert!(!ann.destructive_hint, "grep_http should not be destructive");
    assert!(ann.idempotent_hint, "grep_http should be idempotent");
    assert!(ann.open_world_hint, "grep_http should be open-world");
}

#[test]
fn http_headers_annotations_correct() {
    let defs = builtin_tool_defs();
    let headers = defs.iter().find(|d| d.namespaced_name == "http_headers").unwrap();
    let ann = headers.annotations.as_ref().unwrap();
    assert!(ann.read_only_hint, "http_headers should be read-only");
    assert!(!ann.destructive_hint, "http_headers should not be destructive");
    assert!(ann.idempotent_hint, "http_headers should be idempotent");
    assert!(ann.open_world_hint, "http_headers should be open-world");
}

#[test]
fn is_builtin_tool_recognizes_all_three() {
    assert!(is_builtin_tool("fetch_http"));
    assert!(is_builtin_tool("grep_http"));
    assert!(is_builtin_tool("http_headers"));
}

#[test]
fn is_builtin_tool_rejects_unknown() {
    assert!(!is_builtin_tool("unknown_tool"));
    assert!(!is_builtin_tool("builtin__fetch_http"));
    assert!(!is_builtin_tool(""));
}

#[test]
fn builtin_http_security_allows_when_no_rule_matches() {
    let rules = default_dev_security_rules();
    let result = evaluate_builtin_http_request("https://github.com/foo/bar", "GET", &rules, &BTreeMap::new());
    assert!(result.is_ok());
    assert_eq!(result.unwrap().domain, "github.com");
}

#[test]
fn builtin_http_security_blocks_matching_rule() {
    let rules = default_dev_security_rules();
    let result = evaluate_builtin_http_request("https://evil-unknown-domain.xyz/hack", "GET", &rules, &BTreeMap::new());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("blocked"));
}

#[test]
fn builtin_http_security_rejects_invalid_url() {
    let rules = default_dev_security_rules();
    let result = evaluate_builtin_http_request("not a url at all", "GET", &rules, &BTreeMap::new());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("invalid URL"));
}

#[test]
fn extract_text_simple_bold() {
    let text = extract_text_from_html("Hello <b>World</b>");
    assert_eq!(text, "Hello World");
}

#[test]
fn extract_text_block_elements_produce_newlines() {
    let text = extract_text_from_html("<div>A</div><div>B</div>");
    assert!(text.contains("A\nB"), "got: {text:?}");
}

#[test]
fn extract_text_scripts_dropped() {
    let text = extract_text_from_html("<script>alert(1);</script>Text");
    assert_eq!(text, "Text");
}

#[test]
fn extract_text_style_dropped() {
    let text = extract_text_from_html("<style>.foo { color: red; }</style>Visible");
    assert_eq!(text, "Visible");
}

#[test]
fn collapse_whitespace_basic() {
    let result = collapse_whitespace("  Lots   of   space  \n\n\n\n");
    assert_eq!(result, "Lots of space");
}

#[test]
fn collapse_whitespace_preserves_single_newlines() {
    let result = collapse_whitespace("Line 1\nLine 2\nLine 3");
    assert_eq!(result, "Line 1\nLine 2\nLine 3");
}

#[test]
fn paginate_basic() {
    let text = "Hello, world!";
    let (chunk, total, has_more) = paginate(text, 0, 5);
    assert_eq!(chunk, "Hello");
    assert_eq!(total, 13);
    assert!(has_more);
}

#[test]
fn paginate_full_content() {
    let text = "Short";
    let (chunk, total, has_more) = paginate(text, 0, 50000);
    assert_eq!(chunk, "Short");
    assert_eq!(total, 5);
    assert!(!has_more);
}

#[test]
fn paginate_past_end() {
    let text = "ABC";
    let (chunk, total, has_more) = paginate(text, 100, 50000);
    assert_eq!(chunk, "");
    assert_eq!(total, 3);
    assert!(!has_more);
}

#[test]
fn paginate_continuation() {
    let text = "0123456789";
    let (chunk1, _, more1) = paginate(text, 0, 5);
    assert_eq!(chunk1, "01234");
    assert!(more1);
    let (chunk2, _, more2) = paginate(text, 5, 5);
    assert_eq!(chunk2, "56789");
    assert!(!more2);
}

#[tokio::test]
async fn call_unknown_builtin_returns_error() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "nonexistent",
        &serde_json::json!({}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("unknown builtin tool"));
}

#[tokio::test]
async fn fetch_http_missing_url_returns_error() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(resp.error.is_none()); // tool errors use isError in result, not JSON-RPC error
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("missing required parameter"));
}

#[tokio::test]
async fn fetch_http_blocked_domain() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({"url": "https://evil-unknown-domain.xyz/"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
    assert!(result["content"][0]["text"].as_str().unwrap().contains("blocked"));
}

#[tokio::test]
async fn grep_http_missing_pattern_returns_error() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "grep_http",
        &serde_json::json!({"url": "https://example.com"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("missing required parameter"));
}

#[tokio::test]
async fn grep_http_invalid_regex() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "grep_http",
        &serde_json::json!({"url": "https://github.com", "pattern": "[invalid"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
    assert!(result["content"][0]["text"].as_str().unwrap().contains("invalid regex"));
}

// -----------------------------------------------------------------------
// is_binary_content_type unit tests
// -----------------------------------------------------------------------

#[test]
fn binary_ct_image_png() {
    assert!(is_binary_content_type("image/png"));
}

#[test]
fn binary_ct_with_params() {
    assert!(is_binary_content_type("image/jpeg; charset=utf-8"));
}

#[test]
fn binary_ct_application_pdf() {
    assert!(is_binary_content_type("application/pdf"));
}

#[test]
fn binary_ct_audio() {
    assert!(is_binary_content_type("audio/mpeg"));
}

#[test]
fn binary_ct_video() {
    assert!(is_binary_content_type("video/mp4"));
}

#[test]
fn binary_ct_font() {
    assert!(is_binary_content_type("font/woff2"));
}

#[test]
fn binary_ct_octet_stream() {
    assert!(is_binary_content_type("application/octet-stream"));
}

#[test]
fn binary_ct_wasm() {
    assert!(is_binary_content_type("application/wasm"));
}

#[test]
fn text_ct_html() {
    assert!(!is_binary_content_type("text/html"));
}

#[test]
fn text_ct_json() {
    assert!(!is_binary_content_type("application/json"));
}

#[test]
fn text_ct_xml() {
    assert!(!is_binary_content_type("application/xml"));
}

#[test]
fn text_ct_javascript() {
    assert!(!is_binary_content_type("application/javascript"));
}

#[test]
fn text_ct_empty() {
    assert!(!is_binary_content_type(""));
}

// -----------------------------------------------------------------------
// Built-in HTTP security boundary scheme rejection tests
// -----------------------------------------------------------------------

#[test]
fn builtin_http_security_rejects_ftp() {
    let rules = default_dev_security_rules();
    let result = evaluate_builtin_http_request("ftp://example.com/file", "GET", &rules, &BTreeMap::new());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("only http"));
}

#[test]
fn builtin_http_security_rejects_file() {
    let rules = default_dev_security_rules();
    let result = evaluate_builtin_http_request("file:///etc/passwd", "GET", &rules, &BTreeMap::new());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("only http"));
}

#[test]
fn builtin_http_security_rejects_data_uri() {
    let rules = default_dev_security_rules();
    let result = evaluate_builtin_http_request("data:text/html,<h1>hi</h1>", "GET", &rules, &BTreeMap::new());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("only http"));
}

#[test]
fn builtin_http_security_rejects_javascript() {
    let rules = default_dev_security_rules();
    let result = evaluate_builtin_http_request("javascript:alert(1)", "GET", &rules, &BTreeMap::new());
    assert!(result.is_err());
    // reqwest::Url::parse may reject this as invalid, either way it errors
    assert!(result.is_err());
}

#[test]
fn builtin_http_security_rejects_empty_url() {
    let rules = default_dev_security_rules();
    let result = evaluate_builtin_http_request("", "GET", &rules, &BTreeMap::new());
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// extract_text_from_html edge cases
// -----------------------------------------------------------------------

#[test]
fn extract_text_empty_input() {
    assert_eq!(extract_text_from_html(""), "");
}

#[test]
fn extract_text_plain_text_no_tags() {
    assert_eq!(extract_text_from_html("just plain text"), "just plain text");
}

#[test]
fn extract_text_json_content() {
    let text = extract_text_from_html(r#"{"key":"value"}"#);
    assert!(text.contains("key"), "JSON keys preserved: {text:?}");
    assert!(text.contains("value"), "JSON values preserved: {text:?}");
}

#[test]
fn extract_text_svg_only_returns_empty() {
    let text = extract_text_from_html("<svg><text>hello</text></svg>");
    assert_eq!(text, "");
}

#[test]
fn extract_text_noscript_skipped() {
    let text = extract_text_from_html("<noscript>hidden</noscript>visible");
    assert!(text.contains("visible"), "visible text preserved: {text:?}");
    assert!(!text.contains("hidden"), "noscript content skipped: {text:?}");
}

#[test]
fn extract_text_template_skipped() {
    let text = extract_text_from_html("<template><p>hidden</p></template>visible");
    assert!(text.contains("visible"), "visible text preserved: {text:?}");
    assert!(!text.contains("hidden"), "template content skipped: {text:?}");
}

#[test]
fn extract_text_html_entities_preserved() {
    // tl parser preserves raw text nodes including HTML entities
    let text = extract_text_from_html("&amp; &lt; &gt;");
    // The raw entity strings are preserved in the output
    assert!(!text.is_empty(), "non-empty output: {text:?}");
}

#[test]
fn extract_text_nested_scripts_in_divs() {
    let text = extract_text_from_html("<div><script>evil()</script>Good</div>");
    assert!(text.contains("Good"), "visible text kept: {text:?}");
    assert!(!text.contains("evil"), "script content dropped: {text:?}");
}

#[test]
fn extract_text_multiple_skip_tags() {
    let html = concat!(
        "<script>js()</script>",
        "<style>.x{}</style>",
        "<noscript>no</noscript>",
        "<svg><text>svg</text></svg>",
        "Visible content"
    );
    let text = extract_text_from_html(html);
    assert_eq!(text, "Visible content");
}

// -----------------------------------------------------------------------
// paginate edge cases
// -----------------------------------------------------------------------

#[test]
fn paginate_max_zero() {
    let (chunk, total, has_more) = paginate("Hello", 0, 0);
    assert_eq!(chunk, "");
    assert_eq!(total, 5);
    assert!(has_more);
}

#[test]
fn paginate_start_at_exact_end() {
    let (chunk, total, has_more) = paginate("ABC", 3, 100);
    assert_eq!(chunk, "");
    assert_eq!(total, 3);
    assert!(!has_more);
}

// -----------------------------------------------------------------------
// fetch_http edge cases (async)
// -----------------------------------------------------------------------

#[tokio::test]
async fn fetch_http_rejects_ftp_scheme() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({"url": "ftp://example.com/file"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp));
    let text = extract_tool_text(&resp);
    assert!(text.contains("only http"), "error should mention http: {text}");
}

#[tokio::test]
async fn fetch_http_rejects_file_scheme() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({"url": "file:///etc/passwd"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp));
    let text = extract_tool_text(&resp);
    assert!(text.contains("only http"), "error should mention http: {text}");
}

#[tokio::test]
async fn fetch_http_rejects_data_uri() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({"url": "data:text/plain,hello"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp));
}

#[tokio::test]
async fn fetch_http_url_is_number_not_string() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({"url": 42}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp));
    let text = extract_tool_text(&resp);
    assert!(text.contains("missing required parameter"), "got: {text}");
}

#[tokio::test]
async fn fetch_http_url_is_null() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({"url": null}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp));
    let text = extract_tool_text(&resp);
    assert!(text.contains("missing required parameter"), "got: {text}");
}

#[tokio::test]
async fn fetch_http_start_index_negative_defaults_to_zero() {
    // as_u64() returns None for -1, so it should default to 0
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let url = format!("{}/", fixture.base_url);
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({
            "url": url,
            "start_index": -1
        }),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    // Should succeed (negative start_index is silently treated as 0)
    assert!(!is_tool_error(&resp), "should succeed with default start_index=0");
    let text = extract_tool_text(&resp);
    assert!(text.contains(&format!("URL: {}/", fixture.base_url)), "got: {text}");
}

// -----------------------------------------------------------------------
// grep_http edge cases (async)
// -----------------------------------------------------------------------

#[tokio::test]
async fn grep_http_empty_pattern_rejected() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "grep_http",
        &serde_json::json!({"url": "https://github.com", "pattern": ""}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp));
    let text = extract_tool_text(&resp);
    assert!(text.contains("must not be empty"), "got: {text}");
}

#[tokio::test]
async fn grep_http_missing_url_returns_error() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "grep_http",
        &serde_json::json!({"pattern": "test"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp));
    let text = extract_tool_text(&resp);
    assert!(text.contains("missing required parameter"), "got: {text}");
}

#[tokio::test]
async fn grep_http_url_is_number() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "grep_http",
        &serde_json::json!({"url": 123, "pattern": "test"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp));
    let text = extract_tool_text(&resp);
    assert!(text.contains("missing required parameter"), "got: {text}");
}

#[tokio::test]
async fn grep_http_rejects_ftp_scheme() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "grep_http",
        &serde_json::json!({"url": "ftp://example.com", "pattern": "test"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp));
    let text = extract_tool_text(&resp);
    assert!(text.contains("only http"), "got: {text}");
}

#[tokio::test]
async fn grep_http_regex_catastrophic_backtracking_safe() {
    // Rust regex crate uses finite automaton, no catastrophic backtracking.
    // This test ensures (a+)+$ doesn't hang on an allowed domain.
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "grep_http",
        &serde_json::json!({
            "url": format!("{}/", fixture.base_url),
            "pattern": "(a+)+$"
        }),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    // Should complete without hanging (pass or no matches, either is fine)
    assert!(
        !is_tool_error(&resp),
        "should not error: {:?}",
        extract_tool_text(&resp)
    );
}

// -----------------------------------------------------------------------
// http_headers edge cases (async)
// -----------------------------------------------------------------------

#[tokio::test]
async fn http_headers_missing_url() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "http_headers",
        &serde_json::json!({}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp));
    let text = extract_tool_text(&resp);
    assert!(text.contains("missing required parameter"), "got: {text}");
}

#[tokio::test]
async fn http_headers_rejects_ftp_scheme() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "http_headers",
        &serde_json::json!({"url": "ftp://example.com"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp));
    let text = extract_tool_text(&resp);
    assert!(text.contains("only http"), "got: {text}");
}

#[tokio::test]
async fn http_headers_invalid_method_falls_back_to_head() {
    // Any method other than "GET" falls through to HEAD
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "http_headers",
        &serde_json::json!({"url": format!("{}/", fixture.base_url), "method": "POST"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    // Should succeed with HEAD fallback
    assert!(!is_tool_error(&resp), "should succeed with HEAD fallback");
    let text = extract_tool_text(&resp);
    assert!(text.contains("Status:"), "got: {text}");
    assert_eq!(fixture.state.requests()[0].method, http::Method::HEAD);
}

#[tokio::test]
async fn http_headers_method_case_sensitive() {
    // "get" (lowercase) is not "GET", so falls through to HEAD
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "http_headers",
        &serde_json::json!({"url": format!("{}/", fixture.base_url), "method": "get"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(!is_tool_error(&resp), "should succeed with HEAD fallback");
    assert_eq!(fixture.state.requests()[0].method, http::Method::HEAD);
}

#[test]
fn collect_grep_matches_reports_true_total_and_caps_shown_blocks() {
    let re = regex::Regex::new("hit").unwrap();
    let lines = ["hit", "x", "hit", "hit", "y", "hit", "hit"]; // 5 matches
    let refs: Vec<&str> = lines.to_vec();

    let (blocks, total) = collect_grep_matches(&refs, &re, 0, 2);

    // The count must be the true number of matches (5), not max_matches+1 (3),
    // which is what the old early-break left in the counter.
    assert_eq!(total, 5, "reported total must count all matches, not stop at the cap");
    assert_eq!(blocks.len(), 2, "only the first max_matches blocks are built");
    assert!(blocks[0].contains(">>> 1: hit"));
    assert!(blocks[1].contains(">>> 3: hit"));
}

#[test]
fn collect_grep_matches_below_cap_returns_all() {
    let re = regex::Regex::new("hit").unwrap();
    let refs = vec!["hit", "no", "hit"];
    let (blocks, total) = collect_grep_matches(&refs, &re, 0, 10);
    assert_eq!(total, 2);
    assert_eq!(blocks.len(), 2);
}

// -- capped HTTP body reads --

#[tokio::test]
async fn read_body_capped_truncates_oversized_body() {
    let recorder = crate::test_support::http::spawn_static_http_recorder(vec![(
        "/",
        crate::test_support::http::RecordedHttpResponse::text("a".repeat(5000)),
    )])
    .await
    .unwrap();
    let resp = test_client()
        .get(format!("{}/", recorder.base_url))
        .send()
        .await
        .unwrap();
    let body = read_body_capped(resp, 1000).await.unwrap();
    assert_eq!(
        body.len(),
        1000,
        "an oversized response body must be truncated at the cap"
    );
}

#[tokio::test]
async fn read_body_capped_returns_full_body_under_cap() {
    let recorder = crate::test_support::http::spawn_static_http_recorder(vec![(
        "/",
        crate::test_support::http::RecordedHttpResponse::text("hello world"),
    )])
    .await
    .unwrap();
    let resp = test_client()
        .get(format!("{}/", recorder.base_url))
        .send()
        .await
        .unwrap();
    let body = read_body_capped(resp, 1_000_000).await.unwrap();
    assert_eq!(body, "hello world");
}

// Shared fixture helpers for this module and its submodules.

/// Helper to extract the text content from a tool response.
fn extract_tool_text(resp: &JsonRpcResponse) -> &str {
    resp.result.as_ref().unwrap()["content"][0]["text"].as_str().unwrap()
}

fn is_tool_error(resp: &JsonRpcResponse) -> bool {
    resp.result.as_ref().map(|r| r["isError"] == true).unwrap_or(false)
}

fn load_fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/mcp/html/{name}",
        env!("CARGO_MANIFEST_DIR").replace("/crates/capsem-core", "")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to load fixture {path}: {e}"))
}
