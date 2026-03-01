//! Built-in MCP tools that run on the host.
//!
//! Three HTTP tools checked against DomainPolicy:
//! - `fetch_http`: fetch a URL and return text content
//! - `grep_http`: fetch a URL and search for a regex pattern
//! - `http_headers`: return HTTP headers for a URL

use reqwest::Client;
use serde_json::Value;

use crate::net::domain_policy::{Action, DomainPolicy};

use super::types::{JsonRpcResponse, McpToolDef};

/// Return the three built-in tool definitions.
pub fn builtin_tool_defs() -> Vec<McpToolDef> {
    vec![
        McpToolDef {
            namespaced_name: "builtin__fetch_http".into(),
            original_name: "fetch_http".into(),
            description: Some(
                "Fetch a URL and return its text content. Supports raw or extracted text mode."
                    .into(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch"
                    },
                    "format": {
                        "type": "string",
                        "enum": ["raw", "content"],
                        "description": "Output format: 'raw' returns body as-is, 'content' extracts text from HTML (default: content)"
                    },
                    "start_index": {
                        "type": "integer",
                        "description": "Character offset to start from (default: 0)"
                    },
                    "max_length": {
                        "type": "integer",
                        "description": "Maximum characters to return (default: 50000)"
                    }
                },
                "required": ["url"]
            }),
            server_name: "builtin".into(),
        },
        McpToolDef {
            namespaced_name: "builtin__grep_http".into(),
            original_name: "grep_http".into(),
            description: Some(
                "Fetch a URL and search for a regex pattern in the content.".into(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch"
                    },
                    "pattern": {
                        "type": "string",
                        "description": "Regex pattern to search for (case-insensitive)"
                    },
                    "context_lines": {
                        "type": "integer",
                        "description": "Lines of context around matches (default: 3)"
                    },
                    "max_matches": {
                        "type": "integer",
                        "description": "Maximum matches to return (default: 50)"
                    },
                    "raw": {
                        "type": "boolean",
                        "description": "Search raw HTML instead of extracted text (default: false)"
                    },
                    "start_index": {
                        "type": "integer",
                        "description": "Character offset to start from (default: 0)"
                    },
                    "max_length": {
                        "type": "integer",
                        "description": "Maximum characters to return (default: 50000)"
                    }
                },
                "required": ["url", "pattern"]
            }),
            server_name: "builtin".into(),
        },
        McpToolDef {
            namespaced_name: "builtin__http_headers".into(),
            original_name: "http_headers".into(),
            description: Some("Return HTTP status and headers for a URL.".into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to check"
                    },
                    "method": {
                        "type": "string",
                        "enum": ["HEAD", "GET"],
                        "description": "HTTP method (default: HEAD)"
                    },
                    "start_index": {
                        "type": "integer",
                        "description": "Character offset to start from (default: 0)"
                    },
                    "max_length": {
                        "type": "integer",
                        "description": "Maximum characters to return (default: 50000)"
                    }
                },
                "required": ["url"]
            }),
            server_name: "builtin".into(),
        },
    ]
}

/// Dispatch a built-in tool call by local name (after namespace stripping).
pub async fn call_builtin_tool(
    local_name: &str,
    arguments: &Value,
    client: &Client,
    domain_policy: &DomainPolicy,
    request_id: Option<Value>,
) -> JsonRpcResponse {
    match local_name {
        "fetch_http" => handle_fetch_http(arguments, client, domain_policy, request_id).await,
        "grep_http" => handle_grep_http(arguments, client, domain_policy, request_id).await,
        "http_headers" => handle_http_headers(arguments, client, domain_policy, request_id).await,
        _ => JsonRpcResponse::err(
            request_id,
            -32602,
            format!("unknown builtin tool: {local_name}"),
        ),
    }
}

// ---------------------------------------------------------------------------
// fetch_http
// ---------------------------------------------------------------------------

