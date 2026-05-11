---
title: Session Telemetry
description: Per-VM SQLite database schema, data flow, and query patterns.
sidebar:
  order: 20
---

Every Capsem VM gets its own SQLite database (`session.db`) that records network requests, DNS queries, AI model calls, MCP tool invocations, exec activity, kernel audit events, file changes, and snapshots. The database lives in the session directory and is destroyed with the VM (ephemeral) or preserved (persistent/forked).

## Schema overview

```mermaid
erDiagram
    net_events {
        int id PK
        text domain
        text decision
        text method
        text path
        int status_code
        int bytes_sent
        int bytes_received
        int duration_ms
    }
    model_calls {
        int id PK
        text provider
        text model
        int input_tokens
        int output_tokens
        real estimated_cost_usd
        text trace_id
    }
    tool_calls {
        int id PK
        int model_call_id FK
        text call_id
        text tool_name
        text origin
    }
    tool_responses {
        int id PK
        int model_call_id FK
        text call_id
        text content_preview
    }
    mcp_calls {
        int id PK
        text server_name
        text method
        text tool_name
        text decision
        text policy_action
        text policy_rule
        int duration_ms
    }
    dns_events {
        int id PK
        text qname
        int qtype
        int rcode
        text decision
        text matched_rule
    }
    policy_hook_events {
        int id PK
        text endpoint_id
        text callback
        text decision
        text status
        text fallback
        text trace_id
    }
    exec_events {
        int id PK
        int exec_id
        text command
        int exit_code
        int duration_ms
    }
    audit_events {
        int id PK
        int pid
        int ppid
        text exe
        text argv
    }
    fs_events {
        int id PK
        text action
        text path
        int size
    }
    snapshot_events {
        int id PK
        int slot
        text origin
        int start_fs_event_id
        int stop_fs_event_id
    }

    model_calls ||--o{ tool_calls : "has"
    model_calls ||--o{ tool_responses : "has"
    snapshot_events }o--o{ fs_events : "references range"
```

## Tables

### net_events

Every HTTP request through the MITM proxy, whether allowed or denied.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp` | TEXT | ISO 8601 |
| `domain` | TEXT | Target domain |
| `port` | INTEGER | Default 443 |
| `decision` | TEXT | `allowed`, `denied`, `error` |
| `process_name` | TEXT | Guest process that initiated the request |
| `pid` | INTEGER | Guest process ID |
| `method` | TEXT | HTTP method |
| `path` | TEXT | Request path |
| `query` | TEXT | Query string |
| `status_code` | INTEGER | Upstream response status |
| `bytes_sent` | INTEGER | Request body size |
| `bytes_received` | INTEGER | Response body size |
| `duration_ms` | INTEGER | End-to-end latency |
| `matched_rule` | TEXT | Which policy rule matched |
| `request_headers` | TEXT | Request headers (when body logging enabled) |
| `response_headers` | TEXT | Response headers |
| `request_body_preview` | TEXT | First 4 KB of request body |
| `response_body_preview` | TEXT | First 4 KB of response body |
| `conn_type` | TEXT | Default `https`, `https-mitm` for proxied |
| `policy_mode` | TEXT | Policy engine mode, when set |
| `policy_action` | TEXT | Typed policy action (`allow`, `ask`, `block`, `rewrite`) |
| `policy_rule` | TEXT | Matching policy rule key |
| `policy_reason` | TEXT | Optional audit reason or fail-closed detail |
| `trace_id` | TEXT | Cross-table correlation ID |

### model_calls

AI provider API calls with parsed response metadata.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp` | TEXT | ISO 8601 |
| `provider` | TEXT | `anthropic`, `openai`, `google` |
| `model` | TEXT | e.g. `claude-opus-4` |
| `process_name` | TEXT | Guest process |
| `pid` | INTEGER | Guest process ID |
| `method` | TEXT | HTTP method (always `POST`) |
| `path` | TEXT | API path (e.g. `/v1/messages`) |
| `stream` | INTEGER | Boolean: 1 if SSE streaming |
| `system_prompt_preview` | TEXT | First N chars of system prompt |
| `messages_count` | INTEGER | Number of messages in request |
| `tools_count` | INTEGER | Number of tools in request |
| `request_bytes` | INTEGER | Request body size |
| `request_body_preview` | TEXT | First 4 KB of request body |
| `message_id` | TEXT | Provider message ID |
| `status_code` | INTEGER | HTTP status |
| `text_content` | TEXT | Concatenated text output |
| `thinking_content` | TEXT | Chain-of-thought output |
| `stop_reason` | TEXT | `end_turn`, `tool_use`, `max_tokens`, `content_filter` |
| `input_tokens` | INTEGER | Input token count |
| `output_tokens` | INTEGER | Output token count |
| `duration_ms` | INTEGER | Request duration |
| `response_bytes` | INTEGER | Response body size |
| `estimated_cost_usd` | REAL | Cost estimate from pricing table |
| `trace_id` | TEXT | Links multi-turn agent conversations |
| `usage_details` | TEXT | JSON: `{"cache_read": 800, "thinking": 200}` |

