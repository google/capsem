# Capsem UI Improvement Plan

Informed by Maestro analysis (`docs/maestro_analysis.md`) and the current frontend state. The LLM gateway (`crates/capsem-core/src/gateway/`) is assumed wired up -- we have full `GatewayEvent` data (provider, model, method, path, status, duration, request/response bytes, streaming flag, bodies).

Everything ships in one pass. No throwaway code, no polling-then-replace, no intermediate states.

## Design Principle

The StatusBar is the persistent information surface. It already shows VM state and HTTPS counts. We expand it into a **collapsible activity panel** that absorbs the current NetworkView and becomes the primary dashboard for both network telemetry and LLM usage. The sidebar Network item stays but toggles the activity panel's Network tab (rather than navigating to a separate view). SessionsView keeps session history + LLM summary card but drops its network analytics (which move to the activity panel).

---

## Milestone 1: Gateway Backend (tokens, cost, tools, IPC)

All backend work in one shot: token extraction, cost estimation, tool call tracking, IPC commands, Tauri event emission, and AppState wiring.

### 1.1 AppState wiring

**File: `crates/capsem-app/src/state.rs`**

Add `Arc<Mutex<GatewayDb>>` to the app state struct. Initialize it on boot alongside the existing `WebDb` (in the Tauri setup hook). The gateway DB lives at `~/.capsem/sessions/<session_id>/ai.db`.

### 1.2 Token extraction

**New file: `crates/capsem-core/src/gateway/tokens.rs`**

Extract token counts from request/response bodies. Each provider returns usage data in different JSON shapes:

```rust
/// Extract (input_tokens, output_tokens) from a gateway event.
pub fn extract_tokens(event: &GatewayEvent) -> (u64, u64) {
    match event.provider.as_str() {
        "anthropic" => extract_anthropic_tokens(event),
        "openai" => extract_openai_tokens(event),
        "google" => extract_google_tokens(event),
        _ => (0, 0),
    }
}

fn extract_anthropic_tokens(event: &GatewayEvent) -> (u64, u64) {
    // Response body (or final SSE event with type=message_stop) contains:
    //   {"type":"message","usage":{"input_tokens":N,"output_tokens":M}}
    // For streaming: the message_delta event has usage.output_tokens
    // Parse response_body, scan for "usage" key with serde_json::Value
}

fn extract_openai_tokens(event: &GatewayEvent) -> (u64, u64) {
    // Non-streaming: {"usage":{"prompt_tokens":N,"completion_tokens":M,"total_tokens":T}}
    // Streaming: final chunk has usage object
}

fn extract_google_tokens(event: &GatewayEvent) -> (u64, u64) {
    // {"usageMetadata":{"promptTokenCount":N,"candidatesTokenCount":M,"totalTokenCount":T}}
}
```

### 1.3 Tool call extraction

**New file: `crates/capsem-core/src/gateway/tools.rs`**

Extract tool use from request/response bodies:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_name: String,
    pub tool_input: Option<String>,  // JSON string, truncated at 1KB
    pub tool_result: Option<String>, // JSON string, truncated at 1KB
}

/// Extract tool calls from a gateway event's response body.
pub fn extract_tool_calls(event: &GatewayEvent) -> Vec<ToolCall> {
    match event.provider.as_str() {
        "anthropic" => extract_anthropic_tools(event),
        "openai" => extract_openai_tools(event),
        "google" => extract_google_tools(event),
        _ => vec![],
    }
}

fn extract_anthropic_tools(event: &GatewayEvent) -> Vec<ToolCall> {
    // Response SSE has content blocks with type="tool_use":
    //   {"type":"tool_use","id":"toolu_xxx","name":"Read","input":{"file_path":"..."}}
    // Request body (next turn) has tool_result blocks with content
    // Parse both request_body and response_body
}

fn extract_openai_tools(event: &GatewayEvent) -> Vec<ToolCall> {
    // Response has choices[0].message.tool_calls array:
    //   [{"type":"function","function":{"name":"read_file","arguments":"{...}"}}]
}

fn extract_google_tools(event: &GatewayEvent) -> Vec<ToolCall> {
    // Response has candidates[0].content.parts[] with functionCall:
    //   {"functionCall":{"name":"read_file","args":{...}}}
}
```

### 1.4 Extend GatewayDb schema + summary

**File: `crates/capsem-core/src/gateway/audit.rs`**

Add columns to `gateway_events` for pre-extracted data (avoids re-parsing JSON on every query):

```sql
-- Added to CREATE_SCHEMA (new installs get them directly)
-- Existing DBs get a migration on open()
ALTER TABLE gateway_events ADD COLUMN input_tokens INTEGER DEFAULT 0;
ALTER TABLE gateway_events ADD COLUMN output_tokens INTEGER DEFAULT 0;
```

Add `tool_calls` table:

```sql
CREATE TABLE IF NOT EXISTS tool_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    gateway_event_id INTEGER NOT NULL,
    tool_name TEXT NOT NULL,
    tool_input TEXT,
    tool_result TEXT,
    FOREIGN KEY (gateway_event_id) REFERENCES gateway_events(id)
);
CREATE INDEX IF NOT EXISTS idx_tool_calls_name ON tool_calls(tool_name);
CREATE INDEX IF NOT EXISTS idx_tool_calls_event ON tool_calls(gateway_event_id);
```

Migration: on `GatewayDb::open()`, check if `input_tokens` column exists via `PRAGMA table_info(gateway_events)`. If not, run ALTER statements.

**New struct:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewaySummary {
    pub total_calls: u64,
    pub calls_by_provider: Vec<(String, u64)>,   // [("anthropic", 12), ("openai", 3)]
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_request_bytes: u64,
    pub total_response_bytes: u64,
    pub models_used: Vec<(String, u64)>,          // [("claude-sonnet-4-20250514", 10)]
    pub errors: u64,
}
```

