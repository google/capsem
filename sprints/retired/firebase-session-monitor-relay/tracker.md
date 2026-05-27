# Sprint: Firebase Session Monitor Relay

## Tasks

- [x] Draft sprint plan and tracker
- [ ] Confirm Firebase project setup requirements
- [ ] Add `capsem-remote` crate skeleton
- [ ] Add remote configuration settings
- [ ] Add service-managed companion spawning
- [ ] Add local remote auth/status endpoints
- [ ] Implement Firebase credential storage and refresh
- [ ] Implement Firebase RTDB client abstraction
- [ ] Implement HTTP relay allowlist validation
- [ ] Implement gateway HTTP forwarding and response writing
- [ ] Implement terminal attach/detach control handling
- [ ] Implement local gateway terminal WebSocket bridge
- [ ] Implement terminal input/output Firebase chunk relay
- [ ] Add Firebase security rules and emulator tests
- [ ] Add remote API transport in frontend
- [ ] Add paired device picker
- [ ] Add existing session list as first remote screen
- [ ] Add session chat/transcript view
- [ ] Add raw xterm fallback drawer
- [ ] Add mobile visual verification screenshots
- [ ] Add adversarial leak tests
- [ ] Run full testing gate
- [ ] Update `CHANGELOG.md`
- [ ] Commit functional milestones

## Notes

- Product priority: monitor existing sessions and talk to the agent already
  running in a session.
- The talk path is terminal-backed PTY input/output, not `/exec`.
- Mobile default is a new chat/transcript view. xterm is raw fallback only.
- Firebase is a conduit. It should not become a Capsem domain database.
- HTTP relay rejects non-allowlisted methods, paths, or headers. It does not
  silently strip and forward modified requests.
- `capsem-app` should remain thin. Login should be orchestrated through
  gateway/service endpoints plus existing `open_url`, not VM logic in Tauri.

## Coverage Ledger

- Unit/contract:
  - Pending: Rust allowlist validation tests.
  - Pending: relay wire-shape parser/validator tests.
  - Pending: terminal input/output chunk tests.
  - Pending: frontend remote transport/store tests.
- Functional:
  - Pending: mock Firebase request -> mock gateway -> Firebase response.
  - Pending: mock terminal WebSocket -> Firebase output chunks.
  - Pending: Firebase input chunk -> local terminal WebSocket input.
- Adversarial:
  - Pending: cross-user Firebase rules denial.
  - Pending: malformed RTDB path denial.
  - Pending: disallowed HTTP method/path/header rejection.
  - Pending: oversized body/chunk rejection.
  - Pending: stale attach cleanup.
  - Pending: token/local URL/UDS leak scan.
- E2E/VM:
  - Pending: real VM session attach through Firebase relay.
  - Pending: remote composer sends text to an already-running shell/agent.
  - Pending: raw terminal fallback renders same session.
- Telemetry:
  - Pending: bridge lifecycle logs.
  - Pending: relay accepted/rejected counters without secret payloads.
  - Pending: terminal attach/detach logs without PTY byte content.
- Performance:
  - Pending: relay HTTP round-trip budget.
  - Pending: terminal output burst handling.
  - Pending: mobile render responsiveness with sustained output.
- Missing/deferred:
  - New session creation from remote UI is lower priority.
  - Team/shared-device access is deferred.
  - Generic WebSocket proxying is deferred.
  - Full semantic agent-turn extraction is deferred; v1 transcript is terminal
    backed and best-effort.

## Active Decisions

- [x] Use Firebase Realtime Database first.
- [x] Use own-account paired devices only for v1.
- [x] Use chat/transcript as mobile default.
- [x] Use xterm only as raw fallback for remote.
- [x] Use terminal PTY relay, not `/exec`, for talking to running agents.
- [x] Reject non-allowlisted relay envelopes.

## Commands / Gates

```bash
cargo test -p capsem-remote
cargo test -p capsem-service remote
cd frontend && pnpm run check && pnpm run test
just test
just run "capsem-doctor"
```
