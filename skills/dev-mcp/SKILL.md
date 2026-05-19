---
name: dev-mcp
description: MCP development for Capsem. Covers the capsem-mcp host MCP server (AI agent sandbox control via stdio), the guest MCP relay and host MITM MCP endpoint (tool routing to external servers via framed vsock), and using capsem MCP tools for fast debugging. Use when working on the MCP server, endpoint, tool routing, policy evaluation, mcp_calls telemetry, or when you need to debug anything inside a VM. Also use this skill when capsem MCP tools are available and you want to understand the fastest way to test changes interactively.
---

# MCP in Capsem

Capsem has two MCP components:

1. **capsem-mcp** (host): MCP server over stdio that lets AI agents (Claude Code, Gemini CLI) control sandboxes -- create/delete VMs, exec commands, read/write files, query telemetry. Bridges to capsem-service HTTP API over UDS.
2. **Guest MCP relay + MITM MCP endpoint**: bridges AI agents running inside a guest VM to external MCP servers on the host via framed MCP records over vsock port 5002.

## Using capsem MCP tools for fast debugging

When the capsem MCP server is configured in your AI CLI, you have direct VM control without leaving the conversation. This is the fastest debug loop for any in-VM work.

### Available tools

| Tool | Parameters | What it does |
|------|-----------|-------------|
| `capsem_create` | name?, ramMb?, cpuCount?, env?, image? | Boot a fresh VM (~10s). Named VMs are persistent. env = `{"KEY": "VALUE"}` for guest injection. image = boot from a forked template. |
| `capsem_run` | command, timeout? | One-shot: boot temp VM, exec command, destroy, return output |
| `capsem_list` | -- | List all VMs (running + stopped persistent) |
| `capsem_info` | id | VM config, status, persistent, PID |
| `capsem_exec` | id, command, timeout? | Run command in guest, get stdout/stderr/exit_code. No default command timeout; pass `timeout` only when the user asked for a deadline. |
| `capsem_stop` | id | Stop VM (persistent: preserve state; ephemeral: destroy) |
| `capsem_resume` | name | Resume a stopped persistent VM |
| `capsem_persist` | id, name | Convert running ephemeral VM to persistent |
| `capsem_purge` | all? | Kill all temp VMs (all=true includes persistent) |
| `capsem_read_file` | id, path | Read file content from guest |
| `capsem_write_file` | id, path, content | Write file into guest |
| `capsem_vm_logs` | id, grep?, tail? | Serial + process logs. grep filters lines, tail limits to last N. |
| `capsem_service_logs` | grep?, tail? | Service daemon logs (last ~100KB). grep + tail filters. |
| `capsem_inspect_schema` | -- | session.db CREATE TABLE statements |
| `capsem_inspect` | id, sql | Raw SQL against session.db |
| `capsem_delete` | id | Destroy VM and wipe all state |
| `capsem_version` | -- | MCP server version + service connectivity status |
| `capsem_fork` | id, name, description? | Fork a running/stopped VM into a new stopped persistent session (use as a reusable template). |
| `capsem_mcp_connectors` | profile? | List Profile V2 `mcpServers` entries for the selected or requested profile. |
| `capsem_mcp_add` | id, profile?, disabled?, type?, command?, args?, env?, url?, headers?, bearerToken?, credential_refs?, allowed_tools? | Add a standard MCP server entry plus Capsem governance metadata to a user profile. |
| `capsem_mcp_delete` | id, profile? | Delete a direct user Profile V2 MCP server entry. |
| `capsem_panics` | since?, limit? | **Run FIRST when investigating an unexplained failure.** Structured panic + backtrace extractor across `~/.capsem/run/{service,mcp,gateway,tray}.log` and capsem-app's latest jsonl. Returns `[{ ts, binary, thread, location, message, frames }]` with home-dir paths redacted. |
| `capsem_triage` | id?, since?, limit? | Opinionated ranked summary of recent panics, dropped IPC frames (`target=ipc` warns from W1), 4xx/5xx server errors (`target=service`), and slow operations (>500ms). With `id`: also queries session.db for denied net + mcp errors + exec failures. |
| `capsem_host_logs` | name, grep?, tail?, maxBytes? | Read a host log by symbolic name. Names: `service`, `mcp`, `gateway`, `tray`, `app` (latest jsonl in `~/.capsem/logs/`). Hard-coded allowlist; no path traversal. |
| `capsem_timeline` | id, traceId?, since?, limit?, layers? | Unified time-ordered event stream for a session, joining exec/mcp/net/fs/model events. Filter by `traceId` to follow one logical operation across layers. |

