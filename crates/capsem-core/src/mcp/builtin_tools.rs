//! Built-in MCP tools that run on the host.
//!
//! Three HTTP tools checked against the unified security engine:
//! - `fetch_http`: fetch a URL and return text content
//! - `grep_http`: fetch a URL and search for a regex pattern
//! - `http_headers`: return HTTP headers for a URL

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use reqwest::Client;
use serde_json::Value;

use capsem_logger::{DbWriter, Decision, NetEvent, WriteOp};

use crate::net::policy_config::{SecurityPluginConfig, SecurityRuleSet};
use crate::security_engine::{
    evaluate_security_boundary, HttpRequestSecurityEvent, HttpSecurityEvent, IpSecurityEvent, RuntimeSecurityEventType,
    SecurityEnforcementAction, SecurityEnforcementDecision, SecurityEvent, TcpSecurityEvent,
};

use super::types::{JsonRpcResponse, McpToolDef, ToolAnnotations};

/// The three built-in tool names (without any namespace prefix).
const BUILTIN_TOOL_NAMES: &[&str] = &["fetch_http", "grep_http", "http_headers"];

/// Default max characters returned by HTTP tools. Keep small to avoid
/// blowing up the AI agent's context window; callers can paginate for more.
pub(crate) const DEFAULT_MAX_LENGTH: u64 = 5000;
const DEFAULT_CONTEXT_LINES: u64 = 3;
const DEFAULT_MAX_MATCHES: u64 = 50;
const BUILTIN_PROCESS_NAME: &str = "mcp_builtin";

/// Build a JSON schema property for an integer parameter.
fn schema_int(description: &str) -> Value {
    serde_json::json!({"type": "integer", "description": description})
}

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
                "Fetch a URL and return its content. ",
                "In 'markdown' mode (default), HTML is converted to clean markdown preserving headings, links, lists, bold/italic, and code blocks. ",
                "In 'content' mode, HTML is stripped to plain text with newlines at block boundaries. ",
                "In 'raw' mode, the response body is returned unchanged. ",
                "Output starts with metadata lines (URL, Domain, Content length) followed by the page content. ",
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
                        "enum": ["markdown", "content", "raw"],
                        "description": "Output format: 'markdown' (default) converts HTML to markdown preserving structure (headings, links, lists, code). 'content' strips to plain text. 'raw' returns the response body unchanged."
                    },
                    "start_index": {
                        "type": "integer",
                        "description": "Character offset to start reading from (default: 0). Use the value from the 'Remaining' line in a previous response to continue paginating."
                    },
                    "max_length": schema_int(&format!(
                        "Maximum characters to return (default: {DEFAULT_MAX_LENGTH}). If the content exceeds this, a 'Remaining' line indicates how to fetch the rest."
                    ))
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
            timeout_secs: None,
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
                    "context_lines": schema_int(&format!(
                        "Number of lines to show before and after each matching line (default: {DEFAULT_CONTEXT_LINES})"
                    )),
                    "max_matches": schema_int(&format!(
                        "Maximum number of matches to return (default: {DEFAULT_MAX_MATCHES}). If more matches exist, output notes the truncation."
                    )),
                    "raw": {
                        "type": "boolean",
                        "description": "If true, search the raw HTML source instead of extracted text (default: false)"
                    },
                    "start_index": {
                        "type": "integer",
                        "description": "Character offset to start reading output from (default: 0). Use for paginating large result sets."
                    },
                    "max_length": schema_int(&format!(
                        "Maximum characters to return (default: {DEFAULT_MAX_LENGTH}). If truncated, use the indicated start_index to continue."
                    ))
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
            timeout_secs: None,
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
                    "max_length": schema_int(&format!(
                        "Maximum characters to return (default: {DEFAULT_MAX_LENGTH}). Rarely needed since header output is typically small."
                    ))
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
            timeout_secs: None,
        },
    ]
}

