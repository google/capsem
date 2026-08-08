# Sprint: Analytics Dashboard

## Status: Not Started

## T0: Backend Data Prep
- [ ] Bump MAX_FIELD_BYTES from 256KB to 5MB in writer.rs
- [ ] Add cache_creation_tokens, cache_read_tokens columns to model_calls schema
- [ ] Add cache fields to ModelCallEvent struct
- [ ] Update writer INSERT to populate cache columns
- [ ] Update reader SELECT to read cache columns
- [ ] Extract cache tokens from Anthropic API response in MITM parser
- [ ] Extract cache tokens from OpenAI API response in MITM parser
- [ ] Extract cache tokens from Google API response in MITM parser
- [ ] Add cache columns to sessions table in main.db
- [ ] Add cache columns to ai_usage table in main.db
- [ ] Update rollup logic to SUM cache tokens
- [ ] Add cache fields to GlobalStats, ProviderSummary, SessionRecord structs
- [ ] Add cache fields to frontend TypeScript types
- [ ] cargo test passes
- [ ] pnpm run check passes
- [ ] Commit: `feat: promote cache tokens to first-class columns, bump field cap to 5MB`

## T1: Dashboard Stats
- [ ] Create ChartCard.svelte
- [ ] Create DonutChart.svelte (wrapping layerchart PieChart)
- [ ] Create HBarChart.svelte (wrapping layerchart BarChart)
- [ ] Create AreaTimeline.svelte (wrapping layerchart AreaChart)
- [ ] Create CostLine.svelte (wrapping layerchart LineChart)
- [ ] Enhance stat cards (4 -> 6, add sub-labels)
- [ ] Add Provider Usage chart to NewTabPage
- [ ] Add Top Tools chart to NewTabPage
- [ ] Add Request Decisions donut to NewTabPage
- [ ] Add Session Activity area chart to NewTabPage
- [ ] Add Cost Trend line chart to NewTabPage
- [ ] Verify types in gateway.ts match StatsResponse
- [ ] Test empty state
- [ ] pnpm run check passes
- [ ] Commit: `feat: analytics dashboard with charts on New Tab Page`

## T2: Per-Session Charts
- [ ] Add AI_TOKEN_SPLIT_SQL and TOOLS_TOP_COMMANDS_SQL to sql.ts
- [ ] AI tab: add 5 charts (tokens by provider, over time, cost, model donut, token split)
- [ ] Tools tab: add 4 charts (top tools, top commands, MCP servers, usage over time)
- [ ] Network tab: add 3 charts (requests over time, top domains, HTTP methods)
- [ ] Files tab: add 2 charts (events over time, action distribution)
- [ ] Refactor StatsView to use sql.ts query constants
- [ ] Test empty state per tab
- [ ] pnpm run check passes
- [ ] Commit: `feat: per-session charts in StatsView tabs`

## T3: Conversation Viewer
- [ ] Create TraceList.svelte
- [ ] Create UserMessage.svelte (with request_body_preview JSON parsing)
- [ ] Create MessageCard.svelte (markdown + shiki rendering)
- [ ] Create ToolCallCard.svelte (per-tool-type rendering)
- [ ] Create ThinkingBlock.svelte (collapsible)
- [ ] Create ConversationView.svelte (layout: sidebar + summary + timeline)
- [ ] Add Conversation tab to StatsView tab bar
- [ ] Test multi-turn conversation with tool use
- [ ] Test empty state (no model_calls)
- [ ] pnpm run check passes
- [ ] Commit: `feat: conversation viewer for AI model interactions`

## Notes
- layerchart 1.0.13 installed but never imported -- first usage in T1
- 20+ SQL queries in sql.ts are unused -- wired up in T2
- request_body_preview contains user messages (full API request body)
- shiki already set up for syntax highlighting (reuse in conversation viewer)
- Design tokens for charts already exist in global.css
