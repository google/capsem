# T1: Dashboard Stats (Cross-Session Analytics)

Transform the New Tab Page from 4 plain number cards into a rich analytics dashboard. Also create the shared chart component library used by T2.

## Why

The New Tab Page (`frontend/src/lib/components/shell/NewTabPage.svelte`) already calls `api.getStats()` which returns:
- `global: GlobalStats` -- total_sessions, total_input/output_tokens, total_estimated_cost, total_tool_calls, total_mcp_calls, total_file_events, total_requests, total_allowed, total_denied, total_cache_creation_tokens, total_cache_read_tokens (after T0)
- `sessions: Vec<SessionRecord>` -- last 100 sessions with per-session created_at, stopped_at, tokens, cost, tool_calls, mcp_calls, file_events, requests
- `top_providers: Vec<ProviderSummary>` -- per-provider call_count, input/output_tokens, estimated_cost, total_duration_ms, cache tokens (after T0)
- `top_tools: Vec<ToolSummary>` -- per-tool call_count, total_bytes, total_duration_ms
- `top_mcp_tools: Vec<McpToolSummary>` -- per-MCP-tool call_count, total_bytes, total_duration_ms, server_name

Most of this data is **fetched but not rendered**. The page only shows 4 number cards (Sessions, Total Tokens, Total Cost, Requests) and the session list table.

**Charting**: `layerchart 1.0.13` is installed in package.json but never imported. It provides `BarChart`, `PieChart`, `LineChart`, `AreaChart` + 60 low-level Svelte components. Svelte 5 compatible (wraps LayerCake + D3).

**Design tokens**: Already defined in `frontend/src/lib/styles/global.css`:
- `--color-allowed` / `--color-denied` (blue/purple)
- `--color-token-input` / `--color-token-output` / `--color-token-cache`
- `--color-provider-anthropic` / `-google` / `-openai` / `-mistral`
- `--color-chart-grid` / `--color-chart-label`

## Dependencies

- T0 (cache token columns in GlobalStats, ProviderSummary, SessionRecord)

---

## Task 1.1: Chart component library

Create `frontend/src/lib/components/charts/` with reusable Svelte 5 chart wrappers that consume our design tokens. These are thin wrappers around layerchart's high-level components with Capsem theming applied.

| Component | Wraps | Used for | Props |
|---|---|---|---|
| `ChartCard.svelte` | -- | Card container | `title: string`, `loading: boolean`, slot for chart content. Uses `bg-card border border-card-line rounded-xl`. Shows skeleton on loading, "No data" on empty slot. |
| `DonutChart.svelte` | layerchart `PieChart` | Token distribution, request decisions, HTTP methods | `segments: {label: string, value: number, color: string}[]`. Renders donut with center total + legend. |
| `HBarChart.svelte` | layerchart `BarChart` | Top tools, top providers, top domains | `bars: {label: string, value: number, color: string, secondary?: number, secondaryColor?: string}[]`. Horizontal bars, optional stacked secondary value. |
| `AreaTimeline.svelte` | layerchart `AreaChart` | Activity over time, tokens over time | `series: {label: string, color: string, data: {x: number, y: number}[]}[]`. Stacked filled area. |
| `CostLine.svelte` | layerchart `LineChart` | Cumulative cost trends | `points: {x: number, y: number}[]`, `color: string`. Simple line chart. |

All components:
- Accept data as typed props (no SQL coupling)
- Use CSS custom properties for colors (`var(--color-allowed)`, etc.)
- Handle empty data gracefully (show "No data" message)
- Use `text-muted-foreground` for axis labels, `--color-chart-grid` for grid lines

---

## Task 1.2: Enhanced stat cards

Expand from 4 to 6 cards using fields already in `GlobalStats` (all data already fetched):

| Card | Data field | Enhancement |
|---|---|---|
| Sessions | `total_sessions` | Keep as-is |
| Tokens | `total_input_tokens + total_output_tokens` | Split sub-label: "in: X / out: Y" |
| Cost | `total_estimated_cost` | Keep as-is |
| Requests | `total_requests` | Sub-label: "X allowed / Y denied" from `total_allowed`/`total_denied` |
| **Tool Calls** (new) | `total_tool_calls + total_mcp_calls` | Sub-label: "X native / Y MCP" |
| **File Events** (new) | `total_file_events` | -- |