/// Dispatch a built-in tool call by local name (after namespace stripping).
pub async fn call_builtin_tool(
    local_name: &str,
    arguments: &Value,
    client: &Client,
    security_rules: &SecurityRuleSet,
    plugin_policy: &BTreeMap<String, SecurityPluginConfig>,
    request_id: Option<Value>,
    db: &Arc<DbWriter>,
) -> JsonRpcResponse {
    match local_name {
        "fetch_http" => handle_fetch_http(arguments, client, security_rules, plugin_policy, request_id, db).await,
        "grep_http" => handle_grep_http(arguments, client, security_rules, plugin_policy, request_id, db).await,
        "http_headers" => handle_http_headers(arguments, client, security_rules, plugin_policy, request_id, db).await,
        _ => JsonRpcResponse::err(request_id, -32602, format!("unknown builtin tool: {local_name}")),
    }
}

/// Emit a NetEvent for a builtin tool HTTP request.
#[allow(clippy::too_many_arguments)]
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
    enforcement: &SecurityEnforcementDecision,
) {
    crate::security_engine::emit_security_write(
        db,
        WriteOp::NetEvent(NetEvent {
            event_id: None,
            timestamp: SystemTime::now(),
            domain: domain.to_string(),
            port: 443,
            decision,
            process_name: Some(BUILTIN_PROCESS_NAME.to_string()),
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
            request_body_full: None,
            response_body_full: None,
            conn_type: Some(BUILTIN_PROCESS_NAME.to_string()),
            policy_mode: Some("security_event".to_string()),
            policy_action: Some(enforcement.action.as_str().to_string()),
            policy_rule: enforcement.rule_id.clone(),
            policy_reason: enforcement.reason.clone(),
            trace_id: capsem_foundation::telemetry::ambient_capsem_trace_id(),
            credential_ref: None,
        }),
    )
    .await;
}

// ---------------------------------------------------------------------------
// fetch_http
// ---------------------------------------------------------------------------