### tool_calls

Tool invocations extracted from model responses. One row per `tool_use` content block.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `model_call_id` | INTEGER FK | References `model_calls.id` |
| `call_index` | INTEGER | Position in the response |
| `call_id` | TEXT | Provider-assigned call ID |
| `tool_name` | TEXT | Tool name |
| `arguments` | TEXT | JSON arguments |
| `origin` | TEXT | `native`, `local`, `mcp_proxy` |
| `mcp_call_id` | INTEGER | Optional FK to `mcp_calls`; current model traffic does not populate it |
| `trace_id` | TEXT | Cross-table correlation ID |

### tool_responses

Tool results from subsequent requests (matched by `call_id`).

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `model_call_id` | INTEGER FK | References `model_calls.id` |
| `call_id` | TEXT | Matches `tool_calls.call_id` |
| `content_preview` | TEXT | Truncated tool result |
| `is_error` | INTEGER | Boolean: 1 if tool returned error |
| `trace_id` | TEXT | Cross-table correlation ID |

### mcp_calls

MCP JSON-RPC tool invocations through the guest MCP relay and host MITM MCP endpoint (framed vsock:5002).

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp` | TEXT | ISO 8601 |
| `server_name` | TEXT | MCP server name (e.g. `builtin`, `github`) |
| `method` | TEXT | JSON-RPC method (`tools/call`, `tools/list`, etc.) |
| `tool_name` | TEXT | Tool name (for `tools/call`) |
| `request_id` | TEXT | JSON-RPC request ID |
| `request_preview` | TEXT | Truncated request body |
| `response_preview` | TEXT | Truncated response body |
| `decision` | TEXT | `allowed`, `denied`, `error` |
| `duration_ms` | INTEGER | Call duration |
| `error_message` | TEXT | Error details if failed |
| `process_name` | TEXT | Guest process |
| `bytes_sent` | INTEGER | Request size |
| `bytes_received` | INTEGER | Response size |
| `policy_mode` | TEXT | Policy engine mode (`audit_only` or `enforce`) |
| `policy_action` | TEXT | Typed policy action (`allow`, `ask`, `block`, `rewrite`) |
| `policy_rule` | TEXT | Matching rule key, for example `policy.mcp.block_prod_token` |
| `policy_reason` | TEXT | Optional audit reason |
| `trace_id` | TEXT | Cross-table correlation ID |

### dns_events

DNS queries handled by the host DNS proxy.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp` | TEXT | ISO 8601 |
| `qname` | TEXT | Queried name |
| `qtype` | INTEGER | DNS record type |
| `qclass` | INTEGER | DNS class |
| `rcode` | INTEGER | DNS response code |
| `decision` | TEXT | `allowed`, `denied`, `redirected`, or `error` |
| `matched_rule` | TEXT | Domain or Policy DNS rule that matched |
| `source_proto` | TEXT | DNS transport source |
| `process_name` | TEXT | Guest process, when known |
| `upstream_resolver_ms` | INTEGER | Upstream resolver latency |
| `trace_id` | TEXT | Cross-table correlation ID |
| `policy_mode` | TEXT | Policy engine mode, when set |
| `policy_action` | TEXT | Typed policy action (`allow`, `ask`, `block`, `rewrite`) |
| `policy_rule` | TEXT | Matching policy rule key |
| `policy_reason` | TEXT | Optional audit reason or fail-closed detail |

