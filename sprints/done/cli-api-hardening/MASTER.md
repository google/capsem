# Sprint Master: cli-api-hardening

## Status
| Item | Status |
|---|---|
| Planning (`plan.md`) | Complete |
| Tracking (`tracker.md`) | In sync |
| Implementation | Complete for Phase 2 scope |
| Verification | Rust suites complete; frontend suite blocked locally |

## Scope Snapshot
- Strict CLI surface (no `-n`, no alias compatibility commands)
- Canonical file API only (`/files/{id}/content`)
- Frontend + MCP migrated to canonical file API
- CLI/API documentation updated to match

## Verification Commands
- `cargo test -p capsem parse_ -- --nocapture`
- `cargo test -p capsem-service -- --nocapture`
- `cargo test -p capsem-mcp -- --nocapture`
- `npm --prefix frontend test -- src/lib/__tests__/api.test.ts` (blocked: `vitest` not installed locally)

## Open Holds
- Frontend test run pending dependency install.
- Some VM-backed Python tests are environment-sensitive (exec-ready boot dependency).
