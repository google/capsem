//! Fixture-backed extraction, pagination, grep, and local-HTTP integration tests.

use super::*;

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
    assert!(text.contains("Engineer"), "must contain Engineer, got: {text:?}");
}

#[test]
fn extract_text_handles_links_and_attrs() {
    let html = r#"<html><body>
<a href="/about">About page</a>
<a href="https://example.com" class="external">Visit Example</a>
<img src="photo.jpg" alt="Photo of labs">
</body></html>"#;
    let text = extract_text_from_html(html);
    assert!(text.contains("About page"), "must contain link text, got: {text:?}");
    assert!(text.contains("Visit Example"), "must contain link text, got: {text:?}");
}

// -----------------------------------------------------------------------
// Integration tests -- use local HTTP fixtures only
// -----------------------------------------------------------------------

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
    assert!(text.contains(&fixture.base_url), "response must reference the domain");
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
    assert!(text.contains("Status: 200"), "must return a valid HTTP status: {text}");
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
    assert!(text.to_lowercase().contains("security"), "must contain 'security'");
    assert!(text.contains("Stanford"), "must contain 'Stanford'");
    assert!(text.len() > 3000, "extracted text too short: {} chars", text.len());
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
    assert!(chunk.is_char_boundary(chunk.len()), "chunk must end at char boundary");
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
    assert!(chunk.is_char_boundary(0), "chunk start must be char boundary");
    assert!(chunk.is_char_boundary(chunk.len()), "chunk end must be char boundary");
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
    assert_eq!(collected, text, "round-trip pagination must reconstruct original text");
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
    assert_eq!(collected, text, "round-trip must match: {collected:?} vs {text:?}");
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
    assert!(!content_mode.contains("<script"), "content mode must strip scripts");
    assert!(!content_mode.contains("<div"), "content mode must strip div tags");
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
        let len: usize = cl.trim_start_matches("Content length: ").parse().unwrap_or(0);
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
    assert!(!text.contains("]("), "content mode must not have markdown links");
    assert!(!text.contains("**"), "content mode must not have bold markers");
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
    assert!(!text.contains("Matches found: 0"), "must find matches: {text}");
    assert!(text.contains("Match 1"), "must have at least one match block");
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
    assert!(!text.contains("Matches found: 0"), "must find Mozilla matches");
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
    assert!(!is_tool_error(&resp), "fetch should succeed (no panic from multi-byte)");
    let text = extract_tool_text(&resp);
    assert!(text.contains("Unicode"), "must contain 'Unicode'");
}

// -- grep match collection --
