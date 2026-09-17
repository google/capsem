---
name: dev-mcp
description: MCP for Capsem: the SDK-backed npm host server, guest relay, and tool routing. Use when working on MCP servers, tool policy, telemetry, or in-VM debugging.
---

# MCP in Capsem

Capsem has two MCP entry points:

1. **`@capsem/mcp`** (host): a standalone TypeScript stdio server. It presents typed tools and uses `@capsem/sdk` for authenticated gateway HTTP. It does not open the service UDS, discover local services, or read VM runtime state.
2. **Guest MCP relay + MITM MCP endpoint**: bridges AI agents inside a VM to built-in and external MCP servers through bounded framed records on vsock port 5002.

The native installer does not install Node.js or download the npm host package.
The guest relay, aggregator, and built-in Rust components remain native product
components.

## Using the npm MCP tools for fast debugging

Install and register the package with an explicit gateway URL, bearer token, and
transport timeout:

```sh
npm install --global @capsem/mcp
CAPSEM_GATEWAY_TOKEN="$(cat ~/.capsem/run/gateway.token)" \
  capsem-mcp --gateway-url http://127.0.0.1:19222 --timeout-ms 30000
```

The token comes from `CAPSEM_GATEWAY_TOKEN` or `--token-file`; `--token` is
refused because argv is world-readable. The bearer token belongs only to
the host MCP process; never copy it into a VM, container, guest tool argument,
or workload. stdout is protocol-only and sanitized diagnostics use stderr.

The current registry is defined in `mcp/typescript/src/host-tools.ts`,
`network-tools.ts`, and `profile-tools.ts`. Parameters follow the TypeScript
SDK names, including `vm_id`, `vcpu`, `memory`, `env`, and
`timeout_secs`. Canonical tools include `capsem_pause` and
`capsem_status`; the retired `capsem_suspend`, `capsem_version`, and
duplicate `capsem_service_logs` names are not exposed.

The host tools cover:

- VM lifecycle, one-shot execution, persistence, files, snapshots, statistics,
  logs, timelines, panic extraction, and triage.
- Private network lifecycle, membership, and cursor-based audit logs.
- Typed profile MCP discovery, refresh, permission inspection, and invocation
  through the running VM's existing relay and security engine.

### Debug workflow

```text
1. capsem_create
2. capsem_exec { vm_id: "...", command: "capsem-doctor -k net" }
3. capsem_read_file { vm_id: "...", path: "/tmp/capsem-init.log" }
4. capsem_timeline { vm_id: "...", layers: "net,tool,fs", limit: 50 }
5. Fix and rebuild the owning native component when required.
6. capsem_delete { vm_id: "..." }
```

The `--timeout-ms` option bounds the SDK HTTP request.
`timeout_secs` on execution tools bounds the guest command. Cancellation
closes the local request; it does not delete the VM or undo an accepted
mutation. The SDK does not retry mutations.

### When to use MCP tools vs just recipes

| Scenario | Use |
| --- | --- |
| Targeted guest behavior | `capsem_exec` |
| Read a guest file | `capsem_read_file` |
| Inspect typed audit evidence | `capsem_timeline`, statistics, and network logs |
| Run capsem-doctor | `capsem_exec` |
| Full regression suite | `just test` |
| Focused native validation | `just focus-test functional` |

## npm host MCP server

```text
Host AI agent -> @capsem/mcp (stdio) -> @capsem/sdk
  -> authenticated capsem-gateway HTTP -> capsem-service
```

The npm process owns tool presentation and typed input/output only. The gateway
authenticates control requests. The service, VM owner, security engine, and
logger retain lifecycle, trusted identity, policy, credentials, and audit
storage. The npm process receives no virtualization entitlement, registry
credential, CA private key, direct database access, or service socket access.

### Key source files

| File | Purpose |
| --- | --- |
| `mcp/typescript/src/cli.ts` | Protocol-only stdio executable |
| `mcp/typescript/src/server.ts` | SDK client and tool registration |
| `mcp/typescript/src/host-tools.ts` | VM, file, and diagnostic tools |
| `mcp/typescript/src/network-tools.ts` | Private network tools |
| `mcp/typescript/src/profile-tools.ts` | Profile MCP discovery and calls |
| `mcp/typescript/src/results.ts` | Structured results and sanitized errors |
| `sdk/typescript/src/` | Typed gateway client and validators |

## MCP subprocess architecture

The guest MCP path is not a single process. `capsem-process` (the per-VM host process) owns the MITM MCP endpoint and spawns two privilege-isolated subprocesses that together handle MCP traffic from the guest:

