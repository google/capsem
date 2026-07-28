use super::*;

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
    .and_then(|profile| {
        SecurityRuleSet::compile_profile(
            &profile,
            crate::net::policy_config::SecurityRuleSource::User,
        )
    })
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
    let fetch = defs
        .iter()
        .find(|d| d.namespaced_name == "fetch_http")
        .unwrap();
    let ann = fetch.annotations.as_ref().unwrap();
    assert!(ann.read_only_hint, "fetch_http should be read-only");
    assert!(
        !ann.destructive_hint,
        "fetch_http should not be destructive"
    );
    assert!(ann.idempotent_hint, "fetch_http should be idempotent");
    assert!(ann.open_world_hint, "fetch_http should be open-world");
}

#[test]
fn grep_http_annotations_correct() {
    let defs = builtin_tool_defs();
    let grep = defs
        .iter()
        .find(|d| d.namespaced_name == "grep_http")
        .unwrap();
    let ann = grep.annotations.as_ref().unwrap();
    assert!(ann.read_only_hint, "grep_http should be read-only");
    assert!(!ann.destructive_hint, "grep_http should not be destructive");
    assert!(ann.idempotent_hint, "grep_http should be idempotent");
    assert!(ann.open_world_hint, "grep_http should be open-world");
}

#[test]
fn http_headers_annotations_correct() {
    let defs = builtin_tool_defs();
    let headers = defs
        .iter()
        .find(|d| d.namespaced_name == "http_headers")
        .unwrap();
    let ann = headers.annotations.as_ref().unwrap();
    assert!(ann.read_only_hint, "http_headers should be read-only");
    assert!(
        !ann.destructive_hint,
        "http_headers should not be destructive"
    );
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
    let result = evaluate_builtin_http_request(
        "https://github.com/foo/bar",
        "GET",
        &rules,
        &BTreeMap::new(),
    );
    assert!(result.is_ok());
    assert_eq!(result.unwrap().domain, "github.com");
}

#[test]
fn builtin_http_security_blocks_matching_rule() {
    let rules = default_dev_security_rules();
    let result = evaluate_builtin_http_request(
        "https://evil-unknown-domain.xyz/hack",
        "GET",
        &rules,
        &BTreeMap::new(),
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("blocked"));
}

#[test]
fn builtin_http_security_rejects_invalid_url() {
    let rules = default_dev_security_rules();
    let result =
        evaluate_builtin_http_request("not a url at all", "GET", &rules, &BTreeMap::new());
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
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("blocked"));
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
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("invalid regex"));
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
    let result = evaluate_builtin_http_request(
        "ftp://example.com/file",
        "GET",
        &rules,
        &BTreeMap::new(),
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("only http"));
}

#[test]
fn builtin_http_security_rejects_file() {
    let rules = default_dev_security_rules();
    let result =
        evaluate_builtin_http_request("file:///etc/passwd", "GET", &rules, &BTreeMap::new());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("only http"));
}

#[test]
fn builtin_http_security_rejects_data_uri() {
    let rules = default_dev_security_rules();
    let result = evaluate_builtin_http_request(
        "data:text/html,<h1>hi</h1>",
        "GET",
        &rules,
        &BTreeMap::new(),
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("only http"));
}

#[test]
fn builtin_http_security_rejects_javascript() {
    let rules = default_dev_security_rules();
    let result =
        evaluate_builtin_http_request("javascript:alert(1)", "GET", &rules, &BTreeMap::new());
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
    assert!(
        !text.contains("hidden"),
        "noscript content skipped: {text:?}"
    );
}