### Debug workflow

```
-- Quick one-shot (no VM management needed):
capsem_run { command: "capsem-doctor -k net" }

-- Iterative debugging (long-lived VM):
1. capsem_create        -- boot a fresh sandbox (add name for persistence)
2. capsem_exec          -- run the thing you want to test
3. capsem_read_file     -- check config, logs, state
4. capsem_inspect       -- query telemetry tables
5. (fix code on host, rebuild with `just build`)
6. capsem_delete        -- tear down
7. repeat from 1
```

### Common debug patterns

**Verify a guest command works:**
```
capsem_exec { id: "vm-1", command: "capsem-doctor -k net" }
```

**Check network policy enforcement:**
```
capsem_exec { id: "vm-1", command: "curl -s https://blocked-domain.com" }
capsem_inspect { id: "vm-1", sql: "SELECT domain, action, status_code FROM net_events ORDER BY timestamp DESC LIMIT 10" }
```

**Verify telemetry pipeline:**
```
capsem_inspect { id: "vm-1", sql: "SELECT server_name, tool_name, decision, duration_ms FROM mcp_calls ORDER BY timestamp DESC" }
capsem_inspect { id: "vm-1", sql: "SELECT COUNT(*) as n, operation FROM fs_events GROUP BY operation" }
```

**Read guest config/state:**
```
capsem_read_file { id: "vm-1", path: "/etc/resolv.conf" }
capsem_read_file { id: "vm-1", path: "/tmp/capsem-init.log" }
```

**Write a test script and run it:**
```
capsem_write_file { id: "vm-1", path: "/tmp/test.sh", content: "#!/bin/bash\necho hello" }
capsem_exec { id: "vm-1", command: "chmod +x /tmp/test.sh && /tmp/test.sh" }
```

### When to use MCP tools vs just recipes

| Scenario | Use |
|----------|-----|
| Quick check: "does this work in the guest?" | `capsem_exec` |
| Read a guest file to understand state | `capsem_read_file` |
| Verify telemetry was recorded | `capsem_inspect` with SQL |
| Run capsem-doctor diagnostics | `capsem_exec` with `capsem-doctor` |
| Full regression suite | `just test` |
| Build + boot + validate in one shot | `just smoke` |
| Benchmark performance | `just bench` |

MCP tools are for fast, targeted checks during development. Just recipes are for comprehensive validation before committing.

## capsem-mcp (host MCP server)

### Architecture

```
AI Agent (Claude Code) <-> capsem-mcp (stdio, rmcp) <-> HTTP/UDS <-> capsem-service
```

Uses the `rmcp` crate with `#[tool_router]` macro for tool definitions. Stateless -- creates a fresh HTTP connection to `~/.capsem/run/service.sock` per request.

### Parameter conventions

MCP tools use **camelCase** on the wire (ramMb, cpuCount) because that is the MCP/JSON convention. The capsem-service HTTP API uses **snake_case** (ram_mb, cpus). The conversion happens inside each tool method -- the `#[serde(rename)]` attributes on param structs handle deserialization, and the tool builds a new JSON body with the service's field names.

### Key source files

| File | Purpose |
|------|---------|
| `crates/capsem-mcp/src/main.rs` | rmcp tool router, UDS HTTP client, tool implementations |
| `crates/capsem-mcp/Cargo.toml` | Dependencies (rmcp, hyper, capsem-core, capsem-logger) |

### Configuration

Registered in AI CLI settings:
```json
{ "mcpServers": { "capsem": { "command": "target/debug/capsem-mcp" } } }
```

### Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `CAPSEM_RUN_DIR` | `~/.capsem/run` | Where to find service socket and write mcp.log |
| `CAPSEM_UDS_PATH` | `$CAPSEM_RUN_DIR/service.sock` | Override service socket path |
| `RUST_LOG` | `info` | Logging level |

## MCP subprocess architecture

The guest MCP path is not a single process. `capsem-process` (the per-VM host process) owns the MITM MCP endpoint and spawns two privilege-isolated subprocesses that together handle MCP traffic from the guest:

