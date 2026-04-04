//! Headless browser MCP tools for AI agent web automation.
//!
//! Provides browser automation capabilities via Playwright, enabling
//! AI agents running inside the VM to interact with web pages through
//! MCP tools. The browser runs on the host for security and performance.
//!
//! Tools provided:
//! - `browser_navigate`: Navigate to a URL
//! - `browser_click`: Click an element by CSS selector
//! - `browser_type`: Type text into an input field
//! - `browser_screenshot`: Capture a screenshot of the current page
//! - `browser_evaluate`: Execute JavaScript and return the result
//! - `browser_get_text`: Extract text content from an element
//! - `browser_fill_form`: Fill multiple form fields at once
//! - `browser_get_content`: Get the page HTML or text content
//! - `browser_close`: Close the browser session

use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use capsem_logger::{DbWriter, WriteOp};

use super::builtin_tools::DEFAULT_MAX_LENGTH;
use super::types::{JsonRpcResponse, McpToolDef, ToolAnnotations};

/// Browser tool names (without namespace prefix).
const BROWSER_TOOL_NAMES: &[&str] = &[
    "browser_navigate",
    "browser_click",
    "browser_type",
    "browser_screenshot",
    "browser_evaluate",
    "browser_get_text",
    "browser_fill_form",
    "browser_get_content",
    "browser_close",
];

/// Check if a tool name is a browser tool.
pub fn is_browser_tool(name: &str) -> bool {
    BROWSER_TOOL_NAMES.contains(&name)
}

/// Playwright server process wrapper.
struct PlaywrightServer {
    /// Child process handle
    child: tokio::process::Child,
    /// WebSocket endpoint URL
    ws_endpoint: String,
    /// HTTP client for sending commands to Playwright server
    http_client: reqwest::Client,
}

impl PlaywrightServer {
    /// Start a new Playwright server process.
    async fn start() -> Result<Self, String> {
        // Write the embedded JS to a temp file
        let script_content = include_str!("playwright_server.js");
        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join(format!("capsem_playwright_{}.js", uuid::Uuid::new_v4()));

        tokio::fs::write(&script_path, script_content)
            .await
            .map_err(|e| format!("Failed to write Playwright script: {}", e))?;

        let script_path_clone = script_path.clone();

        // Start Node.js Playwright server subprocess
        let mut child = Command::new("node")
            .arg(&script_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                let _ = std::fs::remove_file(&script_path);
                format!("Failed to start Playwright server: {}. Ensure Node.js and Playwright are installed (run 'npx playwright install').", e)
            })?;

        // Read stdout to get WebSocket endpoint
        let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
        let mut reader = BufReader::new(stdout);

        let mut ws_endpoint = String::new();
        let mut error_output = String::new();

        // Wait for WebSocket endpoint URL with timeout
        let timeout = Duration::from_secs(30);
        let start = Instant::now();

        loop {
            if start.elapsed() > timeout {
                let _ = child.kill().await;
                let _ = std::fs::remove_file(&script_path);
                return Err("Playwright server startup timeout (30s)".to_string());
            }

            // Try reading a line with timeout
            let mut line = String::new();
            match tokio::time::timeout(Duration::from_millis(100), reader.read_line(&mut line))
                .await
            {
                Ok(Ok(n)) => {
                    if n > 0 {
                        let trimmed = line.trim();
                        if trimmed.starts_with("ws://") {
                            ws_endpoint = trimmed.to_string();
                            break;
                        }
                        // Collect stderr output for error messages
                        error_output.push_str(trimmed);
                        error_output.push('\n');
                    } else {
                        // EOF - process exited
                        let status = child.try_wait().map_err(|e| {
                            let _ = std::fs::remove_file(&script_path);
                            format!("Failed to check status: {}", e)
                        })?;
                        if let Some(status) = status {
                            let _ = std::fs::remove_file(&script_path);
                            return Err(format!(
                                "Playwright server exited with status: {}\nError output:\n{}",
                                status, error_output
                            ));
                        }
                    }
                }
                Ok(Err(e)) => {
                    let _ = std::fs::remove_file(&script_path);
                    return Err(format!("Failed to read stdout: {}", e));
                }
                Err(_) => continue, // Timeout on this iteration, retry
            }
        }

