# Meta Sprint: Analytics Dashboard

Make Capsem's stats the GOAT: truthful, fresh, safety-forward, cost-transparent, showing things nobody else can show.

## Why this sprint exists

Capsem captures unparalleled telemetry via the MITM proxy (full HTTP bodies, policy decisions, per-process attribution), the session DB (every model call, tool call, file event, snapshot), and the MCP gateway (per-server, per-tool decisions). Today almost none reaches the user: the dashboard shows 4 numbers, per-session views are data tables, nothing refreshes after mount, and the `main.db` rollup all cross-session views depend on doesn't actually run for UI-created sessions.

This sprint fixes the data pipeline AND the presentation together, so the stats experience is trustworthy end-to-end.

## Design philosophy

Five rules. If a proposed addition violates one, drop it.

1. **Show truth, never stale.** Every surface refreshes reactively. No mount-only fetches. If data is loading, say so. If empty, say why.
2. **Answer real questions.** "How safe is my agent?" "Where is the money going?" "Is cache working?" "What just changed on disk?" -- not charts for the sake of charts.
3. **Drill from hero to detail.** Dashboard shows 4-6 headline numbers; each clicks through to a chart; each chart row drills to raw events. No dead-end visualizations.
4. **Lean into Capsem's moat.** MITM, policy decisions, file events, snapshot attribution, process attribution, MCP detail -- these are unique. Lead with them.
5. **Respect historical data.** Finished ephemeral sessions must still show full analytics (session.db is gone, `main.db` rollup must carry the load). No "(empty)" for rows you already saw yesterday.

## Stack and component conventions

**Frontend:** Astro 6 static shell + Svelte 5 runes + Tailwind v4 + Preline (CSS-only).

**Preline usage is CSS-only** (per `skills/frontend-design/references/preline-docs/framework-integration.md`): we copy Preline's CSS component patterns, drive state with Svelte runes, and never touch `hs-*` JS plugins or `data-hs-*` attributes.

Every UI surface in T1-T3 uses these Preline patterns (and only these -- do not invent new base components):

| Surface | Preline pattern | Reference |
|---|---|---|
| Hero stat cards | Card with stats | `components-base.md` Card (L72) |
| Chart wrappers | Card (with header slot) | `components-base.md` Card |
| Status pills (running / allowed / denied / risky) | Badge | `components-base.md` Badge (L121) |
| Safety warnings, empty states | Alert (soft style) | `components-base.md` Alert (L46) |
| Session timeline, unified event timeline | Timeline | `components-base.md` Timeline (L174) |
| Loading placeholders | Skeleton | `components-base.md` Skeleton (L152) |
| Inline spinners | Spinner | `components-base.md` Spinner (L142) |
| Cache efficiency / safety score bars | Progress | `components-base.md` Progress (L134) |
| Data rows (sessions, requests, tools, files) | Table | `components-layout.md` Table (L128) |
| Tab navs (per-session view, dashboard sections) | Navs (tabs) | `components-navigation.md` Navs (L49) |
| Side nav (stats tab rail) | Sidebar | `components-navigation.md` Sidebar (L67) |
| Drill-down path on detail pages | Breadcrumb | `components-navigation.md` Breadcrumb (L109) |

Charts are **layerchart 1.0.13** (already in `package.json`, never imported). Wrap each layerchart primitive in a Preline Card shell so loading / empty / error states share the same chrome.

All colors come from design tokens in `frontend/src/styles/global.css` -- never hardcode hex. New tokens added in this sprint (listed in T0) follow the same `--color-*` naming.

## Phases

| Phase | Sub-sprint | Scope | Depends on |
|---|---|---|---|
| Foundation | **T0 Pipeline** | Rollup, per-field caps, cache tokens, ingest-time extractions, server aggregations, lifecycle SSE, new tokens | -- |
| Primitives + Surface | **T1 Charts + Dashboard** | Chart library (Preline Card shells + layerchart), hero cards, cost intelligence, safety posture, activity heatmap, cross-session breakdowns | T0 |
| Deep Dive | **T2 Session Views** | AI / Tools / Network / Files / **Security (new)** tabs -- full charts, unified Preline Timeline, domain graph, risky-command detection | T0, T1 |
| Conversation | **T3 Conversation Viewer** | Rich conversation browser with trace list, message cards, tool rendering, thinking blocks | T0 (ingest-time user message extraction) |

T1 and T2 share the chart component library (built in T1, reused in T2). T3 is independent of T1/T2 once T0 lands.

## What we're adding that nobody else has

Our moat, made visible:

| Capability | Surface in this sprint |
|---|---|
| MITM-captured HTTP bodies | Conversation viewer (T3), request timeline (T2 Network), denied-request hero (T2 Security) |
| Policy decisions per request | Safety posture hero card (T1), policy violation feed (T2 Security), allowed/denied donut |
| Per-process attribution | Process filter on Network tab (T2), process column in unified timeline |
| MCP per-server decisions | MCP server health chart with success rate per server (T2 Tools) |
| File system events with snapshot attribution | Files treemap, language breakdown, file churn ranking (T2 Files) |
| Command-level risk heuristics | Risky command detection in Security tab (T2) |

