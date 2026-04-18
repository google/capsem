# T2: Per-Session Charts

Add layerchart visualizations to each StatsView tab, wiring up the 20+ SQL queries already defined in `sql.ts` but never used. Reuses chart components from T1.

## Why

The StatsView (`frontend/src/lib/components/views/StatsView.svelte`) has 5 tabs (AI, Tools, Network, Files, Snapshots) that currently display data in **tables only**. Meanwhile, `frontend/src/lib/sql.ts` defines 20+ SQL queries specifically designed for chart aggregations -- tokens over time, tools over time, cost over time, top domains, etc. -- but **none are wired up**.

The chart components built in T1 (ChartCard, DonutChart, HBarChart, AreaTimeline, CostLine) are designed to be reused here.

## Dependencies

- T0 (cache token first-class columns for the token split donut)
- T1 (chart component library)

## Current state

**StatsView** runs raw inline SQL queries (not the sql.ts constants) and renders everything as tables. It needs to:
1. Switch from inline SQL to sql.ts query constants
2. Add chart visualizations above the existing tables

**SQL queries available in sql.ts** (all ready to use):

AI tab:
- `AI_USAGE_PER_PROVIDER_SQL` -- provider, input_tokens, output_tokens, cost, call_count
- `AI_TOKENS_OVER_TIME_SQL` -- bucket, provider, tokens (5-call buckets)
- `AI_TOKENS_OVER_TIME_BY_MODEL_SQL` -- per-call model, tokens
- `AI_COST_OVER_TIME_SQL` -- bucket, provider, cost (5-call buckets)
- `AI_MODEL_USAGE_SQL` -- model, provider, input_tokens, output_tokens, tokens, cost, call_count

Tools tab:
- `TOOLS_STATS_SQL` -- total, native, mcp, allowed, denied
- `TOOLS_TOP_TOOLS_SQL` -- tool_name, cnt, source (native/mcp)
- `TOOLS_TOP_SERVERS_SQL` -- server_name, cnt
- `TOOLS_OVER_TIME_SQL` -- bucket, native, mcp (5-call buckets)

Network tab:
- `NET_STATS_SQL` -- total, allowed, denied, avg_latency
- `NET_REQUESTS_OVER_TIME_SQL` -- bucket, allowed, denied (3-event buckets)
- `NET_TOP_DOMAINS_SQL` -- domain, allowed, denied
- `NET_METHODS_SQL` -- method, cnt

Files tab:
- `FILE_STATS_SQL` -- total, created, modified, deleted
- `FILE_EVENTS_OVER_TIME_SQL` -- bucket, action, cnt (10-event buckets)
- `FILE_ACTIONS_SQL` -- action, cnt

---

## Task 2.1: AI tab charts

| Chart | SQL query | Component | Notes |
|---|---|---|---|
| Tokens by provider | `AI_USAGE_PER_PROVIDER_SQL` | HBarChart | input/output stacked per provider. Primary = input_tokens, secondary = output_tokens. Colors from `--color-provider-*`. |
| Tokens over time | `AI_TOKENS_OVER_TIME_SQL` | AreaTimeline | Stacked by provider, bucketed per 5 model calls. Each provider = one series. |
| Cost over time | `AI_COST_OVER_TIME_SQL` | CostLine | Cumulative by provider. Sum cost values across buckets. |
| Token distribution by model | `AI_MODEL_USAGE_SQL` | DonutChart | Segments = models, value = total tokens. |
| Input vs output vs cache split | **New query (below)** | DonutChart | 4-segment: input/output/cache-create/cache-read. Colors: `--color-token-input`, `--color-token-output`, `--color-token-cache`, and a 4th for cache-create. |

**New SQL to add to sql.ts** (`AI_TOKEN_SPLIT_SQL`):
```sql
SELECT
  COALESCE(SUM(input_tokens), 0) as input_tokens,
  COALESCE(SUM(output_tokens), 0) as output_tokens,
  COALESCE(SUM(cache_creation_tokens), 0) as cache_creation_tokens,
  COALESCE(SUM(cache_read_tokens), 0) as cache_read_tokens
FROM model_calls
```

---

## Task 2.2: Tools tab charts

