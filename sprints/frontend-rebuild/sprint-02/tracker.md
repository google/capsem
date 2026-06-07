# Sprint 02: Simple Tab Views (Stats, Logs)

Build simpler per-tab views with mock data.

Worktree: `worktrees/capsem-ui` (branch: `frontend-ui`)
Depends on: Sprint 01

## Architecture Decisions

- **Views render in parent frame**, not inside the VM iframe. Only the terminal needs iframe isolation (xterm.js with untrusted shell output). Stats/logs display structured data from the gateway API -- no XSS vector, no cross-VM contamination.
- **Terminal stays mounted** when switching to stats/logs via `class:hidden`. Preserves xterm.js state without iframe destroy/recreate.
- **View switcher in toolbar** (right side, next to menu): segmented button group (Terminal/Stats) shown only for VM tabs. Changes `tabStore.updateView()` which App.svelte routes.
- **Exec view removed** -- not useful; terminal already provides shell access.
- **Logs moved to menu dropdown** -- not a key VM view; accessible via Settings menu alongside Service Logs.
- **No virtual scroll** for logs: mock data is small. Virtual scroll deferred to Sprint 05 when real log volumes warrant it.
- **Log timestamps**: absolute `HH:MM:SS.sss` format (more useful than relative "2m ago" for debugging).

## Done

### Stats View
- [x] `StatsView.svelte` -- tabbed layout (AI, Tools, Network, Files)
- [x] AI tab: summary cards (calls, input/output tokens, cost) + per-model table
- [x] Tools tab: tool call list (name, server, args, result, duration, time)
- [x] Network tab: request list (method, URL, status, decision badge, duration, size, time)
- [x] File tab: event list (path, operation badge, size, time)
- [x] All data from mock.ts

### Logs View
- [x] `LogsView.svelte` -- VM log entry list (scrollable table)
- [x] Filter bar: source dropdown, level dropdown, text search
- [x] Auto-scroll to bottom (toggleable)
- [x] Entry count display
- [x] Level styling: info=blue, warn=amber, error=purple (per design system)
- [x] Accessible from Settings dropdown menu (not toolbar switcher)

### Service Logs
- [x] `ServiceLogsView.svelte` -- service-level log viewer (separate from per-VM logs)
- [x] Accessible from Settings dropdown menu

### Mock Data
- [x] `mock.ts` extended with stats and log fixtures
- [x] Model stats, tool calls, network events, file events, log entries

### Infrastructure
- [x] `TabView` type extended with `'stats'`
- [x] App.svelte routes VM views (terminal/stats) with terminal preserved via hidden class
- [x] Toolbar view switcher (right side, Terminal/Stats only) with Phosphor icons
- [x] Logs + Service Logs in menu dropdown

## Testing Gate

- [x] `pnpm run check` passes (0 errors, 3 pre-existing warnings)
- [x] `pnpm run build` passes
- [x] 162 vitest tests pass (137 existing + 25 new)
- [x] Log filter/search tested (level, source, text, combined, empty result)
- [x] Mock data integrity tested (unique IDs, valid fields, chronological order)
- [ ] Chrome DevTools MCP screenshot of each view in light + dark
