# Meta-Sprint: Forensic Capabilities

## Context

Session inspection relies on brittle Python scripts and shell recipes. We're building a complete forensic layer into capsem: structured logging to SQLite, FTS5 full-text search, a namespaced REST API, and CLI commands that surface it all with generic formatting and JSON output.

## API Design

### Naming: `service` (not `system`)

Explicit -- the service daemon is what's being queried.

### Route Structure

**VM-scoped (`/vm/...`):**
| Route | Method | Description |
|-------|--------|-------------|
| `/vm/{id}/info` | GET | VM metadata (pid, status, ram, cpus, version) |
| `/vm/{id}/query` | POST | SQL against VM's session.db |
| `/vm/{id}/search` | POST | FTS5 search scoped to one VM's session.db |
| `/vm/{id}/logs` | GET | Raw text logs (serial.log, process.log) |
| `/vm/{id}/exec` | POST | Execute command in VM |
| `/vm/{id}/write_file` | POST | Write file to VM |
| `/vm/{id}/read_file` | POST | Read file from VM |
| `/vm/{id}/stop` | POST | Stop VM |
| `/vm/{id}/persist` | POST | Convert ephemeral to persistent |
| `/vm/{id}/fork` | POST | Fork VM to image |
| `/vm/{id}` | DELETE | Delete VM |
| `/vm/{name}/resume` | POST | Resume persistent VM |
| `/vm/search` | POST | FTS5 search across ALL VMs' session.dbs |

**Service-scoped (`/service/...`):**
| Route | Method | Description |
|-------|--------|-------------|
| `/service/info` | GET | Service version, uptime, VM counts, disk usage |
| `/service/query` | POST | SQL against main.db (session rollups) |
| `/service/search` | POST | FTS5 search across main.db |
| `/service/logs` | GET | Raw service.log text |
| `/service/provision` | POST | Create VM |
| `/service/list` | GET | List all VMs |
| `/service/purge` | POST | Kill temp VMs |
| `/service/run` | POST | One-shot VM execution |
| `/service/reload-config` | POST | Reload policy config |

**Image-scoped (`/image/...`):**
| Route | Method | Description |
|-------|--------|-------------|
| `/image/list` | GET | List images |
| `/image/{name}` | GET | Inspect image |
| `/image/{name}` | DELETE | Delete image |

### Query/Search API

All query and search endpoints use the same request/response format:

```
POST /vm/{id}/query   { "sql": "SELECT ...", "limit": 100, "offset": 0 }
POST /vm/search       { "term": "anthropic", "id": "optional-scope", "limit": 20, "offset": 0 }
POST /service/query   { "sql": "SELECT ...", "limit": 100, "offset": 0 }
POST /service/search  { "term": "error", "limit": 20, "offset": 0 }
```

**Pagination**: All endpoints support `limit` + `offset` for basic paging, plus cursor-based pagination via `after_id` (row ID of last seen result) for efficient deep paging without large-offset performance degradation. The SQL user provides is wrapped: `SELECT * FROM ({user_sql}) LIMIT ? OFFSET ?` for query endpoints. Search endpoints paginate internally.

Response (always columnar JSON):
```json
{ "columns": ["col1", "col2"], "rows": [["val1", "val2"], ...], "has_more": true }
```

`has_more` indicates whether more results exist beyond the current page.

Search returns results ranked by FTS5 `bm25()` score, normalized across tables. Results can be sorted by relevance (default) or by timestamp (via `sort` parameter). Each result row includes a `_source` column (table name) and `_score` column (relevance) for interleaved cross-table display.

### Forensic Timeline Projection

All event tables map to a standard timeline shape for `capsem logs`:

| Column | Type | Source |
|--------|------|--------|
| `timestamp` | TEXT | Direct from each table |
| `event_type` | TEXT | Constant per table: NET, AI, TOOL, FILE, MCP, PROCESS |
| `summary` | TEXT | Table-specific: e.g., `domain || ' ' || method || ' ' || path` for net_events |
| `status` | TEXT | decision/stop_reason/origin/action depending on table |
| `duration_ms` | INTEGER | Where available, NULL otherwise |
| `metadata` | TEXT | JSON object with table-specific extra fields |

