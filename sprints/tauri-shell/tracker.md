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
- [ ] GET /settings -- load merged tree + MCP policy
- [ ] POST /settings -- save user.toml (batch settings + guest env + MCP config mutations)
- [ ] GET /settings/presets -- list presets (security_presets())
- [ ] POST /settings/presets/{id} -- apply preset
- [ ] POST /settings/lint -- validate config (load_merged_lint())
- [ ] Extend batch_update_settings() to handle [mcp] section of SettingsFile
- [ ] Extend /info/{id} to include state machine history
- [ ] Wire api.ts settings functions
- [ ] Verify settings view loads/saves real config

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
- [ ] Settings view works (including MCP config)
- [ ] MCP view works
- [ ] VM events push in real time
- [ ] Old capsem-app fully deleted
- [ ] Changelog
- [ ] Commit

## Notes
- T0 is fixes and wiring, not a rewrite. Existing code works -- add to it.
- T1 is a fresh crate (capsem-app2), old crate deleted after.
- MCP config is a [mcp] section in user.toml -- mutations go through POST /settings (T2), not separate /mcp/* routes.
- T3 is runtime-only (5 endpoints): server list, tool catalog, refresh, approve, call.
- WebSocket for events (T4) -- reuse existing gateway WebSocket infrastructure, not SSE.
- Gateway catch-all proxy forwards new service endpoints automatically.
- Out of scope: host detection (installer sprint), log search (search sprint).
- Entry point switched from old single-VM App.svelte to shell/App.svelte (tabbed multi-VM dashboard).