async fn handle_fetch_http(
    args: &Value,
    client: &Client,
    policy: &DomainPolicy,
    id: Option<Value>,
) -> JsonRpcResponse {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return tool_error(id, "missing required parameter: url"),
    };

    let domain = match check_domain_policy(url, policy) {
        Ok(d) => d,
        Err(e) => return tool_error(id, &e),
    };

    let format = args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("content");
    let start_index = args
        .get("start_index")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let max_length = args
        .get("max_length")
        .and_then(|v| v.as_u64())
        .unwrap_or(50000) as usize;

    let body = match client.get(url).send().await {
        Ok(resp) => match resp.text().await {
            Ok(t) => t,
            Err(e) => return tool_error(id, &format!("failed to read response body: {e}")),
        },
        Err(e) => return tool_error(id, &format!("HTTP request failed: {e}")),
    };

    let text = if format == "raw" {
        body
    } else {
        extract_text_from_html(&body)
    };

    let (chunk, total, has_more) = paginate(&text, start_index, max_length);
    let mut output = format!("URL: {url}\nDomain: {domain}\nContent length: {total}\n");
    if start_index > 0 || has_more {
        output.push_str(&format!(
            "Showing: {start_index}..{}\n",
            start_index + chunk.len()
        ));
        if has_more {
            output.push_str(&format!(
                "Remaining: {} characters. Use start_index={} to continue.\n",
                total - start_index - chunk.len(),
                start_index + chunk.len()
            ));
        }
    }
    output.push('\n');
    output.push_str(&chunk);

    tool_ok(id, &output)
}

// ---------------------------------------------------------------------------
// grep_http
// ---------------------------------------------------------------------------

async fn handle_grep_http(
    args: &Value,
    client: &Client,
    policy: &DomainPolicy,
    id: Option<Value>,
) -> JsonRpcResponse {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return tool_error(id, "missing required parameter: url"),
    };
    let pattern_str = match args.get("pattern").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return tool_error(id, "missing required parameter: pattern"),
    };

    if let Err(e) = check_domain_policy(url, policy) {
        return tool_error(id, &e);
    }

    let context_lines = args
        .get("context_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(3) as usize;
    let max_matches = args
        .get("max_matches")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as usize;
    let raw = args.get("raw").and_then(|v| v.as_bool()).unwrap_or(false);
    let start_index = args
        .get("start_index")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let max_length = args
        .get("max_length")
        .and_then(|v| v.as_u64())
        .unwrap_or(50000) as usize;

    let re = match regex::RegexBuilder::new(pattern_str)
        .case_insensitive(true)
        .build()
    {
        Ok(r) => r,
        Err(e) => return tool_error(id, &format!("invalid regex: {e}")),
    };

    let body = match client.get(url).send().await {
        Ok(resp) => match resp.text().await {
            Ok(t) => t,
            Err(e) => return tool_error(id, &format!("failed to read response body: {e}")),
        },
        Err(e) => return tool_error(id, &format!("HTTP request failed: {e}")),
    };

    let text = if raw {
        body
    } else {
        extract_text_from_html(&body)
    };

    let lines: Vec<&str> = text.lines().collect();
    let mut matches = Vec::new();
    let mut match_count = 0;

    for (i, line) in lines.iter().enumerate() {
        if re.is_match(line) {
            match_count += 1;
            if match_count > max_matches {
                break;
            }
            let start = i.saturating_sub(context_lines);
            let end = (i + context_lines + 1).min(lines.len());
            let mut block = String::new();
            for j in start..end {
                let marker = if j == i { ">>>" } else { "   " };
                block.push_str(&format!("{marker} {}: {}\n", j + 1, lines[j]));
            }
            matches.push(block);
        }
    }

    let mut output = format!(
        "URL: {url}\nPattern: {pattern_str}\nMatches found: {match_count}\n"
    );
    if match_count > max_matches {
        output.push_str(&format!(
            "(showing first {max_matches} of {match_count} matches)\n"
        ));
    }
    output.push('\n');
    for (i, block) in matches.iter().enumerate() {
        output.push_str(&format!("--- Match {} ---\n{}\n", i + 1, block));
    }

    let (chunk, total, has_more) = paginate(&output, start_index, max_length);
    if has_more {
        let header = format!(
            "Content length: {total}\nShowing: {start_index}..{}\nUse start_index={} to continue.\n\n",
            start_index + chunk.len(),
            start_index + chunk.len()
        );
        tool_ok(id, &format!("{header}{chunk}"))
    } else {
        tool_ok(id, &chunk)
    }
}

// ---------------------------------------------------------------------------
// http_headers
// ---------------------------------------------------------------------------