This standardized projection simplifies the UNION ALL query and means the generic formatter doesn't need per-table logic.

## CLI Design

### Discoverability: subcommand groups, not flags

Top-level commands operate on VMs (default scope). `capsem service` subgroup operates on the service.

```
capsem info [id]                  # VM info (defaults to latest)
capsem query <sql> [id]           # SQL against VM session.db
capsem search <term> [--id <id>]  # FTS5 search across ALL VMs (--id to scope)
capsem logs [id]                  # Event timeline from session.db
capsem syslog [id]                # Raw serial/process text logs

capsem service info               # Service metadata
capsem service query <sql>        # SQL against main.db
capsem service search <term>      # FTS5 search across main.db
capsem service logs               # Raw service.log

capsem create/stop/shell/...      # Existing VM lifecycle commands
capsem image list/inspect/delete  # Existing image commands
```

All commands support `--json` for machine-readable columnar JSON output.

## Phases

| Phase | Name | Status | Description |
|-------|------|--------|-------------|
| 0 | API restructure | Not Started | Namespace endpoints under /vm, /service, /image |
| 1 | Process logs to SQLite | Not Started | process_events table, tracing layer -> DbWriter |
| 2 | FTS5 search | Not Started | bundled-full, FTS5 virtual tables, sync triggers, /search |
| 3 | Forensic CLI | Not Started | info, query, search, logs, syslog + service subgroup |
| 4 | MCP + integration | Not Started | MCP tools, just recipes, deprecate Python scripts |
| 5 | Documentation | Not Started | Skills, architecture docs, user guide, developer guide |

---

## Phase 0: API Restructure

### Commit 0a: `refactor: namespace API under /vm, /service, /image`

Mechanical mass-rename of all routes + all callers (CLI, MCP). Logic changes:
1. `handle_inspect` renamed to `handle_query` + persistent registry fallback fix
2. New `handle_service_info` endpoint (service version, uptime, VM counts)
3. New `handle_service_query` endpoint (SQL against main.db via DbReader)
4. New `handle_service_logs` endpoint (reads service.log from run_dir)
5. **"Latest VM" heuristic**: Add `last_accessed_at: SystemTime` field to `InstanceInfo` and persistent registry entries. Update on any info/query/search/exec/logs access. The `/service/list` response is sorted by `last_accessed_at DESC` so "pick first" always returns the most recently interacted-with VM, not just the most recently created one.

**Files:**
- `crates/capsem-service/src/main.rs` -- route registration, new handlers
- `crates/capsem-service/src/api.rs` -- rename InspectRequest -> QueryRequest, InspectResponse -> QueryResponse
- `crates/capsem/src/main.rs` -- update all URL paths
- `crates/capsem-mcp/src/main.rs` -- update all URL paths

---

## Phase 1: Process Logs to SQLite

### Commit 1a: `feat: process_events table + WriteOp`

**`crates/capsem-logger/src/events.rs`** -- new type:
```rust
pub struct ProcessEvent {
    pub timestamp: SystemTime,
    pub level: String,          // trace, debug, info, warn, error
    pub target: String,         // module path
    pub message: String,
    pub fields: Option<String>, // JSON span fields
}
```

**`crates/capsem-logger/src/schema.rs`** -- new table + indexes
**`crates/capsem-logger/src/writer.rs`** -- `WriteOp::ProcessEvent`, `insert_process_event`
**`crates/capsem-logger/src/lib.rs`** -- re-export

### Commit 1b: `feat: DbTracingLayer writes process logs to session.db`

**`crates/capsem-logger/src/tracing_layer.rs`** -- new file: custom `tracing_subscriber::Layer` that constructs `ProcessEvent` from tracing events and sends via `DbWriter::try_write()` (non-blocking).

**Level filtering**: Configurable minimum level for SQLite writes (default: INFO and above go to DB, TRACE/DEBUG stay in process.log text only). This avoids flooding session.db with high-volume trace events while preserving them in the text fallback for debugging. Configured via `DbTracingLayer::new(db, min_level)`.

