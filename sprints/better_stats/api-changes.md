# API Changes for Better Stats

Backend changes landed on `next-gen`. This doc tells the UI team what's new, what changed, and what to wire up.

## What Changed

Three things:

1. **New `GET /stats` endpoint** -- one call, full main.db dump
2. **`/inspect/_main` now works** -- `queryDbMain()` is no longer dead code
3. **`SandboxInfo` has telemetry fields** -- `/info/{id}` returns live stats for running VMs, `/list` includes uptime

---

## 1. `GET /stats` -- Cross-Session Dashboard Data

Single endpoint returns everything from main.db. No raw SQL needed.

**Request:** `GET /stats` (proxied through gateway)

**Response shape:**

```typescript
interface StatsResponse {
  global: GlobalStats;
  sessions: SessionRecord[];      // last 100, newest first
  top_providers: ProviderSummary[];  // top 20 by call count
  top_tools: ToolSummary[];          // top 20 by call count
  top_mcp_tools: McpToolSummary[];   // top 20 by call count
}

interface GlobalStats {
  total_sessions: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_estimated_cost: number;   // USD
  total_tool_calls: number;
  total_mcp_calls: number;
  total_file_events: number;
  total_requests: number;
  total_allowed: number;
  total_denied: number;
}

interface SessionRecord {
  id: string;                       // "20260412-143022-a1f3"
  mode: string;                     // "virtiofs" | "block" | "run"
  command: string | null;           // command that created the VM
  status: string;                   // "running" | "stopped" | "crashed" | "vacuumed" | "terminated"
  created_at: string;               // ISO 8601
  stopped_at: string | null;        // ISO 8601
  scratch_disk_size_gb: number;
  ram_bytes: number;
  total_requests: number;
  allowed_requests: number;
  denied_requests: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_estimated_cost: number;     // USD
  total_tool_calls: number;
  total_mcp_calls: number;
  total_file_events: number;
  compressed_size_bytes: number | null;
  vacuumed_at: string | null;
  storage_mode: string;
  rootfs_hash: string | null;
  rootfs_version: string | null;
  forked_from: string | null;
  persistent: boolean;
}

interface ProviderSummary {
  provider: string;                 // "anthropic", "openai", etc.
  call_count: number;
  input_tokens: number;
  output_tokens: number;
  estimated_cost: number;           // USD
  total_duration_ms: number;
}

interface ToolSummary {
  tool_name: string;
  call_count: number;
  total_bytes: number;
  total_duration_ms: number;
}

interface McpToolSummary {
  tool_name: string;
  server_name: string;
  call_count: number;
  total_bytes: number;
  total_duration_ms: number;
}
```

**Use for:** history/dashboard view, cross-session cost tracking, "top providers" chart, session list with per-session stats.

**Note:** `sessions` contains rolled-up summary data. For stopped VMs the numbers are final. For running VMs, the numbers reflect the last rollup (may lag behind live data). Use `/info/{id}` for real-time stats on a specific running VM.

---

## 2. `/inspect/_main` Now Works

`queryDbMain()` in `db.ts` was wired up but the backend always returned 404. Fixed.

```typescript
// This now works -- queries main.db (the global session index)
const result = await queryDbMain('SELECT COUNT(*) as cnt FROM sessions');
```

Available tables on `_main`:

| Table | What |
|-------|------|
| `sessions` | All sessions with summary telemetry (see SessionRecord above) |
| `ai_usage` | Per-session per-provider token/cost breakdown |
| `tool_usage` | Per-session per-tool call counts and duration |
| `mcp_usage` | Per-session per-MCP-tool call counts and duration |

**Recommendation:** prefer `GET /stats` for standard views. Use `queryDbMain()` only for custom queries the structured endpoint doesn't cover (e.g., filtering sessions by date range, specific provider lookups).

---

## 3. `SandboxInfo` Enriched

`/list` and `/info/{id}` now return additional optional fields.

### New fields on `SandboxInfo`