### policy_hook_events

Policy Hook Spec0 client attempts and fail-closed fallbacks. These rows are
written when the hook client path is exercised; configured external hook
dispatch is not a shipped user-facing runtime path in this release.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp` | TEXT | ISO 8601 |
| `endpoint_id` | TEXT | Hook endpoint identifier |
| `spec_version` | TEXT | Hook wire-spec version |
| `spec_hash` | TEXT | Hash of the OpenAPI/Spec0 contract |
| `decision_id` | TEXT | Hook response decision ID, when a valid response is received |
| `callback` | TEXT | Callback subject, such as `http.request` |
| `decision` | TEXT | `allow`, `ask`, `block`, or `rewrite` for valid hook responses; NULL for transport/schema failures |
| `rule_id` | TEXT | Hook-provided rule ID, when present |
| `reason` | TEXT | Hook-provided reason, when present |
| `latency_ms` | INTEGER | Hook round-trip latency |
| `status` | TEXT | `allowed`, `denied`, or `error` |
| `error` | TEXT | Transport, status, schema, timeout, or body-cap error text |
| `fallback` | TEXT | Fail-closed fallback decision (`block` or `ask`), when used |
| `audit_tags` | TEXT | JSON array of hook audit tags |
| `trace_id` | TEXT | Cross-table correlation ID |
| `session_id` | TEXT | Session identifier, when known |

### exec_events

Commands executed through Capsem service APIs and MCP tools.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp` | TEXT | ISO 8601 |
| `exec_id` | INTEGER | Per-session exec identifier |
| `command` | TEXT | Command string |
| `exit_code` | INTEGER | Process exit code, when complete |
| `duration_ms` | INTEGER | Runtime duration, when complete |
| `stdout_preview` | TEXT | Truncated stdout |
| `stderr_preview` | TEXT | Truncated stderr |
| `stdout_bytes` | INTEGER | Full stdout byte count |
| `stderr_bytes` | INTEGER | Full stderr byte count |
| `source` | TEXT | Source path, usually `api` or MCP |
| `mcp_call_id` | INTEGER | Related `mcp_calls.id`, when known |
| `trace_id` | TEXT | Cross-table correlation ID |
| `process_name` | TEXT | Guest process name, when known |
| `pid` | INTEGER | Guest process ID, when known |

### audit_events

Kernel audit `execve` records streamed from the guest over vsock:5006.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp` | TEXT | ISO 8601 |
| `pid` | INTEGER | Guest process ID |
| `ppid` | INTEGER | Guest parent process ID |
| `uid` | INTEGER | Guest user ID |
| `exe` | TEXT | Executable path |
| `comm` | TEXT | Kernel command name |
| `argv` | TEXT | Reconstructed command arguments |
| `cwd` | TEXT | Working directory |
| `exit_code` | INTEGER | Exit code, when known |
| `session_id` | INTEGER | Kernel audit session ID |
| `tty` | TEXT | TTY, when present |
| `audit_id` | TEXT | Kernel audit event ID |
| `exec_event_id` | INTEGER | Related `exec_events.id`, when correlated |
| `parent_exe` | TEXT | Parent executable, when known |
| `trace_id` | TEXT | Cross-table correlation ID |

### fs_events

File system changes in the workspace (tracked by VirtioFS).

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp` | TEXT | ISO 8601 |
| `action` | TEXT | `created`, `modified`, `deleted`, `restored` |
| `path` | TEXT | File path relative to workspace |
| `size` | INTEGER | File size in bytes |
| `trace_id` | TEXT | Cross-table correlation ID |