**Retention policy**: Add `max_process_events` config (default: 50,000 rows). The writer periodically prunes oldest entries when the count exceeds the limit. Implemented as a `DELETE FROM process_events WHERE id <= (SELECT id FROM process_events ORDER BY id DESC LIMIT 1 OFFSET ?)` run every N inserts.

**`crates/capsem-process/src/main.rs`** -- layered subscriber: stderr fmt layer (keeps process.log text fallback for all levels) + DbTracingLayer (INFO+ to session.db).

---

## Phase 2: FTS5 Search

### Commit 2a: `feat: upgrade rusqlite to bundled-full`

**`Cargo.toml` (workspace)** -- `bundled` -> `bundled-full`

### Commit 2b: `feat: FTS5 virtual tables + sync triggers`

**`crates/capsem-logger/src/schema.rs`** -- FTS5 content tables for all event tables:
- `fts_net_events` (domain, path, query, matched_rule, request/response previews)
- `fts_model_calls` (provider, model, text_content, thinking_content, system_prompt_preview)
- `fts_tool_calls` (tool_name, arguments)
- `fts_mcp_calls` (server_name, tool_name, method)
- `fts_fs_events` (path)
- `fts_process_events` (target, message)

Plus `AFTER INSERT` triggers to keep FTS in sync.

### Commit 2c: `feat: /vm/{id}/search and /vm/search endpoints`

**`crates/capsem-service/src/main.rs`** -- search handlers:
- `/vm/{id}/search` -- FTS5 MATCH across one VM's session.db, results grouped by table
- `/vm/search` -- iterates all active/persistent VMs, searches each session.db, merges results
- `/service/search` -- FTS5 MATCH across main.db

Request: `SearchRequest { term: String, id: Option<String>, limit: Option<usize> }`
Response: same columnar JSON format, with source table name column prepended.

---

## Phase 3: Forensic CLI

### Commit 3a: `feat: generic columnar table formatter`

**`Cargo.toml` (workspace)** -- add `comfy-table = "7"`
**`crates/capsem/Cargo.toml`** -- add dep

Generic formatter: takes `{"columns":[],"rows":[[]]}`, renders via comfy-table. Smart formatting for known column patterns (bytes, cost, tokens, duration_ms). Works for every command.

Add `QueryRequest` type and POST helper for `/vm/{id}/query`.
Add "resolve latest VM" helper (calls `/service/list`, picks first).

### Commit 3b: `feat: capsem info and capsem service info`

```rust
// Top-level
Info {
    /// ID or name (defaults to latest)
    id: Option<String>,
    #[arg(long)] json: bool,
},

// Under Service subgroup
enum ServiceCommands {
    Info {
        #[arg(long)] json: bool,
    },
    // ...
}
```

`capsem info [id]` -- VM metadata from `/vm/{id}/info` + canned SQL summaries from `/vm/{id}/query`
`capsem service info` -- service metadata from `/service/info` + canned SQL from `/service/query`

### Commit 3c: `feat: capsem query and capsem service query`

```rust
Query {
    sql: String,
    id: Option<String>,
    #[arg(long)] json: bool,
},
```

`capsem query <sql> [id]` -> `POST /vm/{id}/query`
`capsem service query <sql>` -> `POST /service/query`

### Commit 3d: `feat: capsem search and capsem service search`

```rust
Search {
    /// FTS5 search term (supports: words, "phrases", OR/AND/NOT)
    term: String,
    /// Scope to specific VM (default: search all)
    #[arg(long)]
    id: Option<String>,
    #[arg(long, default_value_t = 20)]
    limit: usize,
    #[arg(long)] json: bool,
},
```

`capsem search <term>` -> `POST /vm/search` (ALL VMs)
`capsem search <term> --id <id>` -> `POST /vm/{id}/search` (scoped)
`capsem service search <term>` -> `POST /service/search`

### Commit 3e: `feat: capsem logs -- event timeline`