Note: no `estimated_cost_usd` in the Rust struct. Cost estimation lives in the frontend (see 1.6) so pricing updates don't require recompilation.

**New methods on `GatewayDb`:**

```rust
/// Aggregate stats across all recorded events.
pub fn summary(&self) -> rusqlite::Result<GatewaySummary> {
    // Query 1: SELECT COUNT(*), SUM(input_tokens), SUM(output_tokens),
    //          SUM(request_bytes), SUM(response_bytes),
    //          SUM(CASE WHEN error IS NOT NULL THEN 1 ELSE 0 END) FROM gateway_events
    // Query 2: SELECT provider, COUNT(*) FROM gateway_events GROUP BY provider
    // Query 3: SELECT model, COUNT(*) FROM gateway_events WHERE model IS NOT NULL GROUP BY model
}

/// Record tool calls for a gateway event (by rowid).
pub fn record_tool_calls(&self, event_id: i64, calls: &[ToolCall]) -> rusqlite::Result<()> { ... }

/// Aggregate tool call counts: [(tool_name, count)], sorted by count desc.
pub fn tool_call_summary(&self) -> rusqlite::Result<Vec<(String, u64)>> {
    // SELECT tool_name, COUNT(*) FROM tool_calls GROUP BY tool_name ORDER BY COUNT(*) DESC
}
```

**Modify `GatewayDb::record()`:**

After inserting the event, call `extract_tokens()` to populate `input_tokens` / `output_tokens`, and call `extract_tool_calls()` to insert into `tool_calls` table. Return the inserted rowid for the tool_calls foreign key.

```rust
pub fn record(&self, event: &GatewayEvent) -> rusqlite::Result<i64> {
    let (input_tok, output_tok) = tokens::extract_tokens(event);
    // INSERT with input_tokens, output_tokens columns
    let rowid = self.conn.last_insert_rowid();

    let tool_calls = tools::extract_tool_calls(event);
    if !tool_calls.is_empty() {
        self.record_tool_calls(rowid, &tool_calls)?;
    }
    Ok(rowid)
}
```

### 1.5 Add `total_llm_calls` and `estimated_cost_usd` to session.rs

**File: `crates/capsem-core/src/session.rs`**

Add columns to the `sessions` table:

```sql
ALTER TABLE sessions ADD COLUMN total_llm_calls INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN total_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN total_output_tokens INTEGER NOT NULL DEFAULT 0;
```

Add `SessionIndex::update_llm_counts()` method. Update `SessionRecord` struct to include these fields.

### 1.6 Tauri IPC commands + events

**File: `crates/capsem-app/src/commands.rs`**

New commands:

```rust
#[tauri::command]
pub fn gateway_events(limit: Option<usize>) -> Result<Vec<GatewayEvent>, String> {
    // Read from GatewayDb via AppState, default limit 200
}

#[tauri::command]
pub fn gateway_summary() -> Result<GatewaySummary, String> {
    // Calls GatewayDb::summary()
}

#[tauri::command]
pub fn tool_call_summary() -> Result<Vec<(String, u64)>, String> {
    // Calls GatewayDb::tool_call_summary()
}
```

**Tauri event emission** -- in the gateway server handler (where `GatewayDb::record()` is called), emit events so the frontend can update reactively:

```rust
// When a gateway request starts processing:
app_handle.emit("gateway-activity", json!({
    "type": "start",
    "provider": provider_kind.as_str(),
    "model": model_name,
}));

// When the gateway request completes (after record()):
app_handle.emit("gateway-activity", json!({
    "type": "complete",
    "provider": provider_kind.as_str(),
    "model": model_name,
    "input_tokens": input_tokens,
    "output_tokens": output_tokens,
    "duration_ms": duration_ms,
}));
```

### 1.7 Tests

- **Unit tests** for `tokens.rs`: parse sample Anthropic/OpenAI/Google response bodies (both streaming and non-streaming), extract correct token counts. Test malformed JSON, missing `usage` key, empty bodies, null model.
- **Unit tests** for `tools.rs`: parse sample tool call responses per provider. Test missing tools, malformed JSON, truncation at 1KB, tool names with special characters, empty tool input.
- **Unit tests** for `GatewayDb::summary()`: insert mixed events across providers, verify aggregation (total counts, per-provider counts, token sums, error count, model grouping).
- **Unit tests** for `GatewayDb::tool_call_summary()`: insert events with tool calls, verify counts and sort order.
- **Unit tests** for `GatewayDb::record()` returning rowid and auto-extracting tokens.
- **Adversarial tests**: extremely long response bodies (verify no OOM), unicode in tool names, zero-token events, events with error + no response body.
- **Integration test** in `crates/capsem-core/tests/gateway_integration.rs`: full flow -- create DB, record events with real-ish bodies, query summary + tool_call_summary, verify token extraction + tool call extraction end-to-end.
- **Migration test**: open a DB without the new columns, verify migration adds them, verify existing data is preserved with 0 defaults.

---

## Milestone 2: Frontend -- Types, API, Store, Mock Data

All frontend plumbing without touching any components yet.

### 2.1 Frontend types

**File: `frontend/src/lib/types.ts`**

Add:

```typescript
/** A single LLM API call event from ai.db (mirrors gateway/audit.rs GatewayEvent). */
export interface GatewayEvent {
  timestamp: number;
  provider: string;
  model: string | null;
  method: string;
  path: string;
  status_code: number;
  duration_ms: number;
  request_bytes: number;
  response_bytes: number;
  streamed: boolean;
  input_tokens: number;
  output_tokens: number;
  error: string | null;
}

/** Aggregated LLM gateway stats (mirrors gateway/audit.rs GatewaySummary). */
export interface GatewaySummary {
  total_calls: number;
  calls_by_provider: [string, number][];
  total_input_tokens: number;
  total_output_tokens: number;
  total_request_bytes: number;
  total_response_bytes: number;
  models_used: [string, number][];
  errors: number;
}

/** Gateway activity event (real-time, pushed from backend). */
export interface GatewayActivity {
  type: 'start' | 'complete';
  provider: string;
  model: string | null;
  input_tokens?: number;
  output_tokens?: number;
  duration_ms?: number;
}
```

