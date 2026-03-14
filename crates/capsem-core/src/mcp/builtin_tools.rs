//! Built-in MCP tools that run on the host.
//!
//! Three HTTP tools checked against DomainPolicy:
//! - `fetch_http`: fetch a URL and return text content
//! - `grep_http`: fetch a URL and search for a regex pattern
//! - `http_headers`: return HTTP headers for a URL

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use reqwest::Client;
use serde_json::Value;

use capsem_logger::{DbWriter, Decision, NetEvent, WriteOp};

use crate::net::domain_policy::{Action, DomainPolicy};

use super::types::{JsonRpcResponse, McpToolDef, ToolAnnotations};

/// The three built-in tool names (without any namespace prefix).
const BUILTIN_TOOL_NAMES: &[&str] = &["fetch_http", "grep_http", "http_headers"];

/// Returns true if the given tool name is a built-in tool.
pub fn is_builtin_tool(name: &str) -> bool {
    BUILTIN_TOOL_NAMES.contains(&name)
}

/// Return the three built-in tool definitions.
pub fn builtin_tool_defs() -> Vec<McpToolDef> {
    vec![
        McpToolDef {
            namespaced_name: "fetch_http".into(),
            original_name: "fetch_http".into(),
            description: Some(concat!(
                "Fetch a URL and return its text content. ",
                "In 'content' mode (default), HTML is cleaned: scripts, styles, and hidden elements are stripped, ",
                "and block-level elements (div, p, h1-h6, li, etc.) are converted to newlines. ",
                "Output starts with metadata lines (URL, Domain, Content length) followed by the page text. ",
                "Use start_index and max_length for pagination -- if the response is truncated, ",
                "a 'Remaining' line shows the next start_index value to continue. ",
                "The URL's domain must be allowed by network policy; blocked or unknown domains return an error. ",
                "Errors: domain blocked by policy, invalid URL, HTTP request failed.",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch. The domain must be allowed by network policy or the request will be rejected."
                    },
                    "format": {
                        "type": "string",
                        "enum": ["raw", "content"],
                        "description": "Output format: 'content' (default) extracts visible text from HTML, stripping scripts/styles and converting block elements to newlines. 'raw' returns the response body unchanged."
                    },
                    "start_index": {
                        "type": "integer",
                        "description": "Character offset to start reading from (default: 0). Use the value from the 'Remaining' line in a previous response to continue paginating."
                    },
                    "max_length": {
                        "type": "integer",
                        "description": "Maximum characters to return (default: 50000). If the content exceeds this, a 'Remaining' line indicates how to fetch the rest."
                    }
                },
                "required": ["url"]
            }),
            server_name: "builtin".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Fetch HTTP".into()),
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
                open_world_hint: true,
            }),
        },
        McpToolDef {
            namespaced_name: "grep_http".into(),
            original_name: "grep_http".into(),
            description: Some(concat!(
                "Fetch a URL and search its content for a regex pattern (case-insensitive). ",
                "By default, searches extracted text (HTML cleaned as in fetch_http); set raw=true to search the original HTML. ",
                "Output starts with metadata (URL, Pattern, Matches found), then match blocks. ",
                "Each match block shows context lines around the matching line, with '>>>' marking the match and line numbers. ",
                "Use start_index and max_length for pagination of large result sets. ",
                "The URL's domain must be allowed by network policy; blocked or unknown domains return an error. ",
                "Errors: domain blocked by policy, invalid URL, invalid regex syntax, HTTP request failed.",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch and search. The domain must be allowed by network policy or the request will be rejected."
                    },
                    "pattern": {
                        "type": "string",
                        "description": "Regex pattern to search for (case-insensitive). Uses Rust regex syntax (similar to PCRE without lookaround)."
                    },
                    "context_lines": {
                        "type": "integer",
                        "description": "Number of lines to show before and after each matching line (default: 3)"
                    },
                    "max_matches": {
                        "type": "integer",
                        "description": "Maximum number of matches to return (default: 50). If more matches exist, output notes the truncation."
                    },
                    "raw": {
                        "type": "boolean",
                        "description": "If true, search the raw HTML source instead of extracted text (default: false)"
                    },
                    "start_index": {
                        "type": "integer",
                        "description": "Character offset to start reading output from (default: 0). Use for paginating large result sets."
                    },
                    "max_length": {
                        "type": "integer",
                        "description": "Maximum characters to return (default: 50000). If truncated, use the indicated start_index to continue."
                    }
                },
                "required": ["url", "pattern"]
            }),
            server_name: "builtin".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Grep HTTP".into()),
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
                open_world_hint: true,
            }),
        },
        McpToolDef {
            namespaced_name: "http_headers".into(),
            original_name: "http_headers".into(),
            description: Some(concat!(
                "Return HTTP status code and response headers for a URL. ",
                "By default uses HEAD (no body downloaded, faster). Set method='GET' to see headers from a full response ",
                "(some servers return different headers for HEAD vs GET). ",
                "Output format: 'URL:' line, 'Status:' line, then 'Headers:' section with one 'name: value' per line. ",
                "The URL's domain must be allowed by network policy; blocked or unknown domains return an error. ",
                "Errors: domain blocked by policy, invalid URL, HTTP request failed.",
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to check. The domain must be allowed by network policy or the request will be rejected."
                    },
                    "method": {
                        "type": "string",
                        "enum": ["HEAD", "GET"],
                        "description": "HTTP method to use (default: HEAD). HEAD is faster as it skips the body, but some servers return different headers for GET."
                    },
                    "start_index": {
                        "type": "integer",
                        "description": "Character offset to start reading from (default: 0). Rarely needed since header output is typically small."
                    },
                    "max_length": {
                        "type": "integer",
                        "description": "Maximum characters to return (default: 50000). Rarely needed since header output is typically small."
                    }
                },
                "required": ["url"]
            }),
            server_name: "builtin".into(),
            annotations: Some(ToolAnnotations {
                title: Some("HTTP Headers".into()),
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
                open_world_hint: true,
            }),
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
    db: &Arc<DbWriter>,
) -> JsonRpcResponse {
    match local_name {
        "fetch_http" => handle_fetch_http(arguments, client, domain_policy, request_id, db).await,
        "grep_http" => handle_grep_http(arguments, client, domain_policy, request_id, db).await,
        "http_headers" => handle_http_headers(arguments, client, domain_policy, request_id, db).await,
        _ => JsonRpcResponse::err(
            request_id,
            -32602,
            format!("unknown builtin tool: {local_name}"),
        ),
    }
}