```rust
Logs {
    id: Option<String>,
    #[arg(long)] tail: Option<usize>,
    #[arg(long)] grep: Option<String>,
    #[arg(long)] r#type: Option<EventType>, // net, ai, tool, file, mcp, process
    #[arg(long)] json: bool,
},
```

UNION ALL timeline query across all session.db tables, sent via `POST /vm/{id}/query`. No dedicated timeline endpoint -- the event timeline is built from SQL. Filters -> SQL WHERE/LIMIT.

### Commit 3f: `feat: capsem syslog and capsem service logs`

```rust
// Top-level: raw VM text logs
Syslog {
    id: Option<String>,
    #[arg(long)] tail: Option<usize>,
    #[arg(long)] grep: Option<String>,
    #[arg(long)] r#type: Option<SyslogType>, // serial, process
    #[arg(long)] json: bool,
},

// Service subgroup: raw service text logs
enum ServiceCommands {
    Logs {
        #[arg(long)] tail: Option<usize>,
        #[arg(long)] grep: Option<String>,
        #[arg(long)] json: bool,
    },
}
```

`capsem syslog [id]` -- serial.log + process.log text (current `capsem logs` behavior)
`capsem service logs` -- service.log text

---

## Phase 4: MCP + Integration

### Commit 4a: `feat: forensic MCP tools (VM-scoped)`

**`crates/capsem-mcp/src/main.rs`** -- new/updated VM-scoped tools:

| Tool | Endpoint | Description |
|------|----------|-------------|
| `capsem_query` (new) | `POST /vm/{id}/query` | Run SQL against VM session.db. Replaces `capsem_inspect`. |
| `capsem_search` (new) | `POST /vm/search` or `/vm/{id}/search` | FTS5 search across all VMs or scoped to one. |
| `capsem_schema` (renamed) | (built-in) | Returns CREATE TABLE statements. Was `capsem_inspect_schema`. |
| `capsem_info` (updated) | `GET /vm/{id}/info` | Updated URL path. |
| `capsem_vm_logs` (updated) | `GET /vm/{id}/logs` | Updated URL path. |
| `capsem_exec` (updated) | `POST /vm/{id}/exec` | Updated URL path. |
| `capsem_read_file` (updated) | `POST /vm/{id}/read_file` | Updated URL path. |
| `capsem_write_file` (updated) | `POST /vm/{id}/write_file` | Updated URL path. |
| `capsem_stop` (updated) | `POST /vm/{id}/stop` | Updated URL path. |
| `capsem_delete` (updated) | `DELETE /vm/{id}` | Updated URL path. |
| `capsem_persist` (updated) | `POST /vm/{id}/persist` | Updated URL path. |
| `capsem_fork` (updated) | `POST /vm/{id}/fork` | Updated URL path. |
| `capsem_resume` (updated) | `POST /vm/{name}/resume` | Updated URL path. |

### Commit 4b: `feat: service-scoped MCP tools`

| Tool | Endpoint | Description |
|------|----------|-------------|
| `capsem_service_info` (new) | `GET /service/info` | Service metadata: version, uptime, VM counts, disk. |
| `capsem_service_query` (new) | `POST /service/query` | SQL against main.db (session rollups, history). |
| `capsem_service_search` (new) | `POST /service/search` | FTS5 search across main.db. |
| `capsem_service_logs` (updated) | `GET /service/logs` | Updated to use endpoint instead of direct file read. |
| `capsem_create` (updated) | `POST /service/provision` | Updated URL path. |
| `capsem_list` (updated) | `GET /service/list` | Updated URL path. |
| `capsem_purge` (updated) | `POST /service/purge` | Updated URL path. |
| `capsem_run` (updated) | `POST /service/run` | Updated URL path. |
| `capsem_version` (updated) | `GET /service/info` | Can merge with service_info or keep separate. |

Image tools (`capsem_image_list`, `capsem_image_inspect`, `capsem_image_delete`) updated to `/image/...` paths.

### Commit 4c: `chore: update just recipes, deprecate Python scripts`

