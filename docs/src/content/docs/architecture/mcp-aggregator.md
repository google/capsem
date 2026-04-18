---
title: MCP Aggregator
description: Isolated subprocess for managing external MCP server connections with privilege separation.
sidebar:
  order: 26
---

The MCP aggregator (`capsem-mcp-aggregator`) is a low-privilege subprocess that manages connections to external MCP servers. It runs in an isolated process with only network access -- no VM, no session database, no filesystem, no service IPC.

## Why a separate process

External MCP servers require network access, bearer tokens, and custom HTTP headers. The main per-VM process (`capsem-process`) has extensive privileges: VM control, session database, VirtioFS workspace, service IPC. Running external server connections inside capsem-process would expose all of those privileges to any vulnerability in an MCP server connection or the HTTP/SSE transport layer.

The aggregator subprocess enforces a hard privilege boundary:

| | capsem-process | capsem-mcp-aggregator |
|---|---|---|
| VM control (vsock) | Yes | No |
| Session database | Yes | No |
| VirtioFS workspace | Yes | No |
| Service IPC | Yes | No |
| Network (external MCP servers) | No | Yes |
| Bearer tokens / API keys | No | Yes |

If the aggregator is compromised, the attacker has network access and MCP server credentials -- but cannot reach the VM, read telemetry, or modify files.

## Architecture

The aggregator sits between the MCP gateway (which handles guest VM requests) and external MCP servers (which provide tools like GitHub, Slack, etc.).

```mermaid
graph LR
    subgraph "Guest VM"
        AGENT["AI agent"]
    end

    subgraph "capsem-process"
        GW["MCP Gateway<br/>(vsock:5003)"]
        CLIENT["AggregatorClient<br/>(mpsc channel)"]
        WRITER["Writer task<br/>(stdin)"]
        READER["Reader task<br/>(stdout)"]
    end

    subgraph "capsem-mcp-aggregator"
        MAIN["NDJSON loop"]
        MGR["McpServerManager"]
    end

    subgraph "External"
        EXT1["GitHub MCP"]
        EXT2["Slack MCP"]
    end

    AGENT -->|"vsock:5003<br/>JSON-RPC"| GW
    GW --> CLIENT
    CLIENT --> WRITER
    WRITER -->|"stdin<br/>NDJSON"| MAIN
    MAIN --> MGR
    MGR -->|"HTTP/SSE"| EXT1
    MGR -->|"HTTP/SSE"| EXT2
    MAIN -->|"stdout<br/>NDJSON"| READER
    READER --> CLIENT
```

Four layers handle the flow:

1. **AggregatorClient** (in capsem-process) -- typed async API wrapping an mpsc channel. Multiple gateway sessions share one client via `Arc`.
2. **Driver tasks** (in capsem-process) -- writer task serializes requests to subprocess stdin; reader task deserializes responses from stdout and routes them to pending callers via oneshot channels.
3. **NDJSON loop** (in capsem-mcp-aggregator) -- reads requests from stdin, dispatches to `McpServerManager`, writes responses to stdout.
4. **McpServerManager** (in capsem-core) -- manages `rmcp` HTTP connections to external servers, builds unified tool/resource/prompt catalogs with namespacing.

## Subprocess lifecycle

### Spawn

capsem-process spawns the aggregator during VM startup, after loading MCP server definitions from user and corp config files.

```mermaid
sequenceDiagram
    participant Proc as capsem-process
    participant Agg as capsem-mcp-aggregator
    participant Ext as External MCP servers

    Proc->>Agg: spawn (stdin/stdout piped, stderr inherited)
    Proc->>Agg: [{"name":"github","url":"...","bearer_token":"..."}]\n (first line)
    Agg->>Ext: HTTP MCP initialize (per enabled server)
    Ext-->>Agg: tools/list, resources/list, prompts/list
    Note over Agg: Build unified catalogs
    Note over Agg: Enter NDJSON request loop
```

The binary is located next to `capsem-process` in `~/.capsem/bin/`. If not found (dev builds without a full install), capsem-process falls back to an in-process mock that returns empty results for catalog queries and errors for tool calls.

### Steady state

The subprocess runs for the lifetime of the VM. Requests arrive on stdin, responses go to stdout, logs go to stderr (inherited by the parent).

### Shutdown

Two paths:

1. **Normal**: capsem-process sends a `shutdown` request. The aggregator disconnects all servers and exits.
2. **Parent exit**: capsem-process closes stdin (process exit, crash, or signal). The aggregator detects EOF, calls `shutdown_all()`, and exits.

### Crash recovery

If the aggregator crashes, the reader and writer driver tasks in capsem-process exit (broken pipe / EOF). Subsequent requests from the gateway receive a channel-closed error. The gateway returns a JSON-RPC error to the guest -- the VM continues running, only external MCP tools become unavailable.

## NDJSON protocol

Communication uses newline-delimited JSON over stdin/stdout. Each message is a single JSON object terminated by `\n`. Maximum line length is 1 MB.

### Initialization

The first line on stdin is a JSON array of server definitions:

```json
[
  {
    "name": "github",
    "url": "https://api.githubcopilot.com/mcp/",
    "headers": {},
    "bearer_token": "ghp_xxxx",
    "enabled": true,
    "source": "claude",
    "unsupported_stdio": false
  }
]
```

Servers marked `unsupported_stdio: true` are stdio-only servers that cannot be connected over HTTP -- the aggregator skips them. Disabled servers are also skipped.