async fn handle_fetch_http(
    args: &Value,
    client: &Client,
    security_rules: &SecurityRuleSet,
    plugin_policy: &BTreeMap<String, SecurityPluginConfig>,
    id: Option<Value>,
    db: &Arc<DbWriter>,
) -> JsonRpcResponse {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return tool_error(id, "missing required parameter: url"),
    };

    let checked = match evaluate_builtin_http_request(url, "GET", security_rules, plugin_policy) {
        Ok(checked) => checked,
        Err(e) => {
            let blocked = blocked_decision(e.clone());
            let path = reqwest::Url::parse(url)
                .map(|u| u.path().to_string())
                .unwrap_or_default();
            emit_net_event(
                db,
                &extract_domain(url),
                "GET",
                &path,
                Decision::Denied,
                None,
                0,
                0,
                0,
                &blocked,
            )
            .await;
            return tool_error(id, &e);
        }
    };
    let domain = checked.domain.clone();

    let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("markdown");
    let start_index = args.get("start_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let max_length = args
        .get("max_length")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_LENGTH) as usize;

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
    let path = reqwest::Url::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_default();
    emit_net_event(
        db,
        &domain,
        "GET",
        &path,
        Decision::Allowed,
        Some(status_code),
        0,
        bytes_received,
        duration_ms,
        &checked.decision,
    )
    .await;

    let text = match format {
        "raw" => body,
        "content" => extract_text_from_html(&body),
        _ => extract_markdown_from_html(&body), // "markdown" or default
    };

    let (chunk, total, has_more) = paginate(&text, start_index, max_length);
    let mut output = format!("URL: {url}\nDomain: {domain}\nContent length: {total}\n");
    if start_index > 0 || has_more {
        output.push_str(&format!("Showing: {start_index}..{}\n", start_index + chunk.len()));
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
    security_rules: &SecurityRuleSet,
    plugin_policy: &BTreeMap<String, SecurityPluginConfig>,
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

    let checked = match evaluate_builtin_http_request(url, "GET", security_rules, plugin_policy) {
        Ok(checked) => checked,
        Err(e) => {
            let blocked = blocked_decision(e.clone());
            let path = reqwest::Url::parse(url)
                .map(|u| u.path().to_string())
                .unwrap_or_default();
            emit_net_event(
                db,
                &extract_domain(url),
                "GET",
                &path,
                Decision::Denied,
                None,
                0,
                0,
                0,
                &blocked,
            )
            .await;
            return tool_error(id, &e);
        }
    };

    let context_lines = args
        .get("context_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_CONTEXT_LINES) as usize;
    let max_matches = args
        .get("max_matches")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_MATCHES) as usize;
    let raw = args.get("raw").and_then(|v| v.as_bool()).unwrap_or(false);
    let start_index = args.get("start_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let max_length = args
        .get("max_length")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_LENGTH) as usize;

    if pattern_str.is_empty() {
        return tool_error(id, "pattern must not be empty");
    }

    let re = match regex::RegexBuilder::new(pattern_str).case_insensitive(true).build() {
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
    let url_path = reqwest::Url::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_default();
    emit_net_event(
        db,
        &extract_domain(url),
        "GET",
        &url_path,
        Decision::Allowed,
        Some(status_code),
        0,
        bytes_received,
        duration_ms,
        &checked.decision,
    )
    .await;

    let text = if raw { body } else { extract_text_from_html(&body) };

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
            for (offset, line) in lines[start..end].iter().enumerate() {
                let j = start + offset;
                let marker = if j == i { ">>>" } else { "   " };
                block.push_str(&format!("{marker} {}: {}\n", j + 1, line));
            }
            matches.push(block);
        }
    }

    let mut output = format!("URL: {url}\nPattern: {pattern_str}\nMatches found: {match_count}\n");
    if match_count > max_matches {
        output.push_str(&format!("(showing first {max_matches} of {match_count} matches)\n"));
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
    security_rules: &SecurityRuleSet,
    plugin_policy: &BTreeMap<String, SecurityPluginConfig>,
    id: Option<Value>,
    db: &Arc<DbWriter>,
) -> JsonRpcResponse {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return tool_error(id, "missing required parameter: url"),
    };

    let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("HEAD");

    let checked = match evaluate_builtin_http_request(url, method, security_rules, plugin_policy) {
        Ok(checked) => checked,
        Err(e) => {
            let blocked = blocked_decision(e.clone());
            let path = reqwest::Url::parse(url)
                .map(|u| u.path().to_string())
                .unwrap_or_default();
            emit_net_event(
                db,
                &extract_domain(url),
                "HEAD",
                &path,
                Decision::Denied,
                None,
                0,
                0,
                0,
                &blocked,
            )
            .await;
            return tool_error(id, &e);
        }
    };
    let start_index = args.get("start_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let max_length = args
        .get("max_length")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_LENGTH) as usize;

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
        output.push_str(&format!("  {}: {}\n", name, value.to_str().unwrap_or("<binary>")));
    }
    let url_path = reqwest::Url::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_default();
    emit_net_event(
        db,
        &extract_domain(url),
        method,
        &url_path,
        Decision::Allowed,
        Some(status_code),
        0,
        output.len() as u64,
        duration_ms,
        &checked.decision,
    )
    .await;

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
    let ct = content_type.split(';').next().unwrap_or("").trim().to_lowercase();
    BINARY_MIME_PREFIXES.iter().any(|prefix| ct.starts_with(prefix))
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

#[derive(Debug, Clone)]
struct BuiltinHttpDecision {
    domain: String,
    decision: SecurityEnforcementDecision,
}

fn blocked_decision(reason: String) -> SecurityEnforcementDecision {
    SecurityEnforcementDecision {
        action: SecurityEnforcementAction::Block,
        rule_id: None,
        rule_name: None,
        reason: Some(reason),
        ask_id: None,
    }
}

