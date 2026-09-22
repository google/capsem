---
title: Session Telemetry
description: Per-VM SQLite database schema, data flow, and query patterns.
sidebar:
  order: 20
---

Every Capsem VM gets its own session ledger: a SQLite database (`session.db`) that records network requests, DNS queries, AI model calls, MCP tool invocations, exec activity, kernel audit events, file changes, security rule matches, credential substitutions, and snapshots, and beside it a body archive (`session.bodies`) that holds the full request, response, tool, exec and security payloads those rows describe. The two files live in the session directory and follow the VM lifecycle; retained and forked VMs keep both for forensic review, and one without the other is not a ledger.

The session ledger has no migrations. A ledger written by an older build fails to open, by name, instead of being upgraded in place.

## Session Identity

The route/session `id` is an opaque VM id and is the only key that may select a
session directory, `session.db`, active instance, DB handle, terminal/log/stats
route, or UI tab. A user-facing VM name such as `co-work1` or `code-vm1` is a
display/resume alias only.

API payloads that describe persistent sessions must keep both fields:
`id` for routing and DB lookup, `name` for display. User surfaces may accept
commands like `capsem resume co-work1`, but that layer must translate the name
to the VM id before calling `/vms/{id}/...`. Service route code must not
collapse `name` into `id`; doing so can make a selected session read another
session's database and show the wrong provider/model data.

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
        text event_id
        text provider
        text model
        int input_tokens
        int output_tokens
        real estimated_cost_usd
        text trace_id
        text turn_id
    }
    model_items {
        int id PK
        int model_call_id FK
        text item_type
        int item_index
        text call_id
        text trace_id
        text turn_id
    }
    tool_calls {
        int id PK
        text event_id
        int model_call_id FK
        text call_id
        text tool_name
        text origin
        text trace_id
        text turn_id
    }
    tool_responses {
        int id PK
        int model_call_id FK
        text call_id
        text content_preview
        text trace_id
        text turn_id
    }
    dns_events {
        int id PK
        text event_id
        text qname
        int qtype
        int rcode
        text decision
    }
    event_body_blobs {
        int id PK
        text event_id
        text event_type
        text source_table
        text direction
        text body_hash
        int block_offset FK
        int body_offset
        int body_len
    }
    body_blocks {
        int block_offset PK
        int raw_len
        int disk_len
        text sealed_at
    }
    security_rule_events {
        int id PK
        text event_id
        text event_type
        text rule_id
        text rule_action
        text detection_level
        text rule_json
    }
    security_decision_events {
        int id PK
        text event_id
        text event_type
        text stage
        text actor
        text effective_decision
    }
    security_ask_events {
        int id PK
        text ask_id
        text event_id
        text event_type
        text rule_id
        text status
        text rule_json
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
        text kind
        text path
        int size
    }
    model_calls ||--o{ model_items : "orders"
    model_calls ||--o{ tool_calls : "emits"
    tool_calls ||--o{ tool_responses : "same call_id"
    model_calls ||--o{ tool_responses : "consumes"
    net_events ||--o{ security_rule_events : "event_id"
    net_events ||--o{ event_body_blobs : "event_id"
    model_calls ||--o{ event_body_blobs : "event_id"
    tool_calls ||--o{ event_body_blobs : "event_id"
    tool_responses ||--o{ event_body_blobs : "event_id"
    exec_events ||--o{ event_body_blobs : "event_id"
    security_rule_events ||--o{ event_body_blobs : "event_id"
    security_decision_events ||--o{ event_body_blobs : "event_id"
    security_ask_events ||--o{ event_body_blobs : "event_id"
    body_blocks ||--o{ event_body_blobs : "block_offset"
    dns_events ||--o{ security_rule_events : "event_id"
    security_rule_events ||--o{ security_ask_events : "event_id"
```

## Identity Graph

The ledger uses a small set of scoped ids. Each id has one job, and no
provider id replaces these joins:

- `event_id` identifies one emitted ledger event row. Security rows, body blobs,
  and event detail routes join back through this id.
- `trace_id` groups runtime work caused by one causal operation across tables:
  HTTP, DNS, model, tool, file, process, credentials, and security.
- `turn_id` groups all work caused by one user-visible agent turn: the user's
  input, every provider exchange needed to answer it, every tool request and
  response, and every emitted HTTP/DNS/file/process/security row caused by it.
- `model_call_id` is the `model_calls.id` value for exactly one provider
  request/response exchange inside a turn. It owns that exchange's request,
  reasoning/thinking, response, model-emitted tool-call items, token counts, and
  provider metadata. It is not the whole user turn; a single `turn_id` can
  contain multiple `model_call_id` values.
- `tool_call_id` identifies one logical tool invocation across model-native
  tools, MCP transport, Capsem built-ins, and local tools. In SQLite it is stored
  as `tool_calls.call_id` and `tool_responses.call_id`.

Provider response ids, message ids, and transport request ids are provider or
transport metadata. They are not Capsem's join contract.

```mermaid
flowchart TD
    Session["session_id<br/>one Capsem session database"]
    Trace["trace_id<br/>causal runtime chain"]
    Turn["turn_id<br/>one user-visible agent turn<br/>user input + all resulting work"]

    ModelA["model_call_id A = model_calls.id<br/>one provider exchange<br/>one request + one response"]
    ModelB["model_call_id B = model_calls.id<br/>later provider exchange<br/>same user-visible turn"]
    ModelAItems["model_items for model_call_id A<br/>request, reasoning, response, tool_call"]
    ModelBItems["model_items for model_call_id B<br/>tool_response input, request, reasoning, response"]

    ToolA1["tool_call_id A1 = tool_calls.call_id<br/>logical tool invocation"]
    ToolA2["tool_call_id A2 = tool_calls.call_id<br/>logical tool invocation"]
    ToolA1Response["tool_responses.call_id = A1<br/>same tool_call_id"]
    ToolA2Response["tool_responses.call_id = A2<br/>same tool_call_id"]
    McpFacts["MCP transport facts<br/>origin/type enrichment<br/>same tool_call_id, no duplicate ledger"]

    EventRows["event_id rows<br/>http, dns, model, tool, file, process, credential, security"]
    BodyBlobs["event_body_blobs<br/>index by event_id into session.bodies"]
    SecurityRows["security_rule_events<br/>rule matches by event_id"]
    ProviderIds["provider response_id / message_id / transport ids<br/>metadata only"]

    Session --> Trace
    Trace --> Turn
    Turn -->|"contains 1..N"| ModelA
    Turn -->|"contains 1..N"| ModelB

    ModelA -->|"owns ordered rows"| ModelAItems
    ModelB -->|"owns ordered rows"| ModelBItems
    ModelA -->|"emits 0..N"| ToolA1
    ModelA -->|"emits 0..N"| ToolA2

    ToolA1 -->|"response reuses id"| ToolA1Response
    ToolA2 -->|"response reuses id"| ToolA2Response
    McpFacts -.->|"enriches"| ToolA1
    McpFacts -.->|"enriches"| ToolA2
    ToolA1Response -->|"can feed later exchange"| ModelB
    ToolA2Response -->|"can feed later exchange"| ModelB

    Trace --> EventRows
    Turn --> EventRows
    ModelA --> EventRows
    ModelB --> EventRows
    ToolA1 --> EventRows
    ToolA2 --> EventRows
    EventRows --> BodyBlobs
    EventRows --> SecurityRows
    ModelA -.-> ProviderIds
    ModelB -.-> ProviderIds
```

One `turn_id` is the user-input scope. It can contain multiple
`model_call_id` values when an agent calls the model, executes tools, then calls
the model again with tool results. One `model_call_id` is one provider-exchange
scope and carries that exchange's request, reasoning/thinking, response, token
counts, and ordered `model_items`. It can emit zero or more `tool_call_id`
values; this is the canonical one-to-many relationship for model-visible tools.
Stated as the debugging invariant: one `model_call_id` can emit N
`tool_call_id` values, and each emitted tool response must reuse that
`tool_call_id`.
A tool response must carry the same `tool_call_id` as the tool request. In the
current SQLite schema, the persisted `tool_call_id` value is stored in
`tool_calls.call_id` and `tool_responses.call_id`.

MCP is not a second user-facing tool ledger. MCP-origin `tools/call` activity
must resolve to a `tool_calls` row with `origin = 'mcp'` or enrich an existing
logical `tool_call_id`. An MCP call observed without a corresponding logical
tool call is an integrity/security finding, not a separate product counter.

The key cardinalities are:

- One session has many `trace_id` values.
- One `trace_id` has one or more `turn_id` values.
- One `turn_id` has one or more `model_call_id` values.
- One `model_call_id` has one provider request and one provider response.
- One `model_call_id` has many `model_items` rows: request, reasoning,
  response, tool_call, and tool_response items in observed order.
- One `model_call_id` can emit many `tool_call_id` values.
- One `tool_call_id` has one tool request and zero or more observed response
  rows, all with the same `tool_call_id`.
- One `event_id` identifies one emitted row and joins its security, body, and
  display details.

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
| `matched_rule` | TEXT | Compatibility helper; security rule truth is in `security_rule_events` |
| `request_headers` | TEXT | Request headers (when body logging enabled) |
| `response_headers` | TEXT | Response headers |
| `request_body_preview` | TEXT | Compact display field; forensic body truth is in `event_body_blobs` |
| `response_body_preview` | TEXT | Compact display field; forensic body truth is in `event_body_blobs` |
| `conn_type` | TEXT | Default `https`, `https-mitm` for proxied |
| `policy_mode` | TEXT | Transport-local policy mode hint, when set |
| `policy_action` | TEXT | Denormalized transport hint; `security_rule_events.rule_action` is rule truth |
| `policy_rule` | TEXT | Denormalized transport hint; `security_rule_events.rule_id` is rule truth |
| `policy_reason` | TEXT | Denormalized transport hint; `security_rule_events.rule_json` is rule truth |
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
| `request_body_preview` | TEXT | Compact display field; forensic body truth is in `event_body_blobs` |
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
| `turn_id` | TEXT | User-visible agent turn that contains this model exchange |
| `usage_details` | TEXT | JSON: `{"cache_read": 800, "thinking": 200}` |

### event_body_blobs

The index into `session.bodies`, the compressed block archive that holds full
captured bodies: HTTP and model requests and responses, tool results, exec
stdout and stderr, and security rule payloads. The bytes are not in SQLite:
each row names the block they sit in and their span inside it. The primary
protocol tables keep compact display fields (2 KB previews) for table scans;
forensic body truth lives in the archive and joins by `event_id` plus
`direction`.

The blob table is the ledger index for archived bodies; `session.bodies` is
the forensic byte source those rows authenticate and locate. A valid session
ledger requires both.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `event_id` | TEXT | 12-hex event id of the row in `source_table` |
| `event_type` | TEXT | Canonical event type such as `http.request`, `model.call`, or `mcp.tool_call` |
| `source_table` | TEXT | `net_events`, `model_calls`, `tool_calls`, `tool_responses`, `exec_events`, `security_rule_events`, `security_decision_events`, or `security_ask_events` |
| `direction` | TEXT | `request`, `response`, `payload` (the event a rule match, decision or ask is about), `stdout` or `stderr` |
| `content_type` | TEXT | MIME type or protocol content type, when known |
| `original_bytes` | INTEGER | Full body byte count observed at the boundary |
| `stored_bytes` | INTEGER | Bytes actually archived, after the 10 MiB per-direction cap |
| `truncated` | INTEGER | `1` when the persisted body hit the capture limit |
| `body_hash` | TEXT | `blake3:*` hash of the **archived** bytes, so a read can verify what it got against the row that named it |
| `block_offset` | INTEGER | Offset of the `session.bodies` block holding this body, keyed to `body_blocks` |
| `body_offset` | INTEGER | Offset of this body inside that block's inflated bytes |
| `body_len` | INTEGER | Length of this body inside that block, always equal to `stored_bytes` |
| `trace_id` | TEXT | Cross-table correlation ID |
| `turn_id` | TEXT | User-visible agent turn |
| `created_at` | TEXT | Insert timestamp |

`UNIQUE(event_id, source_table, direction)`: one event has at most one body per
table and direction. Several tables can each hold a body for the same event --
a rule match, the decision it drove and an ask it raised each archive a
`payload`.

Rows may share a span. Bytes identical to a body already in the block being
written are indexed against that body's `block_offset`, `body_offset` and
`body_len` rather than written again; `original_bytes` and `truncated` stay per
row. Sharing never crosses a block, so every row naming a block lives and dies
with it under retention.

The UI and debug routes may render parsed JSON, text, or binary summaries from
the archived bytes, but they must not invent a second body source. If a compact
preview and an archived body disagree, the archive is the ledger.

### body_blocks

One row per block of `session.bodies`, so a reader seeks straight to a block
instead of scanning the file. A block stays open across disk flushes and grows
by one segment each, so its row is updated as it grows.

| Column | Type | Description |
|--------|------|-------------|
| `block_offset` | INTEGER PK | Byte offset of the block's header in `session.bodies` |
| `raw_len` | INTEGER | Inflated size of the block's committed segments |
| `disk_len` | INTEGER | Committed extent of the block in the file, headers included |
| `sealed_at` | TEXT | When the block's last segment was written; retention cuts by this |

### tool_calls

Canonical product/security tool invocation ledger. One row per model-native,
built-in/local, or MCP-origin tool invocation. User-facing tool counts, CEL tool
evidence, and forensic tool activity start here.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `model_call_id` | INTEGER FK | References `model_calls.id` |
| `call_index` | INTEGER | Position in the response |
| `call_id` | TEXT | Canonical `tool_call_id` value for this logical tool invocation |
| `tool_name` | TEXT | Tool name |
| `arguments` | TEXT | JSON arguments |
| `origin` | TEXT | `native`, `mcp`, `builtin`, `local`, or `mcp_proxy` |
| `trace_id` | TEXT | Cross-table correlation ID |
| `turn_id` | TEXT | User-visible agent turn that contains this tool invocation |

### tool_responses

Tool results from subsequent requests (matched by `call_id`).

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `model_call_id` | INTEGER FK | References `model_calls.id` |
| `call_id` | TEXT | Same canonical `tool_call_id` value as `tool_calls.call_id` |
| `content_preview` | TEXT | Truncated tool result |
| `is_error` | INTEGER | Boolean: 1 if tool returned error |
| `trace_id` | TEXT | Cross-table correlation ID |
| `turn_id` | TEXT | User-visible agent turn that contains this tool response |

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
| `matched_rule` | TEXT | Compatibility helper; security rule truth is in `security_rule_events` |
| `source_proto` | TEXT | DNS transport source |
| `process_name` | TEXT | Guest process, when known |
| `upstream_resolver_ms` | INTEGER | Upstream resolver latency |
| `trace_id` | TEXT | Cross-table correlation ID |
| `policy_mode` | TEXT | Transport-local policy mode hint, when set |
| `policy_action` | TEXT | Denormalized transport hint; `security_rule_events.rule_action` is rule truth |
| `policy_rule` | TEXT | Denormalized transport hint; `security_rule_events.rule_id` is rule truth |
| `policy_reason` | TEXT | Denormalized transport hint; `security_rule_events.rule_json` is rule truth |

### security_rule_events

Every matched security rule, across HTTP, DNS, MCP, model, file, and process
events. Credential substitution and snapshot lifecycle rows may appear in the
ledger, but 1.3 does not expose fake `credential.*` or `snapshot.*` rule roots.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp_unix_ms` | INTEGER | Match timestamp |
| `event_id` | TEXT | 12-hex primary event id from the protocol/event table |
| `event_type` | TEXT | Canonical security event type such as `http.request`, `mcp.tool_call`, or `file.read` |
| `rule_id` | TEXT | Stable rule id such as `profiles.rules.skill_loaded` |
| `rule_action` | TEXT | `allow`, `ask`, `block`, `preprocess`, `rewrite`, or `postprocess` |
| `detection_level` | TEXT | `none`, `informational`, `low`, `medium`, `high`, or `critical` |
| `rule_json` | TEXT | JSON rule snapshot at match time |
| `trace_id` | TEXT | Cross-table correlation ID |

This table is the forensic rule ledger. Runtime `/latest` and `/status` views
must be regeneratable from these rows and the primary event tables.

The normalized `SecurityEvent` payload the rule matched is **not** a column
here. It is a body like any other: stored in the `session.bodies` archive and
indexed by `event_body_blobs` with `source_table = 'security_rule_events'` and
`direction = 'payload'`. It averaged a kilobyte and peaked at 297 KB in one
real session, and the owning process mirrors this table in RAM, so the row
keeps what the views filter and group on and the payload is fetched by event id
when someone actually wants it.

### security_decision_events

Append-only decision transitions: what a stage wanted and what the effective
decision became. Roughly 25 of these a request, most of them `process.audit`.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `timestamp_unix_ms` | INTEGER | Transition timestamp |
| `event_id` | TEXT | 12-hex primary event id |
| `event_type` | TEXT | Canonical security event type |
| `stage` | TEXT | `preprocess`, `rule`, `rewrite`, `postprocess`, or `ask_resolution` |
| `actor` | TEXT | Rule or plugin that requested the decision |
| `rule_id` / `plugin_id` | TEXT | Which one it was, when known |
| `previous_decision` / `requested_decision` / `effective_decision` | TEXT | `allow`, `ask`, or `block` |
| `reason` | TEXT | Why, when stated |
| `trace_id` / `turn_id` / `credential_ref` | TEXT | Correlation and brokered credential |

The event the decision was made about is not a column. It was, and at about
6 KB a row it made this the largest table in a measured session -- 7.5 MB of a
10.5 MB ledger, all of it also mirrored in the owning process's RAM. It is
archived like every other security payload, under
`source_table = 'security_decision_events'`, `direction = 'payload'`.

### security_ask_events

Append-only lifecycle rows for `ask` decisions. The asked-about event is
archived under `source_table = 'security_ask_events'`, `direction = 'payload'`;
the pending row and its resolution carry the same event, and it is archived
once.

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
| `resolver` | TEXT | Approver/resolver identity, when present |
| `reason` | TEXT | Resolution reason, when present |
| `trace_id` | TEXT | Cross-table correlation ID |

### exec_events

Commands executed through Capsem service APIs and MCP tools.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `event_id` | TEXT | 12-hex primary event id for ledger joins |
| `timestamp` | TEXT | ISO 8601 |
| `exec_id` | INTEGER | Per-session exec identifier |
| `command` | TEXT | Command string |
| `exit_code` | INTEGER | Process exit code, when complete |
| `duration_ms` | INTEGER | Runtime duration, when complete |
| `stdout_preview` | TEXT | Display excerpt of stdout; the archived body is in `event_body_blobs` (`direction = 'stdout'`) |
| `stderr_preview` | TEXT | Display excerpt of stderr (`direction = 'stderr'`) |
| `stdout_bytes` | INTEGER | Full stdout byte count |
| `stderr_bytes` | INTEGER | Full stderr byte count |
| `source` | TEXT | Source path, usually `api` or MCP |
| `trace_id` | TEXT | Cross-table correlation ID |
| `process_name` | TEXT | Guest process name, when known |
| `pid` | INTEGER | Guest process ID, when known |
| `credential_ref` | TEXT | Brokered credential reference, when present |

Guest exec output currently reaches the ledger cut to 1 KiB of stdout per
command, so the archived body holds at most that much while `stdout_bytes`
records the true size ([#220](https://github.com/google/capsem/issues/220)).

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

Every change under the workspace, found by the host file monitor polling the
VirtioFS share.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Auto-increment |
| `event_id` | TEXT | 12-hex primary event id for ledger joins |
| `timestamp` | TEXT | ISO 8601 |
| `action` | TEXT | `created`, `modified`, `deleted`, `restored`, or `overflow` |
| `path` | TEXT | Path relative to the workspace (empty for `overflow`) |
| `size` | INTEGER | File size in bytes; none for directories and deletes; for `overflow`, the number of changes deferred |
| `kind` | TEXT | `file`, `dir`, `symlink`, or `other`; security rules read it as `file.kind` |
| `trace_id` | TEXT | Cross-table correlation ID |
| `credential_ref` | TEXT | Brokered credential reference, when present |

**There is no exclusion list.** `.git/hooks`, `.git/config`, `node_modules`,
`.venv` and `target` are where a compromise persists, so every path is
recorded and evaluated by the profile's file rules like any other. Symlinks are
recorded as symlinks and never followed. A change is detected by size, mtime,
inode change time and inode number, so rewriting a file and restoring its
timestamp does not hide the write.

The cost of watching everything is paid in poll cadence, not in dropped events:
each scan times itself and the next one runs ten scan-durations later, between
500ms and 10s. If one scan sees more changes than a window emits, the rest are
held for the next scan and the window is marked by an `overflow` row whose
`size` is how many were held back.

### Snapshot State

Automatic and manual workspace snapshot state is not a session DB table.
Snapshots are host recovery state, exposed through VM-scoped snapshot routes.
Running VMs answer from the `capsem-process` in-memory scheduler over IPC;
stopped VMs reconstruct status from that VM's snapshot metadata only when a
snapshot route is requested. Explicit snapshot MCP calls remain visible as MCP
activity, and file restores remain visible as `fs_events`.

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
        SNAPAPI["VM snapshot routes<br/>/vms/{id}/snapshots/*"]
    end

    subgraph "Writer Pipeline"
        CH["tokio mpsc channel"]
        WT["Dedicated writer thread<br/>(capsem-db-writer)"]
        DB["session.db<br/>(SQLite WAL)"]
        BODIES["session.bodies<br/>(block archive)"]
    end

    MITM -->|"WriteOp::NetEvent<br/>WriteOp::ModelCall"| CH
    MCP -->|"WriteOp::McpCall"| CH
    DNS -->|"WriteOp::DnsEvent"| CH
    EXEC -->|"WriteOp::ExecEvent<br/>WriteOp::ExecEventComplete"| CH
    AUDIT -->|"WriteOp::AuditEvent"| CH
    FS -->|"WriteOp::FileEvent"| CH
    SNAP -->|"in-memory IPC status"| SNAPAPI
    CH --> WT
    WT --> DB
    WT -->|"bodies, one segment per flush"| BODIES
```

The writer thread owns both files. A body is fed to the open block's
compressor as its event is written; each disk flush appends what the block
produced since the last one to `session.bodies` as a segment, syncs it, and
only then commits the body's index row and the block's grown extent in
`body_blocks`.

## Body archive

`session.bodies` is append-only: a 16-byte file header, then blocks. A block
is one raw-deflate stream: an 8-byte header (magic and codec), then segments.
Each disk flush sync-flushes the stream -- byte-aligned, dictionary kept -- and
appends what it produced as a segment behind a 52-byte header (`raw_start`,
`raw_len`, `comp_len` and the blake3 of the segment's raw bytes). The block
stays open, so every body compresses against the ones before it, and closes
with a FINAL segment at about 1 MiB, after an hour, at retention, and at
shutdown. The codec is recorded per block, so a second one is a new codec id
rather than a new file version. The format is defined once, in
`crates/capsem-archive/src/format.rs`.

Reading a body is one index lookup, one seek, and an inflate of the block's
segments up to the one that ends the body's span; a reader keeps its place in
the block, so the next body in it costs only the segments after. Every read
checks each segment against its own hash and the body against the
`body_hash` of the row that named it, so an edited index row or a damaged
segment is refused rather than served as someone else's bytes. A reader never
reads past the segment it needs, which is why another process can read a block
the writer is still appending to.

The writer checks at open that every block the index names is inside the file.
If the index names bytes past the end of `session.bodies` -- a crash between
retention's compaction and its index rewrite, or a `session.db` copied without
its archive -- the session stores no further bodies and says so in its log;
the stale rows fail their hash check instead of answering with the wrong bytes.

| Access | What it returns |
|--------|-----------------|
| `DbHandle::read_body(event_id, source_table, direction)` | One archived body, named by the index's whole key |
| `DbHandle::read_bodies(event_id)` | Every archived body of one event |
| `DbHandle::read_bodies_for_events(...)` | One direction for a page of events, bounded by a byte budget |
| `GET /vms/{id}/bodies/{event_id}` | Every body of one event as JSON, 1 MiB each by default (`?max_bytes=` up to 16 MiB); `truncated` means the capture was cut, `truncated_for_transport` that this response was |
| `GET /vms/{id}/bodies/export.warc.gz` | The whole session as a WARC 1.1 file |

### WARC export

The export writes one gzip-member-framed `resource` record per archived body,
so `warcio`, `pywb` and the rest of the web-archive toolchain can read it and
seek within it without Capsem code. It is streamed, not buffered.

- **Record id**: `urn:capsem:{session}:{source_table}:{event_id}:{direction}`.
  The session names the ledger and the rest is the body index's unique key, so
  an id is unique across sessions merged into one collection and points
  straight back at its source. The table is needed: a rule match, the decision
  it drove and an ask it raised each archive a `payload` for the same event.
- **One record per body**: a body several rows share -- every rule a request
  matched, an ask's pending row and its resolution -- is one record, named by
  the first of them.
- **Target URI**: the real `https://` URI for network and model traffic, and a
  `capsem://` URI otherwise: the tool, the exec stream, the security rule
  (`capsem://security/{rule_id}`), the decision's actor
  (`capsem://security-decision/{actor}`) or the ask (`capsem://security-ask/{ask_id}`).
- **Date**: the source row's time. `tool_responses` has no timestamp of its
  own, so its records carry the time the body was archived.
- **Digest**: `WARC-Block-Digest` is blake3, the digest the ledger already
  records; tools that expect base32 sha1 will not verify it.
- **What is left out**: a body whose source row is gone, whose timestamp does
  not parse, whose URI carries a line break, whose bytes fail their hash, or
  that the archive cannot produce is skipped and counted, never described with
  a guess. The file opens and closes with a `warcinfo` record; the closing one
  counts the skips by reason. **A file with no closing `warcinfo` is an export
  that did not finish**, and its records are not the whole session.

### Retention

`vm.resources.retention_days` (default 30) bounds how long bodies are kept:

- At service start, failed-session directories older than the period are
  deleted.
- When a persistent VM stops, its `capsem-process` drops the blocks sealed
  before the cutoff, compacts `session.bodies`, and rewrites the index rows.
  Only the process that owns the ledger's writes does this.
- Ephemeral sessions are deleted whole and are not trimmed.

### Write operations

| Variant | Source | Table(s) |
|---------|--------|----------|
| `WriteOp::NetEvent` | MITM proxy | `net_events` |
| `WriteOp::ModelCall` | MITM proxy (AI traffic) | `model_calls` + `tool_calls` + `tool_responses` |
| `WriteOp::McpCall` | MITM MCP endpoint | `tool_calls` for `tools/call`; `security_rule_events` for protocol evidence |
| `WriteOp::ExecEvent` / `ExecEventComplete` | Service exec path | `exec_events` |
| `WriteOp::AuditEvent` | Guest audit stream | `audit_events` |
| `WriteOp::FileEvent` | VirtioFS watcher | `fs_events` |
| `WriteOp::DnsEvent` | DNS proxy | `dns_events` |
| `WriteOp::SecurityRuleEvent` | Security engine | `security_rule_events` |
| `WriteOp::SecurityDecisionEvent` | Security engine | `security_decision_events` |
| `WriteOp::SecurityAskEvent` | Security engine | `security_ask_events` |

## Security Rule Audit

Query the session database directly to prove that a security rule matched and
which primary event it matched, and read the normalized payload the rule saw
from the body archive. The ledger is `security_rule_events`; protocol tables
provide the boundary-specific details. The queries below set
`SESSION_DB=~/.capsem/run/sessions/<id>/session.db` (a named VM's ledger is
under `~/.capsem/run/persistent/<name>/`).

### Latest Rule Matches

```bash
sqlite3 -readonly "$SESSION_DB" "
SELECT event_id, event_type, rule_id, rule_action, detection_level, trace_id
FROM security_rule_events
ORDER BY timestamp_unix_ms DESC
LIMIT 20;"
```

For forensic review, inspect the stored rule snapshot:

```bash
sqlite3 -readonly "$SESSION_DB" "
SELECT rule_id, rule_action, detection_level, rule_json, trace_id, credential_ref
FROM security_rule_events
WHERE event_id = '<event_id>'
ORDER BY id DESC;"
```

The matched event's payload is not a column and no SQL recipe returns it: it is
archive-backed, and `event_body_blobs` says where:

```bash
sqlite3 -readonly "$SESSION_DB" "
SELECT direction, content_type, original_bytes, stored_bytes, truncated, body_hash
FROM event_body_blobs
WHERE event_id = '<event_id>' AND source_table = 'security_rule_events';"
```

The bytes come back through `GET /vms/{id}/bodies/{event_id}`, with their size
and hash. Offline, from a copy of a session, `tests/helpers/body_archive.py`
reads `session.bodies` the way the product does and verifies both hashes on the
way out.

### HTTP Join

```bash
sqlite3 -readonly "$SESSION_DB" "
SELECT n.event_id, n.domain, n.method, n.path, n.decision,
       s.rule_id, s.rule_action, s.detection_level
FROM net_events n
JOIN security_rule_events s ON s.event_id = n.event_id
ORDER BY n.id DESC
LIMIT 20;"
```

### DNS Join

```bash
sqlite3 -readonly "$SESSION_DB" "
SELECT d.event_id, d.qname, d.qtype, d.rcode, d.decision,
       s.rule_id, s.rule_action, s.detection_level
FROM dns_events d
JOIN security_rule_events s ON s.event_id = d.event_id
ORDER BY d.id DESC
LIMIT 20;"
```

### MCP-Origin Tool Join

```bash
sqlite3 -readonly "$SESSION_DB" "
SELECT t.event_id, t.server_name, t.method, t.tool_name, t.decision,
       s.rule_id, s.rule_action, s.detection_level, t.error_message
FROM tool_calls t
LEFT JOIN security_rule_events s ON s.event_id = t.event_id
WHERE t.origin = 'mcp'
ORDER BY t.id DESC
LIMIT 20;"
```

### Ask Lifecycle

```bash
sqlite3 -readonly "$SESSION_DB" "
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

SQLite pragmas: WAL journal mode, NORMAL synchronous. Text fields are defensively capped at 256 KB, headers at 16 KB and display previews at 2 KB; full bodies go to the archive, capped at 10 MiB per direction.

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
| `capsem info <id> --stats` | CLI -> service HTTP `/vms/{id}/info` | Pre-built `SessionStats` |
| Frontend Stats tab | Gateway -> typed VM-scoped ledger routes | Per-table summaries and event inspection |
| MCP `capsem_timeline` | MCP -> service HTTP `/vms/{id}/timeline` | Typed time-ordered event stream |
| MCP logs/triage tools | MCP -> typed service routes | Logs, panic triage, and operational diagnostics |

Capsem does not expose arbitrary SQL over HTTP, gateway, frontend, or MCP.
`session.db` and `session.bodies` are the durable ledger and can be inspected
directly by a developer when doing local forensics, but product routes use
typed logger/database APIs.
Any hot `mem`/disk split belongs inside the logger DB object, never in service
route state.

## Frontend Stats And Inspection

The VM **Stats** tab is ledger/database backed. It does not infer security
state from profile config or live rules. It reads typed service ledger routes
that are backed by the logger DB API and VM-scoped rule routes:

| Stats tab | Primary source |
|-----------|----------------|
| Model | `model_calls` |
| Tools | `tool_calls` |
| HTTP | `net_events` |
| DNS | `dns_events` |
| Files | `fs_events` |
| Process | `exec_events`, `audit_events` |
| Credentials | `substitution_events` |
| Security | `/vms/{id}/security/latest`, `/vms/{id}/security/status`, `/vms/{id}/detection/latest`, `/vms/{id}/enforcement/latest` |
| Snapshots | `/vms/{id}/snapshots/status`, `/vms/{id}/snapshots/list` |

The old raw SQL Inspector tab and `/vms/{id}/inspect` route were removed. Add
new typed logger DB APIs when the UI, TUI, MCP, or CLI needs more ledger
evidence; do not reintroduce a general SQL proxy or service-owned logged-data
projection.

## Per-VM isolation

| Property | Value |
|----------|-------|
| Location | `~/.capsem/run/sessions/{id}/` (ephemeral) or `~/.capsem/run/persistent/{name}/` (named): `session.db` and `session.bodies` |
| Lifetime | Created at VM boot and retained or deleted with the VM's lifecycle state; a persistent VM's bodies are trimmed to the retention period at stop |
| Access | Only the owning capsem-process writes, retention included; the service reads the files directly through SQLite's WAL |
| VirtioFS boundary | The ledger is outside the VirtioFS share; the guest cannot access it |
| Concurrent access | WAL mode allows concurrent readers and one writer |
| Fork behavior | `capsem fork` checkpoints and copies both files |

## Key source files

| File | Purpose |
|------|---------|
| `capsem-logger/src/schema.rs` | Table DDL and pragmas |
| `capsem-archive/src/format.rs` | The `session.bodies` byte format |
| `capsem-archive/src/warc.rs` | WARC record writer |
| `capsem-logger/src/db/bodies.rs` | `read_body`, `read_bodies`, retention entry point |
| `capsem-logger/src/db/warc_export.rs` | Session-to-WARC mapping |
| `capsem-logger/src/events.rs` | Event structs (NetEvent, ModelCall, McpCall, etc.) |
| `capsem-logger/src/writer.rs` | DbWriter, WriteOp, block-then-drain loop |
| `capsem-logger/src/reader.rs` | DbReader, aggregation queries, raw SQL |
| `capsem-logger/src/db.rs` | `DbHandle`, the async handle routes and processes use; `SessionDb` wrapper |