| Crate | Role | Privileges |
|-------|------|-----------|
| `capsem-mcp-aggregator` | Manages connections to **external** MCP servers (GitHub, Slack, custom HTTP/stdio servers). Receives msgpack frames from `capsem-process` on stdin, routes tool calls. | Network only; no access to the VM, session DB, filesystem, or service socket. |
| `capsem-mcp-builtin` | Stdio MCP server that implements **built-in** tools: HTTP (`fetch_http`, `grep_http`, `http_headers`) and file/snapshot tools (when `CAPSEM_SESSION_DIR` is set). Managed by the aggregator as just another MCP server. | Scoped by environment variables: `CAPSEM_SESSION_DIR`, `CAPSEM_DOMAIN_ALLOW`, `CAPSEM_DOMAIN_BLOCK`, `CAPSEM_SESSION_DB`. |

Rationale: isolating external-server connections in a low-privilege subprocess means a compromised third-party MCP server cannot reach the host filesystem or the session DB. The built-in tool server runs in its own process for the same reason.

Wire protocol between `capsem-process` and the aggregator: **length-prefixed msgpack frames** on stdio (`[4-byte big-endian length][msgpack payload]`). Between the aggregator and the built-in server: **stdio MCP** (standard JSON-RPC per line). Between the in-guest AI agent and `capsem-process`: `/run/capsem-mcp-server` relays stdio JSON-RPC as bounded framed MCP records over **vsock port 5002**. MCP calls pass through the MITM parser/interpreter and write MITM-owned `mcp_calls`.

Binaries land in `~/.capsem/bin/` at install time: `capsem-mcp-aggregator`, `capsem-mcp-builtin`.

## Guest MCP Endpoint

The guest MCP relay bridges AI agents in the guest VM to the host MITM MCP endpoint. It runs over vsock port 5002 using bounded length-prefixed MCP frames that carry JSON-RPC payloads and per-frame process attribution.

Framed guest MCP over `vsock:5002` must be tested as the default transport, not as an opt-in benchmark mode. The minimum hardening matrix for that path is:
- parser/interpreter: bounded frames, invalid JSON, malformed flags, stream-id reuse, notification/request-id mismatch
- dispatch: `initialize`, `tools/list`, builtin `tools/call`, configured external stdio `tools/call`, `resources/list`, `prompts/list`, and method error mapping
- policy: live policy mutation, per-tool block, resource URI rule, argument-name rule, argument-value rule, return-value rule, deny-over-allow precedence, and proof that blocked requests/responses do not leak original data
- telemetry: `session.db` rows for success, denial, timeout, process attribution, request/response previews, policy fields, and terminal errors
- boundary: aggregator remains DB-free; MITM/process owns MCP audit writes
- VM E2E: boot a real VM, run `/run/capsem-mcp-server` with no transport override, then query `session.db`

### Architecture

```
Guest (Claude/Gemini) -> capsem-mcp-server (stdin/stdout relay)
  -> framed vsock:5002 -> MITM MCP endpoint (capsem-core)
  -> Policy check -> Route to: builtin tools | external MCP servers (via rmcp)
  -> Telemetry -> session.db mcp_calls table
```

### Wire format

Length-prefixed MCP frames over vsock. Each frame contains a bounded JSON-RPC payload plus a stream id, flags, and sanitized process name.

#### Handshake

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

### Tool namespacing

Tools are namespaced with `__` (double underscore) to prevent collisions:
- `github` + `search_repos` -> `github__search_repos`
- `builtin` + `http_get` -> `builtin__http_get`

The endpoint parses the namespace to route to the correct server.

### Supported methods

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

### Policy evaluation

```
1. Blocked servers list (highest priority)
2. Allowed servers whitelist (if non-empty)
3. Per-tool decision map
4. Default fallback (Allow/Warn/Block)
```

Config hierarchy: corp.toml > user.toml > auto-detected from AI CLI settings.

Decisions: `Allow`, `Warn` (log + continue), `Block` (error -32600).

### Built-in tools

#### Snapshot tools (VirtioFS mode only)
`snapshots_list`, `snapshots_changes`, `snapshots_create`, `snapshots_delete`, `snapshots_revert`, `snapshots_history`, `snapshots_compact`

