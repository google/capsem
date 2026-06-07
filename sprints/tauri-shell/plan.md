# Sprint: Tauri Shell Wiring

## Goal

Ship a functional desktop app. Build a new capsem-app from scratch as a thin Tauri webview shell that delegates to capsem-gateway. Add the missing service endpoints so the frontend is fully wired. Delete the old capsem-app.

End state: `just install` produces a working desktop app on macOS and Linux.

## What "done" looks like

1. `just install` works on macOS and Linux
2. Launching capsem-ui opens a native window showing the frontend
3. Frontend connects to gateway, all views are wired to real data
4. Settings view loads/saves real config (including MCP config)
5. MCP management view manages real MCP servers
6. VM state changes push to the frontend in real time
7. App has proper icons from graphics/tauri/
8. `cargo check` and `pnpm run check` pass clean
9. Old capsem-app deleted

## Sub-sprints

### T0: Frontend fixes and wiring

Fix broken imports and wire stores to real gateway endpoints. Not a rewrite -- evolve what exists.

**Fix `isMock` build blocker:**
- `mock.ts` does not export `isMock` but `db.ts` (line 2) and `Terminal.svelte` (line 5) import it
- Export `isMock` from mock.ts with proper detection (connected vs disconnected)

**api.ts** -- add new endpoints (existing VM endpoints stay):
- Settings: `getSettings()`, `saveSettings()`, `getPresets()`, `applyPreset()`, `lintConfig()`
- MCP runtime: `getMcpServers()`, `getMcpTools()`, `refreshMcpTools()`, `approveMcpTool()`, `callMcpTool()`
- Real-time: `onVmStateChanged(cb)`, `onDownloadProgress(cb)` via WebSocket `/events`
- Terminal WebSocket URL helper (already exists)

**mock.ts** -- fix `isMock` export, clean up detection pattern

**db.ts** -- fix `isMock` import

**Stores** -- wire to real endpoints:
- `vm.svelte.ts` -- wired to real WebSocket events
- `mcp.svelte.ts` -- wired to real MCP runtime endpoints
- `wizard.svelte.ts` -- remove `detectHostConfig` (installer sprint), wire `saveSettings`
- Delete `logs.svelte.ts` (dead code, VM logs use `/logs/{id}`, search sprint handles log search)

**Already done (no work needed):**
- `shiki.ts` -- properly typed, no issues
- `terminal/rate-limiter.ts` -- fully implemented (54 lines)
- `components/shell/TabBar.svelte` -- fully implemented (91 lines)

**Tests for everything changed.**

### T1: New capsem-app (from scratch)

Build `crates/capsem-app2/` as a clean Tauri webview shell. Delete the old `crates/capsem-app/` when done.

**Architecture:** webview window -> `http://localhost:19222` (gateway). Bundled `frontend/dist` as offline fallback.

**Cargo.toml:**
- `tauri` with `custom-protocol`
- `tauri-plugin-updater`, `tauri-plugin-dialog`, `tauri-plugin-opener`
- `anyhow`, `serde`, `serde_json`, `tracing`
- Nothing else. No capsem-core, no tokio, no objc2.

**main.rs:**
- Minimal tracing setup
- Tauri builder with updater + dialog + opener
- Setup hook: check gateway reachability
- 2 IPC commands: `open_url`, `check_for_app_update`

**tauri.conf.json:**
- `frontendDist: "../../frontend/dist"`
- `devUrl: "http://localhost:5173"`
- Icons from `graphics/tauri/` (replacing old icons in crates/capsem-app/icons/)
- Updater config
- No VM resource bundling

**build.rs:** just `tauri_build::build()`

**Then:** delete `crates/capsem-app/`, rename `capsem-app2` -> `capsem-app`, update workspace Cargo.toml.

### T2: Service endpoints -- settings (including MCP config)

Add settings management to capsem-service, wire to frontend api.ts. MCP config is a `[mcp]` section in the same `user.toml` -- all MCP config mutations go through the settings API.

**Service endpoints:**
- `GET /settings` -- load merged settings tree (user + corp + defaults), includes MCP policy
- `POST /settings` -- save settings (writes user.toml), handles: batch setting updates, guest env, AND MCP config mutations (global policy, default permission, tool permissions, server enable/disable, add/remove server)
- `GET /settings/presets` -- list security presets
- `POST /settings/presets/{id}` -- apply a preset
- `POST /settings/lint` -- validate config, return issues

**Implementation:** capsem-core already has the functions (note actual names):
- `policy_config::load_merged_settings()`, `load_settings_tree()`, `load_settings_response()`
- `policy_config::security_presets()` (not `list_presets`)
- `policy_config::apply_preset()`
- `policy_config::load_merged_lint()` (not `lint_config`)
- `policy_config::batch_update_settings()` -- extend to handle `[mcp]` section of `SettingsFile`
- `policy_config::save_mcp_user_config()` for MCP mutations

### T3: Service endpoints -- MCP runtime

Add MCP runtime operations to capsem-service. These are NOT config (that's T2) -- these are runtime actions.

**Service endpoints:**
- `GET /mcp/servers` -- list configured MCP servers (auto-detected + corp + user, with tool counts)
- `GET /mcp/tools` -- list discovered tools with cache/approval status
- `POST /mcp/tools/refresh` -- re-discover tools from servers
- `POST /mcp/tools/{name}/approve` -- approve a tool (writes tool cache)
- `POST /mcp/tools/{name}/call` -- call a built-in file tool (snapshots etc.)

**Implementation:** capsem-core `McpServerManager::definitions()`, `tool_catalog()`, `initialize_all()`. Tool cache via `mcp::load_tool_cache()`, `save_tool_cache()`. Built-in file tools via `file_tools` module.

### T4: Real-time VM events

Add WebSocket endpoint to capsem-gateway for push-based state updates.

**Gateway:**
- `GET /events` -- WebSocket endpoint emitting `vm-state-changed` and `download-progress` events
- Gateway subscribes to service state changes and forwards as WebSocket messages

**Why WebSocket:** gateway already has WebSocket infrastructure for terminal. Reuse the same transport rather than adding SSE as a second real-time mechanism.

### T5: Integration

- `cargo check` (full workspace) passes
- `pnpm run check` -- 0 type errors
- `pnpm run test` -- all pass
- `pnpm run build` -- dist/ clean
- `just install` succeeds on macOS
- Launch capsem-ui -- window opens, frontend loads, connects to gateway
- Settings view works with real config (including MCP config)
- MCP view works with real servers
- VM events push in real time
- Old capsem-app deleted
- Changelog + commit

## Not in scope

- Host detection / API key validation -- installer sprint
- Log search unification -- search sprint (SQLite)
- Structured app log viewer -- dead code, deleted

## Architecture notes

- Gateway catch-all proxy (`fallback(handle_proxy)`) forwards any unmatched route to capsem-service over UDS. New service endpoints in T2/T3 are automatically accessible through the gateway.
- The current capsem-app has 39 Tauri IPC commands, all calling capsem-core directly. 17 are already covered by existing gateway/service endpoints. 10 new endpoints (T2 + T3) cover 22 more. 5 are deferred to other sprints.
- `/info/{id}` should be extended to include state machine history (transitions, durations) -- currently only returns static metadata.