## New metrics that matter (not in the old sprint)

Each answers a question users actually ask:

| Metric | Question it answers | Why it's GOATed |
|---|---|---|
| Cache efficiency % | "Is my prompt design cache-friendly?" | `cache_read / (cache_read + input)`. Anthropic users can cut cost 90% with good caching -- nobody else shows this |
| Cost per turn | "How expensive is one user interaction?" | Running avg across sessions. Useful for comparing agents and providers |
| Tokens per successful tool call (ROI) | "How much context am I spending per useful action?" | Diagnoses bloated system prompts or unnecessary context |
| Stop-reason breakdown | "Why do my sessions end? Am I hitting limits?" | If 30% hit `max_tokens`, raise the limit; 50% ending on `tool_use` means the agent is mid-loop |
| Denied-request trend (7d sparkline) | "Is my agent probing new boundaries?" | Spikes = the agent is trying something new, worth reviewing |
| Activity heatmap (hour x day) | "When is my usage concentrated?" | Classic developer insight |
| Risky command detection | "Did my agent try anything dangerous?" | Pattern match on Bash args (`rm -rf`, `curl \| sh`, `sudo`, credential args). Unique to sandboxing context |
| MCP server success rate | "Which MCP servers are flaky?" | Per-server error rate across sessions |
| Concurrent VM peak | "Am I hitting capacity limits?" | Max simultaneous VMs in last 7d |

## Key files touched

**Backend (T0):**
- `crates/capsem-logger/src/writer.rs` -- per-field caps, new columns, ingest-time extraction
- `crates/capsem-logger/src/schema.rs` -- cache tokens, latest_user_message, is_streaming
- `crates/capsem-logger/src/events.rs` -- extended ModelCallEvent
- `crates/capsem-logger/src/reader.rs` -- updated queries
- `crates/capsem-core/src/net/ai_traffic/` -- provider parsers incl. streaming paths
- `crates/capsem-core/src/session/index.rs` -- sessions + per-session `ai_usage` / `tool_usage` / `mcp_usage` rollup, daily_stats, global_stats filter, cache/session-type columns
- `crates/capsem-core/src/session/types.rs` -- extended structs
- `crates/capsem-service/src/main.rs` -- rollup triggers on `/provision`/`/stop`/`/delete`/`/persist`, backfill, new endpoints (`/stats/activity`, `/stats/safety`, `/sessions/{id}/timeline`), SSE lifecycle stream

**Frontend (T1/T2/T3):**
- `frontend/src/lib/components/charts/` -- reusable library wrapping layerchart inside Preline Card shells (T1)
- `frontend/src/lib/components/shell/NewTabPage.svelte` -- dashboard (T1)
- `frontend/src/lib/components/views/StatsView.svelte` -- per-session Navs (T2)
- `frontend/src/lib/components/views/SecurityView.svelte` -- new Security tab (T2)
- `frontend/src/lib/components/conversation/` -- conversation viewer using Preline Timeline (T3)
- `frontend/src/lib/stores/stats.svelte.ts` -- reactive store with SSE subscription
- `frontend/src/lib/styles/global.css` -- new design tokens (listed in T0)
- `frontend/src/lib/sql.ts` -- extended query constants
- `frontend/src/lib/types/gateway.ts` -- new response types

## Out of scope (intentionally)

- Per-model global aggregation in `main.db` (keep per-provider; per-model stays per-session)
- Billing reconciliation (we're observability, not accounting)
- Cross-host federation (stats are local-first)
- Historical re-ingestion from JSONL (we're MITM-native; legacy rows stay as they are)

## Verification

End-to-end gate before the sprint is called done:

1. Fresh install, boot a VM from the UI, make 5 model calls (one streaming, one with cache, one with tool use, one denied, one that errors), stop the VM.
2. Dashboard: 6 hero cards correct, 5 charts populated, activity heatmap shows today's session, cache efficiency > 0.
3. Per-session view: all 6 tabs populated (AI / Tools / Network / Files / Security / Conversation); unified timeline renders in order; denied request appears in Security tab with matched_rule.
4. Kill the service mid-session; restart. Backfill rollup picks up the orphaned `session.db` on startup; dashboard numbers reflect it.
5. Everything refreshes without reload when a new VM is created or an existing VM stops (SSE-driven).
6. `just test` passes with new tests for rollup, per-provider cache extraction (streaming + non-streaming), user-message extraction, and risky-command detection.

## Commit discipline

Per project CLAUDE.md: every commit includes a `CHANGELOG.md` entry. Small, focused commits. Each T has explicit commit boundaries listed at the end of its doc -- do not bundle.

## Relevant just recipes

```bash
just test              # Full test suite (includes new tests)
just smoke             # Fast integration
just build             # Cargo workspace + frontend
just dev-frontend      # Vite dev server
```