fn evaluate_builtin_http_request(
    url: &str,
    method: &str,
    security_rules: &SecurityRuleSet,
    plugin_policy: &BTreeMap<String, SecurityPluginConfig>,
) -> Result<BuiltinHttpDecision, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("only http:// and https:// URLs are supported (got {other}://)")),
    }
    let domain = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?
        .to_string();
    let mut event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_http(HttpSecurityEvent {
            host: Some(domain.clone()),
            method: Some(method.to_string()),
            path: Some(parsed.path().to_string()),
            query: parsed.query().map(str::to_string),
            status: None,
            body: None,
        })
        .with_http_request(HttpRequestSecurityEvent::new(
            domain.clone(),
            None,
            http::HeaderMap::new(),
            parsed.query().map(str::to_string),
        ));
    if let Some(trace_id) = capsem_foundation::telemetry::ambient_capsem_trace_id() {
        event = event.with_trace_id(trace_id);
    }
    if let Some(port) = parsed.port_or_known_default() {
        event = event.with_tcp(TcpSecurityEvent {
            port: Some(port.to_string()),
        });
    }
    if let Ok(ip) = domain.parse::<IpAddr>() {
        event = event.with_ip(IpSecurityEvent {
            value: Some(ip.to_string()),
            version: Some(match ip {
                IpAddr::V4(_) => "4".to_string(),
                IpAddr::V6(_) => "6".to_string(),
            }),
        });
    }
    let evaluated = evaluate_security_boundary(security_rules, plugin_policy.clone(), event)
        .map_err(|error| format!("security engine failed: {error}"))?;
    if !evaluated.enforcement.is_allowed() {
        let reason = evaluated
            .enforcement
            .reason
            .as_deref()
            .unwrap_or("security rule blocked request");
        return Err(format!("HTTP request blocked: {domain} ({reason})"));
    }
    Ok(BuiltinHttpDecision {
        domain,
        decision: evaluated.enforcement,
    })
}

/// Extract visible text from HTML using scraper (html5ever).
/// Skips script, style, noscript, svg, and template elements.
/// Inserts newlines around block elements.
pub fn extract_text_from_html(html: &str) -> String {
    use scraper::Html;

    let doc = Html::parse_document(html);
    let mut output = String::new();
    let root = doc.root_element();

    // Prefer <body> if present, otherwise use the root
    let start = scraper::Selector::parse("body")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|el| el.id())
        .unwrap_or_else(|| root.id());

    extract_text_recursive_scraper(&doc, start, &mut output);
    collapse_whitespace(&output)
}

/// Convert HTML to markdown, preserving headings, links, lists, bold/italic,
/// code blocks, and blockquotes.
pub fn extract_markdown_from_html(html: &str) -> String {
    use scraper::Html;

    let doc = Html::parse_document(html);
    let mut output = String::new();
    let root = doc.root_element();

    let start = scraper::Selector::parse("body")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|el| el.id())
        .unwrap_or_else(|| root.id());

    extract_md_recursive(&doc, start, &mut output);
    collapse_whitespace(&output)
}

const SKIP_TAGS: &[&str] = &["script", "style", "noscript", "svg", "template"];
const BLOCK_TAGS: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "tr",
    "br",
    "hr",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "main",
    "blockquote",
    "pre",
    "table",
    "ul",
    "ol",
    "dl",
    "dt",
    "dd",
    "figcaption",
    "figure",
    "details",
    "summary",
];

fn extract_text_recursive_scraper(doc: &scraper::Html, node_id: ego_tree::NodeId, output: &mut String) {
    let node_ref = match doc.tree.get(node_id) {
        Some(n) => n,
        None => return,
    };

    match node_ref.value() {
        scraper::Node::Text(text) => {
            output.push_str(text);
        }
        scraper::Node::Element(el) => {
            let tag = el.name.local.as_ref();
            if SKIP_TAGS.contains(&tag) {
                return;
            }
            let is_block = BLOCK_TAGS.contains(&tag);
            if is_block {
                output.push('\n');
            }
            for child in node_ref.children() {
                extract_text_recursive_scraper(doc, child.id(), output);
            }
            if is_block {
                output.push('\n');
            }
        }
        scraper::Node::Document => {
            for child in node_ref.children() {
                extract_text_recursive_scraper(doc, child.id(), output);
            }
        }
        _ => {}
    }
}

