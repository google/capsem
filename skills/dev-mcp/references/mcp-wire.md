# MCP Wire Format

Source: `crates/capsem-core/src/mcp/types.rs`, `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs`, and `crates/capsem-core/src/net/mitm_proxy/mcp_endpoint.rs`.

## Transport

Framed MCP over vsock (AF_VSOCK stream socket, port 5002). Each frame is length-prefixed and contains one JSON-RPC 2.0 payload plus stream id, flags, and process attribution. Payloads are bounded.

## Connection setup

1. Guest connects to `vsock://2:5002` (CID=2 is host)
2. Sends metadata: `\0CAPSEM_META:{process_name}\n` (NUL-prefixed)
3. Sends/receives bounded MCP frames from here

Vsock I/O: EINTR retried, EAGAIN fatal. Arbitrary user work is controlled by MCP method timeouts, not by a hidden command watchdog.

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
    pub namespaced_name: String,     // "github__search" (endpoint-facing)
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
3. Route: local builtin or external server through `AggregatorClient`
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

Every MCP-origin `tools/call` is logged to the canonical `tool_calls` table with
`origin = 'mcp'`, `server_name`, `method`, `request_id`, policy fields, byte
counts, and trace id. Non-tool protocol frames are typed security events and
rule-ledger rows.

Decision logic: policy block -> "denied", error -> "error", success -> "allowed".

## W5: optional `_meta` envelope on JSON-RPC

JsonRpcRequest and JsonRpcResponse can carry an optional `_meta` object
with W3C Trace Context fields:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": { ... },
  "_meta": {
    "traceparent": "00-<32hex>-<16hex>-01",
    "tracestate": ""
  }
}
```

All `_meta` fields are optional with serde defaults. Third-party MCP
clients and pre-W5 capsem peers round-trip cleanly. The endpoint echoes
the same envelope back so callers can cross-check.

The vsock control bridge's `BootConfig` message (host->guest, first
frame after Ready) gained a parallel `traceparent: String` field with
the same optional semantics. Empty string means "no parent context".

## rmcp integration

External MCP servers are called via `StreamableHttpClientTransport` (HTTP + SSE). The server manager:
1. Maintains client pool
2. Queries each server's tools/resources/prompts on startup
3. Namespaces all tools
4. Routes by parsing namespace from tool name
5. Bearer token auth, custom headers from server config