```just
inspect-session id='':
    {{cli_binary}} info {{id}}

query-session sql id='':
    {{cli_binary}} query "{{sql}}" {{id}}

list-sessions *args='':
    {{cli_binary}} service info

last-logs:
    {{cli_binary}} logs
```

Deprecation headers on `scripts/check_session.py` and `scripts/list_sessions.py`.

### Commit 4d: `test: performance stress tests`

Generate a heavy session.db fixture (100k net_events, 50k tool_calls, 10k model_calls) and verify query performance:
- Full table scan < 2s
- Indexed domain lookup < 100ms
- FTS5 MATCH on 100k rows < 500ms
- Timeline UNION ALL < 1s
- Cursor-based deep pagination < 100ms (vs offset which degrades)
- Cross-VM search across 10 VMs < 3s

Use these benchmarks to validate index effectiveness and tune FTS5 tokenizer choice. Document findings in sprint notes.

### Commit 4e: `test: E2E forensic tests`

Full end-to-end tests that boot a VM, generate telemetry, and verify forensic output.

**E2E scenarios** (Python integration tests, `tests/capsem-forensic/`):

| Test | What it verifies |
|------|-----------------|
| `test_info_default` | `capsem info` returns human output with all sections (summary, domains, providers, tools) |
| `test_info_json` | `capsem info --json` returns valid JSON with `info` + query result sections |
| `test_info_empty_session` | `capsem info` on freshly booted VM shows "(no telemetry yet)" |
| `test_service_info` | `capsem service info` returns service metadata + session listing |
| `test_query_select` | `capsem query "SELECT COUNT(*) FROM net_events"` returns table |
| `test_query_json` | `capsem query --json` returns columnar JSON |
| `test_service_query` | `capsem service query "SELECT * FROM sessions LIMIT 5"` queries main.db |
| `test_search_basic` | `capsem search "anthropic"` returns FTS5 results grouped by table |
| `test_search_scoped` | `capsem search "error" --id <id>` scopes to one VM |
| `test_search_phrase` | `capsem search '"POST /v1/messages"'` phrase search works |
| `test_search_boolean` | `capsem search "error OR timeout"` boolean FTS5 syntax |
| `test_service_search` | `capsem service search "doctor"` searches main.db |
| `test_logs_timeline` | `capsem logs` returns chronological events across tables |
| `test_logs_type_filter` | `capsem logs --type net` shows only NET events |
| `test_logs_tail` | `capsem logs --tail 5` returns exactly 5 events |
| `test_logs_grep` | `capsem logs --grep "anthropic"` filters events |
| `test_logs_json` | `capsem logs --json` returns columnar JSON |
| `test_logs_process_events` | `capsem logs --type process` shows process events from session.db |
| `test_syslog_serial` | `capsem syslog --type serial` shows serial.log text |
| `test_syslog_process` | `capsem syslog --type process` shows process.log text |
| `test_service_logs` | `capsem service logs` returns service.log text |
| `test_mcp_query` | MCP `capsem_query` tool returns same results as CLI |
| `test_mcp_search` | MCP `capsem_search` tool returns FTS5 results |
| `test_mcp_service_info` | MCP `capsem_service_info` returns service metadata |
| `test_mcp_service_query` | MCP `capsem_service_query` queries main.db |

**Adversarial tests** (`tests/capsem-forensic/test_adversarial.py`):