Extend `SessionRecord`:

```typescript
export interface SessionRecord {
  // ... existing fields ...
  total_llm_calls: number;
  total_input_tokens: number;
  total_output_tokens: number;
}
```

Change `ViewName`:

```typescript
export type ViewName = 'terminal' | 'settings' | 'sessions';
```

### 2.2 Frontend API

**File: `frontend/src/lib/api.ts`**

Add:

```typescript
export async function gatewayEvents(limit = 200): Promise<GatewayEvent[]> { ... }
export async function gatewaySummary(): Promise<GatewaySummary> { ... }
export async function toolCallSummary(): Promise<[string, number][]> { ... }
export async function onGatewayActivity(
  callback: (event: GatewayActivity) => void
): Promise<UnlistenFn> { ... }
```

### 2.3 Cost estimation in frontend

**New file: `frontend/src/lib/pricing.ts`**

Cost estimation lives in the frontend so pricing table updates don't need a Rust rebuild:

```typescript
/** Per-million-token pricing: [input, output]. */
const MODEL_PRICING: Record<string, [number, number]> = {
  'opus': [15.0, 75.0],
  'sonnet': [3.0, 15.0],
  'haiku': [0.25, 1.25],
  'gpt-4o': [2.50, 10.0],
  'gpt-4.1': [2.0, 8.0],
  'o3': [2.0, 8.0],
  'o4-mini': [1.10, 4.40],
  'gemini-2.5-pro': [1.25, 10.0],
  'gemini-2.5-flash': [0.15, 0.60],
};

/** Estimate cost in USD for a model + token counts. */
export function estimateCost(model: string | null, inputTokens: number, outputTokens: number): number {
  if (!model) return 0;
  const key = Object.keys(MODEL_PRICING).find(k => model.includes(k)) ?? 'sonnet';
  const [inputRate, outputRate] = MODEL_PRICING[key];
  return (inputTokens * inputRate + outputTokens * outputRate) / 1_000_000;
}

/** Estimate total cost from a summary's per-model breakdown. Not perfectly accurate
 *  (uses call count * average tokens per model), but the events-level calculation
 *  in the store is precise. This is a convenience for the summary card. */
export function estimateTotalCost(events: GatewayEvent[]): number {
  return events.reduce((sum, e) => sum + estimateCost(e.model, e.input_tokens, e.output_tokens), 0);
}
```

### 2.4 Gateway store (event-driven, no polling)

**New file: `frontend/src/lib/stores/gateway.svelte.ts`**

```typescript
import { gatewayEvents, gatewaySummary, toolCallSummary, onGatewayActivity } from '../api';
import { estimateCost } from '../pricing';
import type { GatewayEvent, GatewaySummary, GatewayActivity } from '../types';

class GatewayStore {
  events = $state<GatewayEvent[]>([]);
  summary = $state<GatewaySummary | null>(null);
  toolCalls = $state<[string, number][]>([]);
  inflight = $state<GatewayActivity | null>(null);  // currently in-flight request

  // Derived stats
  totalCalls = $derived(this.summary?.total_calls ?? 0);
  totalInputTokens = $derived(this.summary?.total_input_tokens ?? 0);
  totalOutputTokens = $derived(this.summary?.total_output_tokens ?? 0);
  errors = $derived(this.summary?.errors ?? 0);

  // Cost computed in frontend from event-level data
  totalCost = $derived(
    this.events.reduce((sum, e) => sum + estimateCost(e.model, e.input_tokens, e.output_tokens), 0)
  );

  private unlistenActivity: (() => void) | null = null;

  async start() {
    // Initial fetch
    await this.refresh();

    // Subscribe to real-time events (pushed from backend on every gateway request)
    this.unlistenActivity = await onGatewayActivity((event) => {
      if (event.type === 'start') {
        this.inflight = event;
      } else {
        this.inflight = null;
        // Refresh data after each completed request
        this.refresh();
      }
    });
  }

  stop() {
    this.unlistenActivity?.();
    this.unlistenActivity = null;
  }

  private async refresh() {
    try {
      const [ev, sum, tc] = await Promise.all([
        gatewayEvents(100),
        gatewaySummary(),
        toolCallSummary(),
      ]);
      this.events = ev;
      this.summary = sum;
      this.toolCalls = tc;
    } catch { /* VM not running */ }
  }
}

export const gatewayStore = new GatewayStore();
```

No polling. The store fetches once on start, then refreshes on each `gateway-activity` complete event. The `inflight` field drives the "thinking..." activity pill.

### 2.5 Mock data

**File: `frontend/src/lib/mock.ts`**

Add:

```typescript
export const MOCK_GATEWAY_EVENTS: GatewayEvent[] = [
  // 10 events across 3 providers, mix of models:
  // - 5 Anthropic (3 sonnet, 1 opus, 1 haiku) with varying token counts
  // - 3 OpenAI (2 gpt-4.1, 1 o4-mini)
  // - 2 Google (gemini-2.5-pro)
  // Include 1 error event (status 500, error field set)
  // Timestamps spread across last 30 minutes
  // Token counts: range from 500-50000 input, 100-20000 output
];

export const MOCK_GATEWAY_SUMMARY: GatewaySummary = {
  total_calls: 10,
  calls_by_provider: [['anthropic', 5], ['openai', 3], ['google', 2]],
  total_input_tokens: 125000,
  total_output_tokens: 48000,
  total_request_bytes: 256000,
  total_response_bytes: 980000,
  models_used: [
    ['claude-sonnet-4-20250514', 3],
    ['gpt-4.1-2025-04-14', 2],
    ['claude-opus-4-20250514', 1],
    ['claude-haiku-4-20250514', 1],
    ['o4-mini-2025-04-16', 1],
    ['gemini-2.5-pro', 2],
  ],
  errors: 1,
};

export const MOCK_TOOL_CALLS: [string, number][] = [
  ['Read', 45], ['Bash', 23], ['Grep', 15], ['Write', 12],
  ['Edit', 10], ['Glob', 8], ['Task', 3], ['WebSearch', 2],
];
```