fn extract_md_recursive(doc: &scraper::Html, node_id: ego_tree::NodeId, output: &mut String) {
    let node_ref = match doc.tree.get(node_id) {
        Some(n) => n,
        None => return,
    };

    match node_ref.value() {
        scraper::Node::Text(text) => {
            output.push_str(text);
        }
        scraper::Node::Element(el) => {
            let tag = el.name.local.as_ref();
            if SKIP_TAGS.contains(&tag) {
                return;
            }

            match tag {
                "h1" => {
                    output.push_str("\n# ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "h2" => {
                    output.push_str("\n## ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "h3" => {
                    output.push_str("\n### ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "h4" => {
                    output.push_str("\n#### ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "h5" => {
                    output.push_str("\n##### ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "h6" => {
                    output.push_str("\n###### ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "a" => {
                    let href = el.attr("href").unwrap_or("");
                    output.push('[');
                    md_children(doc, node_ref, output);
                    output.push_str("](");
                    output.push_str(href);
                    output.push(')');
                }
                "strong" | "b" => {
                    output.push_str("**");
                    md_children(doc, node_ref, output);
                    output.push_str("**");
                }
                "em" | "i" => {
                    output.push('_');
                    md_children(doc, node_ref, output);
                    output.push('_');
                }
                "code" => {
                    output.push('`');
                    md_children(doc, node_ref, output);
                    output.push('`');
                }
                "pre" => {
                    output.push_str("\n```\n");
                    md_children(doc, node_ref, output);
                    output.push_str("\n```\n");
                }
                "blockquote" => {
                    output.push_str("\n> ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "li" => {
                    // Check parent to decide bullet vs number
                    if let Some(parent) = node_ref.parent() {
                        if let scraper::Node::Element(pel) = parent.value() {
                            if pel.name.local.as_ref() == "ol" {
                                // Find our index among siblings
                                let idx = parent.children()
                                    .filter(|c| matches!(c.value(), scraper::Node::Element(e) if e.name.local.as_ref() == "li"))
                                    .position(|c| c.id() == node_id)
                                    .unwrap_or(0);
                                output.push_str(&format!("\n{}. ", idx + 1));
                            } else {
                                output.push_str("\n- ");
                            }
                        } else {
                            output.push_str("\n- ");
                        }
                    } else {
                        output.push_str("\n- ");
                    }
                    md_children(doc, node_ref, output);
                }
                "br" => {
                    output.push('\n');
                }
                "hr" => {
                    output.push_str("\n---\n");
                }
                "img" => {
                    let alt = el.attr("alt").unwrap_or("");
                    if !alt.is_empty() {
                        output.push_str(&format!("[image: {alt}]"));
                    }
                }
                _ => {
                    let is_block = BLOCK_TAGS.contains(&tag);
                    if is_block {
                        output.push('\n');
                    }
                    md_children(doc, node_ref, output);
                    if is_block {
                        output.push('\n');
                    }
                }
            }
        }
        scraper::Node::Document => {
            for child in node_ref.children() {
                extract_md_recursive(doc, child.id(), output);
            }
        }
        _ => {}
    }
}

fn md_children(doc: &scraper::Html, node_ref: ego_tree::NodeRef<scraper::Node>, output: &mut String) {
    for child in node_ref.children() {
        extract_md_recursive(doc, child.id(), output);
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
/// Uses `floor_char_boundary` to avoid panicking on multi-byte UTF-8.
pub fn paginate(text: &str, start: usize, max: usize) -> (String, usize, bool) {
    let total = text.len();
    let safe_start = text.floor_char_boundary(start.min(total));
    if safe_start >= total {
        return (String::new(), total, false);
    }
    let safe_end = text.floor_char_boundary((safe_start + max).min(total));
    let chunk = &text[safe_start..safe_end];
    (chunk.to_string(), total, safe_end < total)
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
mod tests;