| Test | Attack vector | Expected behavior |
|------|--------------|-------------------|
| `test_query_sql_injection_drop` | `capsem query "DROP TABLE net_events"` | Error: "DROP statements are not allowed" |
| `test_query_sql_injection_insert` | `capsem query "INSERT INTO net_events ..."` | Error: "INSERT statements are not allowed" |
| `test_query_sql_injection_attach` | `capsem query "ATTACH DATABASE '/etc/passwd' AS x"` | Error: "ATTACH statements are not allowed" |
| `test_query_sql_injection_pragma` | `capsem query "PRAGMA table_info(net_events)"` | Error: "PRAGMA statements are not allowed" |
| `test_query_sql_injection_union_write` | `capsem query "SELECT 1; DROP TABLE net_events"` | Error or only SELECT executes |
| `test_query_empty` | `capsem query ""` | Error: "empty query" |
| `test_query_timeout` | `capsem query "WITH RECURSIVE r(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM r) SELECT * FROM r"` | Error: "query timed out after 5 seconds" |
| `test_search_empty_term` | `capsem search ""` | Error or empty results |
| `test_search_fts5_injection` | `capsem search "* OR 1=1"` | Returns results or clean error (no crash) |
| `test_search_special_chars` | `capsem search "'; DROP TABLE"` | No SQL injection, clean error |
| `test_query_nonexistent_vm` | `capsem query "SELECT 1" nonexistent-id` | Error: "sandbox not found" |
| `test_query_nonexistent_table` | `capsem query "SELECT * FROM bogus_table"` | SQLite error (table not found) |
| `test_search_no_fts_tables` | Search against old DB without FTS5 tables | Graceful error, not crash |
| `test_info_stopped_persistent` | `capsem info` on stopped persistent VM | Works (persistent registry fallback) |
| `test_query_max_rows` | Query returning >10,000 rows | Capped at 10,000 |
| `test_service_query_write_attempt` | `capsem service query "DELETE FROM sessions"` | Error: "DELETE statements are not allowed" |
| `test_concurrent_queries` | Multiple simultaneous queries | WAL mode handles concurrent reads |

---

## Phase 5: Documentation

### 5a: Skill: `dev-forensics`

New skill covering the forensic layer for developers:
- API endpoint reference (all /vm and /service routes)
- How query/search work (SQL -> DbReader -> columnar JSON)
- How to add new event types (WriteOp pattern)
- How FTS5 indexing works (triggers, MATCH syntax)
- Testing forensic features

### 5b: Update `site-architecture` skill

Add forensic layer to the architecture overview:
- Data flow: events -> DbWriter -> session.db -> DbReader -> query/search endpoints -> CLI/MCP
- The symmetry principle (VM vs service scope)
- FTS5 indexing strategy

### 5c: Update `dev-mcp` skill

Document new MCP tools: `capsem_query`, `capsem_search`, `capsem_schema`, `capsem_service_info`, `capsem_service_query`, `capsem_service_search`. Document updated URL paths for all existing tools.

### 5d: User guide pages (site/)

- `site/src/content/docs/guides/forensics.mdx` -- overview of forensic capabilities
- `site/src/content/docs/reference/cli-forensics.mdx` -- CLI command reference for info/query/search/logs
- `site/src/content/docs/reference/api.mdx` -- update API reference with new endpoint structure

### 5e: Update `dev-testing` skill

Document how to test forensic features:
- Using `capsem query --json` in integration tests
- Using `capsem search` for test assertions
- Replacing Python script calls with CLI commands in test fixtures

---

## Key Files

| File | Phases | Changes |
|------|--------|---------|
| `Cargo.toml` (workspace) | 2, 3 | `bundled-full`, `comfy-table` |
| `crates/capsem-service/src/main.rs` | 0, 2 | Route restructure, new handlers (service_info, service_logs, search) |
| `crates/capsem-service/src/api.rs` | 0 | Rename types, add SearchRequest |
| `crates/capsem/src/main.rs` | 0, 3 | URL updates, new commands, Service subgroup, formatter |
| `crates/capsem/Cargo.toml` | 3 | `comfy-table` |
| `crates/capsem-mcp/src/main.rs` | 0, 4 | URL updates, new MCP tools |
| `crates/capsem-logger/src/events.rs` | 1 | ProcessEvent type |
| `crates/capsem-logger/src/schema.rs` | 1, 2 | process_events table, FTS5 tables + triggers |
| `crates/capsem-logger/src/writer.rs` | 1 | WriteOp::ProcessEvent |
| `crates/capsem-logger/src/tracing_layer.rs` | 1 | New: DbTracingLayer |
| `crates/capsem-logger/src/lib.rs` | 1, 2 | Re-exports |
| `crates/capsem-process/src/main.rs` | 1 | Layered tracing subscriber |
| `justfile` | 4 | Recipe updates |
| `skills/dev-forensics/SKILL.md` | 5 | New skill |
| `skills/site-architecture/SKILL.md` | 5 | Update |
| `skills/dev-mcp/SKILL.md` | 5 | Update |
| `skills/dev-testing/SKILL.md` | 5 | Update |
| `site/src/content/docs/...` | 5 | User guide + API reference |