| Chart | SQL query | Component | Notes |
|---|---|---|---|
| Top 10 tools | `TOOLS_TOP_TOOLS_SQL` | HBarChart | Color by source: native = `--color-allowed` (blue), mcp = `--color-denied` (purple). |
| Top commands | `TOOLS_TOP_COMMANDS_SQL` (**new**) | HBarChart | Most frequently run shell commands. Extract base command (first word) from Bash tool_calls arguments. |
| MCP servers | `TOOLS_TOP_SERVERS_SQL` | DonutChart | Call distribution across MCP server names. |
| Usage over time | `TOOLS_OVER_TIME_SQL` | AreaTimeline | Two stacked series: native (blue) and mcp (purple), bucketed per 5 calls. |

**New SQL to add to sql.ts** (`TOOLS_TOP_COMMANDS_SQL`):
```sql
SELECT
  CASE
    WHEN INSTR(cmd, ' ') > 0 THEN SUBSTR(cmd, 1, INSTR(cmd, ' ') - 1)
    ELSE cmd
  END as base_command,
  COUNT(*) as cnt,
  cmd as example
FROM (
  SELECT TRIM(json_extract(tc.arguments, '$.command')) as cmd
  FROM tool_calls tc
  WHERE tc.tool_name IN ('Bash', 'bash', 'bash_code_execution')
    AND tc.arguments IS NOT NULL
    AND json_extract(tc.arguments, '$.command') IS NOT NULL
)
WHERE cmd != ''
GROUP BY base_command
ORDER BY cnt DESC
LIMIT 10
```

This extracts the base command (e.g., `cargo` from `cargo build`, `git` from `git status`) and counts occurrences. The `example` column keeps one full command for tooltip display.

---

## Task 2.3: Network tab charts

| Chart | SQL query | Component | Notes |
|---|---|---|---|
| Requests over time | `NET_REQUESTS_OVER_TIME_SQL` | AreaTimeline | Two stacked series: allowed (blue) vs denied (purple), bucketed per 3 events. |
| Top domains | `NET_TOP_DOMAINS_SQL` | HBarChart | Stacked bars: allowed (primary) + denied (secondary) per domain. |
| HTTP methods | `NET_METHODS_SQL` | DonutChart | Segments = GET, POST, PUT, DELETE, etc. |

---

## Task 2.4: Files tab charts

| Chart | SQL query | Component | Notes |
|---|---|---|---|
| Events over time | `FILE_EVENTS_OVER_TIME_SQL` | AreaTimeline | Three stacked series by action (created, modified, deleted), bucketed per 10 events. Colors: `--color-file-created`, `--color-file-modified`, `--color-file-deleted`. |
| Action distribution | `FILE_ACTIONS_SQL` | DonutChart | Segments = created/modified/deleted using `--color-file-*` tokens. |

---

## Task 2.5: Layout changes to StatsView

- Each tab: chart grid (`grid grid-cols-1 lg:grid-cols-2 gap-4`) above the existing data table
- Data tables remain as drill-down detail view below charts
- Refactor StatsView to use `sql.ts` query constants instead of inline SQL strings currently hardcoded in onMount

---

## Files to modify

| File | Action |
|---|---|
| `frontend/src/lib/components/views/StatsView.svelte` | Modify -- add chart sections per tab, switch to sql.ts queries |
| `frontend/src/lib/sql.ts` | Add `AI_TOKEN_SPLIT_SQL` and `TOOLS_TOP_COMMANDS_SQL` queries |
| Chart components from T1 | Reuse (ChartCard, DonutChart, HBarChart, AreaTimeline, CostLine) |

## Verification

1. Boot a VM, run an AI agent that makes model calls, tool calls, and network requests
2. Open per-session Stats view
3. AI tab: verify 5 charts render (tokens by provider, tokens over time, cost over time, model donut, token split donut)
4. Tools tab: verify 4 charts (top tools, top commands, MCP servers, usage over time)
5. Network tab: verify 3 charts (requests over time, top domains, HTTP methods)
6. Files tab: verify 2 charts (events over time, action distribution)
7. Verify AI tab token split donut shows 4 segments including cache_creation and cache_read
8. Verify StatsView uses sql.ts query constants (no inline SQL)
9. Test empty state (fresh session, no telemetry yet) -- charts show "No data"
10. `pnpm run check` passes with no warnings
