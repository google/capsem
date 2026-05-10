---
title: Guest MCP Endpoint
description: Host and guest MCP servers, tool routing, framed transport, and telemetry.
sidebar:
  order: 25
---

Capsem has two MCP entry points: a **host-side server** (`capsem-mcp`) that exposes sandbox management tools to AI agents via stdio, and a **guest-side relay** (`capsem-mcp-server`) that carries tool calls from inside the VM to the host MITM MCP endpoint over framed vsock.

## Two-server architecture

```mermaid
graph TB
    subgraph "AI Agent (Claude Code, Gemini CLI)"
        AGENT["MCP Client<br/>(stdio)"]
    end

    subgraph "Host"
        HOST_MCP["capsem-mcp<br/>(stdio MCP server, rmcp)"]
        SVC["capsem-service<br/>(HTTP/UDS)"]
        GW["MITM MCP Endpoint<br/>(framed vsock:5002)"]
        AGG["capsem-mcp-aggregator<br/>(isolated subprocess)"]
        BUILTIN["capsem-mcp-builtin<br/>(isolated subprocess)"]
        EXT["External MCP servers<br/>(HTTP/SSE)"]
    end

    subgraph "Guest VM"
        GUEST_MCP["capsem-mcp-server<br/>(stdio-to-vsock relay)"]
        GUEST_AGENT["AI agent process"]
    end

    AGENT -->|stdio JSON-RPC| HOST_MCP
    HOST_MCP -->|HTTP/UDS| SVC

    GUEST_AGENT -->|stdio| GUEST_MCP
    GUEST_MCP -->|"framed MCP<br/>vsock:5002"| GW
    GW -->|"policy + telemetry"| AGG
    AGG -->|"stdio MCP"| BUILTIN
    AGG -->|"HTTP/SSE"| EXT
```

The host MCP server manages VMs. The guest relay provides MCP tools to code running inside the VM while the host endpoint owns parsing, policy, telemetry, and dispatch.

## Host MCP server (capsem-mcp)

The host MCP server runs as a stdio process, typically spawned by an AI agent (Claude Code, Gemini CLI). It uses the `rmcp` crate for JSON-RPC handling.

### Request flow

```mermaid
sequenceDiagram
    participant Agent as AI Agent
    participant MCP as capsem-mcp
    participant Svc as capsem-service

    Agent->>MCP: tools/call (capsem_exec)
    MCP->>Svc: POST /exec/{id} (HTTP/UDS)
    Svc-->>MCP: {stdout, stderr, exit_code}
    MCP-->>Agent: tool result
```

### Tool registry

26 tools for full sandbox lifecycle management, telemetry, host diagnostics, and guest MCP routing:

| Tool | Description | Service endpoint |
|------|-------------|-----------------|
| `capsem_create` | Create a new VM (name, RAM, CPUs, env, image) | `POST /provision` |
| `capsem_list` | List all VMs with status and config | `GET /list` |
| `capsem_info` | VM details (ID, PID, status, persistent) | `GET /info/{id}` |
| `capsem_exec` | Run shell command inside VM (timeout param) | `POST /exec/{id}` |
| `capsem_run` | One-shot: provision + exec + destroy | `POST /run` |
| `capsem_read_file` | Read file from guest filesystem | `GET /read_file/{id}` |
| `capsem_write_file` | Write file to guest filesystem | `POST /write_file/{id}` |
| `capsem_stop` | Stop VM (persistent: preserve, ephemeral: destroy) | `POST /stop/{id}` |
| `capsem_suspend` | Suspend VM (save RAM/CPU state) | `POST /suspend/{id}` |
| `capsem_resume` | Resume stopped persistent VM | `POST /resume/{name}` |
| `capsem_persist` | Convert ephemeral VM to persistent | `POST /persist/{id}` |
| `capsem_delete` | Permanently destroy VM and all state | `DELETE /delete/{id}` |
| `capsem_purge` | Kill all temp VMs (all=true includes persistent) | `POST /purge` |
| `capsem_fork` | Fork VM into reusable image | `POST /fork/{id}` |
| `capsem_vm_logs` | Get serial/process logs (grep + tail params) | `GET /logs/{id}` |
| `capsem_service_logs` | Get service logs (grep + tail params) | Service log file |
| `capsem_host_logs` | Get an allowlisted host log by symbolic name | `GET /host-logs/{name}` |
| `capsem_panics` | Extract structured panics and backtraces from host logs | `GET /panics` |
| `capsem_triage` | Summarize recent panics, IPC drops, server errors, and slow ops | `GET /triage` |
| `capsem_timeline` | Render a time-ordered session timeline by event layer and trace ID | `GET /timeline/{id}` |
| `capsem_inspect_schema` | Get CREATE TABLE statements for telemetry DB | Schema constant |
| `capsem_inspect` | Run SQL query against VM's session.db | `POST /inspect/{id}` |
| `capsem_version` | MCP server version and service connectivity | Local + service |
| `capsem_mcp_servers` | List configured guest MCP servers | Service MCP IPC |
| `capsem_mcp_tools` | List discovered guest MCP tools | Service MCP IPC |
| `capsem_mcp_call` | Call a namespaced guest MCP tool | Service MCP IPC |