        let http_client = reqwest::Client::new();

        // Spawn a background task to clean up the temp file when process exits
        let cleanup_path = script_path_clone.clone();
        tokio::spawn(async move {
            // Wait for child process to exit, then cleanup
            // (we can't access child here, so just schedule cleanup)
            tokio::time::sleep(Duration::from_secs(60)).await;
            let _ = tokio::fs::remove_file(&cleanup_path).await;
        });

        Ok(Self {
            child,
            ws_endpoint,
            http_client,
        })
    }

    /// Execute a Playwright command via HTTP API.
    async fn execute_command(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let payload = serde_json::json!({
            "action": action,
            "params": params
        });

        // Convert ws:// to http:// for HTTP requests
        let url = format!("{}/execute", self.ws_endpoint.replace("ws://", "http://"));

        let resp = self
            .http_client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Playwright server error: {}\n{}", status, body));
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        if let Some(error) = result.get("error") {
            return Err(error.as_str().unwrap_or("Unknown error").to_string());
        }

        result
            .get("result")
            .cloned()
            .ok_or_else(|| "No result in response".to_string())
    }

    /// Shutdown the Playwright server process and cleanup temp file.
    async fn shutdown(&mut self) {
        if let Err(e) = self.child.kill().await {
            tracing::warn!(error = %e, "Failed to kill Playwright server process");
        }
        // Temp file will be cleaned up by background task or OS
    }
}

impl Drop for PlaywrightServer {
    fn drop(&mut self) {
        // Ensure process is cleaned up on drop
        let _ = self.child.start_kill();
    }
}

/// Browser session state.
pub struct BrowserSession {
    /// Playwright server instance
    server: Option<PlaywrightServer>,
    /// Session created timestamp
    created_at: Instant,
}

impl BrowserSession {
    /// Create a new browser session (lazily initializes Playwright).
    pub async fn new() -> Result<Self, String> {
        Ok(Self {
            server: None,
            created_at: Instant::now(),
        })
    }

    /// Get or initialize the Playwright server.
    pub async fn get_server(&mut self) -> Result<&PlaywrightServer, String> {
        if self.server.is_none() {
            let server = PlaywrightServer::start().await?;
            self.server = Some(server);
        }
        Ok(self.server.as_ref().unwrap())
    }

    /// Get mutable reference to server for shutdown.
    pub async fn get_server_mut(&mut self) -> Result<&mut PlaywrightServer, String> {
        if self.server.is_none() {
            let server = PlaywrightServer::start().await?;
            self.server = Some(server);
        }
        Ok(self.server.as_mut().unwrap())
    }

    /// Check if browser is initialized.
    pub fn is_initialized(&self) -> bool {
        self.server.is_some()
    }

    /// Shutdown the browser session.
    pub async fn shutdown(&mut self) {
        if let Some(ref mut server) = self.server {
            server.shutdown().await;
        }
    }
}

/// Global browser session manager.
pub struct BrowserManager {
    /// Current active session (if any)
    session: Mutex<Option<BrowserSession>>,
    /// Logger for telemetry
    db: Arc<DbWriter>,
}

impl BrowserManager {
    /// Create a new browser manager.
    pub fn new(db: Arc<DbWriter>) -> Self {
        Self {
            session: Mutex::new(None),
            db,
        }
    }

    /// Get server reference, initializing session if needed.
    async fn get_server(&self) -> Result<&PlaywrightServer, String> {
        let mut session_lock = self.session.lock().await;
        if session_lock.is_none() {
            *session_lock = Some(BrowserSession::new().await?);
        }
        let session = session_lock.as_mut().unwrap();
        session.get_server().await
    }

    /// Execute a command on the browser.
    pub async fn execute_command(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        // We need mutable access to session for get_server_mut
        drop(self.session.lock().await); // Release previous lock
        let mut session_lock = self.session.lock().await;
        if session_lock.is_none() {
            *session_lock = Some(BrowserSession::new().await?);
        }
        let session = session_lock.as_mut().unwrap();
        let server = session.get_server().await?;
        server.execute_command(action, params).await
    }