Wire mock returns into the existing mock fallback pattern in `api.ts`.

---

## Milestone 3: UI Restructure (StatusBar, Panels, Sidebar, SessionsView)

All UI changes in one shot. No intermediate states.

### 3.1 StatusBar -> collapsible activity panel + status pills

**File: `frontend/src/lib/components/StatusBar.svelte`**

Complete rewrite. The StatusBar becomes two parts:
1. **Footer bar** (always visible): interactive status pills + expand/collapse chevron
2. **Activity panel** (above footer, collapsible): tabbed Network + LLM content

```
+---------------------------------------------------------------------+
| Activity panel (when expanded, height resizable via drag handle)     |
|  [Network] [LLM]    <-- tab bar                                     |
|  (tab content: charts, tables)                                       |
+---------------------------------------------------------------------+
| [* Running 2m 13s] | [HTTPS 7/3] | [LLM $0.42 15] | [thinking...] | [^] |
+---------------------------------------------------------------------+
```

**Footer pills (always visible):**

**VM pill:**
- Colored dot + state + live uptime (1s interval while running)
- Colors: text-info (running), text-warning (booting), text-secondary (stopped/error)
- Click: no action (informational)

**Network pill:**
- `HTTPS {allowed}/{denied}` -- compact format
- Click: toggles activity panel, switches to Network tab
- Brief flash animation on new events (blue for allowed, purple for denied)

**LLM pill:**
- `LLM ${cost} {calls}` -- compact
- Click: toggles activity panel, switches to LLM tab
- Brief flash on new gateway-activity complete event

**Activity pill (conditional):**
- Only visible when `gatewayStore.inflight` is non-null
- Shows: `{provider} thinking...` with pulsing animation + elapsed time (1s tick)
- Disappears when inflight becomes null

**Expand/collapse chevron:**
- Click: toggles panel
- Rotates 180deg when expanded

**Panel behavior:**
- `expanded` and `panelHeight` stored in localStorage for persistence across reloads
- Drag handle on top edge for resize (mousedown -> mousemove -> mouseup)
- Min height: 150px, max height: 60% of viewport
- Escape closes the panel (only when panel is focused, not when terminal is focused)
- Clicking a pill that's already active on the current tab closes the panel (toggle behavior)

**Implementation:**

```svelte
<script lang="ts">
  import { networkStore } from '../stores/network.svelte';
  import { gatewayStore } from '../stores/gateway.svelte';
  import { vmStore } from '../stores/vm.svelte';
  import NetworkPanel from './NetworkPanel.svelte';
  import LlmPanel from './LlmPanel.svelte';
  import ChevronIcon from '../icons/ChevronIcon.svelte';

  let expanded = $state(localStorage.getItem('capsem_panel_expanded') === 'true');
  let activeTab = $state<'network' | 'llm'>('network');
  let panelHeight = $state(Number(localStorage.getItem('capsem_panel_height')) || 300);

  // Persist panel state
  $effect(() => { localStorage.setItem('capsem_panel_expanded', String(expanded)); });
  $effect(() => { localStorage.setItem('capsem_panel_height', String(panelHeight)); });

  // Live uptime ticker
  let uptimeSec = $state(0);
  let uptimeInterval: ReturnType<typeof setInterval> | null = null;
  $effect(() => {
    if (vmStore.isRunning) {
      uptimeInterval = setInterval(() => uptimeSec++, 1000);
    } else {
      if (uptimeInterval) clearInterval(uptimeInterval);
      uptimeSec = 0;
    }
    return () => { if (uptimeInterval) clearInterval(uptimeInterval); };
  });

  // Network flash tracking
  let lastAllowed = $state(networkStore.allowedCount);
  let lastDenied = $state(networkStore.deniedCount);
  let netFlash = $state<'blue' | 'purple' | null>(null);
  $effect(() => {
    if (networkStore.allowedCount > lastAllowed) netFlash = 'blue';
    else if (networkStore.deniedCount > lastDenied) netFlash = 'purple';
    lastAllowed = networkStore.allowedCount;
    lastDenied = networkStore.deniedCount;
    if (netFlash) setTimeout(() => netFlash = null, 600);
  });

  // Inflight elapsed timer
  let inflightStart = $state(0);
  let inflightElapsed = $state(0);
  $effect(() => {
    if (gatewayStore.inflight) {
      inflightStart = Date.now();
      const id = setInterval(() => inflightElapsed = Math.floor((Date.now() - inflightStart) / 1000), 1000);
      return () => clearInterval(id);
    }
    inflightElapsed = 0;
  });

  function toggleTab(tab: 'network' | 'llm') {
    if (expanded && activeTab === tab) {
      expanded = false;
    } else {
      expanded = true;
      activeTab = tab;
    }
  }

  function startResize(e: MouseEvent) { /* standard drag resize logic */ }
  function formatUptime(sec: number): string { /* same as existing SessionsView */ }
  function formatCost(usd: number): string {
    if (usd >= 1000) return `$${(usd / 1000).toFixed(1)}K`;
    if (usd >= 1) return `$${usd.toFixed(2)}`;
    if (usd >= 0.01) return `${(usd * 100).toFixed(0)}c`;
    return '<1c';
  }
</script>
```

