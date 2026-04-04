# Browser Tools Reference

## PlaywrightServer

The Node.js server is embedded in `playwright_server.js`. On startup:

1. Rust writes it to a temp file with UUID name
2. Node.js process starts: `node <temp_file>`
3. Server listens on random port on 127.0.0.1
4. Server prints `ws://127.0.0.1:<port>` to stdout
5. Rust parses the URL and converts ws:// to http:// for requests

The HTTP API is a single POST endpoint at `/execute`:

```json
// Request
{"action": "navigate", "params": {"url": "https://example.com"}}

// Response (success)
{"result": {"url": "https://example.com", "title": "Example", "status": 200}}

// Response (error)
{"error": "Navigation timeout of 30000ms exceeded"}
```

## Tool Schemas

### browser_navigate

```json
{
  "url": "string (required) - must include http:// or https://",
  "timeout": "integer (optional, default 30000)",
  "wait_until": "string (optional, default 'load') - 'load' | 'domcontentloaded' | 'networkidle'"
}
```

Returns: URL, final URL (after redirects), page title, HTTP status.

### browser_click

```json
{
  "selector": "string (required) - CSS selector",
  "timeout": "integer (optional, default 5000)"
}
```

### browser_type

```json
{
  "selector": "string (required) - CSS selector for input",
  "text": "string (required)",
  "clear_first": "boolean (optional, default true)"
}
```

Special keys: `{Enter}`, `{Tab}`, `{Backspace}`, `{ArrowLeft}`, etc.

### browser_screenshot

```json
{
  "selector": "string (optional) - crop to element",
  "full_page": "boolean (optional, default false)",
  "max_width": "integer (optional, default 1280)"
}
```

Returns base64 PNG with dimensions. The response format is:

```
Screenshot captured
Width: 1280 px
Format: png
Size: 234567 bytes

data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAA...
```

### browser_evaluate

```json
{
  "javascript": "string (required) - JS code to run in page context",
  "timeout": "integer (optional, default 10000)"
}
```

Can access document, window, DOM APIs. Result is serialized to JSON.

### browser_get_text

```json
{
  "selector": "string (required) - CSS selector",
  "max_length": "integer (optional, default 5000)"
}
```

Returns text from all matching elements, joined with double newlines. Truncates if longer than max_length.

### browser_fill_form

```json
{
  "fields": "array (required) - [{selector, value}, ...]"
}
```

Each field needs both selector and value. Fails on first error.

### browser_get_content

```json
{
  "selector": "string (optional) - CSS selector, defaults to full page",
  "format": "string (optional, default 'text') - 'html' | 'text'",
  "max_length": "integer (optional, default 5000)"
}
```

### browser_close

No parameters. Safe to call even if browser isn't initialized.

## Telemetry

All calls go to `mcp_calls` table:

| Column | Value |
|--------|-------|
| server_name | "browser" |
| method | "tools/call" |
| tool_name | "browser_navigate", etc |
| decision | "allowed" or "error" |
| process_name | "browser" |
| bytes_sent | request JSON size |
| bytes_received | response JSON size |

Request/response previews are capped at 256KB by the logger.

## Error Codes

| Code | When |
|------|------|
| -32602 | Missing required parameter |
| -32603 | Playwright execution failure |

## Common Issues

**"browser tools unavailable"** - browser_manager wasn't passed to gateway config. Check that the VM boot code initializes it.

**"Failed to start Playwright server"** - Node.js or Playwright not installed. Run `npx playwright install`.

**"Navigation timeout"** - Page didn't load in time. Increase timeout parameter or check if the URL is reachable.

**"Element not found"** - CSS selector doesn't match anything. Could be timing issue (page hasn't fully loaded) or wrong selector.

## Temp File Handling

The JS script is written to `<temp_dir>/capsem_playwright_<uuid>.js`. A background tokio task cleans it up after 60s. The file is also removed on startup failure or when the PlaywrightServer drops.