    /// Close current browser session.
    pub async fn close_session(&self) {
        let mut session_lock = self.session.lock().await;
        if let Some(ref mut session) = *session_lock {
            session.shutdown().await;
        }
        *session_lock = None;
    }
}

/// Return browser tool definitions.
pub fn browser_tool_defs() -> Vec<McpToolDef> {
    vec![
        McpToolDef {
            namespaced_name: "browser_navigate".into(),
            original_name: "browser_navigate".into(),
            description: Some(concat!(
                "Navigate the browser to a URL. ",
                "Opens the specified URL in a headless browser and waits for the page to load. ",
                "Returns the page title, final URL (after redirects), and a summary of the page content. ",
                "Use this before other browser tools to load a page. ",
                "Errors: invalid URL, navigation timeout, page load failure."
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to navigate to. Must include http:// or https:// protocol."
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Navigation timeout in milliseconds (default: 30000). Increase for slow pages."
                    },
                    "wait_until": {
                        "type": "string",
                        "enum": ["load", "domcontentloaded", "networkidle"],
                        "description": "When to consider navigation succeeded (default: 'load'). 'load': wait for load event. 'domcontentloaded': wait for DOMContentLoaded. 'networkidle': wait until no network connections for 500ms."
                    }
                },
                "required": ["url"]
            }),
            server_name: "browser".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Browser Navigate".into()),
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
                open_world_hint: true,
            }),
        },
        McpToolDef {
            namespaced_name: "browser_click".into(),
            original_name: "browser_click".into(),
            description: Some(concat!(
                "Click an element on the page by CSS selector. ",
                "Finds the element matching the selector and performs a click action. ",
                "Useful for clicking links, buttons, form elements, and interactive elements. ",
                "Common selectors: 'a[href=\"/path\"]', 'button:has-text(\"Submit\")', '#submit-btn', '.nav-item'. ",
                "Returns success message or error if element not found. ",
                "Errors: element not found, element not visible, click intercepted."
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for the element to click. Examples: 'button', '#my-id', '.my-class', 'a[href=\"/link\"]'."
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in milliseconds to wait for element to appear (default: 5000)."
                    }
                },
                "required": ["selector"]
            }),
            server_name: "browser".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Browser Click".into()),
                read_only_hint: false,
                destructive_hint: false,
                idempotent_hint: false,
                open_world_hint: true,
            }),
        },
        McpToolDef {
            namespaced_name: "browser_type".into(),
            original_name: "browser_type".into(),
            description: Some(concat!(
                "Type text into an input field on the page. ",
                "Finds the input element by CSS selector and types the specified text. ",
                "Works with <input>, <textarea>, and [contenteditable] elements. ",
                "Supports special keys using caret notation: 'Enter', 'Tab', 'Backspace', 'ArrowLeft', etc. ",
                "Example: 'Hello World' or 'username{tab}password{enter}'. ",
                "Returns success message or error if element not found. ",
                "Errors: element not found, element not an input, type failure."
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for the input element. Examples: 'input[name=\"username\"]', '#search-box', 'textarea'."
                    },
                    "text": {
                        "type": "string",
                        "description": "Text to type into the input. Use {Enter}, {Tab}, etc. for special keys."
                    },
                    "clear_first": {
                        "type": "boolean",
                        "description": "Whether to clear existing text before typing (default: true)."
                    }
                },
                "required": ["selector", "text"]
            }),
            server_name: "browser".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Browser Type".into()),
                read_only_hint: false,
                destructive_hint: false,
                idempotent_hint: false,
                open_world_hint: true,
            }),
        },
        McpToolDef {
            namespaced_name: "browser_screenshot".into(),
            original_name: "browser_screenshot".into(),
            description: Some(concat!(
                "Capture a screenshot of the current page or a specific element. ",
                "Returns a base64-encoded PNG image that can be displayed or analyzed. ",
                "Optionally capture only a specific element using a CSS selector. ",
                "Useful for visual verification, debugging layouts, or reading content. ",
                "Returns base64 PNG data with dimensions. ",
                "Errors: page not loaded, element not found, screenshot failure."
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for a specific element to screenshot. If omitted, captures the full page."
                    },
                    "full_page": {
                        "type": "boolean",
                        "description": "Capture full scrollable page instead of viewport (default: false)."
                    },
                    "max_width": {
                        "type": "integer",
                        "description": "Maximum width in pixels (default: 1280). Scales down if larger."
                    }
                }
            }),
            server_name: "browser".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Browser Screenshot".into()),
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
                open_world_hint: false,
            }),
        },
        McpToolDef {
            namespaced_name: "browser_evaluate".into(),
            original_name: "browser_evaluate".into(),
            description: Some(concat!(
                "Execute JavaScript code in the browser context and return the result. ",
                "The code runs in the page's JavaScript environment with access to the DOM. ",
                "Can return primitive values, objects, or DOM element references. ",
                "Example: 'document.title' or 'document.querySelectorAll(\"a\").length'. ",
                "Returns the serialized result as JSON. ",
                "Errors: JavaScript syntax error, execution timeout, page not loaded."
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "javascript": {
                        "type": "string",
                        "description": "JavaScript code to execute. Can access document, window, and DOM APIs. Example: 'document.title' or 'Array.from(document.querySelectorAll(\"h2\")).map(h => h.textContent)'."
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Execution timeout in milliseconds (default: 10000)."
                    }
                },
                "required": ["javascript"]
            }),
            server_name: "browser".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Browser Evaluate".into()),
                read_only_hint: false,
                destructive_hint: true,
                idempotent_hint: false,
                open_world_hint: true,
            }),
        },
        McpToolDef {
            namespaced_name: "browser_get_text".into(),
            original_name: "browser_get_text".into(),
            description: Some(concat!(
                "Extract text content from elements matching a CSS selector. ",
                "Returns the visible text from all matching elements, trimmed and cleaned. ",
                "Useful for reading article content, form labels, navigation items, etc. ",
                "Example selector: 'h1', '.article-body', 'nav a', '#content p'. ",
                "Returns text with one entry per matching element. ",
                "Errors: element not found, page not loaded, extraction failure."
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for elements to extract text from. Examples: 'h1', '.content', 'p', '#main-text'."
                    },
                    "max_length": {
                        "type": "integer",
                        "description": "Maximum characters to return (default: 5000). Truncates if longer."
                    }
                },
                "required": ["selector"]
            }),
            server_name: "browser".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Browser Get Text".into()),
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
                open_world_hint: false,
            }),
        },
        McpToolDef {
            namespaced_name: "browser_fill_form".into(),
            original_name: "browser_fill_form".into(),
            description: Some(concat!(
                "Fill multiple form fields at once. ",
                "Finds each input by its CSS selector and sets its value. ",
                "More efficient than calling browser_type multiple times for forms. ",
                "Supports text inputs, textareas, selects, checkboxes, and radio buttons. ",
                "Each field is an object with 'selector' and 'value' keys. ",
                "Example: [{\"selector\": \"#username\", \"value\": \"user\"}, {\"selector\": \"#password\", \"value\": \"pass\"}]. ",
                "Returns success message with count of fields filled or error on first failure. ",
                "Errors: field not found, invalid field type, fill failure."
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fields": {
                        "type": "array",
                        "description": "Array of field objects, each with 'selector' and 'value' properties.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "selector": {
                                    "type": "string",
                                    "description": "CSS selector for the input element."
                                },
                                "value": {
                                    "type": "string",
                                    "description": "Value to set in the field."
                                }
                            },
                            "required": ["selector", "value"]
                        }
                    }
                },
                "required": ["fields"]
            }),
            server_name: "browser".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Browser Fill Form".into()),
                read_only_hint: false,
                destructive_hint: false,
                idempotent_hint: false,
                open_world_hint: true,
            }),
        },
        McpToolDef {
            namespaced_name: "browser_get_content".into(),
            original_name: "browser_get_content".into(),
            description: Some(concat!(
                "Get the page content as HTML or text. ",
                "Returns either the full page HTML, or cleaned text content. ",
                "Useful for scraping, verifying content, or extracting data. ",
                "Optionally target a specific element with a CSS selector. ",
                "Returns content with metadata (URL, content length). ",
                "Errors: page not loaded, selector not found, extraction failure."
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for a specific element. If omitted, returns full page content."
                    },
                    "format": {
                        "type": "string",
                        "enum": ["html", "text"],
                        "description": "Output format: 'html' returns the HTML markup, 'text' (default) returns visible text only."
                    },
                    "max_length": {
                        "type": "integer",
                        "description": "Maximum characters to return (default: 5000)."
                    }
                }
            }),
            server_name: "browser".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Browser Get Content".into()),
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
                open_world_hint: false,
            }),
        },
        McpToolDef {
            namespaced_name: "browser_close".into(),
            original_name: "browser_close".into(),
            description: Some(concat!(
                "Close the browser session and free resources. ",
                "Closes all pages and the browser instance. ",
                "Use when done with web automation to clean up. ",
                "Returns success message. Safe to call even if browser is not initialized. ",
                "No parameters required."
            ).into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            server_name: "browser".into(),
            annotations: Some(ToolAnnotations {
                title: Some("Browser Close".into()),
                read_only_hint: true,
                destructive_hint: true,
                idempotent_hint: true,
                open_world_hint: false,
            }),
        },
    ]
}