/// Emit a NetEvent for a builtin tool HTTP request.
async fn emit_net_event(
    db: &Arc<DbWriter>,
    domain: &str,
    method: &str,
    path: &str,
    decision: Decision,
    status_code: Option<u16>,
    bytes_sent: u64,
    bytes_received: u64,
    duration_ms: u64,
) {
    db.write(WriteOp::NetEvent(NetEvent {
        timestamp: SystemTime::now(),
        domain: domain.to_string(),
        port: 443,
        decision,
        process_name: Some("mcp_builtin".to_string()),
        pid: None,
        method: Some(method.to_string()),
        path: Some(path.to_string()),
        query: None,
        status_code,
        bytes_sent,
        bytes_received,
        duration_ms,
        matched_rule: None,
        request_headers: None,
        response_headers: None,
        request_body_preview: None,
        response_body_preview: None,
        conn_type: Some("mcp_builtin".to_string()),
    }))
    .await;
}

// ---------------------------------------------------------------------------
// fetch_http
// ---------------------------------------------------------------------------

async fn handle_fetch_http(
    args: &Value,
    client: &Client,
    policy: &DomainPolicy,
    id: Option<Value>,
    db: &Arc<DbWriter>,
) -> JsonRpcResponse {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return tool_error(id, "missing required parameter: url"),
    };

    let domain = match check_domain_policy(url, policy) {
        Ok(d) => d,
        Err(e) => {
            let path = reqwest::Url::parse(url).map(|u| u.path().to_string()).unwrap_or_default();
            emit_net_event(db, &extract_domain(url), "GET", &path, Decision::Denied, None, 0, 0, 0).await;
            return tool_error(id, &e);
        }
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

    let start = Instant::now();
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => return tool_error(id, &format!("HTTP request failed: {e}")),
    };

    let status_code = resp.status().as_u16();

    // Reject binary content unless the user explicitly wants raw bytes
    let ct = get_content_type(&resp);
    if format != "raw" && is_binary_content_type(&ct) {
        return tool_error(
            id,
            &format!(
                "cannot extract text from binary content (content-type: {ct}). \
                 Use format='raw' to fetch the raw bytes."
            ),
        );
    }

    let body = match resp.text().await {
        Ok(t) => t,
        Err(e) => return tool_error(id, &format!("failed to read response body: {e}")),
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let bytes_received = body.len() as u64;
    let path = reqwest::Url::parse(url).map(|u| u.path().to_string()).unwrap_or_default();
    emit_net_event(db, &domain, "GET", &path, Decision::Allowed, Some(status_code), 0, bytes_received, duration_ms).await;

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
    db: &Arc<DbWriter>,
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
        let path = reqwest::Url::parse(url).map(|u| u.path().to_string()).unwrap_or_default();
        emit_net_event(db, &extract_domain(url), "GET", &path, Decision::Denied, None, 0, 0, 0).await;
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

    if pattern_str.is_empty() {
        return tool_error(id, "pattern must not be empty");
    }

    let re = match regex::RegexBuilder::new(pattern_str)
        .case_insensitive(true)
        .build()
    {
        Ok(r) => r,
        Err(e) => return tool_error(id, &format!("invalid regex: {e}")),
    };

    let start = Instant::now();
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => return tool_error(id, &format!("HTTP request failed: {e}")),
    };

    let status_code = resp.status().as_u16();

    // Reject binary content unless the user explicitly wants raw search
    let ct = get_content_type(&resp);
    if !raw && is_binary_content_type(&ct) {
        return tool_error(
            id,
            &format!(
                "cannot search binary content (content-type: {ct}). \
                 Binary files like images and PDFs are not searchable."
            ),
        );
    }

    let body = match resp.text().await {
        Ok(t) => t,
        Err(e) => return tool_error(id, &format!("failed to read response body: {e}")),
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let bytes_received = body.len() as u64;
    let url_path = reqwest::Url::parse(url).map(|u| u.path().to_string()).unwrap_or_default();
    emit_net_event(db, &extract_domain(url), "GET", &url_path, Decision::Allowed, Some(status_code), 0, bytes_received, duration_ms).await;

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
    db: &Arc<DbWriter>,
) -> JsonRpcResponse {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return tool_error(id, "missing required parameter: url"),
    };

    if let Err(e) = check_domain_policy(url, policy) {
        let path = reqwest::Url::parse(url).map(|u| u.path().to_string()).unwrap_or_default();
        emit_net_event(db, &extract_domain(url), "HEAD", &path, Decision::Denied, None, 0, 0, 0).await;
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

    let start = Instant::now();
    let resp = match method {
        "GET" => client.get(url).send().await,
        _ => client.head(url).send().await,
    };

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return tool_error(id, &format!("HTTP request failed: {e}")),
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let status_code = resp.status().as_u16();

    let mut output = format!("URL: {url}\nStatus: {}\n\nHeaders:\n", resp.status());
    for (name, value) in resp.headers() {
        output.push_str(&format!(
            "  {}: {}\n",
            name,
            value.to_str().unwrap_or("<binary>")
        ));
    }
    let url_path = reqwest::Url::parse(url).map(|u| u.path().to_string()).unwrap_or_default();
    emit_net_event(db, &extract_domain(url), method, &url_path, Decision::Allowed, Some(status_code), 0, output.len() as u64, duration_ms).await;

    let (chunk, _total, _has_more) = paginate(&output, start_index, max_length);
    tool_ok(id, &chunk)
}