## Existing Code to Reuse

- `DbWriter` channel pattern: `crates/capsem-logger/src/writer.rs`
- `insert_net_event`: writer.rs ~line 188 -- template for new insert functions
- `query_raw()`: `crates/capsem-logger/src/reader.rs:266` -- generic columnar JSON
- `validate_select_only()`: reader.rs:213
- `handle_logs` session_dir resolution: `crates/capsem-service/src/main.rs:718-732`
- CLI `tail_lines`: `crates/capsem/src/main.rs:737`
- MCP tool pattern: `crates/capsem-mcp/src/main.rs:407`

## Security: Defense in Depth

Query endpoints have three layers of write protection:

1. **String validation** (`validate_select_only()`): Rejects non-SELECT/WITH/EXPLAIN statements at the API boundary. Already exists in `capsem-logger/src/reader.rs:213`.

2. **Read-only connection flag**: `DbReader::open()` already uses `SQLITE_OPEN_READ_ONLY` flag (reader.rs:246). SQLite will reject any write attempt at the engine level regardless of what SQL passes string validation.

3. **SQLite authorizer callback**: Add `conn.authorizer(Some(...))` in `DbReader::open()` that returns `SQLITE_DENY` for any action type that isn't `SQLITE_READ`, `SQLITE_SELECT`, `SQLITE_FUNCTION`. This catches edge cases like `SELECT load_extension(...)` or future SQLite features that might bypass the read-only flag. Implemented via rusqlite's `set_authorizer()`.

4. **PRAGMA query_only**: Already set in `apply_reader_pragmas()` (schema.rs). Belt-and-suspenders with the read-only open flag.

For search endpoints, the FTS5 MATCH term is always passed as a bind parameter (`?`), never interpolated into SQL. This prevents SQL injection through search terms.

## Edge Cases

- **Early boot (no DB)**: DbTracingLayer uses `try_write()`, drops if channel full. serial.log covers early boot via stderr.
- **Process log volume**: Level filtering (INFO+ to DB) + retention policy (50k row cap) prevent unbounded growth.
- **FTS5 migration on existing DBs**: `CREATE VIRTUAL TABLE IF NOT EXISTS`. Old DBs get empty FTS -- auto-rebuild on first search via `INSERT INTO fts_X(fts_X) VALUES('rebuild')`.
- **Search across all VMs**: `/vm/search` iterates active + persistent VMs. Cap at 100 VMs, return partial results with `truncated: true` flag.
- **Cross-table search ranking**: FTS5 `bm25()` scores normalized per table, interleaved by score or timestamp.
- **Deep pagination**: Cursor-based (`after_id`) avoids O(n) skip cost of large offsets.
- **bundled-full binary size**: ~1MB increase. Acceptable.
- **No session.db**: Return empty results, "(no telemetry yet)".
- **TTY detection**: Disable colors when piped.

## Verification

1. `cargo build -p capsem -p capsem-service -p capsem-mcp -p capsem-logger`
2. `cargo test` -- all unit tests
3. `capsem info` -- formatted VM session summary
4. `capsem service info` -- service metadata + session listing
5. `capsem query "SELECT * FROM net_events LIMIT 5"`
6. `capsem service query "SELECT * FROM sessions ORDER BY created_at DESC LIMIT 5"`
7. `capsem search "anthropic"` -- FTS5 results across all VMs
8. `capsem search "error" --id <id>` -- scoped search
9. `capsem logs --type net --tail 20` -- network event timeline
10. `capsem syslog` -- raw serial + process text
11. `capsem service logs` -- raw service.log
12. All with `--json | jq .` -- valid JSON
13. `just inspect-session` / `just query-session` -- delegate to CLI
14. `just test` -- all existing tests pass