/// Dispatch a browser tool call by local name.
pub async fn call_browser_tool(
    local_name: &str,
    arguments: &Value,
    browser_manager: &Arc<BrowserManager>,
    request_id: Option<Value>,
    db: &Arc<DbWriter>,
) -> JsonRpcResponse {
    let start = Instant::now();
    let result = match local_name {
        "browser_navigate" => {
            handle_browser_navigate(arguments, browser_manager, request_id.clone()).await
        }
        "browser_click" => {
            handle_browser_click(arguments, browser_manager, request_id.clone()).await
        }
        "browser_type" => handle_browser_type(arguments, browser_manager, request_id.clone()).await,
        "browser_screenshot" => {
            handle_browser_screenshot(arguments, browser_manager, request_id.clone()).await
        }
        "browser_evaluate" => {
            handle_browser_evaluate(arguments, browser_manager, request_id.clone()).await
        }
        "browser_get_text" => {
            handle_browser_get_text(arguments, browser_manager, request_id.clone()).await
        }
        "browser_fill_form" => {
            handle_browser_fill_form(arguments, browser_manager, request_id.clone()).await
        }
        "browser_get_content" => {
            handle_browser_get_content(arguments, browser_manager, request_id.clone()).await
        }
        "browser_close" => handle_browser_close(browser_manager, request_id.clone()).await,
        _ => JsonRpcResponse::err(
            request_id.clone(),
            -32602,
            format!("unknown browser tool: {local_name}"),
        ),
    };

    // Log the browser call
    let duration_ms = start.elapsed().as_millis() as u64;
    log_browser_call(db, local_name, arguments, &result, duration_ms).await;

    result
}

