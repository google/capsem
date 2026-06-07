# Sprint: Tauri Shell Wiring

## T0: Frontend fixes and wiring
- [x] Fix `isMock` export in `mock.ts` -- build blocker (db.ts and Terminal.svelte import it)
- [x] Fix `isMock` import in `db.ts`
- [x] Add settings endpoints to `api.ts` (getSettings, saveSettings, getPresets, applyPreset, lintConfig)
- [x] Add MCP runtime endpoints to `api.ts` (getMcpServers, getMcpTools, refreshMcpTools, approveMcpTool, callMcpTool)
- [x] Add WebSocket `/events` to `api.ts` (onVmStateChanged, onDownloadProgress)
- [x] Wire `vm.svelte.ts` to real WebSocket events
- [x] Wire `mcp.svelte.ts` to real MCP runtime endpoints
- [x] Wire `wizard.svelte.ts` -- remove detectHostConfig, wire saveSettings
- [x] Delete `logs.svelte.ts` -- dead code
- [x] Tests for api.ts changes (45 tests)
- [x] Tests for mock.ts / db.ts fixes (15 tests)
- [x] Tests for stores (34 tests -- mcp, wizard)
- [x] `pnpm run check` -- 0 type errors
- [x] `pnpm run test` -- all 413 pass

## T1: New capsem-app (from scratch)
- [x] Create `crates/capsem-app/` -- clean Tauri shell (70 lines)
- [x] main.rs -- thin webview shell, 2 IPC commands (open_url, check_for_app_update)
- [x] Cargo.toml -- minimal deps (tauri, plugins, anyhow, serde, tracing)
- [x] tauri.conf.json -- gateway-first (window URL http://127.0.0.1:19222), bundled fallback
- [x] build.rs -- just tauri_build
- [x] Icons from graphics/tauri/
- [x] capabilities/ -- window + updater + opener only
- [x] `cargo check -p capsem-ui` passes
- [x] Delete old `crates/capsem-app/` (was 2000+ lines, 39 IPC commands)
- [x] Rename capsem-app2 -> capsem-app
- [x] Update workspace Cargo.toml
- [x] Switch entry point: index.astro now imports shell/App.svelte (tabbed multi-VM UI)
- [x] Delete dead old components (App.svelte, Sidebar.svelte)
- [x] `cargo check` (full workspace) passes

## T2: Service endpoints -- settings (including MCP config)
- [x] GET /settings -- load merged tree + MCP policy
- [x] POST /settings -- save user.toml (batch settings + guest env + MCP config mutations)
- [x] GET /settings/presets -- list presets (security_presets())
- [x] POST /settings/presets/{id} -- apply preset
- [x] POST /settings/lint -- validate config (load_merged_lint())
- [x] Extend batch_update_settings() to handle [mcp] section of SettingsFile
- [ ] Extend /info/{id} to include state machine history
- [x] Wire api.ts settings functions
- [x] Verify settings view loads/saves real config

## T2.5: Better stats wiring (from better_stats sprint -- backend already landed)
- [x] Update `SandboxInfo` in `types/gateway.ts` -- add 12 telemetry fields (created_at, uptime_secs, tokens, cost, tool_calls, requests, etc.)
- [x] Add `StatsResponse` + sub-types to `types/gateway.ts` (GlobalStats, SessionRecord, ProviderSummary, ToolSummary, McpToolSummary)
- [x] Add `getStats()` to `api.ts` -- `GET /stats` (cross-session dashboard data from main.db)
- [x] Delete dead `SessionInfo` type from `types.ts` (superseded by enriched SandboxInfo)
- [x] Wire NewTabPage (dashboard) to show live VM stats from enriched SandboxInfo (uptime, tokens, cost)
- [x] Wire dashboard global stats from `GET /stats` (total cost, total tokens, session count)
- [x] Wire `/inspect/_main` -- queryDbMain() now works for custom cross-session queries

## T2.8: VM creation UI + tray integration

Folds the tray-ui-integration sprint (STU) into the tauri-shell pipeline. The tray is fully functional (S12 complete) and already provisions VMs + launches the UI via `open -a Capsem [--args ...]`. This sprint makes the frontend handle those launches and adds a "New Sandbox" flow directly in the dashboard.

### Frontend: Create Sandbox dialog
- [x] `CreateSandboxDialog.svelte` -- modal dialog for creating VMs
  - Mode toggle: **Quick** (one-click temp VM) vs **Named** (persistent with name)
  - Quick mode: just a "Create" button, uses defaults (2 GB RAM, 2 CPUs, ephemeral)
  - Named mode: name field (focused, validated), RAM select (1/2/4/8 GB), CPU select (1/2/4/8)
  - Optional: fork-from dropdown (list existing persistent VMs from `vmStore.vms`)
  - Submit calls `vmStore.provision(opts)` -> `tabStore.openVM(id, name)` -> terminal opens
  - Cancel closes dialog, returns to dashboard
  - Preline styling: match existing SettingsPage card/modal patterns
  - Loading state while provisioning (disable submit, show spinner)
  - Error state if provision fails (inline message, not toast)

### Frontend: Dashboard "New Sandbox" button
- [x] Add "New Sandbox" button to `NewTabPage.svelte` -- top-right, next to heading
- [x] Empty state: when no VMs exist, show centered card with "No sandboxes yet" + large "Create Sandbox" button
- [x] After creation: new VM appears in table immediately (vmStore.refresh), terminal tab opens

### Frontend: Toolbar integration
- [x] Add "New Sandbox" to toolbar dropdown menu (Toolbar.svelte, next to "Service Logs")
- [x] Add quick "New Temporary" button in toolbar (always visible, one-click provision)

### Tauri: CLI argument handling (from STU1/STU2/STU3)
- [x] Parse `--connect <vm_id>` in capsem-app main.rs
- [x] Parse `--action` (save/fork/new-named) in capsem-app main.rs
- [x] Dispatch via `window.__capsemDeepLink` JS hook; Toolbar listens for `capsem:tab-action`
- [x] If no args: show dashboard (existing behavior)

### Tauri: Single-instance handling (from STU4)
- [x] `tauri-plugin-single-instance` wired in capsem-app Cargo.toml + main.rs
- [x] Second launch forwards args to running instance via direct binary path (macOS `open -a` drops args; tray invokes `/Applications/Capsem.app/Contents/MacOS/capsem-ui` directly)

### Frontend: URL-based deep linking
- [x] Deep link dispatched from Rust via `window.__capsemDeepLink({ connect, action })` JS hook on app load + on second-instance wake-up

### Tests
- [ ] Unit test: CreateSandboxDialog renders, submits provision, handles errors
- [ ] Unit test: URL param parsing in App.svelte
- [ ] Unit test: toolbar menu items trigger dialog
- [ ] `pnpm run check` -- 0 errors
- [ ] `pnpm run test` -- all pass
- [ ] `cargo check -p capsem-ui` -- Tauri app compiles
- [ ] Chrome DevTools MCP: screenshot of dialog (quick + named modes)
- [ ] Chrome DevTools MCP: screenshot of empty state
- [ ] Chrome DevTools MCP: screenshot of toolbar with new buttons

### Acceptance criteria
- [ ] Dashboard has a visible "New Sandbox" button
- [ ] One-click temp VM: click -> provision -> terminal opens in new tab
- [ ] Named VM: dialog with name/RAM/CPU -> provision -> terminal opens
- [ ] Empty state shows "No sandboxes" + create button
- [ ] `open -a Capsem --args --connect <id>` opens focused on that VM
- [ ] `open -a Capsem --args --new-named` opens create dialog
- [ ] Second launch reuses existing window (single instance)
- [ ] No hardcoded colors, Preline only, matches existing UI vibe

## T3: Service endpoints -- MCP runtime
- [ ] GET /mcp/servers -- runtime server list with tool counts
- [ ] GET /mcp/tools -- tool catalog with cache/approval status
- [ ] POST /mcp/tools/refresh -- re-discover tools
- [ ] POST /mcp/tools/{name}/approve -- approve tool (write tool cache)
- [ ] POST /mcp/tools/{name}/call -- call built-in file tool
- [ ] Wire api.ts MCP runtime functions
- [ ] Verify MCP view works

## T4: Real-time VM events (WebSocket)
- [ ] GET /events WebSocket endpoint on gateway
- [ ] Emit vm-state-changed events
- [ ] Emit download-progress events
- [ ] Wire api.ts onVmStateChanged, onDownloadProgress
- [ ] Verify real-time updates in frontend

## T5: Integration
- [ ] `cargo check` (full workspace) passes
- [ ] `pnpm run check` -- 0 type errors
- [ ] `pnpm run test` -- all pass
- [ ] `pnpm run build` -- dist/ clean
- [ ] `just install` succeeds
- [ ] Launch capsem-ui -- window opens with frontend
- [ ] Dashboard lists VMs with live stats (tokens, cost, uptime)
- [ ] Settings view works (including MCP config)
- [ ] MCP view works
- [ ] VM events push in real time
- [ ] Old capsem-app fully deleted
- [ ] Changelog
- [ ] Commit

## Notes
- T0 is fixes and wiring, not a rewrite. Existing code works -- add to it.
- T1 is a fresh crate, old crate deleted.
- MCP config is a [mcp] section in user.toml -- mutations go through POST /settings (T2), not separate /mcp/* routes.
- T3 is runtime-only (5 endpoints): server list, tool catalog, refresh, approve, call.
- WebSocket for events (T4) -- reuse existing gateway WebSocket infrastructure, not SSE.
- Gateway catch-all proxy forwards new service endpoints automatically.
- Out of scope: host detection (installer sprint), log search (search sprint).
- Entry point switched from old single-VM App.svelte to shell/App.svelte (tabbed multi-VM dashboard).
- T2.5 folds better_stats backend (already landed on next-gen) into the frontend. GET /stats returns full main.db dump. SandboxInfo now has 12 optional telemetry fields. /inspect/_main works.
- T2.8 folds the tray-ui-integration sprint (STU) into this pipeline. The tray (S12) is fully functional and launches the UI via `open -a Capsem [--args ...]`. T2.8 makes the frontend handle those args + adds a "New Sandbox" flow in the dashboard. VM defaults: 2 GB RAM, 2 CPUs (matching service defaults). Deep linking via URL query params -- no new IPC commands needed.