### snapshot_events

Automatic and manual workspace snapshots.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp` | TEXT | ISO 8601 |
| `slot` | INTEGER | Ring buffer slot (0-11 for auto) |
| `origin` | TEXT | `auto` or `manual` |
| `name` | TEXT | Optional snapshot name |
| `files_count` | INTEGER | Files in snapshot |
| `start_fs_event_id` | INTEGER | First fs_event in range |
| `stop_fs_event_id` | INTEGER | Last fs_event in range |
| `trace_id` | TEXT | Cross-table correlation ID |

## Data flow

```mermaid
graph LR
    subgraph "Event Sources"
        MITM["MITM Proxy<br/>(vsock:5002)"]
        MCP["MITM MCP Endpoint<br/>(framed vsock:5002)"]
        DNS["DNS Proxy"]
        EXEC["Service exec path"]
        AUDIT["Guest audit stream<br/>(vsock:5006)"]
        FS["VirtioFS<br/>(file watcher)"]
        SNAP["Snapshot scheduler"]
        HOOK["Policy Hook client"]
    end

    subgraph "Writer Pipeline"
        CH["tokio mpsc channel"]
        WT["Dedicated writer thread<br/>(capsem-db-writer)"]
        DB["session.db<br/>(SQLite WAL)"]
    end

    MITM -->|"WriteOp::NetEvent<br/>WriteOp::ModelCall"| CH
    MCP -->|"WriteOp::McpCall"| CH
    DNS -->|"WriteOp::DnsEvent"| CH
    HOOK -->|"WriteOp::PolicyHookEvent"| CH
    EXEC -->|"WriteOp::ExecEvent<br/>WriteOp::ExecEventComplete"| CH
    AUDIT -->|"WriteOp::AuditEvent"| CH
    FS -->|"WriteOp::FileEvent"| CH
    SNAP -->|"WriteOp::SnapshotEvent"| CH
    CH --> WT
    WT --> DB
```

### Write operations

| Variant | Source | Table(s) |
|---------|--------|----------|
| `WriteOp::NetEvent` | MITM proxy | `net_events` |
| `WriteOp::ModelCall` | MITM proxy (AI traffic) | `model_calls` + `tool_calls` + `tool_responses` |
| `WriteOp::McpCall` | MITM MCP endpoint | `mcp_calls` |
| `WriteOp::ExecEvent` / `ExecEventComplete` | Service exec path | `exec_events` |
| `WriteOp::AuditEvent` | Guest audit stream | `audit_events` |
| `WriteOp::FileEvent` | VirtioFS watcher | `fs_events` |
| `WriteOp::SnapshotEvent` | Snapshot scheduler | `snapshot_events` |
| `WriteOp::DnsEvent` | DNS proxy | `dns_events` |
| `WriteOp::PolicyHookEvent` | Policy Hook client | `policy_hook_events` |

## Policy Decision Audit

Use `just query-session` to prove that a policy decision happened at the
intended boundary and that blocked or rewritten payloads did not leak.

### MCP

```bash
just query-session "
SELECT timestamp, tool_name, decision, policy_action, policy_rule, policy_reason, error_message
FROM mcp_calls
WHERE policy_rule IS NOT NULL
ORDER BY id DESC
LIMIT 20;"
```

For no-dispatch checks, pair the policy row with the expected error response:

```bash
just query-session "
SELECT tool_name, policy_action, policy_rule, response_preview
FROM mcp_calls
WHERE policy_action IN ('ask', 'block', 'rewrite')
ORDER BY id DESC
LIMIT 20;"
```

MCP named Policy V2 blocks use `policy_action = 'block'`. The coarse
`mcp_calls.decision` field still uses `denied` for denied JSON-RPC outcomes.

### HTTP

```bash
just query-session "
SELECT timestamp, domain, method, path, decision, matched_rule, status_code
     , policy_action, policy_rule, policy_reason