async fn handle_http_headers(
    args: &Value,
    client: &Client,
    policy: &DomainPolicy,
    id: Option<Value>,
) -> JsonRpcResponse {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return tool_error(id, "missing required parameter: url"),
    };

    if let Err(e) = check_domain_policy(url, policy) {
        return tool_error(id, &e);
    }

    let method = args
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("HEAD");
    let start_index = args
        .get("start_index")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let max_length = args
        .get("max_length")
        .and_then(|v| v.as_u64())
        .unwrap_or(50000) as usize;

    let resp = match method {
        "GET" => client.get(url).send().await,
        _ => client.head(url).send().await,
    };

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return tool_error(id, &format!("HTTP request failed: {e}")),
    };

    let mut output = format!("URL: {url}\nStatus: {}\n\nHeaders:\n", resp.status());
    for (name, value) in resp.headers() {
        output.push_str(&format!(
            "  {}: {}\n",
            name,
            value.to_str().unwrap_or("<binary>")
        ));
    }

    let (chunk, _total, _has_more) = paginate(&output, start_index, max_length);
    tool_ok(id, &chunk)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if the URL's domain is allowed by policy. Returns domain on success.
fn check_domain_policy(url: &str, policy: &DomainPolicy) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;
    let domain = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?
        .to_string();
    let (action, reason) = policy.evaluate(&domain);
    if action == Action::Deny {
        return Err(format!("domain blocked by policy: {domain} ({reason})"));
    }
    Ok(domain)
}

/// Extract visible text from HTML using tl DOM parser.
/// Skips script, style, noscript, svg, and template elements.
/// Inserts newlines around block elements.
pub fn extract_text_from_html(html: &str) -> String {
    let dom = match tl::parse(html, tl::ParserOptions::default()) {
        Ok(d) => d,
        Err(_) => return html.to_string(),
    };
    let parser = dom.parser();
    let mut output = String::new();
    for child in dom.children() {
        extract_text_recursive(child, parser, &mut output);
    }
    collapse_whitespace(&output)
}

const SKIP_TAGS: &[&str] = &["script", "style", "noscript", "svg", "template"];
const BLOCK_TAGS: &[&str] = &[
    "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "tr", "br", "hr", "section",
    "article", "header", "footer", "nav", "main", "blockquote", "pre", "table", "ul", "ol", "dl",
    "dt", "dd", "figcaption", "figure", "details", "summary",
];

fn extract_text_recursive(
    node_handle: &tl::NodeHandle,
    parser: &tl::Parser,
    output: &mut String,
) {
    let node = match node_handle.get(parser) {
        Some(n) => n,
        None => return,
    };

    match node {
        tl::Node::Raw(text) => {
            let s = text.as_utf8_str();
            output.push_str(&s);
        }
        tl::Node::Tag(tag) => {
            let tag_name = tag.name().as_utf8_str().to_lowercase();

            if SKIP_TAGS.contains(&tag_name.as_str()) {
                return;
            }

            let is_block = BLOCK_TAGS.contains(&tag_name.as_str());
            if is_block {
                output.push('\n');
            }

            let children = tag.children();
            for child in children.top().iter() {
                extract_text_recursive(child, parser, output);
            }

            if is_block {
                output.push('\n');
            }
        }
        tl::Node::Comment(_) => {}
    }
}

/// Collapse runs of whitespace and newlines into single space/newline, then trim.
pub fn collapse_whitespace(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut prev_was_newline = false;
    let mut prev_was_space = false;

    for ch in input.chars() {
        if ch == '\n' {
            if !prev_was_newline {
                result.push('\n');
            }
            prev_was_newline = true;
            prev_was_space = false;
        } else if ch.is_whitespace() {
            if !prev_was_space && !prev_was_newline {
                result.push(' ');
            }
            prev_was_space = true;
        } else {
            prev_was_newline = false;
            prev_was_space = false;
            result.push(ch);
        }
    }

    result.trim().to_string()
}

/// Paginate text: return (chunk, total_length, has_more).
pub fn paginate(text: &str, start: usize, max: usize) -> (String, usize, bool) {
    let total = text.len();
    if start >= total {
        return (String::new(), total, false);
    }
    let end = (start + max).min(total);
    let chunk = &text[start..end];
    (chunk.to_string(), total, end < total)
}

fn tool_ok(id: Option<Value>, text: &str) -> JsonRpcResponse {
    JsonRpcResponse::ok(
        id,
        serde_json::json!({
            "content": [{"type": "text", "text": text}]
        }),
    )
}

