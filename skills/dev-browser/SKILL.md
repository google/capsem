---
name: dev-browser
description: Headless browser MCP tools. Covers the 9 browser tools (navigate, click, type, screenshot, etc), Playwright integration, and browser session management.
---

# Browser MCP Tools

AI agents in the guest VM can control a headless Chromium browser through MCP tools. The browser runs on the host, not in the VM.

## Architecture

```
Guest (Claude/Gemini) -> capsem-mcp-server -> vsock:5003 -> MCP Gateway
  -> BrowserManager -> PlaywrightServer (Node.js subprocess)
  -> Headless Chromium
  -> Telemetry -> session.db mcp_calls table
```

The Playwright server is a Node.js process that starts on first browser tool call. It outputs a ws:// endpoint on stdout that the Rust side parses to get the HTTP endpoint. Communication is via HTTP POST to `/execute`.

## Tools

| Tool | What it does |
|------|-------------|
| `browser_navigate` | Navigate to URL, returns title + final URL + status |
| `browser_click` | Click element by CSS selector |
| `browser_type` | Type text into input field |
| `browser_screenshot` | Capture screenshot as base64 PNG |
| `browser_evaluate` | Run JavaScript in page context |
| `browser_get_text` | Extract text from elements |
| `browser_fill_form` | Fill multiple form fields at once |
| `browser_get_content` | Get page HTML or text |
| `browser_close` | Close browser session |

All tools are namespaced under `browser` in telemetry. They show up in `tools/list` automatically when browser_manager is initialized.

## Usage

AI agent calls them like any other MCP tool:

```json
{"method": "tools/call", "params": {"name": "browser_navigate", "arguments": {"url": "https://example.com/login"}}}
{"method": "tools/call", "params": {"name": "browser_fill_form", "arguments": {"fields": [{"selector": "#user", "value": "admin"}, {"selector": "#pass", "value": "secret"}]}}}
{"method": "tools/call", "params": {"name": "browser_click", "arguments": {"selector": "button[type='submit']"}}}
{"method": "tools/call", "params": {"name": "browser_screenshot", "arguments": {}}}
```

## Setup

Browser tools need Node.js and Playwright on the host:

```bash
npx playwright install
```

If Playwright isn't installed, the first browser tool call will fail with a clear error message telling the user to run the install command.

## Key files

| File | What's in it |
|------|-------------|
| `crates/capsem-core/src/mcp/browser_tools.rs` | Tool defs, handlers, BrowserManager, PlaywrightServer |
| `crates/capsem-core/src/mcp/playwright_server.js` | Node.js server that wraps Playwright |
| `crates/capsem-core/src/mcp/gateway.rs` | Browser tool routing in tools/call |
| `skills/dev-browser/references/browser-wire.md` | Full wire format and schema reference |

## Testing

```bash
cargo test -p capsem-core mcp::browser_tools
```

For integration testing, boot a VM and have an AI agent call the browser tools. Check session.db afterward with `just inspect-session`.

## Notes

- Browser is lazily initialized (starts on first tool call)
- Session is stateful across tool calls (cookies, localStorage persist)
- Browser binds to 127.0.0.1 only
- All calls logged to mcp_calls table with server_name="browser"
- Temp JS file is written to temp dir with UUID name, cleaned up after 60s
