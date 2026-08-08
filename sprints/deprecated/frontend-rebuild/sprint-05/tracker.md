# Sprint 05: Gateway Wiring

Replace all mock data with real capsem-gateway HTTP/WebSocket calls.

Worktree: `worktrees/capsem-ui` (branch: `frontend-ui`)
Depends on: Sprints 01-04

## Acceptance Criteria

### API Client
- [x] `api.ts` — HTTP client to `http://127.0.0.1:19222` with Bearer auth
- [x] Token loaded from `GET /token` endpoint (hardcoded 127.0.0.1 IP check)
- [x] Bearer token safety: memory only, never in localStorage/DOM/logs/URLs
- [x] Mock fallback: detect gateway unreachable, switch to empty state
- [x] Error handling: connection refused, auth failure, timeout

### Terminal WebSocket
- [x] WebSocket to `ws://127.0.0.1:19222/terminal/{id}?token=`
- [x] VM ID validation before connecting (alphanumeric + hyphens only)
- [x] Bidirectional data: xterm input -> WS, WS -> xterm output
- [x] Resize events sent as JSON text frames
- [x] Reconnect on disconnect with backoff

### Status Polling
- [x] `GET /status` polled every 2s for VM status
- [x] Connection dot in toolbar (green=connected, gray=offline)
- [x] VM list on new-tab page reflects live state

### VM Lifecycle
- [x] Create VM from new-tab page
- [x] Stop/delete/fork from toolbar and overview
- [x] Resume from new-tab page action buttons
- [x] Restart (stop + resume) from toolbar

### Inspector Wiring
- [x] SQL queries via `POST /inspect/{id}` in api.ts
- [x] Results flow into InspectorView table

### Settings Wiring
- [x] Settings load via `GET /settings` on SettingsPage mount
- [x] Settings save via `POST /settings` with pending changes
- [x] Loading spinner and error banner with retry

### MCP Wiring
- [x] MCP servers loaded from gateway via mcpStore
- [x] Add/remove/toggle servers persists via settings API
- [x] Policy dropdown wired to `setDefaultPermission()`
- [x] Built-in tools and policy loaded on mount

### Cleanup
- [x] Zero `@tauri-apps/api` imports remaining
- [x] No Tauri-specific code paths
- [x] Dead mock-settings.generated.ts deleted
- [x] No runtime mock data outside test fixtures

## Testing Gate

- [ ] Real data flows with `just gateway` + `just service` running
- [ ] Mock mode works standalone (`just ui` with no gateway)
- [ ] Terminal streams real shell I/O
- [ ] VM create/stop/delete works from UI
- [ ] Status dots update within 2s of VM state change
- [ ] Inspector queries return real session DB data
- [x] `grep -r "tauri" frontend/src/` returns zero results
- [x] `pnpm run check` passes
- [x] `pnpm test` passes (373 tests)
- [ ] `cargo test -p capsem-gateway` passes