### 3.2 NetworkPanel component

**New file: `frontend/src/lib/components/NetworkPanel.svelte`**

Absorbs the network analytics content currently in `SessionsView.svelte` lines 73-328. This is a move, not a rewrite -- same bar chart, pie chart, and events table. Adjustments:

- SVG viewBox heights reduced for panel context (120 -> 100)
- Table gets `overflow-x-auto` wrapper for narrow panels
- Component imports `networkStore` directly (no props needed)
- Add total stats row above charts (same 3-card grid from current NetworkView: Total/Allowed/Denied)

Layout:

```
+-------------------------------------------------------------------+
| [Total: 12]  [Allowed: 9]  [Denied: 3]                            |  (3 stat cards)
+-------------------------------------------------------------------+
| [Bar chart: calls over time]   |   [Pie chart: domain distribution]|  (2-col grid)
+-------------------------------------------------------------------+
| Time | Domain | Method | Path | Status | Decision | Duration       |  (table)
+-------------------------------------------------------------------+
```

### 3.3 LlmPanel component

**New file: `frontend/src/lib/components/LlmPanel.svelte`**

Layout:

```
+-------------------------------------------------------------------+
| [Cost: $0.42] [Input: 125K] [Output: 48K] [Calls: 15 (1 err)]     |  (4 stat cards)
+-------------------------------------------------------------------+
| [Bar: calls by provider]    |   [Pie: models used]                 |  (2-col grid)
+-------------------------------------------------------------------+
| Tool Usage                                                         |
| Read          ████████████████████████████████████  45              |
| Bash          ██████████████████                    23              |
| Grep          ████████████                          15              |
| (max 10, "+N more" if truncated)                                   |
+-------------------------------------------------------------------+
| Time | Provider | Model | Tokens (in/out) | Cost | Duration | Err  |  (table)
+-------------------------------------------------------------------+
```

**Stat cards (4 columns):**
- Cost: `$X.XX` (large number) -- computed from `gatewayStore.totalCost`
- Input Tokens: `125K` -- formatted with `formatTokens()`
- Output Tokens: `48K`
- Calls: `15` with `1 error` in secondary text (text-secondary color)

**Bar chart: calls by provider:**
- One vertical bar per provider
- Colors: Anthropic = `oklch(0.7 0.15 250)` (blue), OpenAI = `oklch(0.7 0.12 170)` (teal), Google = `oklch(0.75 0.15 85)` (amber)
- Same SVG pattern as existing network bar chart

**Pie chart: models used:**
- One slice per model, legend on right
- Same SVG pattern as existing domain pie chart
- Colors cycle through blue/purple/teal/indigo palette

**Tool usage horizontal bars:**
- From `gatewayStore.toolCalls`
- Sorted by count descending, max 10 rows
- Each row: tool name (left, text-xs), bar (proportional width, blue fill), count (right, tabular-nums)
- Bar max width based on highest count

**Events table:**
- Columns: Time, Provider, Model, Tokens (`{in}K / {out}K` format), Cost (`$0.XX`), Duration (`ms`), Status (badge: info for 2xx, secondary for 4xx/5xx)
- Default sort: newest first
- Pagination: 10 rows, "Show more" button adds 10
- Error rows: `bg-secondary/5` background tint

**Formatting functions:**

```typescript
function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

function formatCost(usd: number): string {
  if (usd >= 1000) return `$${(usd / 1000).toFixed(1)}K`;
  if (usd >= 1) return `$${usd.toFixed(2)}`;
  if (usd >= 0.01) return `${(usd * 100).toFixed(0)}c`;
  return '<1c';
}
```

### 3.4 Sidebar: Network item becomes panel toggle

**File: `frontend/src/lib/components/Sidebar.svelte`**

The Network sidebar item stays (keeps its icon and position) but instead of `sidebarStore.setView('network')`, it calls a new function that toggles the activity panel's Network tab. This is exposed via a new store method or a callback prop.

```svelte
<!-- Network button: toggles activity panel instead of switching views -->
<button onclick={() => toggleActivityPanel('network')} ...>
  <NetworkIcon />
  {#if !collapsed}<span>Network</span>{/if}
</button>
```

The `toggleActivityPanel` function is either:
- A method on a new `activityPanelStore`, or
- A callback passed down from App.svelte

The active-highlight logic: when the panel is expanded on the Network tab, the Network sidebar item shows the active style. When collapsed, no highlight.

### 3.5 Clean up SessionsView

**File: `frontend/src/lib/views/SessionsView.svelte`**

Remove:
- The entire "Network Analytics" section (bar chart, pie chart, events table, all derived state)
- The "LLM Usage" placeholder div
- All chart helpers (`timeBuckets`, `barMax`, `domainCounts`, `pieSlices`, `describeArc`, `tableLimit`, `tableEvents`, `hasMore`)
- The `networkStore` import

Add:
- Import `gatewayStore` and `estimateCost` / `formatTokens`
- **LLM Summary section** (between Current Session and Session History):

