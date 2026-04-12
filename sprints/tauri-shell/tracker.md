# Sprint: Tauri Shell Wiring

## T0: Frontend fixes and wiring
- [ ] Fix `isMock` export in `mock.ts` -- build blocker (db.ts and Terminal.svelte import it)
- [ ] Fix `isMock` import in `db.ts`
- [ ] Add settings endpoints to `api.ts` (getSettings, saveSettings, getPresets, applyPreset, lintConfig)
- [ ] Add MCP runtime endpoints to `api.ts` (getMcpServers, getMcpTools, refreshMcpTools, approveMcpTool, callMcpTool)
- [ ] Add WebSocket `/events` to `api.ts` (onVmStateChanged, onDownloadProgress)
- [ ] Wire `vm.svelte.ts` to real WebSocket events
- [ ] Wire `mcp.svelte.ts` to real MCP runtime endpoints
- [ ] Wire `wizard.svelte.ts` -- remove detectHostConfig, wire saveSettings
- [ ] Delete `logs.svelte.ts` -- dead code
- [ ] Tests for api.ts changes
- [ ] Tests for mock.ts / db.ts fixes
- [ ] Tests for stores
- [ ] `pnpm run check` -- 0 type errors
- [ ] `pnpm run test` -- all pass

## T1: New capsem-app (from scratch)
- [ ] Create `crates/capsem-app2/` -- clean Tauri shell
- [ ] main.rs -- thin webview shell, 2 IPC commands (open_url, check_for_app_update)
- [ ] Cargo.toml -- minimal deps (tauri, plugins, anyhow, serde, tracing)
- [ ] tauri.conf.json -- gateway-first, bundled fallback
- [ ] build.rs -- just tauri_build
- [ ] Icons from graphics/tauri/
- [ ] capabilities/ -- window + updater + opener only
- [ ] `cargo check -p capsem-ui` passes
- [ ] Delete old `crates/capsem-app/`
- [ ] Rename capsem-app2 -> capsem-app
- [ ] Update workspace Cargo.toml

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