### Service auto-launch

If the service is not running when the MCP server starts, it attempts to launch `capsem-service` from the same `bin/` directory. It polls the UDS socket for up to 5 seconds before giving up.

## Guest MCP relay (capsem-mcp-server)

The guest MCP relay is a minimal stdio-to-framed-vsock bridge. It does not route or execute tools; the host MITM MCP endpoint owns parsing, policy, telemetry, and dispatch.

### Framed relay

```mermaid
sequenceDiagram
    participant Agent as Guest AI process
    participant Relay as capsem-mcp-server
    participant EP as Host MITM MCP Endpoint

    Relay->>EP: \0CAPSEM_META:claude\n (metadata)
    Agent->>Relay: {"jsonrpc":"2.0","method":"tools/list"}\n (stdin)
    Relay->>EP: MCP frame stream_id=1 process=claude (vsock:5002)
    EP-->>Relay: MCP frame stream_id=1 payload={"jsonrpc":"2.0","result":{...}}
    Relay-->>Agent: {"jsonrpc":"2.0","result":{...}}\n (stdout)
```

### Wire protocol

| Step | Data | Direction |
|------|------|-----------|
| 1. Connect | vsock:5002 (`VSOCK_PORT_SNI_PROXY`) | Guest -> Host |
| 2. Metadata | `\0CAPSEM_META:<process_name>\n` | Guest -> Host |
| 3. Relay | Length-prefixed MCP frames containing JSON-RPC payloads | Bidirectional |
| 4. EOF | stdin closes -> half-close vsock write | Guest -> Host |

The `\0` prefix distinguishes connection metadata from framed content. Process names are sanitized: control characters and spaces replaced with underscores, truncated to 128 characters. The frame envelope also carries the authoritative per-request process name.

Two threads handle the relay:
- **Main thread**: stdin -> vsock (reads from AI agent, writes to host)
- **Reader thread**: vsock -> stdout (reads from host, writes back to AI agent)

## Tool routing (host endpoint)

The MITM MCP endpoint receives framed JSON-RPC over vsock:5002, applies MCP policy, records `mcp_calls`, and routes requests through the aggregator:

```mermaid
graph TD
    REQ["tools/call request"] --> PARSE["Extract tool name"]
    PARSE --> CHECK{"Tool category?"}
    CHECK -->|"local__fetch_http,<br/>local__grep_http,<br/>local__http_headers"| BUILTIN["capsem-mcp-builtin<br/>(HTTP tools)"]
    CHECK -->|"snapshots_*, file_*,<br/>dir_*"| FILE["capsem-mcp-builtin<br/>(VirtioFS file tools)"]
    CHECK -->|"server__tool<br/>(contains '__')"| EXT["capsem-mcp-aggregator<br/>(isolated subprocess)"]
    CHECK -->|"Unknown"| ERR["Error: tool not found"]
```

### Tool routing categories

| Category | Criteria | Handler | Examples |
|----------|----------|---------|----------|
| Builtin HTTP | `local__fetch_http`, `local__grep_http`, `local__http_headers` | `capsem-mcp-builtin` | `local__fetch_http`, `local__grep_http`, `local__http_headers` |
| File tools | Name starts with `snapshots_`, `file_`, `dir_` | `capsem-mcp-builtin` (VirtioFS only) | `file_read`, `dir_list`, `snapshots_create` |
| External | Contains `__` separator (server namespace) | `AggregatorClient` routes to isolated subprocess | `github__list_repos`, `slack__send_message` |

External tool calls are routed through the [MCP Aggregator](/architecture/mcp-aggregator/) -- an isolated subprocess that manages all external MCP server connections with privilege separation.

### Policy enforcement

Every `tools/call` request is checked at the framed MITM boundary before the
aggregator sees it. Named policy rules use `policy.mcp.<rule_name>` and can
match request fields such as `method`, `server.name`, `tool.name`, and
`arguments.*`, or response fields such as `response.text`.

