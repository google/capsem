# Sprint 05: Gateway Wiring

Replace all mock data with real capsem-gateway HTTP/WebSocket calls.

Worktree: `worktrees/capsem-ui` (branch: `frontend-ui`)
Depends on: Sprints 01-04

## Acceptance Criteria

### API Client
- [ ] `api.ts` — HTTP client to `http://127.0.0.1:19222` with Bearer auth
- [ ] Token loaded from config (not hardcoded)
- [ ] Bearer token safety: memory only, never in localStorage/DOM/logs/URLs
- [ ] Mock fallback: detect gateway unreachable, switch to mock.ts data
- [ ] Error handling: connection refused, auth failure, timeout

### Terminal WebSocket
- [ ] WebSocket to `ws://127.0.0.1:19222/terminal/{id}`
- [ ] VM ID validation before connecting (alphanumeric + hyphens only)
- [ ] Bidirectional data: xterm input → WS, WS → xterm output
- [ ] Resize events sent as text frames
- [ ] Reconnect on disconnect with backoff

### Status Polling
- [ ] `GET /status` polled every 1s for VM status
- [ ] Tab status dots update in real-time (running/stopped/error)
- [ ] VM list on new-tab page reflects live state

### VM Lifecycle
- [ ] Create VM from new-tab page
- [ ] Stop/delete/fork/resume from toolbar and overview
- [ ] Optimistic UI updates with rollback on error

### Inspector Wiring
- [ ] `db.ts` — SQL queries via `POST /inspect/{id}`
- [ ] Results flow into InspectorView table

### Settings Wiring
- [ ] Settings save via `POST /reload-config` and settings API
- [ ] Settings load from gateway on startup

### Cleanup
- [ ] Zero `@tauri-apps/api` imports remaining
- [ ] No Tauri-specific code paths

## Testing Gate

- [ ] Real data flows with `just gateway` + `just service` running
- [ ] Mock mode works standalone (`just ui` with no gateway)
- [ ] Terminal streams real shell I/O
- [ ] VM create/stop/delete works from UI
- [ ] Status dots update within 2s of VM state change
- [ ] Inspector queries return real session DB data
- [ ] `grep -r "tauri" frontend/src/` returns zero results
- [ ] `pnpm run check` passes
