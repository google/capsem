# MCP Wire Format

Source: `crates/capsem-core/src/mcp/types.rs` (471 lines), `gateway.rs` (600+ lines)

## Transport

NDJSON over vsock (AF_VSOCK stream socket, port 5003). One JSON-RPC 2.0 message per line, max 1MB.

## Connection setup

1. Guest connects to `vsock://2:5003` (CID=2 is host)
2. Sends metadata: `\0CAPSEM_META:{process_name}\n` (NUL-prefixed)
3. Bidirectional JSON-RPC from here

Vsock I/O: 30s send/recv timeouts, EINTR retried, EAGAIN fatal.

## JSON-RPC 2.0

### Request

```rust
pub struct JsonRpcRequest {
    pub jsonrpc: String,                 // "2.0"
    pub id: Option<serde_json::Value>,   // number or string, omitted for notifications
    pub method: String,
    pub params: Option<serde_json::Value>,
}
```

### Response

```rust
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,  // XOR with error
    pub error: Option<JsonRpcError>,
}

pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}
```

### Error codes

| Code | Meaning |
|------|---------|
| -32700 | Parse error |
| -32600 | Invalid request (blocked by policy, missing tool name) |
| -32601 | Method not found |
| -32602 | Invalid params |
| -32603 | Internal error (tool call failed) |

## Tool definitions

```rust
pub struct McpToolDef {
    pub namespaced_name: String,     // "github__search" (gateway-facing)
    pub original_name: String,       // "search" (sent to actual server)
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
    pub server_name: String,
    pub annotations: Option<ToolAnnotations>,
}
```

### Tool annotations (camelCase on wire)

```rust
pub struct ToolAnnotations {
    pub title: Option<String>,
    pub read_only_hint: bool,       // wire: "readOnlyHint"
    pub destructive_hint: bool,     // wire: "destructiveHint"
    pub idempotent_hint: bool,      // wire: "idempotentHint"
    pub open_world_hint: bool,      // wire: "openWorldHint"
}
```

## tools/list response

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "tools": [
      {
        "name": "github__search_repos",
        "description": "Search GitHub repositories",
        "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}},
        "annotations": {"readOnlyHint": true, "openWorldHint": true}
      },
      {
        "name": "builtin__http_get",
        "description": "HTTP GET request",
        "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}}}
      }
    ]
  }
}
```

## tools/call request

```json
{
  "jsonrpc": "2.0",
  "id": 42,
  "method": "tools/call",
  "params": {
    "name": "github__search_repos",
    "arguments": {"q": "rust async"}
  }
}
```

### Routing flow

1. Parse `params.name` -> extract namespace (`github`) and original name (`search_repos`)
2. Policy check: `policy.evaluate("github", "search_repos")`
3. Route: builtin -> `builtin_tools::call_builtin_tool()`, external -> `peer.call_tool()` via rmcp
4. Response or error

## tools/call response (success)

```json
{
  "jsonrpc": "2.0",
  "id": 42,
  "result": {
    "content": [
      {"type": "text", "text": "Found 42 repositories matching 'rust async'..."}
    ]
  }
}
```

## tools/call response (error)

```json
{
  "jsonrpc": "2.0",
  "id": 42,
  "error": {
    "code": -32600,
    "message": "tool 'github__search_repos' blocked by policy"
  }
}
```

## resources/read request

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "resources/read",
  "params": {"uri": "file:///path/to/resource"}
}
```

## Telemetry

Every request logged to `mcp_calls` table:

```sql
CREATE TABLE mcp_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    server_name TEXT NOT NULL,
    method TEXT NOT NULL,
    tool_name TEXT,
    request_id TEXT,
    request_preview TEXT,     -- first 256KB
    response_preview TEXT,    -- first 256KB
    decision TEXT NOT NULL,   -- "allowed", "warned", "denied", "error"
    duration_ms INTEGER DEFAULT 0,
    error_message TEXT,
    process_name TEXT,
    bytes_sent INTEGER DEFAULT 0,
    bytes_received INTEGER DEFAULT 0
);
```

Decision logic: policy block -> "denied", error -> "error", success -> "allowed".

## rmcp integration

External MCP servers are called via `StreamableHttpClientTransport` (HTTP + SSE). The server manager:
1. Maintains client pool
2. Queries each server's tools/resources/prompts on startup
3. Namespaces all tools
4. Routes by parsing namespace from tool name
5. Bearer token auth, custom headers from server config
