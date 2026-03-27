---
name: dev-mcp
description: MCP (Model Context Protocol) gateway development for Capsem. Use when working on the MCP gateway, tool routing, policy evaluation, server manager, built-in tools, or mcp_calls telemetry. Covers the NDJSON protocol over vsock, JSON-RPC 2.0 framing, tool namespacing, policy evaluation, and the rmcp client integration.
---

# MCP Gateway

The MCP gateway bridges AI agents in the guest VM to external MCP servers on the host. It runs over vsock port 5003 using NDJSON (one JSON-RPC 2.0 message per line).

## Architecture

```
Guest (Claude/Gemini) -> capsem-mcp-server (stdin/stdout relay)
  -> vsock:5003 -> MCP Gateway (capsem-core)
  -> Policy check -> Route to: builtin tools | external MCP servers (via rmcp)
  -> Telemetry -> session.db mcp_calls table
```

## Wire format

NDJSON over vsock. One complete JSON object per line, max 1MB per line.

### Handshake

Guest sends NUL-prefixed metadata line first:
```
\0CAPSEM_META:claude\n
```
Then JSON-RPC messages:
```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"github__search","arguments":{"q":"rust"}}}
```

Gateway responds with protocol version on initialize:
```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{},"resources":{},"prompts":{}},"serverInfo":{"name":"capsem-mcp-gateway","version":"..."}}}
```

## Tool namespacing

Tools are namespaced with `__` (double underscore) to prevent collisions:
- `github` + `search_repos` -> `github__search_repos`
- `builtin` + `http_get` -> `builtin__http_get`

Gateway parses the namespace to route to the correct server.

## Supported methods

| Method | Behavior |
|--------|----------|
| `initialize` | Return protocol version + capabilities |
| `notifications/initialized` | Notification (no response) |
| `tools/list` | Return builtin + all external server tools |
| `tools/call` | Policy check -> route to server -> call via rmcp |
| `resources/list` | Return resource catalog from all servers |
| `resources/read` | Lookup URI -> read via rmcp |
| `prompts/list` | Return prompt catalog |
| `prompts/get` | Lookup name -> get via rmcp |

## Policy evaluation

```
1. Blocked servers list (highest priority)
2. Allowed servers whitelist (if non-empty)
3. Per-tool decision map
4. Default fallback (Allow/Warn/Block)
```

Config hierarchy: corp.toml > user.toml > auto-detected from AI CLI settings.

Decisions: `Allow`, `Warn` (log + continue), `Block` (error -32600).

## Built-in tools

### Snapshot tools (VirtioFS mode only)
`snapshots_list`, `snapshots_changes`, `snapshots_create`, `snapshots_delete`, `snapshots_revert`, `snapshots_history`, `snapshots_compact`

### HTTP tools (always available)
`http_get`, `http_post`, `http_put`, `http_patch`, `http_delete`, `http_head`

All use namespace prefix `builtin` (e.g., `builtin__http_get`).

## Key source files

| File | Purpose |
|------|---------|
| `crates/capsem-core/src/mcp/gateway.rs` | NDJSON loop, JSON-RPC handler, telemetry logging |
| `crates/capsem-core/src/mcp/types.rs` | JsonRpcRequest/Response, McpToolDef, annotations |
| `crates/capsem-core/src/mcp/server_manager.rs` | rmcp client pool, tool routing, catalog |
| `crates/capsem-core/src/mcp/policy.rs` | Tool/server allow/warn/block decisions |
| `crates/capsem-core/src/mcp/mod.rs` | Tool cache, server detection, collision detection |
| `crates/capsem-agent/src/main.rs` | capsem-mcp-server binary (stdin/stdout relay) |

## Telemetry (mcp_calls table)

Every request/response logged with: timestamp, server_name, method, tool_name, request/response preview (256KB cap), decision, duration_ms, error_message, process_name, bytes sent/received.

Read `references/mcp-wire.md` for the full wire format details.

## Testing

- Unit: `cargo test -p capsem-core mcp`
- In-VM: `just run "capsem-doctor -k mcp"` (tool routing, domain blocking)
- Manual: boot interactively, run `claude -p "use fetch to get https://example.com"`, then `just inspect-session`