#### HTTP tools (always available)
`http_get`, `http_post`, `http_put`, `http_patch`, `http_delete`, `http_head`

All use namespace prefix `builtin` (e.g., `builtin__http_get`).

### Endpoint key source files

| File | Purpose |
|------|---------|
| `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs` | Framed transport parser, stream lifecycle, disconnect metrics |
| `crates/capsem-core/src/net/mitm_proxy/mcp_endpoint.rs` | JSON-RPC handler, policy, dispatch, telemetry logging |
| `crates/capsem-core/src/mcp/types.rs` | JsonRpcRequest/Response, McpToolDef, annotations |
| `crates/capsem-core/src/mcp/server_manager.rs` | rmcp client pool, tool routing, catalog |
| `crates/capsem-core/src/mcp/policy.rs` | Tool/server allow/warn/block decisions |
| `crates/capsem-core/src/mcp/mod.rs` | Tool cache, server detection, collision detection |
| `crates/capsem-agent/src/mcp_server.rs` | capsem-mcp-server binary (stdin/stdout relay) |

### Telemetry (mcp_calls table)

Every request/response logged with: timestamp, server_name, method, tool_name, request/response preview (256KB cap), decision, duration_ms, error_message, process_name, bytes sent/received.

Read `references/mcp-wire.md` for the full wire format details.

## Testing

### Unit tests

`cargo test -p capsem-mcp` -- param serde roundtrips, UDS path resolution, tool router registration, schema constants.

`cargo test -p capsem-core mcp` -- gateway, policy, server manager, type serialization.

### Integration tests (Python)

The MCP integration tests (`tests/capsem-mcp/`) are black-box tests that boot a real service + VM and exercise the full MCP protocol over stdio.

**Run with:** `just test-mcp` (or `pytest tests/capsem-mcp/ -m mcp -v`)

**Test files:**

| File | What it covers |
|------|---------------|
| `test_discovery.py` | Tool listing, schema validation |
| `test_lifecycle.py` | Create, delete, list, info, error paths |
| `test_exec.py` | Command execution, stdout/stderr, exit codes |
| `test_file_io.py` | Read/write, unicode, large payloads, edge cases |
| `test_inspect.py` | DB schema query, SQL execution, error cases |
| `test_errors.py` | Deleted VM ops, concurrent isolation, error mapping |
| `test_fork_images.py` | Fork lifecycle, image CRUD, create-from-image, error cases |
| `test_winter_is_coming.py` | Full fork e2e: install packages + write workspace, fork, verify survival, assert fork < 500ms and image < 12MB |

**Fixture architecture:**

- `capsem_service` (session scope) -- spawns capsem-service on isolated temp socket, codesigns binaries on macOS
- `mcp_session` (per-test) -- fresh capsem-mcp subprocess with JSON-RPC handshake, returns `McpSession` helper
- `shared_vm` (session scope) -- one long-lived VM for non-destructive tests, avoids repeated boot overhead
- `fresh_vm` (per-test factory) -- creates uniquely named VMs with auto-cleanup for destructive tests

**McpSession helper** (`tests/capsem-mcp/conftest.py`): wraps capsem-mcp subprocess with JSON-RPC 2.0 protocol. Key methods:
- `request(method, params)` -- send NDJSON, read response
- `call_tool(name, args)` -- call tool, assert success, parse JSON content
- `call_tool_raw(name, args)` -- raw response (no assertions)

### In-VM diagnostics

`just run "capsem-doctor -k mcp"` -- tests tool routing and domain blocking inside the guest.

### Manual validation

Boot interactively, run a workload, then inspect telemetry:
```bash
just run
# (in another terminal)
just inspect-session <vm_id> "SELECT * FROM mcp_calls"
```

Or use MCP tools directly (see "Fast debugging" section above) for the same workflow without leaving Claude Code.

## Lessons learned

1. **Never prepend headers to JSON output.** MCP tool responses with `format=json` must return raw, parseable JSON. Do not wrap JSON in pagination headers, content-length prefixes, or any other text. The `snapshots_changes` tool broke because `paginated_response()` prepended `"Content length: ...\nShowing: ...\n"` to the JSON array, making `json.loads()` fail. Rule: if a tool offers both text and JSON formats, branch early and return JSON directly without passing through text-oriented helpers like `paginated_response()`.