FROM net_events
WHERE matched_rule IS NOT NULL OR policy_rule IS NOT NULL
ORDER BY id DESC
LIMIT 20;"
```

Header-strip rules should be checked against the captured headers:

```bash
just query-session "
SELECT domain, request_headers, response_headers
FROM net_events
WHERE matched_rule = 'policy.http.strip_credentials'
ORDER BY id DESC
LIMIT 5;"
```

The stripped header names may appear as keys depending on capture settings,
but stripped secret values must not appear in header or body preview fields.

### DNS

```bash
just query-session "
SELECT timestamp, qname, qtype, rcode, decision, matched_rule, process_name
     , policy_action, policy_rule, policy_reason
FROM dns_events
WHERE matched_rule IS NOT NULL OR policy_rule IS NOT NULL OR decision != 'allowed'
ORDER BY id DESC
LIMIT 20;"
```

DNS block rows prove no upstream resolution happened when
`upstream_resolver_ms = 0`. DNS rewrite rows should carry the policy rule and
`policy_action = 'rewrite'`; synthetic answer payloads are not stored in
session telemetry.

### Model and Tool Traffic

Model policy uses the existing parsed AI rows plus policy rule metadata as
the enforcement slice lands. Today, use these rows to prove the subject and
no-leak side of model policy tests:

```bash
just query-session "
SELECT id, provider, model, path, trace_id, request_body_preview, text_content
FROM model_calls
ORDER BY id DESC
LIMIT 10;"
```

```bash
just query-session "
SELECT tc.tool_name, tc.origin, tc.arguments, tr.content_preview, tc.trace_id
FROM tool_calls tc
LEFT JOIN tool_responses tr
  ON tr.call_id = tc.call_id AND tr.trace_id = tc.trace_id
ORDER BY tc.id DESC
LIMIT 20;"
```

Model request policy records no-leak decisions on the associated `net_events`
row. Model response, tool-call, and tool-response enforcement use the same
rule, decision, and reason vocabulary on `net_events`; response-side rewrites
must show only the rewritten preview.

For model-extracted tool calls, `tool_calls.origin` uses `native`, `local`, or
`mcp_proxy`. The `tool_calls.mcp_call_id` column exists for future direct
correlation, but the current model telemetry path does not populate it.

### Policy Hooks

Policy Hook Spec0 client attempts write one row per decision attempt:

```bash
just query-session "
SELECT endpoint_id, callback, decision, status, rule_id, latency_ms,
       fallback, error, trace_id
FROM policy_hook_events
ORDER BY id DESC
LIMIT 20;"
```

Rows include the endpoint id, Spec0 version/hash, decision id, callback,
decision, rule id, reason, latency, timeout/schema/transport error text,
fail-closed fallback decision, audit tags, `trace_id`, and `session_id`.
Hook tests should also query the downstream boundary row (`mcp_calls`,
`net_events`, `dns_events`, or `model_calls`) when proving no-dispatch and
no-leak behavior.

## Writer Architecture

The `DbWriter` spawns a dedicated thread that owns the SQLite connection:

1. Async callers send `WriteOp` via `tx.send()` (non-blocking)
2. Writer thread blocks on `rx.blocking_recv()` for the first op
3. After receiving one op, drains the rest of the queue
4. Executes all drained ops in a single SQLite transaction
5. Repeats

This **block-then-drain** pattern batches writes for efficiency while keeping the async callers non-blocking. The channel has configurable backpressure capacity.

SQLite pragmas: WAL journal mode, NORMAL synchronous. Field values are defensively capped at 256 KB.

**Drop order is critical:** `Drop::drop()` takes `tx` before joining the thread. Without this, the join would deadlock (thread waits for all senders to drop, but `tx` drops after the join).

## AI traffic enrichment

```mermaid
graph TD
    A["MITM proxy receives<br/>AI provider response"] --> B["AiResponseBody wraps<br/>hyper Body"]
    B --> C["poll_frame() feeds bytes<br/>to SseParser"]
    C --> D["SseParser emits SseEvent"]
    D --> E["ProviderStreamParser<br/>(Anthropic/OpenAI/Google)"]
    E --> F["Vec&lt;LlmEvent&gt;"]
    F --> G["collect_summary()"]
    G --> H["StreamSummary<br/>(text, tools, tokens, cost)"]
    H --> I["TelemetryEmitter.emit_model_call()"]
    I --> J["WriteOp::ModelCall<br/>with ToolCallEntry + ToolResponseEntry"]