```typescript
interface SandboxInfo {
  // existing fields (unchanged)
  id: string;
  name?: string;
  pid: number;
  status: string;
  persistent: boolean;
  ram_mb?: number;
  cpus?: number;
  version?: string;
  forked_from?: string;
  description?: string;

  // NEW -- all optional, omitted when absent
  created_at?: string;              // ISO 8601 (from session record)
  uptime_secs?: number;             // seconds since boot (running VMs only)
  total_input_tokens?: number;
  total_output_tokens?: number;
  total_estimated_cost?: number;    // USD
  total_tool_calls?: number;
  total_mcp_calls?: number;
  total_requests?: number;
  allowed_requests?: number;
  denied_requests?: number;
  total_file_events?: number;
  model_call_count?: number;
}
```

### What's populated where

| Endpoint | `uptime_secs` | Telemetry fields | Notes |
|----------|--------------|------------------|-------|
| `GET /list` | Running VMs only | Not populated | Kept lightweight (polled every 2s) |
| `GET /info/{id}` | Running VMs only | Running VMs only | Opens session.db, reads live stats |
| `GET /stats` | N/A (use `sessions[]`) | Full history in `sessions[]` | main.db rolled-up data |

### Backwards compatibility

All new fields use `skip_serializing_if = Option::is_none`. Old frontend code that doesn't know about these fields will work unchanged. The gateway's `SandboxInfo` deserialization uses `#[serde(default)]` on all fields so it silently ignores the new ones.

---

## Frontend TODO

### Update `frontend/src/lib/types/gateway.ts`

The `SandboxInfo` interface needs the new optional fields:

```typescript
export interface SandboxInfo {
  id: string;
  name?: string;
  pid: number;
  status: string;
  persistent: boolean;
  ram_mb?: number;
  cpus?: number;
  version?: string;
  forked_from?: string;
  description?: string;
  // new telemetry
  created_at?: string;
  uptime_secs?: number;
  total_input_tokens?: number;
  total_output_tokens?: number;
  total_estimated_cost?: number;
  total_tool_calls?: number;
  total_mcp_calls?: number;
  total_requests?: number;
  allowed_requests?: number;
  denied_requests?: number;
  total_file_events?: number;
  model_call_count?: number;
}
```

### Add `StatsResponse` type and API call

```typescript
// types/gateway.ts
export interface StatsResponse {
  global: GlobalStats;
  sessions: SessionRecord[];
  top_providers: ProviderSummary[];
  top_tools: ToolSummary[];
  top_mcp_tools: McpToolSummary[];
}
// (plus GlobalStats, SessionRecord, ProviderSummary, ToolSummary, McpToolSummary as above)

// api.ts
export async function getStats(): Promise<StatsResponse> {
  const resp = await _get('/stats');
  return await resp.json();
}
```

### Delete dead `SessionInfo` type

`frontend/src/lib/types.ts:103` defines `SessionInfo` but nothing uses it. The new `SandboxInfo` telemetry fields supersede it. Remove it.

### Wire up `queryDbMain`

It now works. Currently only imported in `db.test.ts`. Any view that needs cross-session data can use it, though `GET /stats` covers the common cases.

### Display opportunities

| Where | Data source | What to show |
|-------|-------------|-------------|
| VM list (multi-VM view) | `GET /stats` -> `sessions[]` | Per-VM token count, cost, tool calls |
| VM list row (running) | `GET /list` | Uptime badge |
| VM detail header | `GET /info/{id}` | Live tokens, cost, tool calls, request counts |
| Dashboard / history | `GET /stats` -> `global` | Total cost, total tokens, session count |
| Provider breakdown | `GET /stats` -> `top_providers` | Cost/tokens by provider chart |
| Tool usage chart | `GET /stats` -> `top_tools` + `top_mcp_tools` | Bar chart of tool frequency |

---

## Files Changed (Backend)

| File | What |
|------|------|
| `crates/capsem-service/src/api.rs` | `StatsResponse` type, `SandboxInfo` enriched with 12 telemetry fields, `SandboxInfo::new()` helper |
| `crates/capsem-service/src/main.rs` | `GET /stats` handler, `/inspect/_main` fix, `/list` adds uptime, `/info/{id}` reads live session.db, `main_db_path()` helper, 8 new tests |
| `CHANGELOG.md` | Three new entries under `[Unreleased] > Added` |