| policy decision | Boundary behavior |
|---|---|
| `allow` | Tool call proceeds. |
| `ask` | Fails closed until an approval UI exists. The request is not dispatched. |
| `block` | Returns a policy JSON-RPC error. The request is not dispatched. |
| `rewrite` | Mutates only the configured response target before returning to the guest. |

Legacy MCP default permissions still exist for fallback behavior, but new
tool-specific UI controls save named policy rules. `warn` is historical
legacy terminology and is not a policy decision.

## MCP call logging

Every `tools/call` request is logged to the session database `mcp_calls` table:

| Column | Source |
|--------|--------|
| `server_name` | `builtin`, `file`, or external server name |
| `method` | JSON-RPC method (`tools/call`, `tools/list`, etc.) |
| `tool_name` | Tool name from request params |
| `decision` | Terminal transport result: `allowed`, `denied`, or `error` |
| `duration_ms` | End-to-end call duration |
| `request_preview` | Truncated request body |
| `response_preview` | Truncated response body |
| `process_name` | Guest process from metadata line |
| `policy_action` | policy decision: `allow`, `ask`, `deny`, or `rewrite` |
| `policy_rule` | Matching rule key, for example `policy.mcp.block_prod_token` |
| `policy_reason` | Optional human-readable audit reason |
| `trace_id` | Cross-table correlation ID |

See [Session Telemetry](/architecture/session-telemetry/) for the full `mcp_calls` schema.

## Endpoint runtime state

| Field | Type | Purpose |
|-------|------|---------|
| `aggregator` | `AggregatorClient` | Client handle for the isolated MCP aggregator subprocess |
| `db` | `Arc<DbWriter>` | Async telemetry writer |
| `policy` | `RwLock<Arc<McpPolicy>>` | Hot-reloadable legacy MCP fallback policy |
| named policy rules | `RwLock<Arc<PolicyConfig>>` | Hot-reloadable policy rules |
| `domain_policy` | `RwLock<Arc<DomainPolicy>>` | Domain policy for builtin HTTP tools |

The `AggregatorClient` is cloneable (`Arc`-wrapped mpsc channel) and shared across endpoint sessions for a given VM. The policy uses double-Arc for atomic swap: the outer `RwLock` protects an inner `Arc<McpPolicy>`. New frames read the current policy, so reloads affect already-open guest MCP connections.

## Configuration files

MCP server definitions live in TOML files under `guest/config/mcp/`:

```toml
# guest/config/mcp/capsem.toml
[capsem]
name = "Capsem"
description = "Built-in Capsem MCP server for file and snapshot tools"
transport = "stdio"
command = "/run/capsem-mcp-server"
builtin = true
enabled = true
```

External MCP servers are auto-detected from AI CLI settings (`~/.claude/settings.json`, `~/.gemini/settings.json`), defined manually in `~/.capsem/user.toml`, or injected via corp policy. Definitions are merged by `build_server_list()` and passed to the [MCP Aggregator](/architecture/mcp-aggregator/) subprocess at spawn time.

## Key source files

| File | Purpose |
|------|---------|
| `capsem-mcp/src/main.rs` | Host MCP server: 26 tools, rmcp handler, service bridge |
| `capsem-agent/src/mcp_server.rs` | Guest relay: stdin/stdout <-> framed MCP over vsock:5002 |
| `capsem-core/src/net/mitm_proxy/mcp_frame.rs` | Framed transport parser, stream lifecycle, and disconnect metrics |
| `capsem-core/src/net/mitm_proxy/mcp_endpoint.rs` | Host endpoint: JSON-RPC dispatch, policy, telemetry |
| `capsem-core/src/mcp/aggregator.rs` | Aggregator protocol types and `AggregatorClient` |
| `capsem-core/src/mcp/builtin_tools.rs` | Builtin HTTP tools (fetch_http, grep_http, http_headers) |
| `capsem-core/src/mcp/file_tools.rs` | File and snapshot tools (VirtioFS workspace) |
| `capsem-core/src/mcp/server_manager.rs` | External MCP server lifecycle and tool catalog |
| `capsem-core/src/mcp/policy.rs` | Legacy MCP fallback policy evaluation |
| `capsem-core/src/net/policy_config/types.rs` | policy rule types, validation, and condition evaluation |
| `capsem-mcp-aggregator/src/main.rs` | Isolated subprocess: NDJSON loop, server connections |
| `capsem-process/src/main.rs` | `spawn_mcp_aggregator()`: launch and driver tasks |
| `guest/config/mcp/` | MCP server TOML definitions |

See [MCP Aggregator](/architecture/mcp-aggregator/) for the full subprocess architecture.