/// Log a browser call to the session database.
async fn log_browser_call(
    db: &Arc<DbWriter>,
    tool_name: &str,
    arguments: &Value,
    result: &JsonRpcResponse,
    duration_ms: u64,
) {
    let request_preview = serde_json::to_string(arguments).ok();
    let resp_preview = result
        .result
        .as_ref()
        .and_then(|r| serde_json::to_string(r).ok());

    let bytes_sent = serde_json::to_vec(arguments)
        .ok()
        .map(|v| v.len() as u64)
        .unwrap_or(0);

    let bytes_received = result
        .result
        .as_ref()
        .and_then(|r| serde_json::to_vec(r).ok())
        .map(|v| v.len() as u64)
        .unwrap_or(0);

    db.write(WriteOp::McpCall(capsem_logger::McpCall {
        timestamp: std::time::SystemTime::now(),
        server_name: "browser".to_string(),
        method: "tools/call".to_string(),
        tool_name: Some(tool_name.to_string()),
        request_id: None,
        request_preview,
        response_preview: resp_preview,
        decision: if result.error.is_some() {
            "error"
        } else {
            "allowed"
        }
        .to_string(),
        duration_ms,
        error_message: result.error.as_ref().map(|e| e.message.clone()),
        process_name: Some("browser".to_string()),
        bytes_sent,
        bytes_received,
    }))
    .await;
}