```svelte
<!-- LLM Summary -->
<div>
  <h3 class="text-sm font-semibold mb-3">LLM Usage</h3>
  {#if gatewayStore.totalCalls > 0}
    <div class="grid grid-cols-3 gap-3 mb-3">
      <!-- Cost card -->
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Cost</div>
        <div class="mt-1 text-xl font-semibold tabular-nums">${gatewayStore.totalCost.toFixed(2)}</div>
      </div>
      <!-- Tokens card -->
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Tokens</div>
        <div class="mt-1 text-sm font-semibold tabular-nums">
          {formatTokens(gatewayStore.totalInputTokens)} in / {formatTokens(gatewayStore.totalOutputTokens)} out
        </div>
      </div>
      <!-- Calls card -->
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">LLM Calls</div>
        <div class="mt-1 text-xl font-semibold tabular-nums">{gatewayStore.totalCalls}</div>
        {#if gatewayStore.errors > 0}
          <div class="mt-1 text-[10px] text-secondary">{gatewayStore.errors} errors</div>
        {/if}
      </div>
    </div>

    <!-- Provider breakdown bar (stacked horizontal) -->
    <div class="flex h-3 rounded-full overflow-hidden mb-1">
      {#each gatewayStore.summary?.calls_by_provider ?? [] as [provider, count]}
        <div
          class="h-full"
          style="width: {(count / gatewayStore.totalCalls) * 100}%; background: {providerColor(provider)}"
          title="{provider}: {count} calls"
        ></div>
      {/each}
    </div>
    <div class="flex gap-3 text-[10px] text-base-content/50">
      {#each gatewayStore.summary?.calls_by_provider ?? [] as [provider, count]}
        <span class="flex items-center gap-1">
          <span class="inline-block size-2 rounded-sm" style="background: {providerColor(provider)}"></span>
          {provider} ({count})
        </span>
      {/each}
    </div>
  {:else}
    <div class="flex items-center justify-center rounded-lg border border-base-300 bg-base-200/50 py-8">
      <span class="text-sm text-base-content/40">No LLM calls recorded yet</span>
    </div>
  {/if}
</div>
```

**Session History table** gains two columns after "Requests":
- "LLM Calls" -- `session.total_llm_calls` (number)
- "Tokens" -- `{formatTokens(session.total_input_tokens)} / {formatTokens(session.total_output_tokens)}`

Provider color helper:

```typescript
function providerColor(provider: string): string {
  switch (provider) {
    case 'anthropic': return 'oklch(0.7 0.15 250)';   // blue
    case 'openai': return 'oklch(0.7 0.12 170)';      // teal
    case 'google': return 'oklch(0.75 0.15 85)';      // amber
    default: return 'oklch(0.6 0.1 250)';              // muted blue
  }
}
```

### 3.6 App.svelte changes

**File: `frontend/src/lib/components/App.svelte`**

- Remove `NetworkView` import and its conditional render block
- Add `gatewayStore` import, call `gatewayStore.start()` in `onMount`, `gatewayStore.stop()` in `onDestroy`
- Remove `{#if sidebarStore.activeView === 'network'}` branch (only terminal, settings, sessions remain)

### 3.7 Delete NetworkView

**Delete: `frontend/src/lib/views/NetworkView.svelte`**

Content is now split between:
- NetworkPanel (charts + table, in the activity panel)
- NetworkPanel stat cards (total/allowed/denied counts)

The `NetworkIcon` component is kept (used by sidebar button that toggles the panel).

### 3.8 CSS animations

**File: `frontend/src/styles/global.css`**

Add:

```css
@keyframes pill-flash-blue {
  0% { background-color: oklch(0.7 0.15 250 / 0.3); }
  100% { background-color: transparent; }
}
@keyframes pill-flash-purple {
  0% { background-color: oklch(0.65 0.15 300 / 0.3); }
  100% { background-color: transparent; }
}
.pill-flash-blue { animation: pill-flash-blue 0.6s ease-out; }
.pill-flash-purple { animation: pill-flash-purple 0.6s ease-out; }

@keyframes thinking-pulse {
  0%, 100% { opacity: 0.5; }
  50% { opacity: 1; }
}
.thinking-pulse { animation: thinking-pulse 1.5s ease-in-out infinite; }

@media (prefers-reduced-motion: reduce) {
  .pill-flash-blue, .pill-flash-purple { animation: none; }
  .thinking-pulse { animation: none; opacity: 0.8; }
}
```

### 3.9 New icons

**New file: `frontend/src/lib/icons/ChevronIcon.svelte`**

SVG chevron (up/down), rotatable via CSS transform. Same style as existing icons (size-5, currentColor).

### 3.10 Tests

- `just ui`: verify all views render in mock mode
- Footer: VM pill shows "Running" + ticking uptime, Network pill shows counts, LLM pill shows cost + calls
- Click Network pill: activity panel opens on Network tab with charts + table
- Click LLM pill: panel switches to LLM tab with stats, charts, tool bars, table
- Click same pill again: panel closes (toggle)
- Click chevron: toggles panel
- Sidebar Network button: toggles panel on Network tab (same as clicking Network pill)
- Drag handle: resizes panel, persists height in localStorage
- Panel expanded state persists across page reload
- SessionsView: shows Current Session + LLM Summary + Session History (no network charts)
- Activity pill: appears when mock inflight is set, shows "anthropic thinking... 3s"
- Flash animations: trigger on mock data change, respect prefers-reduced-motion
- Verify `just check` passes (astro check + svelte-check + production build)

---

## Milestone 4: Keyboard Shortcuts + Command Palette

### 4.1 Keyboard shortcut store

**New file: `frontend/src/lib/stores/shortcuts.svelte.ts`**

```typescript
export interface Shortcut {
  id: string;
  label: string;
  keys: string;           // display string: "Cmd+K", "Cmd+1"
  key: string;            // KeyboardEvent.key: "k", "1"
  meta: boolean;          // requires Cmd/Ctrl
  shift: boolean;         // requires Shift
  action: () => void;
  category: string;       // "Navigation", "Actions", "Help"
}

class ShortcutStore {
  shortcuts = $state<Shortcut[]>([]);

  register(shortcut: Omit<Shortcut, 'id'> & { id?: string }) {
    const id = shortcut.id ?? shortcut.keys;
    // Prevent duplicates
    this.shortcuts = this.shortcuts.filter(s => s.id !== id);
    this.shortcuts = [...this.shortcuts, { ...shortcut, id }];
  }

  handleKeydown(e: KeyboardEvent) {
    // Don't intercept when typing in input/textarea
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;

    for (const s of this.shortcuts) {
      if (e.key === s.key && e.metaKey === s.meta && e.shiftKey === s.shift) {
        e.preventDefault();
        s.action();
        return;
      }
    }
  }
}

export const shortcutStore = new ShortcutStore();
```

