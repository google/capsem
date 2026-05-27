# Sprint Plan: CLI API Hardening (Phase 2)

## Goal
Enforce a strict, no-backward-compat surface for the initial release cleanup:
- `capsem create` with no name => temporary VM/session
- `capsem create <name>` => named persistent VM/session
- remove `-n` naming flag from `shell` and `create`
- remove compatibility aliases (`attach`, `ls`, `rm`, `--image`)
- collapse file read/write API onto canonical `/files/{id}/content`

## Scope (this slice)
- Update clap command definitions in `crates/capsem/src/main.rs`
- Remove CLI compatibility aliases and add reject tests
- Remove legacy `/read_file/{id}` and `/write_file/{id}` service routes
- Migrate frontend file calls to `/files/{id}/content`
- Migrate MCP file calls to `/files/{id}/content`
- Refresh CLI/API docs to match strict surface

## Non-goals (next slice)
- Full taxonomy rename (`sandbox` -> `vm`/`session`) across all internals and DB schema
- Router naming normalization beyond this file I/O and CLI alias pass

## Files
- `crates/capsem/src/main.rs`
- `crates/capsem-service/src/main.rs`
- `crates/capsem-service/src/api.rs`
- `crates/capsem-service/src/tests.rs`
- `crates/capsem-mcp/src/main.rs`
- `crates/capsem-mcp/src/tests.rs`
- `frontend/src/lib/api.ts`
- `frontend/src/lib/types/gateway.ts`
- `frontend/src/lib/__tests__/api.test.ts`
- `docs/src/content/docs/usage/cli.md`
- `docs/src/content/docs/getting-started.md`
- `docs/src/content/docs/architecture/service-architecture.md`
- `docs/src/content/docs/architecture/mcp-gateway.md`
- `sprints/cli-api-hardening/plan.md`
- `sprints/cli-api-hardening/tracker.md`

## Done Criteria
- `create` accepts positional optional name
- `shell` accepts positional optional target only
- no `-n` parser path for `create` or `shell`
- no compatibility aliases for `attach`, `ls`, `rm`, `--image`
- service exposes only canonical file endpoints (`/files/{id}/content`)
- frontend and MCP file operations use canonical file endpoints
- docs reflect strict CLI/API surface

## Testing Proof Matrix
- Unit/contract: `cargo test -p capsem parse_`; `cargo test -p capsem-service`; `cargo test -p capsem-mcp`
- Functional: CLI parse + service/mcp unit suites
- Adversarial: alias rejection tests and path sanitization tests
- E2E/VM: deferred
- Telemetry: no behavior change expected in telemetry paths
- Performance: not applicable