// ---------------------------------------------------------------------------
// Browser tool handlers
// ---------------------------------------------------------------------------

async fn handle_browser_navigate(
    args: &Value,
    browser_manager: &Arc<BrowserManager>,
    id: Option<Value>,
) -> JsonRpcResponse {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return tool_error(id, "missing required parameter: url"),
    };

    if !url.starts_with("http://") && !url.starts_with("https://") {
        return tool_error(id, "URL must start with http:// or https://");
    }

    let timeout = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(30000);
    let wait_until = args
        .get("wait_until")
        .and_then(|v| v.as_str())
        .unwrap_or("load");

    match browser_manager
        .execute_command(
            "navigate",
            serde_json::json!({
                "url": url,
                "timeout": timeout,
                "waitUntil": wait_until,
            }),
        )
        .await
    {
        Ok(result) => {
            let output = format!(
                "Navigated to: {}\nFinal URL: {}\nTitle: {}\nStatus: {}",
                url,
                result.get("url").and_then(|v| v.as_str()).unwrap_or(url),
                result
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown"),
                result.get("status").and_then(|v| v.as_u64()).unwrap_or(0),
            );
            tool_ok(id, &output)
        }
        Err(e) => tool_error(id, &e),
    }
}

async fn handle_browser_click(
    args: &Value,
    browser_manager: &Arc<BrowserManager>,
    id: Option<Value>,
) -> JsonRpcResponse {
    let selector = match args.get("selector").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return tool_error(id, "missing required parameter: selector"),
    };
    let timeout = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(5000);

    match browser_manager
        .execute_command(
            "click",
            serde_json::json!({
                "selector": selector,
                "timeout": timeout,
            }),
        )
        .await
    {
        Ok(_) => tool_ok(id, &format!("Clicked: {}", selector)),
        Err(e) => tool_error(id, &e),
    }
}

async fn handle_browser_type(
    args: &Value,
    browser_manager: &Arc<BrowserManager>,
    id: Option<Value>,
) -> JsonRpcResponse {
    let selector = match args.get("selector").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return tool_error(id, "missing required parameter: selector"),
    };
    let text = match args.get("text").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return tool_error(id, "missing required parameter: text"),
    };
    let clear_first = args
        .get("clear_first")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    match browser_manager
        .execute_command(
            "type",
            serde_json::json!({
                "selector": selector,
                "text": text,
                "clearFirst": clear_first,
            }),
        )
        .await
    {
        Ok(_) => tool_ok(id, &format!("Typed '{}' into: {}", text, selector)),
        Err(e) => tool_error(id, &e),
    }
}

async fn handle_browser_screenshot(
    args: &Value,
    browser_manager: &Arc<BrowserManager>,
    id: Option<Value>,
) -> JsonRpcResponse {
    let selector = args.get("selector").and_then(|v| v.as_str());
    let full_page = args
        .get("full_page")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_width = args
        .get("max_width")
        .and_then(|v| v.as_u64())
        .unwrap_or(1280);

    let mut params = serde_json::json!({
        "fullPage": full_page,
        "maxWidth": max_width,
    });
    if let Some(sel) = selector {
        params["selector"] = serde_json::Value::String(sel.to_string());
    }

    match browser_manager.execute_command("screenshot", params).await {
        Ok(result) => {
            let image = result.get("image").and_then(|v| v.as_str()).unwrap_or("");
            let width = result.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
            let output = format!(
                "Screenshot captured\nWidth: {} px\nFormat: png\nSize: {} bytes\n\ndata:image/png;base64,{}",
                width,
                image.len(),
                image
            );
            tool_ok(id, &output)
        }
        Err(e) => tool_error(id, &e),
    }
}