```

For AI provider traffic, the response body is parsed inline to extract:
- Model name and message ID
- Text and thinking output
- Tool calls with arguments and origin classification
- Token usage (input, output, cache_read, thinking breakdowns)
- Cost estimate from embedded pricing table
- Stop reason (end_turn, tool_use, max_tokens)
- Trace ID for multi-turn correlation

## Aggregation queries

The `DbReader` provides pre-built aggregate queries:

| Query | Returns | Use case |
|-------|---------|----------|
| `session_stats()` | `SessionStats` | Dashboard summary: totals for net, model, tokens, cost |
| `provider_token_usage()` | `Vec<ProviderTokenUsage>` | Per-provider breakdown: call count, tokens, cost |
| `domain_counts()` | `Vec<DomainCount>` | Per-domain request counts with allowed/denied split |
| `time_buckets()` | `Vec<TimeBucket>` | Requests over time (for charts) |
| `tool_usage()` | `Vec<ToolUsageCount>` | Most-used tools by call count |
| `tool_usage_with_stats()` | `Vec<ToolUsageWithStats>` | Tool usage with byte and duration stats |
| `mcp_tool_usage()` | `Vec<McpToolUsage>` | MCP tool usage by server and tool name |
| `trace_summaries()` | `Vec<TraceSummary>` | Per-trace: tokens, cost, duration, tool count |
| `trace_detail(id)` | `TraceDetail` | All model calls in a trace with tool data |

## Access patterns

| Access point | Protocol | Query type |
|-------------|----------|------------|
| `capsem inspect <id> "SQL"` | CLI -> service HTTP `/inspect/{id}` | Raw SQL (read-only) |
| `capsem info <id> --stats` | CLI -> service HTTP `/info/{id}` | Pre-built `SessionStats` |
| MCP `capsem_inspect` | MCP -> service HTTP `/inspect/{id}` | Raw SQL (read-only) |
| MCP `capsem_inspect_schema` | MCP -> service HTTP | Table schemas for LLM context |
| Frontend dashboard | Gateway -> `/inspect/{id}` | sql.js in-browser (downloads session.db) |

The `/inspect` endpoint executes arbitrary SQL against the session database in read-only mode (`query_only` pragma). The reader connection uses separate pragmas from the writer.

## Per-VM isolation

| Property | Value |
|----------|-------|
| Location | `~/.capsem/sessions/{id}/session.db` |
| Lifetime | Created at VM boot, destroyed with ephemeral VM or preserved with persistent VM |
| Access | Only the owning capsem-process can write; service reads via IPC |
| VirtioFS boundary | `session.db` is outside the VirtioFS share; guest cannot access it |
| Concurrent access | WAL mode allows concurrent reader + writer |
| Fork behavior | `capsem fork` checkpoints and copies session.db into the image |

## Key source files

| File | Purpose |
|------|---------|
| `capsem-logger/src/schema.rs` | Table DDL, pragmas, migrations |
| `capsem-logger/src/events.rs` | Event structs (NetEvent, ModelCall, McpCall, etc.) |
| `capsem-logger/src/writer.rs` | DbWriter, WriteOp, block-then-drain loop |
| `capsem-logger/src/reader.rs` | DbReader, aggregation queries, raw SQL |
| `capsem-logger/src/db.rs` | SessionDb convenience wrapper |