Card styling unchanged: `bg-card border border-card-line rounded-lg p-3` with `text-[11px] uppercase` label and `text-lg font-semibold` value.

---

## Task 1.3: Cross-session charts on New Tab Page

All data comes from the existing `api.getStats()` response (no new API calls):

| Chart | Data source in StatsResponse | Component | Details |
|---|---|---|---|
| **Provider usage** | `top_providers[]` | HBarChart | Horizontal bars: tokens by provider (input stacked with output). Colors from `--color-provider-*`. Each bar = `input_tokens + output_tokens` with `input_tokens` as primary, `output_tokens` as stacked secondary. |
| **Top tools** | `top_tools[]` + `top_mcp_tools[]` | HBarChart | Merge both arrays, sort by call_count desc, take top 10. Native tools colored `--color-allowed` (blue), MCP tools colored `--color-denied` (purple). |
| **Request decisions** | `global.total_allowed`, `global.total_denied` | DonutChart | 2-segment donut. Allowed = `--color-allowed` (blue), Denied = `--color-denied` (purple). |
| **Session activity** | `sessions[]` grouped by `date(created_at)` | AreaTimeline | Client-side GROUP BY: parse `created_at`, bucket by date, count sessions per day. Single series as filled area. |
| **Cost trend** | `sessions[]` sorted by `created_at` | CostLine | Sort sessions by created_at, compute running sum of `total_estimated_cost`. Each point = (date, cumulative cost). |

---

## Task 1.4: Layout

```
┌─────────────────────────────────────────────────┐
│ Sessions header + [New Session] button           │
├─────────────────────────────────────────────────┤
│ Session list table (unchanged)                   │
├─────────────────────────────────────────────────┤
│ Statistics                                       │
│ ┌────────┬────────┬────────┬────────┬────┬────┐ │
│ │Sessions│ Tokens │  Cost  │Requests│Tool│File│ │
│ └────────┴────────┴────────┴────────┴────┴────┘ │
│ ┌──────────────────┬───────────────────────────┐ │
│ │ Provider Usage   │ Top Tools                 │ │
│ │ (HBarChart)      │ (HBarChart)               │ │
│ ├──────────────────┼───────────────────────────┤ │
│ │ Session Activity │ Cost Trend                │ │
│ │ (AreaTimeline)   │ (CostLine)                │ │
│ ├──────────────────┼───────────────────────────┤ │
│ │ Request Decisions│                           │ │
│ │ (DonutChart)     │                           │ │
│ └──────────────────┴───────────────────────────┘ │
└─────────────────────────────────────────────────┘
```

Charts in a 2-column responsive grid: `grid grid-cols-1 lg:grid-cols-2 gap-4`.

---

## Files to create/modify

| File | Action |
|---|---|
| `frontend/src/lib/components/charts/ChartCard.svelte` | Create -- card wrapper with title, loading, empty |
| `frontend/src/lib/components/charts/DonutChart.svelte` | Create -- wraps layerchart PieChart |
| `frontend/src/lib/components/charts/HBarChart.svelte` | Create -- wraps layerchart BarChart |
| `frontend/src/lib/components/charts/AreaTimeline.svelte` | Create -- wraps layerchart AreaChart |
| `frontend/src/lib/components/charts/CostLine.svelte` | Create -- wraps layerchart LineChart |
| `frontend/src/lib/components/shell/NewTabPage.svelte` | Modify -- add charts grid, enhance stat cards |
| `frontend/src/lib/types/gateway.ts` | Verify -- types include top_providers/top_tools/top_mcp_tools (may already exist) |

## Verification

1. Run sessions to populate main.db with telemetry data
2. Open New Tab Page -- verify 6 stat cards with correct values and sub-labels
3. Verify all 5 charts render with real data from `/stats` response
4. Verify colors match design tokens (provider colors, blue/purple for allowed/denied)
5. Test with empty main.db (fresh install) -- charts and cards show graceful empty/zero state
6. Resize browser -- charts respond to 2-col -> 1-col breakpoint
7. `pnpm run check` passes with no warnings