async fn handle_browser_evaluate(
    args: &Value,
    browser_manager: &Arc<BrowserManager>,
    id: Option<Value>,
) -> JsonRpcResponse {
    let javascript = match args.get("javascript").and_then(|v| v.as_str()) {
        Some(j) => j,
        None => return tool_error(id, "missing required parameter: javascript"),
    };
    let timeout = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(10000);

    match browser_manager
        .execute_command(
            "evaluate",
            serde_json::json!({
                "javascript": javascript,
                "timeout": timeout,
            }),
        )
        .await
    {
        Ok(result) => {
            let output = format!(
                "Executed JavaScript: {}\n\nResult:\n{}",
                javascript,
                serde_json::to_string_pretty(
                    &result.get("result").unwrap_or(&serde_json::Value::Null)
                )
                .unwrap_or_default()
            );
            tool_ok(id, &output)
        }
        Err(e) => tool_error(id, &e),
    }
}

async fn handle_browser_get_text(
    args: &Value,
    browser_manager: &Arc<BrowserManager>,
    id: Option<Value>,
) -> JsonRpcResponse {
    let selector = match args.get("selector").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return tool_error(id, "missing required parameter: selector"),
    };
    let max_length = args
        .get("max_length")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_LENGTH);

    match browser_manager
        .execute_command(
            "getText",
            serde_json::json!({
                "selector": selector,
                "maxLength": max_length,
            }),
        )
        .await
    {
        Ok(result) => {
            let text = result.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let length = result.get("length").and_then(|v| v.as_u64()).unwrap_or(0);
            let elements = result
                .get("elementsFound")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let (chunk, total, has_more) = paginate(text, 0, max_length as usize);
            let mut output = format!(
                "Extracted text from: {}\nElements found: {}\nTotal length: {}\n",
                selector, elements, total
            );
            if has_more {
                output.push_str(&format!("Showing first {} characters\n", max_length));
            }
            output.push('\n');
            output.push_str(&chunk);

            tool_ok(id, &output)
        }
        Err(e) => tool_error(id, &e),
    }
}

async fn handle_browser_fill_form(
    args: &Value,
    browser_manager: &Arc<BrowserManager>,
    id: Option<Value>,
) -> JsonRpcResponse {
    let fields = match args.get("fields").and_then(|v| v.as_array()) {
        Some(f) => f,
        None => return tool_error(id, "missing required parameter: fields (must be an array)"),
    };

    // Validate fields
    for (i, field) in fields.iter().enumerate() {
        if field.get("selector").is_none() || field.get("value").is_none() {
            return tool_error(id, &format!("field {} missing 'selector' or 'value'", i));
        }
    }

    match browser_manager
        .execute_command(
            "fillForm",
            serde_json::json!({
                "fields": fields,
            }),
        )
        .await
    {
        Ok(_) => {
            let output = format!("Filled {} form fields", fields.len());
            tool_ok(id, &output)
        }
        Err(e) => tool_error(id, &e),
    }
}

async fn handle_browser_get_content(
    args: &Value,
    browser_manager: &Arc<BrowserManager>,
    id: Option<Value>,
) -> JsonRpcResponse {
    let format = args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("text");
    let max_length = args
        .get("max_length")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_LENGTH);
    let selector = args.get("selector").and_then(|v| v.as_str());

    let mut params = serde_json::json!({
        "format": format,
        "maxLength": max_length,
    });
    if let Some(sel) = selector {
        params["selector"] = serde_json::Value::String(sel.to_string());
    }

    match browser_manager.execute_command("getContent", params).await {
        Ok(result) => {
            let content = result.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let length = result.get("length").and_then(|v| v.as_u64()).unwrap_or(0);
            let fmt = result
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("text");

            let (chunk, total, has_more) = paginate(content, 0, max_length as usize);
            let mut output = format!("Page content\nFormat: {}\nLength: {}\n", fmt, total);
            if has_more {
                output.push_str(&format!("Showing first {} characters\n", max_length));
            }
            output.push('\n');
            output.push_str(&chunk);

            tool_ok(id, &output)
        }
        Err(e) => tool_error(id, &e),
    }
}