#[test]
fn extract_text_template_skipped() {
    let text = extract_text_from_html("<template><p>hidden</p></template>visible");
    assert!(text.contains("visible"), "visible text preserved: {text:?}");
    assert!(
        !text.contains("hidden"),
        "template content skipped: {text:?}"
    );
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
    assert!(
        text.contains("only http"),
        "error should mention http: {text}"
    );
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
    assert!(
        text.contains("only http"),
        "error should mention http: {text}"
    );
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
    assert!(
        !is_tool_error(&resp),
        "should succeed with default start_index=0"
    );
    let text = extract_tool_text(&resp);
    assert!(
        text.contains(&format!("URL: {}/", fixture.base_url)),
        "got: {text}"
    );
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

// -----------------------------------------------------------------------
// Realistic HTML extraction tests
// -----------------------------------------------------------------------

#[test]
fn extract_text_full_html_document() {
    // Realistic full HTML page like a real website would serve
    let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>Elie Bursztein - Security Research</title>
    <script>window.analytics = {};</script>
    <style>body { font-family: sans-serif; }</style>
</head>
<body>
    <nav><a href="/">Home</a> <a href="/about">About</a></nav>
    <main>
        <h1>Elie Bursztein</h1>
        <p>Google &amp; DeepMind AI Cybersecurity technical and research lead.</p>
        <div class="bio">
            <h2>About</h2>
            <p>Elie works on AI security and has published over 100 papers.</p>
        </div>
        <section>
            <h2>Recent Publications</h2>
            <ul>
                <li>Paper on cryptographic compliance testing</li>
                <li>AI safety research findings</li>
            </ul>
        </section>
    </main>
    <footer><p>Copyright 2024</p></footer>
</body>
</html>"#;
    let text = extract_text_from_html(html);
    // Must contain key content from the page
    assert!(
        text.contains("Elie Bursztein"),
        "extracted text must contain 'Elie Bursztein', got: {text:?}"
    );
    assert!(
        text.contains("About"),
        "extracted text must contain 'About', got: {text:?}"
    );
    assert!(
        text.contains("Google"),
        "extracted text must contain 'Google', got: {text:?}"
    );
    assert!(
        text.contains("AI security"),
        "extracted text must contain 'AI security', got: {text:?}"
    );
    assert!(
        text.contains("cryptographic"),
        "extracted text must contain 'cryptographic', got: {text:?}"
    );
    // Must NOT contain script/style content
    assert!(
        !text.contains("analytics"),
        "extracted text must not contain script content"
    );
    assert!(
        !text.contains("font-family"),
        "extracted text must not contain style content"
    );
}

#[test]
fn extract_text_handles_nested_elements() {
    let html = r#"<html><body>
<div class="card">
    <span class="name">Alice</span>
    <span class="role">Engineer</span>
</div>
<div class="card">
    <span class="name">Bob</span>
    <span class="role">Designer</span>
</div>
</body></html>"#;
    let text = extract_text_from_html(html);
    assert!(text.contains("Alice"), "must contain Alice, got: {text:?}");
    assert!(text.contains("Bob"), "must contain Bob, got: {text:?}");
    assert!(
        text.contains("Engineer"),
        "must contain Engineer, got: {text:?}"
    );
}

#[test]
fn extract_text_handles_links_and_attrs() {
    let html = r#"<html><body>
<a href="/about">About page</a>
<a href="https://example.com" class="external">Visit Example</a>
<img src="photo.jpg" alt="Photo of labs">
</body></html>"#;
    let text = extract_text_from_html(html);
    assert!(
        text.contains("About page"),
        "must contain link text, got: {text:?}"
    );
    assert!(
        text.contains("Visit Example"),
        "must contain link text, got: {text:?}"
    );
}

// -----------------------------------------------------------------------
// Integration tests -- use local HTTP fixtures only
// -----------------------------------------------------------------------

/// Helper to extract the text content from a tool response.
fn extract_tool_text(resp: &JsonRpcResponse) -> &str {
    resp.result.as_ref().unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
}

fn is_tool_error(resp: &JsonRpcResponse) -> bool {
    resp.result
        .as_ref()
        .map(|r| r["isError"] == true)
        .unwrap_or(false)
}

#[tokio::test]
async fn integration_fetch_http_local_fixture() {
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let url = format!("{}/", fixture.base_url);
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({"url": url}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(!is_tool_error(&resp), "fetch should succeed");
    let text = extract_tool_text(&resp);
    assert!(
        text.contains(&fixture.base_url),
        "response must reference the domain"
    );
    // The extracted content must contain real text from the page
    assert!(
        text.to_lowercase().contains("elie"),
        "page content must contain 'elie': {text}"
    );
}

#[tokio::test]
async fn integration_grep_http_local_fixture_finds_matches() {
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "grep_http",
        &serde_json::json!({"url": format!("{}/", fixture.base_url), "pattern": "elie"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(!is_tool_error(&resp), "grep should succeed");
    let text = extract_tool_text(&resp);
    // Must NOT say "Matches found: 0"
    assert!(
        !text.contains("Matches found: 0"),
        "grep_http must find 'elie' on the local fixture but got 0 matches: {text}"
    );
    assert!(
        text.contains("Match 1"),
        "grep_http must have at least one match block: {text}"
    );
}

#[tokio::test]
async fn integration_grep_http_blocked_domain() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "grep_http",
        &serde_json::json!({
            "url": "https://evil-unknown-domain.xyz",
            "pattern": "test"
        }),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp), "blocked domain must return isError");
    let text = extract_tool_text(&resp);
    assert!(
        text.to_lowercase().contains("blocked"),
        "error must mention 'blocked': {text}"
    );
}

#[tokio::test]
async fn integration_http_headers_local_fixture() {
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "http_headers",
        &serde_json::json!({"url": format!("{}/", fixture.base_url)}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(!is_tool_error(&resp), "http_headers should succeed");
    let text = extract_tool_text(&resp);
    assert!(
        text.contains("Status: 200"),
        "must return a valid HTTP status: {text}"
    );
    assert!(
        text.to_lowercase().contains("content-type"),
        "must include content-type header: {text}"
    );
    assert_eq!(fixture.state.requests()[0].method, http::Method::HEAD);
}

#[tokio::test]
async fn integration_fetch_http_blocked_domain() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({"url": "https://evil-unknown-domain.xyz"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp), "blocked domain must return isError");
    let text = extract_tool_text(&resp);
    assert!(
        text.to_lowercase().contains("blocked"),
        "error must mention 'blocked': {text}"
    );
}

#[tokio::test]
async fn integration_http_headers_blocked_domain() {
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "http_headers",
        &serde_json::json!({"url": "https://evil-unknown-domain.xyz"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(is_tool_error(&resp), "blocked domain must return isError");
    let text = extract_tool_text(&resp);
    assert!(
        text.to_lowercase().contains("blocked"),
        "error must mention 'blocked': {text}"
    );
}

// -----------------------------------------------------------------------
// Fixture-based HTML extraction tests
// -----------------------------------------------------------------------

fn load_fixture(name: &str) -> String {
    let path = format!(
        "{}/data/fixtures/html/{name}",
        env!("CARGO_MANIFEST_DIR").replace("/crates/capsem-core", "")
    );
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to load fixture {path}: {e}"))
}

#[test]
fn extract_elie_about_has_real_content() {
    let html = load_fixture("elie_about.html");
    let text = extract_text_from_html(&html);
    assert!(
        text.contains("Bursztein"),
        "must contain 'Bursztein': {}",
        &text[..200.min(text.len())]
    );
    assert!(text.contains("Google"), "must contain 'Google'");
    assert!(
        text.to_lowercase().contains("security"),
        "must contain 'security'"
    );
    assert!(text.contains("Stanford"), "must contain 'Stanford'");
    assert!(
        text.len() > 3000,
        "extracted text too short: {} chars",
        text.len()
    );
    assert!(!text.contains("<script"), "must not contain script tags");
    assert!(!text.contains("<style"), "must not contain style tags");
    assert!(!text.contains("function()"), "must not contain JS code");
}

#[test]
fn extract_wiki_turing_has_real_content() {
    let html = load_fixture("wiki_turing_excerpt.html");
    let text = extract_text_from_html(&html);
    assert!(
        text.contains("Turing"),
        "must contain 'Turing': {}",
        &text[..200.min(text.len())]
    );
    assert!(!text.contains("<script"), "no script leakage");
    assert!(!text.contains("<style"), "no style leakage");
}

#[test]
fn extract_wiki_rust_has_real_content() {
    let html = load_fixture("wiki_rust_excerpt.html");
    let text = extract_text_from_html(&html);
    assert!(
        text.contains("Rust"),
        "must contain 'Rust': {}",
        &text[..200.min(text.len())]
    );
    assert!(!text.contains("<script"), "no script leakage");
}

#[test]
fn extract_wiki_unicode_preserves_multibyte() {
    let html = load_fixture("wiki_unicode_excerpt.html");
    let text = extract_text_from_html(&html);
    // Fixture is from the middle of the article, may or may not have "Unicode"
    // but must have valid UTF-8 with multi-byte chars
    assert!(text.is_char_boundary(0), "valid UTF-8 start");
    assert!(text.is_char_boundary(text.len()), "valid UTF-8 end");
    let multibyte_count = text.chars().filter(|c| c.len_utf8() > 1).count();
    assert!(multibyte_count > 0, "must contain multi-byte chars, got 0");
    assert!(!text.contains("<script"), "no script leakage");
}

// -----------------------------------------------------------------------
// Fixture-based paginate tests (UTF-8 edge cases)
// -----------------------------------------------------------------------

#[test]
fn paginate_multibyte_emoji_boundary() {
    // Emoji are 4-byte UTF-8
    let text = "Hello \u{1F600} World"; // "Hello [grinning face] World"
                                        // emoji starts at byte 6 ("Hello " = 6 bytes)
                                        // Set max to land mid-emoji (byte 7 or 8)
    let (chunk, _total, has_more) = paginate(text, 0, 7);
    assert!(has_more, "should have more content");
    // chunk must end at a valid char boundary
    assert!(
        chunk.is_char_boundary(chunk.len()),
        "chunk must end at char boundary"
    );
    // Should include "Hello " but not the emoji (can't fit 4 bytes after byte 6)
    assert_eq!(chunk, "Hello ", "should stop before emoji: {chunk:?}");
}

#[test]
fn paginate_multibyte_cyrillic() {
    // Cyrillic chars are 2-byte UTF-8
    let text = "\u{041F}\u{0440}\u{0438}\u{0432}\u{0435}\u{0442}"; // "Privet" in Cyrillic
    assert_eq!(text.len(), 12); // 6 chars * 2 bytes each
                                // Start at byte 1 (mid-char) -- should align to byte 0
    let (chunk, _total, _) = paginate(text, 1, 100);
    assert!(!chunk.is_empty(), "should produce content");
    // Start at byte 3 (mid-char) -- should align to byte 2
    let (chunk, _, _) = paginate(text, 3, 4);
    assert!(
        chunk.is_char_boundary(0),
        "chunk start must be char boundary"
    );
    assert!(
        chunk.is_char_boundary(chunk.len()),
        "chunk end must be char boundary"
    );
}

#[test]
fn paginate_start_index_mid_char() {
    // 3-byte UTF-8 char: euro sign
    let text = "A\u{20AC}B"; // "A[euro]B" = 1 + 3 + 1 = 5 bytes
                             // start_index=2 is mid-euro-sign
    let (chunk, _, _) = paginate(text, 2, 100);
    // Should align to byte 1 (start of euro) or byte 4 (after euro)
    // floor_char_boundary(2) on "A\u{20AC}B" -> byte 1 (start of euro sign)
    assert!(
        chunk.contains('\u{20AC}') || chunk.contains('B'),
        "mid-char start should align to valid boundary: {chunk:?}"
    );
}

#[test]
fn paginate_real_wiki_unicode_content() {
    let html = load_fixture("wiki_unicode_excerpt.html");
    let text = extract_text_from_html(&html);
    // Paginate in small chunks to guarantee multi-byte boundary hits
    let mut collected = String::new();
    let mut offset = 0;
    let chunk_size = 100;
    loop {
        let (chunk, _total, has_more) = paginate(&text, offset, chunk_size);
        collected.push_str(&chunk);
        if !has_more {
            break;
        }
        offset += chunk.len();
    }
    assert_eq!(
        collected, text,
        "round-trip pagination must reconstruct original text"
    );
}

#[test]
fn paginate_continuation_round_trip() {
    // Mixed ASCII + multi-byte text
    let text = "Hello \u{041F}\u{0440}\u{0438}\u{0432}\u{0435}\u{0442} World \u{1F600} end";
    let mut collected = String::new();
    let mut offset = 0;
    let chunk_size = 5; // very small to hit many boundaries
    loop {
        let (chunk, _total, has_more) = paginate(text, offset, chunk_size);
        collected.push_str(&chunk);
        if !has_more {
            break;
        }
        offset += chunk.len();
    }
    assert_eq!(
        collected, text,
        "round-trip must match: {collected:?} vs {text:?}"
    );
}

// -----------------------------------------------------------------------
// Fixture-based grep tests
// -----------------------------------------------------------------------

#[test]
fn grep_elie_about_finds_bursztein() {
    let html = load_fixture("elie_about.html");
    let text = extract_text_from_html(&html);
    let count = text.matches("Bursztein").count();
    assert!(count > 0, "must find 'Bursztein' in extracted text");
    // Cross-check with regex (same as grep_http uses)
    let re = regex::Regex::new("(?i)Bursztein").unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let line_matches = lines.iter().filter(|l| re.is_match(l)).count();
    assert!(line_matches > 0, "regex must find matches too");
}

#[test]
fn grep_wiki_turing_finds_turing() {
    let html = load_fixture("wiki_turing_excerpt.html");
    let text = extract_text_from_html(&html);
    let count = text.matches("Turing").count();
    assert!(count > 0, "must find 'Turing' in extracted text, got 0");
}

#[test]
fn grep_wiki_unicode_finds_pattern() {
    let html = load_fixture("wiki_unicode_excerpt.html");
    let text = extract_text_from_html(&html);
    // The fixture is from the middle, so look for any content
    assert!(!text.is_empty(), "extracted text must not be empty");
    // Test regex mode on the extracted text
    let re = regex::Regex::new(r"\w+").unwrap();
    let match_count = text.lines().filter(|l| re.is_match(l)).count();
    assert!(match_count > 0, "must find word-char matches");
}

// -----------------------------------------------------------------------
// Fixture-based raw mode tests
// -----------------------------------------------------------------------

#[test]
fn raw_vs_content_mode_differ() {
    let html = load_fixture("elie_about.html");
    let content_mode = extract_text_from_html(&html);
    let raw_mode = &html; // raw returns the HTML as-is
                          // Raw is longer (has all HTML tags)
    assert!(
        raw_mode.len() > content_mode.len(),
        "raw ({}) should be longer than content ({})",
        raw_mode.len(),
        content_mode.len()
    );
    // Content mode has no HTML tags
    assert!(
        !content_mode.contains("<script"),
        "content mode must strip scripts"
    );
    assert!(
        !content_mode.contains("<div"),
        "content mode must strip div tags"
    );
    // Raw mode has HTML tags
    assert!(
        raw_mode.contains("<script") || raw_mode.contains("<div"),
        "raw mode should preserve HTML tags"
    );
}

#[test]
fn raw_mode_paginate_works_on_html() {
    let html = load_fixture("elie_about.html");
    let (chunk, total, has_more) = paginate(&html, 0, 5000);
    assert!(has_more, "190KB HTML should need pagination at 5KB");
    assert_eq!(total, html.len());
    assert!(chunk.len() <= 5000, "chunk must respect max_length");
    // Round-trip
    let mut collected = String::new();
    let mut offset = 0;
    loop {
        let (c, _, more) = paginate(&html, offset, 10000);
        collected.push_str(&c);
        if !more {
            break;
        }
        offset += c.len();
    }
    assert_eq!(collected, html, "raw HTML pagination round-trip must match");
}

// -----------------------------------------------------------------------
// Fixture-based markdown extraction tests
// -----------------------------------------------------------------------

#[test]
fn markdown_elie_about_has_structure() {
    let html = load_fixture("elie_about.html");
    let md = extract_markdown_from_html(&html);
    // Must contain key content
    assert!(md.contains("Bursztein"), "must contain 'Bursztein'");
    assert!(md.contains("Google"), "must contain 'Google'");
    // Must have markdown headings
    assert!(
        md.contains("# ") || md.contains("## "),
        "must have markdown headings"
    );
    // Must have markdown links
    assert!(md.contains("]("), "must have markdown links [text](url)");
    // Must NOT contain script/style content
    assert!(!md.contains("<script"), "must not contain script tags");
    assert!(!md.contains("<style"), "must not contain style tags");
}

#[test]
fn markdown_preserves_headings() {
    let html = "<h1>Title</h1><h2>Subtitle</h2><p>Body text</p>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("# Title"), "h1 -> '# Title', got: {md:?}");
    assert!(
        md.contains("## Subtitle"),
        "h2 -> '## Subtitle', got: {md:?}"
    );
    assert!(md.contains("Body text"), "body preserved");
}

#[test]
fn markdown_preserves_links() {
    let html = r#"<a href="https://example.com">Example</a>"#;
    let md = extract_markdown_from_html(html);
    assert!(
        md.contains("[Example](https://example.com)"),
        "link preserved: {md:?}"
    );
}

#[test]
fn markdown_preserves_bold_italic() {
    let html = "<strong>Bold</strong> and <em>Italic</em>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("**Bold**"), "bold preserved: {md:?}");
    assert!(md.contains("_Italic_"), "italic preserved: {md:?}");
}

#[test]
fn markdown_preserves_lists() {
    let html = "<ul><li>One</li><li>Two</li></ul>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("- One"), "unordered list: {md:?}");
    assert!(md.contains("- Two"), "unordered list: {md:?}");
}

#[test]
fn markdown_preserves_ordered_lists() {
    let html = "<ol><li>First</li><li>Second</li></ol>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("1. First"), "ordered list: {md:?}");
    assert!(md.contains("2. Second"), "ordered list: {md:?}");
}

#[test]
fn markdown_preserves_code() {
    let html = "<code>let x = 1;</code>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("`let x = 1;`"), "inline code: {md:?}");
}

#[test]
fn markdown_preserves_code_blocks() {
    let html = "<pre><code>fn main() {}</code></pre>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("```"), "code block fencing: {md:?}");
    assert!(md.contains("fn main()"), "code block content: {md:?}");
}

#[test]
fn markdown_preserves_blockquotes() {
    let html = "<blockquote>A wise quote</blockquote>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("> A wise quote"), "blockquote: {md:?}");
}

#[test]
fn markdown_vs_content_mode() {
    let html =
        r#"<h1>Title</h1><p>Text with <a href="/link">link</a> and <strong>bold</strong>.</p>"#;
    let md = extract_markdown_from_html(html);
    let text = extract_text_from_html(html);
    // Markdown has structure markers
    assert!(md.contains("# Title"), "markdown has heading marker");
    assert!(md.contains("](/link)"), "markdown has link");
    assert!(md.contains("**bold**"), "markdown has bold");
    // Plain text has no markers
    assert!(!text.contains("# "), "text has no heading markers");
    assert!(!text.contains("]("), "text has no link markers");
    assert!(!text.contains("**"), "text has no bold markers");
    // Both have the actual content
    assert!(text.contains("Title"), "text has title");
    assert!(text.contains("bold"), "text has bold word");
}

#[test]
fn markdown_wiki_turing_has_structure() {
    let html = load_fixture("wiki_turing_excerpt.html");
    let md = extract_markdown_from_html(&html);
    assert!(md.contains("Turing"), "must contain 'Turing'");
    assert!(!md.contains("<script"), "no script leakage");
    // Wikipedia articles have links
    assert!(md.contains("]("), "must have markdown links");
}

// -----------------------------------------------------------------------
// Integration tests -- local /about fixture
// -----------------------------------------------------------------------

#[tokio::test]
async fn integration_fetch_http_local_about() {
    // Default format is markdown
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({"url": format!("{}/about", fixture.base_url)}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(!is_tool_error(&resp), "fetch should succeed");
    let text = extract_tool_text(&resp);
    assert!(
        text.contains("Bursztein"),
        "must contain 'Bursztein': {}",
        &text[..300.min(text.len())]
    );
    assert!(text.contains("Google"), "must contain 'Google'");
    // Default is markdown -- should have structure markers
    assert!(
        text.contains("](") || text.contains("# "),
        "default mode should return markdown with links or headings"
    );
    // Verify substantial content (not just 93 bytes)
    let content_line = text.lines().find(|l| l.starts_with("Content length:"));
    if let Some(cl) = content_line {
        let len: usize = cl
            .trim_start_matches("Content length: ")
            .parse()
            .unwrap_or(0);
        assert!(len > 3000, "content length must be substantial, got {len}");
    }
}

#[tokio::test]
async fn integration_fetch_http_local_about_content_mode() {
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({"url": format!("{}/about", fixture.base_url), "format": "content"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(!is_tool_error(&resp), "fetch content should succeed");
    let text = extract_tool_text(&resp);
    assert!(text.contains("Bursztein"), "must contain 'Bursztein'");
    // Content mode: no markdown markers
    assert!(
        !text.contains("]("),
        "content mode must not have markdown links"
    );
    assert!(
        !text.contains("**"),
        "content mode must not have bold markers"
    );
}

#[tokio::test]
async fn integration_fetch_http_local_about_raw() {
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({"url": format!("{}/about", fixture.base_url), "format": "raw", "max_length": 50000}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(!is_tool_error(&resp), "fetch raw should succeed");
    let text = extract_tool_text(&resp);
    assert!(
        text.contains("<div") || text.contains("<p"),
        "raw mode must preserve HTML tags"
    );
    assert!(text.contains("Bursztein"), "must contain 'Bursztein'");
}

#[tokio::test]
async fn integration_grep_http_local_about() {
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "grep_http",
        &serde_json::json!({"url": format!("{}/about", fixture.base_url), "pattern": "Bursztein"}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(!is_tool_error(&resp), "grep should succeed");
    let text = extract_tool_text(&resp);
    assert!(
        !text.contains("Matches found: 0"),
        "must find matches: {text}"
    );
    assert!(
        text.contains("Match 1"),
        "must have at least one match block"
    );
}

#[tokio::test]
async fn integration_fetch_http_local_about_pagination() {
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({"url": format!("{}/about", fixture.base_url), "max_length": 500}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(!is_tool_error(&resp), "fetch should succeed");
    let text = extract_tool_text(&resp);
    assert!(
        text.contains("start_index="),
        "must have pagination hint for large page"
    );
}

#[tokio::test]
async fn integration_http_headers_local_about() {
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "http_headers",
        &serde_json::json!({"url": format!("{}/about", fixture.base_url)}),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(!is_tool_error(&resp), "http_headers should succeed");
    let text = extract_tool_text(&resp);
    assert!(text.contains("Status: 200"), "must return 200: {text}");
    assert!(
        text.to_lowercase().contains("content-type"),
        "must include content-type"
    );
    assert_eq!(fixture.state.requests()[0].method, http::Method::HEAD);
}

// -----------------------------------------------------------------------
// Integration tests -- local wiki-shaped fixtures
// -----------------------------------------------------------------------

#[tokio::test]
async fn integration_fetch_http_local_wiki_turing() {
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({
            "url": format!("{}/wiki/Alan_Turing", fixture.base_url),
            "max_length": 5000
        }),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(!is_tool_error(&resp), "fetch should succeed");
    let text = extract_tool_text(&resp);
    assert!(text.contains("Turing"), "must contain 'Turing'");
}

#[tokio::test]
async fn integration_grep_http_local_wiki_rust_finds_mozilla() {
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "grep_http",
        &serde_json::json!({
            "url": format!("{}/wiki/Rust_(programming_language)", fixture.base_url),
            "pattern": "Mozilla"
        }),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(!is_tool_error(&resp), "grep should succeed");
    let text = extract_tool_text(&resp);
    assert!(
        !text.contains("Matches found: 0"),
        "must find Mozilla matches"
    );
}

#[tokio::test]
async fn integration_fetch_http_local_wiki_unicode_multibyte() {
    let fixture = spawn_builtin_http_fixture().await;
    let client = test_client();
    let rules = default_dev_security_rules();
    let resp = call_builtin_tool(
        "fetch_http",
        &serde_json::json!({
            "url": format!("{}/wiki/Unicode", fixture.base_url),
            "max_length": 5000
        }),
        &client,
        &rules,
        &BTreeMap::new(),
        Some(serde_json::json!(1)),
        &test_db(),
    )
    .await;
    assert!(
        !is_tool_error(&resp),
        "fetch should succeed (no panic from multi-byte)"
    );
    let text = extract_tool_text(&resp);
    assert!(text.contains("Unicode"), "must contain 'Unicode'");
}