// ---------------------------------------------------------------------------
// Content-Type helpers
// ---------------------------------------------------------------------------

/// Known-binary MIME type prefixes. These cannot be meaningfully text-extracted.
const BINARY_MIME_PREFIXES: &[&str] = &[
    "image/",
    "audio/",
    "video/",
    "font/",
    "application/octet-stream",
    "application/pdf",
    "application/zip",
    "application/gzip",
    "application/x-tar",
    "application/wasm",
    "application/x-executable",
];

/// Returns true if the Content-Type indicates binary content.
fn is_binary_content_type(content_type: &str) -> bool {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    BINARY_MIME_PREFIXES
        .iter()
        .any(|prefix| ct.starts_with(prefix))
}

/// Extract the Content-Type header value from a response, defaulting to empty.
fn get_content_type(resp: &reqwest::Response) -> String {
    resp.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract domain from a URL string, returning "unknown" on failure.
fn extract_domain(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Check if the URL's domain is allowed by policy. Returns domain on success.
fn check_domain_policy(url: &str, policy: &DomainPolicy) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "only http:// and https:// URLs are supported (got {other}://)"
            ))
        }
    }
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

    fn test_db() -> Arc<DbWriter> {
        Arc::new(DbWriter::open_in_memory(64).unwrap())
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
            &test_db(),
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "fetch_http",
            &serde_json::json!({"url": "https://evil-unknown-domain.xyz/"}),
            &client,
            &policy,
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "grep_http",
            &serde_json::json!({"url": "https://example.com"}),
            &client,
            &policy,
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "grep_http",
            &serde_json::json!({"url": "https://github.com", "pattern": "[invalid"}),
            &client,
            &policy,
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
    // check_domain_policy scheme rejection tests
    // -----------------------------------------------------------------------

    #[test]
    fn check_domain_policy_rejects_ftp() {
        let policy = DomainPolicy::default_dev();
        let result = check_domain_policy("ftp://example.com/file", &policy);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only http"));
    }

    #[test]
    fn check_domain_policy_rejects_file() {
        let policy = DomainPolicy::default_dev();
        let result = check_domain_policy("file:///etc/passwd", &policy);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only http"));
    }

    #[test]
    fn check_domain_policy_rejects_data_uri() {
        let policy = DomainPolicy::default_dev();
        let result = check_domain_policy("data:text/html,<h1>hi</h1>", &policy);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only http"));
    }

    #[test]
    fn check_domain_policy_rejects_javascript() {
        let policy = DomainPolicy::default_dev();
        let result = check_domain_policy("javascript:alert(1)", &policy);
        assert!(result.is_err());
        // reqwest::Url::parse may reject this as invalid, either way it errors
        assert!(result.is_err());
    }

    #[test]
    fn check_domain_policy_empty_url() {
        let policy = DomainPolicy::default_dev();
        let result = check_domain_policy("", &policy);
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
        let text =
            extract_text_from_html("<template><p>hidden</p></template>visible");
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
        let text =
            extract_text_from_html("<div><script>evil()</script>Good</div>");
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "fetch_http",
            &serde_json::json!({"url": "ftp://example.com/file"}),
            &client,
            &policy,
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "fetch_http",
            &serde_json::json!({"url": "file:///etc/passwd"}),
            &client,
            &policy,
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "fetch_http",
            &serde_json::json!({"url": "data:text/plain,hello"}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
            &test_db(),
        )
        .await;
        assert!(is_tool_error(&resp));
    }

    #[tokio::test]
    async fn fetch_http_url_is_number_not_string() {
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "fetch_http",
            &serde_json::json!({"url": 42}),
            &client,
            &policy,
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "fetch_http",
            &serde_json::json!({"url": null}),
            &client,
            &policy,
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "fetch_http",
            &serde_json::json!({
                "url": "https://elie.net",
                "start_index": -1
            }),
            &client,
            &policy,
            Some(serde_json::json!(1)),
            &test_db(),
        )
        .await;
        // Should succeed (negative start_index is silently treated as 0)
        assert!(!is_tool_error(&resp), "should succeed with default start_index=0");
        let text = extract_tool_text(&resp);
        assert!(text.contains("URL: https://elie.net"), "got: {text}");
    }

    // -----------------------------------------------------------------------
    // grep_http edge cases (async)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn grep_http_empty_pattern_rejected() {
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "grep_http",
            &serde_json::json!({"url": "https://github.com", "pattern": ""}),
            &client,
            &policy,
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "grep_http",
            &serde_json::json!({"pattern": "test"}),
            &client,
            &policy,
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "grep_http",
            &serde_json::json!({"url": 123, "pattern": "test"}),
            &client,
            &policy,
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "grep_http",
            &serde_json::json!({"url": "ftp://example.com", "pattern": "test"}),
            &client,
            &policy,
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "grep_http",
            &serde_json::json!({
                "url": "https://elie.net",
                "pattern": "(a+)+$"
            }),
            &client,
            &policy,
            Some(serde_json::json!(1)),
            &test_db(),
        )
        .await;
        // Should complete without hanging (pass or no matches, either is fine)
        assert!(!is_tool_error(&resp), "should not error: {:?}", extract_tool_text(&resp));
    }

    // -----------------------------------------------------------------------
    // http_headers edge cases (async)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn http_headers_missing_url() {
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "http_headers",
            &serde_json::json!({}),
            &client,
            &policy,
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "http_headers",
            &serde_json::json!({"url": "ftp://example.com"}),
            &client,
            &policy,
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "http_headers",
            &serde_json::json!({"url": "https://elie.net", "method": "POST"}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
            &test_db(),
        )
        .await;
        // Should succeed with HEAD fallback
        assert!(!is_tool_error(&resp), "should succeed with HEAD fallback");
        let text = extract_tool_text(&resp);
        assert!(text.contains("Status:"), "got: {text}");
    }

    #[tokio::test]
    async fn http_headers_method_case_sensitive() {
        // "get" (lowercase) is not "GET", so falls through to HEAD
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "http_headers",
            &serde_json::json!({"url": "https://elie.net", "method": "get"}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
            &test_db(),
        )
        .await;
        assert!(!is_tool_error(&resp), "should succeed with HEAD fallback");
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
            &test_db(),
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
            &test_db(),
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
    async fn integration_http_headers_elie_net() {
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "http_headers",
            &serde_json::json!({"url": "https://elie.net"}),
            &client,
            &policy,
            Some(serde_json::json!(1)),
            &test_db(),
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
        let client = Client::new();
        let policy = DomainPolicy::default_dev();
        let resp = call_builtin_tool(
            "http_headers",
            &serde_json::json!({"url": "https://evil-unknown-domain.xyz"}),
            &client,
            &policy,
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
}