async fn handle_browser_close(
    browser_manager: &Arc<BrowserManager>,
    id: Option<Value>,
) -> JsonRpcResponse {
    browser_manager.close_session().await;
    tool_ok(id, "Browser session closed")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Paginate text content.
fn paginate(text: &str, start_index: usize, max_length: usize) -> (String, usize, bool) {
    let total = text.len();
    if start_index >= total {
        return (String::new(), total, false);
    }

    let end = (start_index + max_length).min(total);
    let has_more = end < total;
    let chunk = text[start_index..end].to_string();

    (chunk, total, has_more)
}

/// Create a successful tool response.
fn tool_ok(id: Option<Value>, output: &str) -> JsonRpcResponse {
    JsonRpcResponse::ok(
        id,
        serde_json::json!({
            "content": [
                {
                    "type": "text",
                    "text": output
                }
            ]
        }),
    )
}

/// Create an error tool response.
fn tool_error(id: Option<Value>, message: &str) -> JsonRpcResponse {
    JsonRpcResponse::err(id, -32603, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_browser_tool_basic() {
        assert!(is_browser_tool("browser_navigate"));
        assert!(is_browser_tool("browser_click"));
        assert!(is_browser_tool("browser_close"));
        assert!(!is_browser_tool("fetch_http"));
        assert!(!is_browser_tool("unknown"));
    }

    #[test]
    fn browser_tool_defs_count() {
        let defs = browser_tool_defs();
        assert_eq!(defs.len(), 9);
        assert!(defs.iter().any(|d| d.namespaced_name == "browser_navigate"));
        assert!(defs.iter().any(|d| d.namespaced_name == "browser_close"));
    }

    #[test]
    fn tool_ok_creates_valid_response() {
        let resp = tool_ok(Some(serde_json::json!(1)), "test output");
        assert!(resp.error.is_none());
        assert!(resp.result.is_some());
    }

    #[test]
    fn tool_error_creates_valid_response() {
        let resp = tool_error(Some(serde_json::json!(1)), "something went wrong");
        assert!(resp.error.is_some());
        let err = resp.error.as_ref().unwrap();
        assert_eq!(err.code, -32603);
        assert_eq!(err.message, "something went wrong");
    }

    #[test]
    fn paginate_basic() {
        let text = "Hello World";
        let (chunk, total, has_more) = paginate(text, 0, 5);
        assert_eq!(chunk, "Hello");
        assert_eq!(total, 11);
        assert!(has_more);
    }

    #[test]
    fn paginate_no_more() {
        let text = "Hello";
        let (chunk, total, has_more) = paginate(text, 0, 10);
        assert_eq!(chunk, "Hello");
        assert_eq!(total, 5);
        assert!(!has_more);
    }

    #[test]
    fn paginate_start_index_beyond() {
        let text = "Hello";
        let (chunk, total, has_more) = paginate(text, 10, 5);
        assert_eq!(chunk, "");
        assert_eq!(total, 5);
        assert!(!has_more);
    }

    #[test]
    fn browser_navigate_missing_url() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let db = rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.db");
            Arc::new(DbWriter::open(&path, 64).unwrap())
        });
        let browser_manager = Arc::new(BrowserManager::new(Arc::clone(&db)));

        let resp = rt.block_on(handle_browser_navigate(
            &serde_json::json!({}),
            &browser_manager,
            Some(serde_json::json!(1)),
        ));
        assert!(resp.error.is_some());
        assert!(resp
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("missing required parameter: url"));
    }

    #[test]
    fn browser_click_missing_selector() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let db = rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.db");
            Arc::new(DbWriter::open(&path, 64).unwrap())
        });
        let browser_manager = Arc::new(BrowserManager::new(Arc::clone(&db)));

        let resp = rt.block_on(handle_browser_click(
            &serde_json::json!({}),
            &browser_manager,
            Some(serde_json::json!(1)),
        ));
        assert!(resp.error.is_some());
    }

    #[test]
    fn browser_fill_form_invalid_fields() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let db = rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.db");
            Arc::new(DbWriter::open(&path, 64).unwrap())
        });
        let browser_manager = Arc::new(BrowserManager::new(Arc::clone(&db)));

        let resp = rt.block_on(handle_browser_fill_form(
            &serde_json::json!({"fields": "not_an_array"}),
            &browser_manager,
            Some(serde_json::json!(1)),
        ));
        assert!(resp.error.is_some());
        assert!(resp
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("fields (must be an array)"));
    }
}