### 4.2 Register shortcuts in App.svelte

**File: `frontend/src/lib/components/App.svelte`**

In `onMount`, register all shortcuts:

```typescript
shortcutStore.register({ keys: 'Cmd+K', key: 'k', meta: true, shift: false, label: 'Command palette', category: 'Navigation', action: () => showCommandPalette = true });
shortcutStore.register({ keys: 'Cmd+1', key: '1', meta: true, shift: false, label: 'Console', category: 'Navigation', action: () => sidebarStore.setView('terminal') });
shortcutStore.register({ keys: 'Cmd+2', key: '2', meta: true, shift: false, label: 'Settings', category: 'Navigation', action: () => sidebarStore.setView('settings') });
shortcutStore.register({ keys: 'Cmd+3', key: '3', meta: true, shift: false, label: 'Sessions', category: 'Navigation', action: () => sidebarStore.setView('sessions') });
shortcutStore.register({ keys: 'Cmd+/', key: '/', meta: true, shift: false, label: 'Keyboard shortcuts', category: 'Help', action: () => showShortcutsHelp = true });
shortcutStore.register({ keys: 'Cmd+B', key: 'b', meta: true, shift: false, label: 'Toggle sidebar', category: 'Navigation', action: () => sidebarStore.toggleCollapsed() });
shortcutStore.register({ keys: 'Cmd+J', key: 'j', meta: true, shift: false, label: 'Toggle activity panel', category: 'Navigation', action: togglePanel });
shortcutStore.register({ keys: 'Escape', key: 'Escape', meta: false, shift: false, label: 'Close / return to terminal', category: 'Navigation', action: handleEscape });
```

Add `onkeydown={shortcutStore.handleKeydown}` on the root div (with `svelte:window` or directly).

The `handleEscape` function: if command palette open -> close it. If shortcuts help open -> close it. If activity panel open -> close it. Otherwise -> switch to terminal.

### 4.3 Command palette

**New file: `frontend/src/lib/components/CommandPalette.svelte`**

```svelte
<script lang="ts">
  import { shortcutStore } from '../stores/shortcuts.svelte';

  let { open = $bindable(false) } = $props();
  let query = $state('');
  let selectedIndex = $state(0);

  const filtered = $derived(
    shortcutStore.shortcuts.filter(s =>
      s.label.toLowerCase().includes(query.toLowerCase())
    )
  );

  const grouped = $derived(
    Object.groupBy(filtered, s => s.category)
  );

  function execute(shortcut: Shortcut) {
    open = false;
    query = '';
    shortcut.action();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') { open = false; query = ''; }
    else if (e.key === 'ArrowDown') { selectedIndex = Math.min(selectedIndex + 1, filtered.length - 1); e.preventDefault(); }
    else if (e.key === 'ArrowUp') { selectedIndex = Math.max(selectedIndex - 1, 0); e.preventDefault(); }
    else if (e.key === 'Enter' && filtered[selectedIndex]) { execute(filtered[selectedIndex]); }
  }
</script>

{#if open}
  <!-- Backdrop -->
  <div class="fixed inset-0 bg-black/50 z-50" onclick={() => { open = false; query = ''; }}></div>

  <!-- Palette -->
  <div class="fixed top-[20%] left-1/2 -translate-x-1/2 w-full max-w-md z-50
              bg-base-100 border border-base-300 rounded-xl shadow-2xl overflow-hidden"
       onkeydown={handleKeydown}>

    <!-- Search input -->
    <div class="flex items-center gap-2 border-b border-base-300 px-4 py-3">
      <span class="text-base-content/40 text-sm">></span>
      <input
        type="text"
        class="flex-1 bg-transparent text-sm outline-none placeholder:text-base-content/30"
        placeholder="Search commands..."
        bind:value={query}
        use:autofocus
      />
      <kbd class="kbd kbd-xs">Cmd+K</kbd>
    </div>

    <!-- Results -->
    <div class="max-h-64 overflow-auto py-2">
      {#each Object.entries(grouped) as [category, items], ci}
        <div class="px-4 py-1 text-[10px] font-semibold text-base-content/40 uppercase tracking-wider">
          {category}
        </div>
        {#each items as shortcut, i}
          {@const flatIndex = /* compute flat index across groups */}
          <button
            class="flex w-full items-center justify-between px-4 py-1.5 text-sm
                   {flatIndex === selectedIndex ? 'bg-primary/10 text-primary' : 'hover:bg-base-200'}"
            onclick={() => execute(shortcut)}
          >
            <span>{shortcut.label}</span>
            <kbd class="kbd kbd-xs text-base-content/40">{shortcut.keys}</kbd>
          </button>
        {/each}
      {/each}
    </div>
  </div>
{/if}
```

### 4.4 Shortcuts help modal

**New file: `frontend/src/lib/components/ShortcutsHelp.svelte`**

```svelte
<script lang="ts">
  import { shortcutStore } from '../stores/shortcuts.svelte';

  let { open = $bindable(false) } = $props();

  const grouped = $derived(
    Object.groupBy(shortcutStore.shortcuts, s => s.category)
  );
</script>

{#if open}
  <div class="fixed inset-0 bg-black/50 z-50" onclick={() => open = false}></div>
  <div class="fixed top-[15%] left-1/2 -translate-x-1/2 w-full max-w-lg z-50
              bg-base-100 border border-base-300 rounded-xl shadow-2xl overflow-hidden">

    <div class="flex items-center justify-between border-b border-base-300 px-4 py-3">
      <span class="text-sm font-semibold">Keyboard Shortcuts</span>
      <button class="btn btn-ghost btn-xs" onclick={() => open = false}>x</button>
    </div>

    <div class="max-h-96 overflow-auto p-4 space-y-4">
      {#each Object.entries(grouped) as [category, items]}
        <div>
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-2">
            {category}
          </div>
          {#each items as shortcut}
            <div class="flex items-center justify-between py-1">
              <span class="text-sm">{shortcut.label}</span>
              <kbd class="kbd kbd-sm font-mono">{shortcut.keys}</kbd>
            </div>
          {/each}
        </div>
      {/each}
    </div>
  </div>
{/if}
```