fn tool_error(id: Option<Value>, msg: &str) -> JsonRpcResponse {
    JsonRpcResponse::ok(
        id,
        serde_json::json!({
            "content": [{"type": "text", "text": format!("Error: {msg}")}],
            "isError": true
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_tool_defs_returns_three_tools() {
        let defs = builtin_tool_defs();
        assert_eq!(defs.len(), 3);
        assert!(defs.iter().all(|d| d.server_name == "builtin"));
        let names: Vec<&str> = defs.iter().map(|d| d.namespaced_name.as_str()).collect();
        assert!(names.contains(&"builtin__fetch_http"));
        assert!(names.contains(&"builtin__grep_http"));
        assert!(names.contains(&"builtin__http_headers"));
    }

    #[test]
    fn check_domain_policy_allows_github() {
        let policy = DomainPolicy::default_dev();
        let result = check_domain_policy("https://github.com/foo/bar", &policy);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "github.com");
    }

    #[test]
    fn check_domain_policy_denies_unknown() {
        let policy = DomainPolicy::default_dev();
        let result = check_domain_policy("https://evil-unknown-domain.xyz/hack", &policy);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("blocked"));
    }

    #[test]
    fn check_domain_policy_rejects_invalid_url() {
        let policy = DomainPolicy::default_dev();
        let result = check_domain_policy("not a url at all", &policy);
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "nonexistent",
            &serde_json::json!({}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
        )
        .await;
        assert!(resp.error.is_some());
        assert!(
            resp.error.unwrap().message.contains("unknown builtin tool")
        );
    }

    #[tokio::test]
    async fn fetch_http_missing_url_returns_error() {
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "fetch_http",
            &serde_json::json!({}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "fetch_http",
            &serde_json::json!({"url": "https://evil-unknown-domain.xyz/"}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "grep_http",
            &serde_json::json!({"url": "https://example.com"}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "grep_http",
            &serde_json::json!({"url": "https://github.com", "pattern": "[invalid"}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
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
    // Integration tests -- require network access
    // -----------------------------------------------------------------------

    /// Helper to extract the text content from a tool response.
    fn extract_tool_text(resp: &JsonRpcResponse) -> &str {
        resp.result
            .as_ref()
            .unwrap()["content"][0]["text"]
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
    async fn integration_fetch_http_elie_net() {
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "fetch_http",
            &serde_json::json!({"url": "https://elie.net"}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
        )
        .await;
        assert!(!is_tool_error(&resp), "fetch should succeed");
        let text = extract_tool_text(&resp);
        assert!(
            text.contains("elie.net"),
            "response must reference the domain"
        );
        // The extracted content must contain real text from the page
        assert!(
            text.to_lowercase().contains("elie"),
            "page content must contain 'elie': {text}"
        );
    }

    #[tokio::test]
    async fn integration_grep_http_elie_net_finds_matches() {
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "grep_http",
            &serde_json::json!({"url": "https://elie.net", "pattern": "elie"}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
        )
        .await;
        assert!(!is_tool_error(&resp), "grep should succeed");
        let text = extract_tool_text(&resp);
        // Must NOT say "Matches found: 0"
        assert!(
            !text.contains("Matches found: 0"),
            "grep_http must find 'elie' on elie.net but got 0 matches: {text}"
        );
        assert!(
            text.contains("Match 1"),
            "grep_http must have at least one match block: {text}"
        );
    }

    #[tokio::test]
    async fn integration_grep_http_blocked_domain() {
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "grep_http",
            &serde_json::json!({
                "url": "https://evil-unknown-domain.xyz",
                "pattern": "test"
            }),
            &client,
            &policy,
            Some(serde_json::json!(1)),
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
    async fn integration_http_headers_elie_net() {
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "http_headers",
            &serde_json::json!({"url": "https://elie.net"}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
        )
        .await;
        assert!(!is_tool_error(&resp), "http_headers should succeed");
        let text = extract_tool_text(&resp);
        assert!(
            text.contains("Status: 200") || text.contains("Status: 301") || text.contains("Status: 302"),
            "must return a valid HTTP status: {text}"
        );
        assert!(
            text.to_lowercase().contains("content-type"),
            "must include content-type header: {text}"
        );
    }

    #[tokio::test]
    async fn integration_fetch_http_blocked_domain() {
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "fetch_http",
            &serde_json::json!({"url": "https://evil-unknown-domain.xyz"}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "http_headers",
            &serde_json::json!({"url": "https://evil-unknown-domain.xyz"}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
        )
        .await;
        assert!(is_tool_error(&resp), "blocked domain must return isError");
        let text = extract_tool_text(&resp);
        assert!(
            text.to_lowercase().contains("blocked"),
            "error must mention 'blocked': {text}"
        );
    }
}