### Request format (process to aggregator)

```json
{"id": 1, "method": "list_servers"}
{"id": 2, "method": "list_tools"}
{"id": 3, "method": "list_resources"}
{"id": 4, "method": "list_prompts"}
{"id": 5, "method": "call_tool", "params": {"name": "github__search_repos", "arguments": {"query": "rust"}}}
{"id": 6, "method": "read_resource", "params": {"uri": "capsem://github/repo://owner/repo"}}
{"id": 7, "method": "get_prompt", "params": {"name": "github__review_pr", "arguments": {}}}
{"id": 8, "method": "refresh", "params": {"servers": [...]}}
{"id": 9, "method": "shutdown"}
```

### Response format (aggregator to process)

```json
{"id": 1, "servers": [{"name": "github", "connected": true, "tool_count": 5, ...}]}
{"id": 2, "tools": [{"namespaced_name": "github__search_repos", "server_name": "github", ...}]}
{"id": 5, "result": {"content": [{"type": "text", "text": "..."}]}}
{"id": 8, "ok": true}
{"id": 9, "ok": true}
```

Error responses:

```json
{"id": 5, "error": "server not found: github"}
```

### Correlation

Each request carries an `id` (monotonically increasing `AtomicU64`). The response echoes the same `id`. The driver's reader task uses a `HashMap<u64, oneshot::Sender>` to route responses back to the correct caller.

## Operations

| Method | Purpose | Response |
|--------|---------|----------|
| `list_servers` | Server definitions with connection status | `servers: [...]` |
| `list_tools` | All discovered tools across connected servers | `tools: [...]` |
| `list_resources` | All discovered resources | `resources: [...]` |
| `list_prompts` | All discovered prompts | `prompts: [...]` |
| `call_tool` | Call a namespaced tool on an external server | `result: {...}` |
| `read_resource` | Read a namespaced resource from an external server | `result: {...}` |
| `get_prompt` | Get a namespaced prompt from an external server | `result: {...}` |
| `refresh` | Disconnect all servers, replace definitions, reconnect | `ok: true` |
| `shutdown` | Disconnect all servers and exit | `ok: true` |

## Tool namespacing

External tools are namespaced with `__` (double underscore) to prevent collisions across servers:

```
github__search_repos     (server "github", tool "search_repos")
slack__send_message      (server "slack", tool "send_message")
```

Resources use URI-based namespacing:

```
capsem://github/repo://owner/repo
```

The aggregator splits on the first `__` when routing, so tool names containing `__` are supported (e.g., `github__my__tool` routes to server `github`, tool `my__tool`).

## Server definition sources

Three layers combined with deduplication (first occurrence wins by name):

1. **Auto-detected** from host AI CLI configs (`~/.claude/settings.json`, `~/.gemini/settings.json`)
2. **User manual servers** from `~/.capsem/user.toml` `[mcp]` section
3. **Corp-injected servers** from `/etc/capsem/corp.toml` (enterprise policy, highest priority for enable/disable overrides)

Names containing `__` or matching `builtin` are rejected. Empty names are rejected.

## Hot reload

The `refresh` operation allows live reconfiguration without restarting the VM:

1. Service receives `POST /reload-config`
2. Service sends `McpRefreshTools` IPC to capsem-process
3. capsem-process reads fresh settings from disk, calls `build_server_list()`
4. Client sends `refresh` with new definitions to the aggregator
5. Aggregator disconnects all servers, replaces definitions, reconnects

This supports adding, removing, or reconfiguring MCP servers while a VM is running.

## Service API integration

The service exposes MCP operations through its HTTP API, which capsem-process handles by delegating to the aggregator:

| Service IPC message | capsem-process action |
|---|---|
| `McpListServers` | `aggregator.list_servers()` |
| `McpListTools` | `aggregator.list_tools()` |
| `McpRefreshTools` | Read settings, `aggregator.refresh(new_servers)` |
| `McpCallTool` | `aggregator.call_tool(name, args)` |

These IPC messages let the CLI, gateway, and frontend query and control MCP servers through the standard service API path.

## Error handling

The aggregator is designed for graceful degradation:

| Scenario | Behavior |
|----------|----------|
| Some servers fail to connect at startup | Warning logged, continue with working servers |
| Tool call to disconnected server | Error response to caller, other tools unaffected |
| Malformed request line | Logged, skipped, loop continues |
| Subprocess crash | Gateway returns JSON-RPC errors, VM keeps running |
| Serialization failure | Fallback JSON error response written to stdout |
| Stdin EOF | Graceful shutdown (all servers disconnected) |

## Key source files

| File | Purpose |
|------|---------|
| `capsem-mcp-aggregator/src/main.rs` | Subprocess binary: init, NDJSON loop, request dispatch |
| `capsem-core/src/mcp/aggregator.rs` | Protocol types (`AggregatorRequest/Response`) and `AggregatorClient` |
| `capsem-core/src/mcp/server_manager.rs` | `McpServerManager`: rmcp connections, tool catalog, namespacing |
| `capsem-core/src/mcp/mod.rs` | `build_server_list()`: auto-detect + manual + corp merge |
| `capsem-process/src/main.rs` | `spawn_mcp_aggregator()`: launch, driver tasks, mock fallback |
| `capsem-core/src/mcp/gateway.rs` | MCP gateway: routes external tool calls through the aggregator |
| `capsem-proto/src/ipc.rs` | Service-process IPC messages for MCP operations |