### 4.5 Tests

- `Cmd+K` opens palette, typing filters results, arrow keys navigate, Enter executes, Escape closes
- `Cmd+/` opens shortcuts help, Escape closes
- `Cmd+1`/`2`/`3` switches views
- `Cmd+B` toggles sidebar
- `Cmd+J` toggles activity panel
- Shortcuts don't fire when typing in settings input fields
- Shortcuts don't fire when terminal has focus (terminal's custom key handler intercepts)

---

## Milestone 5: Onboarding & Polish

### 5.1 Welcome screen

**New file: `frontend/src/lib/components/WelcomeScreen.svelte`**

Shown when VM state is "not created" (first launch or after VM stopped):

```svelte
<div class="flex h-full items-center justify-center">
  <div class="text-center space-y-6 max-w-sm">
    <div class="text-4xl font-bold font-mono tracking-tight">capsem</div>
    <p class="text-sm text-base-content/50">Sandboxed AI Agent Execution</p>
    <div class="flex gap-3 justify-center">
      <button class="btn btn-sm btn-primary" onclick={() => sidebarStore.setView('settings')}>
        Configure Settings
      </button>
      <button class="btn btn-sm btn-ghost" onclick={() => showShortcutsHelp = true}>
        View Shortcuts
      </button>
    </div>
    <p class="text-xs text-base-content/30">Press Cmd+K for command palette</p>
  </div>
</div>
```

Shown instead of TerminalView when `!vmStore.isRunning`. In `App.svelte`:

```svelte
{#if sidebarStore.activeView === 'terminal'}
  {#if vmStore.isRunning}
    <TerminalView />
  {:else}
    <WelcomeScreen />
  {/if}
{/if}
```

### 5.2 Tests

- First launch (VM not running): welcome screen shows with action buttons
- Click "Configure Settings": switches to settings view
- Click "View Shortcuts": opens shortcuts help modal
- VM starts running: welcome screen replaced by terminal

---

## Files Created (new)

| File | Milestone | Purpose |
|------|-----------|---------|
| `crates/capsem-core/src/gateway/tokens.rs` | M1 | Token extraction from provider responses |
| `crates/capsem-core/src/gateway/tools.rs` | M1 | Tool call extraction from responses |
| `crates/capsem-core/tests/gateway_integration.rs` | M1 | Gateway integration tests |
| `frontend/src/lib/pricing.ts` | M2 | Model pricing table + cost estimation (frontend-only) |
| `frontend/src/lib/stores/gateway.svelte.ts` | M2 | LLM gateway store (event-driven) |
| `frontend/src/lib/components/NetworkPanel.svelte` | M3 | Network analytics (moved from SessionsView) |
| `frontend/src/lib/components/LlmPanel.svelte` | M3 | LLM analytics + tool usage panel |
| `frontend/src/lib/icons/ChevronIcon.svelte` | M3 | Expand/collapse chevron |
| `frontend/src/lib/stores/shortcuts.svelte.ts` | M4 | Keyboard shortcut registry |
| `frontend/src/lib/components/CommandPalette.svelte` | M4 | Cmd+K command palette |
| `frontend/src/lib/components/ShortcutsHelp.svelte` | M4 | Keyboard shortcuts help modal |
| `frontend/src/lib/components/WelcomeScreen.svelte` | M5 | First-launch empty state |

## Files Modified

| File | Milestone | Changes |
|------|-----------|---------|
| `crates/capsem-app/src/state.rs` | M1 | Add `Arc<Mutex<GatewayDb>>` to AppState |
| `crates/capsem-core/src/gateway/mod.rs` | M1 | Export tokens, tools modules |
| `crates/capsem-core/src/gateway/audit.rs` | M1 | Add token columns, tool_calls table, summary(), record() returns rowid, migration |
| `crates/capsem-core/src/session.rs` | M1 | Add total_llm_calls, total_input_tokens, total_output_tokens columns |
| `crates/capsem-app/src/commands.rs` | M1 | Add gateway_events, gateway_summary, tool_call_summary commands + gateway-activity event emission |
| `frontend/src/lib/types.ts` | M2 | Add GatewayEvent, GatewaySummary, GatewayActivity types; extend SessionRecord; change ViewName |
| `frontend/src/lib/api.ts` | M2 | Add gatewayEvents, gatewaySummary, toolCallSummary, onGatewayActivity |
| `frontend/src/lib/mock.ts` | M2 | Add mock gateway events, summary, tool calls |
| `frontend/src/lib/components/StatusBar.svelte` | M3 | Complete rewrite: collapsible panel + status pills + activity pill + animations |
| `frontend/src/lib/components/Sidebar.svelte` | M3 | Network button toggles activity panel instead of switching view |
| `frontend/src/lib/components/App.svelte` | M3, M4, M5 | Remove NetworkView, add gateway store init, add shortcut listener, add modals, welcome screen |
| `frontend/src/lib/views/SessionsView.svelte` | M3 | Remove network analytics, add LLM summary card + provider bar, add LLM columns to history table |
| `frontend/src/styles/global.css` | M3 | Add pill-flash and thinking-pulse animations |

## Files Deleted

| File | Milestone | Reason |
|------|-----------|--------|
| `frontend/src/lib/views/NetworkView.svelte` | M3 | Content moved to NetworkPanel in activity panel |
