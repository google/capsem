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
        text event_id
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
        text event_id
        text server_name
        text method
        text tool_name
        text decision
        int duration_ms
    }
    dns_events {
        int id PK
        text event_id
        text qname
        int qtype
        int rcode
        text decision
    }
    security_rule_events {
        int id PK
        text event_id
        text event_type
        text rule_id
        text rule_action
        text detection_level
        text rule_json
        text event_json
    }
    security_ask_events {
        int id PK
        text ask_id
        text event_id
        text event_type
        text rule_id
        text status
        text rule_json
        text event_json
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
    net_events ||--o{ security_rule_events : "event_id"
    mcp_calls ||--o{ security_rule_events : "event_id"
    dns_events ||--o{ security_rule_events : "event_id"
    security_rule_events ||--o{ security_ask_events : "event_id"
```

## Tables

### net_events

Every HTTP request through the MITM proxy, whether allowed or denied.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `event_id` | TEXT | 12-hex primary event id for `security_rule_events` joins |
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
| `matched_rule` | TEXT | Legacy/domain policy helper; security rule truth is in `security_rule_events` |
| `request_headers` | TEXT | Request headers (when body logging enabled) |
| `response_headers` | TEXT | Response headers |
| `request_body_preview` | TEXT | First 4 KB of request body |
| `response_body_preview` | TEXT | First 4 KB of response body |
| `conn_type` | TEXT | Default `https`, `https-mitm` for proxied |
| `policy_mode` | TEXT | Policy engine mode, when set |
| `policy_action` | TEXT | Legacy helper; use `security_rule_events.rule_action` for security rules |
| `policy_rule` | TEXT | Legacy helper; use `security_rule_events.rule_id` for security rules |
| `policy_reason` | TEXT | Legacy helper; use `security_rule_events.rule_json` for rule reason |
| `trace_id` | TEXT | Cross-table correlation ID |

### model_calls

AI provider API calls with parsed response metadata.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `event_id` | TEXT | 12-hex primary event id for `security_rule_events` joins |
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
| `mcp_call_id` | INTEGER | FK to `mcp_calls` (reserved, not yet populated) |
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
| `policy_mode` | TEXT | Legacy MCP policy mode, when used |
| `policy_action` | TEXT | Legacy helper; use `security_rule_events.rule_action` for security rules |
| `policy_rule` | TEXT | Legacy helper; use `security_rule_events.rule_id` for security rules |
| `policy_reason` | TEXT | Legacy helper; use `security_rule_events.rule_json` for rule reason |
| `trace_id` | TEXT | Cross-table correlation ID |

### dns_events

DNS queries handled by the host DNS proxy.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `event_id` | TEXT | 12-hex primary event id for `security_rule_events` joins |
| `timestamp` | TEXT | ISO 8601 |
| `qname` | TEXT | Queried name |
| `qtype` | INTEGER | DNS record type |
| `qclass` | INTEGER | DNS class |
| `rcode` | INTEGER | DNS response code |
| `decision` | TEXT | `allowed`, `denied`, `redirected`, or `error` |
| `matched_rule` | TEXT | Legacy/domain policy helper; security rule truth is in `security_rule_events` |
| `source_proto` | TEXT | DNS transport source |
| `process_name` | TEXT | Guest process, when known |
| `upstream_resolver_ms` | INTEGER | Upstream resolver latency |
| `trace_id` | TEXT | Cross-table correlation ID |
| `policy_mode` | TEXT | Policy engine mode, when set |
| `policy_action` | TEXT | Legacy helper; use `security_rule_events.rule_action` for security rules |
| `policy_rule` | TEXT | Legacy helper; use `security_rule_events.rule_id` for security rules |
| `policy_reason` | TEXT | Legacy helper; use `security_rule_events.rule_json` for rule reason |

### security_rule_events

Every matched security rule, across HTTP, DNS, MCP, model, file, process,
credential, and snapshot events.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp_unix_ms` | INTEGER | Match timestamp |
| `event_id` | TEXT | 12-hex primary event id from the protocol/event table |
| `event_type` | TEXT | Canonical security event type such as `http.request`, `mcp.tool_call`, or `file.read` |
| `rule_id` | TEXT | Stable rule id such as `profiles.rules.skill_loaded` |
| `rule_action` | TEXT | `allow`, `ask`, `block`, `preprocess`, or `postprocess` |
| `detection_level` | TEXT | `none`, `informational`, `low`, `medium`, `high`, or `critical` |
| `rule_json` | TEXT | JSON rule snapshot at match time |
| `event_json` | TEXT | JSON normalized `SecurityEvent` payload matched by the rule |
| `trace_id` | TEXT | Cross-table correlation ID |

This table is the forensic rule ledger. Runtime `/latest` and `/info` style
views must be regeneratable from these rows and the primary event tables.

### security_ask_events

Append-only lifecycle rows for `ask` decisions.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp_unix_ms` | INTEGER | Ask lifecycle timestamp |
| `ask_id` | TEXT | 12-hex ask id |
| `event_id` | TEXT | 12-hex primary event id |
| `event_type` | TEXT | Canonical security event type |
| `rule_id` | TEXT | Rule that requested ask |
| `rule_name` | TEXT | Rule telemetry name |
| `status` | TEXT | `pending`, `approved`, or `denied` |
| `rule_json` | TEXT | JSON rule snapshot |
| `event_json` | TEXT | JSON normalized `SecurityEvent` payload |
| `resolver` | TEXT | Approver/resolver identity, when present |
| `reason` | TEXT | Resolution reason, when present |
| `trace_id` | TEXT | Cross-table correlation ID |

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
    end

    subgraph "Writer Pipeline"
        CH["tokio mpsc channel"]
        WT["Dedicated writer thread<br/>(capsem-db-writer)"]
        DB["session.db<br/>(SQLite WAL)"]
    end

    MITM -->|"WriteOp::NetEvent<br/>WriteOp::ModelCall"| CH
    MCP -->|"WriteOp::McpCall"| CH
    DNS -->|"WriteOp::DnsEvent"| CH
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
| `WriteOp::SecurityRuleEvent` | Security engine | `security_rule_events` |
| `WriteOp::SecurityAskEvent` | Security engine | `security_ask_events` |

## Security Rule Audit

Use `just query-session` to prove that a security rule matched, which primary
event it matched, and which normalized payload the rule saw. The ledger is
`security_rule_events`; protocol tables provide the boundary-specific details.

### Latest Rule Matches

```bash
just query-session "
SELECT event_id, event_type, rule_id, rule_action, detection_level, trace_id
FROM security_rule_events
ORDER BY timestamp_unix_ms DESC
LIMIT 20;"
```

For forensic review, inspect the stored rule and event snapshots:

```bash
just query-session "
SELECT rule_id, rule_json, event_json
FROM security_rule_events
WHERE event_id = '<event_id>'
ORDER BY id DESC;"
```

### HTTP Join

```bash
just query-session "
SELECT n.event_id, n.domain, n.method, n.path, n.decision,
       s.rule_id, s.rule_action, s.detection_level
FROM net_events n
JOIN security_rule_events s ON s.event_id = n.event_id
ORDER BY n.id DESC
LIMIT 20;"
```

### DNS Join

```bash
just query-session "
SELECT d.event_id, d.qname, d.qtype, d.rcode, d.decision,
       s.rule_id, s.rule_action, s.detection_level
FROM dns_events d
JOIN security_rule_events s ON s.event_id = d.event_id
ORDER BY d.id DESC
LIMIT 20;"
```

### MCP Join

```bash
just query-session "
SELECT m.event_id, m.server_name, m.method, m.tool_name, m.decision,
       s.rule_id, s.rule_action, s.detection_level, m.error_message
FROM mcp_calls m
JOIN security_rule_events s ON s.event_id = m.event_id
ORDER BY m.id DESC
LIMIT 20;"
```

### Ask Lifecycle

```bash
just query-session "
SELECT ask_id, event_id, rule_id, rule_name, status, resolver, reason
FROM security_ask_events
ORDER BY timestamp_unix_ms DESC
LIMIT 20;"
```

For no-dispatch checks, pair an `ask` or `block` rule row with the primary
event row and the expected boundary result. The rule decision is
`security_rule_events.rule_action`; the primary table's `decision` remains the
transport outcome at that boundary.

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
| `capsem info <id> --stats` | CLI -> service HTTP `/vms/{id}/info` | Pre-built `SessionStats` |
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