| Crate | Role | Privileges |
|-------|------|-----------|
| `capsem-mcp-aggregator` | Manages connections to **external** MCP servers (GitHub, Slack, custom HTTP/stdio servers). Receives msgpack frames from `capsem-process` on stdin, routes tool calls. | Network only; no access to the VM, session DB, filesystem, or service socket. |
| `capsem-mcp-builtin` | Stdio MCP server that implements **built-in** tools: HTTP (`fetch_http`, `grep_http`, `http_headers`) and file/snapshot tools (when `CAPSEM_SESSION_DIR` is set). Managed by the aggregator as just another MCP server. | Scoped by environment variables: `CAPSEM_SESSION_DIR`, `CAPSEM_DOMAIN_ALLOW`, `CAPSEM_DOMAIN_BLOCK`, `CAPSEM_SESSION_DB`. |

Rationale: isolating external-server connections in a low-privilege subprocess means a compromised third-party MCP server cannot reach the host filesystem or the session DB. The built-in tool server runs in its own process for the same reason.

Wire protocol between `capsem-process` and the aggregator: **length-prefixed msgpack frames** on stdio (`[4-byte big-endian length][msgpack payload]`). Between the aggregator and the built-in server: **stdio MCP** (standard JSON-RPC per line). Between the in-guest AI agent and `capsem-process`: `/run/capsem-mcp-server` relays stdio JSON-RPC as bounded framed MCP records over **vsock port 5002**. Tool activity writes the canonical `tool_calls` ledger. Visible MCP protocol facts are represented as typed security events and `security_rule_events`.

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
  -> SecurityEvent rule check -> Route to: builtin tools | external MCP servers (via rmcp)
  -> Telemetry -> session.db tool_calls table plus security_rule_events protocol evidence
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

### Security evaluation

```
1. Parse MCP frame into typed `SecurityEvent` MCP fields.
2. Apply the shared security engine plugin/rule rail.
3. Dispatch only if the effective action allows it.
4. Log canonical tool row, optional MCP protocol row, and matched security rule rows.
```

Config hierarchy: corp config constrains profile config. Profile config owns
MCP servers, tools, resources, default rules, and plugin policy. There is no
MCP-specific decision provider or `user.toml` override rail.

Decisions use the shared security action enum: `allow`, `ask`, `block`,
`rewrite`, `preprocess`, and `postprocess`. `ask` waits for an approval or
denial before dispatch; `block` returns a policy JSON-RPC error without calling
the tool.

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

### Telemetry (tool_calls and security events)

`tool_calls` is the canonical user/security ledger for all tool origins:
model-native, built-in/local, and MCP-origin. UI counts, CEL rules, and
forensic tool activity must use this table.

MCP protocol facts such as initialize/list/resource frames are security events.
A visible MCP `tools/call` without a matching `tool_calls` row is a bug.

Every request/response logged with: timestamp, server_name, method, tool_name,
compact request/response display fields, decision, duration_ms, error_message,
process_name, bytes sent/received. Full MCP request/response payloads share the
same `event_body_blobs` ledger table as HTTP and model traffic; do not add a
second MCP-only body rail.

Read `references/mcp-wire.md` for the full wire format details.

## Testing

The npm package owns its unit, type, coverage, and package-build checks:

```sh
pnpm --dir mcp/typescript test
pnpm --dir mcp/typescript pack
```

`tests/capsem-sdk/test_mcp_cli_parity.py` guards canonical CLI/MCP behavior.
`tests/ironbank/test_mcp_profile_ledger.py` packs the npm artifact, launches it
over stdio with fixture-issued gateway credentials, drives a real VM and guest
MCP path, and verifies correlated tool, network, and security ledger evidence.
`tests/capsem-installed/test_winterfell_gateway.py` verifies the installed
native HTTP cohort without requiring Node.js.

The guest path retains its Rust tests and real-VM diagnostics:

```sh
cargo test -p capsem-core mcp
just exec "capsem-doctor -k mcp"
```

## Lessons learned

1. **Never prepend headers to JSON output.** MCP tool responses with `format=json` must return raw, parseable JSON. Do not wrap JSON in pagination headers, content-length prefixes, or any other text. The `snapshots_changes` tool broke because `paginated_response()` prepended `"Content length: ...\nShowing: ...\n"` to the JSON array, making `json.loads()` fail. Rule: if a tool offers both text and JSON formats, branch early and return JSON directly without passing through text-oriented helpers like `paginated_response()`.
