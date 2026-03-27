---
name: dev-session-debug
description: Debugging Capsem session databases -- the telemetry pipeline output. Use when inspecting session.db, diagnosing missing or incorrect telemetry, understanding table schemas, checking data quality, or correlating events across tables. Covers all 6 session tables, the main.db rollup, the inspect-session tool, and common data quality issues.
---

# Session Database Debugging

Every Capsem VM session produces a SQLite database at `~/.capsem/sessions/<id>/session.db` with 6 tables capturing all telemetry. A global `~/.capsem/main.db` aggregates stats across sessions.

## Quick inspection

### Listing sessions

```bash
just list-sessions                    # Recent non-vacuumed sessions
just list-sessions -n 20              # Show more
just list-sessions --with-model       # Only sessions with AI model calls
just list-sessions --with-db          # Only sessions with session.db on disk
just list-sessions --with-net         # Only sessions with network events
just list-sessions --with-mcp         # Only sessions with MCP calls
just list-sessions --min-cost 0.01    # Only sessions that cost money
just list-sessions --all              # Include vacuumed sessions
just list-sessions --all --with-model # Combine filters
```

Output columns: ID, Created (MM-DD HH:MM:SS), Duration, Cost, net events, tokens (in+out), tool calls, MCP calls, fs events. Sessions with `*` after the ID still have a `session.db` on disk (queryable).

Stats come from the main.db rollup, so they're always available even after the session DB is vacuumed.

### Deep inspection

```bash
just inspect-session              # Full integrity check on latest session
just inspect-session <id>         # Specific session (use full ID from list)
just inspect-session -n 10        # Show 10 preview rows per table
```

Checks: table existence, row counts, tool lifecycle integrity (orphaned tool_calls), AI provider correlation (net_events vs model_calls), NULL detection in critical fields, MCP correlation.

## Session database tables (session.db)

### net_events -- one row per HTTP request through MITM proxy

```sql
CREATE TABLE net_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,          -- RFC 3339
    domain TEXT NOT NULL,             -- "api.anthropic.com"
    port INTEGER DEFAULT 443,
    decision TEXT NOT NULL,           -- "allowed" or "denied"
    process_name TEXT,                -- "claude", "node", "python3"
    pid INTEGER,
    method TEXT,                      -- "POST", "GET"
    path TEXT,                        -- "/v1/messages"
    query TEXT,                       -- URL query string
    status_code INTEGER,              -- 200, 403, etc.
    bytes_sent INTEGER DEFAULT 0,
    bytes_received INTEGER DEFAULT 0,
    duration_ms INTEGER DEFAULT 0,
    matched_rule TEXT,                -- which policy rule matched
    request_headers TEXT,             -- JSON (allowlisted verbatim, others hashed)
    response_headers TEXT,
    request_body_preview TEXT,        -- first N bytes
    response_body_preview TEXT,
    conn_type TEXT DEFAULT 'https'
);
```

### model_calls -- one row per AI API request+response cycle

```sql
CREATE TABLE model_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    provider TEXT NOT NULL,           -- "anthropic", "openai", "google"
    model TEXT,                       -- "claude-sonnet-4-20250514", "gpt-4o"
    process_name TEXT,
    pid INTEGER,
    method TEXT NOT NULL,             -- "POST"
    path TEXT NOT NULL,               -- "/v1/messages"
    stream INTEGER DEFAULT 0,         -- 1 if SSE streaming
    system_prompt_preview TEXT,
    messages_count INTEGER DEFAULT 0,
    tools_count INTEGER DEFAULT 0,
    request_bytes INTEGER DEFAULT 0,
    request_body_preview TEXT,
    message_id TEXT,                  -- "msg_..." (Anthropic), "chatcmpl-..." (OpenAI)
    status_code INTEGER,
    text_content TEXT,                -- full response text
    thinking_content TEXT,            -- thinking/reasoning text
    stop_reason TEXT,                 -- "end_turn", "tool_use", "stop", "STOP"
    input_tokens INTEGER,
    output_tokens INTEGER,
    duration_ms INTEGER DEFAULT 0,
    response_bytes INTEGER DEFAULT 0,
    estimated_cost_usd REAL DEFAULT 0,
    trace_id TEXT,                    -- groups tool call chains across turns
    usage_details TEXT                -- JSON: {"cache_read": N, "thinking": N}
);
```

Only emitted for actual LLM API paths (`/v1/messages`, `/v1/chat/completions`, `/v1beta/models/*/`). Health checks, auth endpoints don't create rows.

### tool_calls -- tool invocations extracted from model responses

```sql
CREATE TABLE tool_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    model_call_id INTEGER NOT NULL,   -- FK to model_calls.id
    call_index INTEGER NOT NULL,      -- position in response
    call_id TEXT NOT NULL,            -- "toolu_..." (Anthropic), "call_..." (OpenAI)
    tool_name TEXT NOT NULL,
    arguments TEXT,                   -- JSON string
    origin TEXT NOT NULL DEFAULT 'native',  -- "native" or "mcp"
    mcp_call_id INTEGER              -- FK to mcp_calls.id if origin=mcp
);
```

### tool_responses -- results sent back for tool calls

```sql
CREATE TABLE tool_responses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    model_call_id INTEGER NOT NULL,
    call_id TEXT NOT NULL,            -- matches tool_calls.call_id
    content_preview TEXT,
    is_error INTEGER DEFAULT 0
);
```

### mcp_calls -- MCP gateway requests

```sql
CREATE TABLE mcp_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    server_name TEXT NOT NULL,         -- "github", "builtin", "gateway"
    method TEXT NOT NULL,              -- "tools/list", "tools/call"
    tool_name TEXT,                    -- namespaced: "github__search"
    request_id TEXT,
    request_preview TEXT,              -- first 256KB
    response_preview TEXT,             -- first 256KB
    decision TEXT NOT NULL,            -- "allowed", "warned", "denied", "error"
    duration_ms INTEGER DEFAULT 0,
    error_message TEXT,
    process_name TEXT,
    bytes_sent INTEGER DEFAULT 0,
    bytes_received INTEGER DEFAULT 0
);
```

### fs_events -- filesystem changes in guest workspace

```sql
CREATE TABLE fs_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    action TEXT NOT NULL,              -- "created", "modified", "deleted"
    path TEXT NOT NULL,                -- relative to workspace root
    size INTEGER                       -- bytes (NULL for deletes)
);
```

## Main database (main.db)

Global rollup at `~/.capsem/main.db`. Key tables:

- **sessions** -- one row per session: id, mode, status, timestamps, aggregated counts (total_requests, allowed/denied, tokens, cost, tool_calls, mcp_calls, file_events)
- **ai_usage** -- per-session per-provider aggregates (call_count, tokens, cost, duration)
- **tool_usage** -- per-session per-tool aggregates
- **mcp_usage** -- per-session per-MCP-tool aggregates

Rollup happens when a session ends.

## Common debugging scenarios

### Missing net_events
- Guest didn't make HTTPS requests, or VM shut down before proxy flushed
- Check: `just run 'curl -s https://api.anthropic.com/ && sleep 1'` then inspect

### model_calls has NULL model or NULL tokens
- **Gzip bug**: response was gzip-compressed and proxy didn't decompress before SSE parsing. Check if `Accept-Encoding: gzip` was sent and `Content-Encoding: gzip` was in response.
- **Non-streaming**: for non-streaming responses, tokens come from response JSON, not SSE. Check if `stream=0`.
- **Provider mismatch**: check if the URL path was detected as the right provider. Model resolution: request body > SSE stream > response JSON > URL path.

### tool_calls without matching tool_responses
- The model invoked a tool but the next turn's tool results weren't captured
- Check if the VM session ended before the tool result was sent back
- `just inspect-session` reports orphaned tool_calls automatically

### Empty fs_events
- `capsem-fs-watch` didn't start (check boot logs for `[capsem-fs-watch] starting`)
- Vsock port 5005 connection failed
- VM shut down before 100ms debouncer flushed (add `sleep 1`)

### Empty mcp_calls
- No AI agent invoked MCP tools during the session
- MCP gateway not started (check for `[mcp-gateway] listening` in logs)

### Cost is zero
- Model not found in pricing table (`config/genai-prices.json`)
- Run `just update-prices` to refresh pricing data

## Ad-hoc SQL queries

Use `just query-session` to run SQL against session DBs. Auto-selects the latest non-vacuumed session with a DB on disk. Pass a session ID as second argument to target a specific session.

```bash
# Decisions breakdown
just query-session "SELECT decision, COUNT(*) FROM net_events GROUP BY decision"

# Token totals by provider
just query-session "SELECT provider, SUM(input_tokens) as in_tok, SUM(output_tokens) as out_tok, SUM(estimated_cost_usd) as cost FROM model_calls GROUP BY provider"

# Find orphaned tool calls
just query-session "SELECT tc.call_id, tc.tool_name FROM tool_calls tc LEFT JOIN tool_responses tr ON tc.call_id = tr.call_id WHERE tr.id IS NULL"

# Check fs_events actions
just query-session "SELECT action, COUNT(*) FROM fs_events GROUP BY action"

# Trace a tool call chain
just query-session "SELECT id, model, stop_reason, trace_id FROM model_calls WHERE trace_id = '<trace_id>' ORDER BY timestamp"

# Query a specific session (use full ID from just list-sessions)
just query-session "SELECT COUNT(*) FROM net_events" 20260327-154418-f907
```

Tip: use `just list-sessions --with-db --with-model` to find sessions worth querying.
