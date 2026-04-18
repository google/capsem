# Sprint: MCP + Service Endpoint Coverage -- tracker

See `plan.md` for context and exit criteria.

## Tasks

### T0: Audit -- produce coverage matrix
- [ ] List every `#[tool(name = "...")]` in `crates/capsem-mcp/src/main.rs`
- [ ] List every HTTP handler registered in `crates/capsem-service/` (axum routes)
- [ ] For each, find the test(s) that invoke it end-to-end -- write to `coverage-matrix.md`
- [ ] Mark rows with zero real coverage as blind spots

### T1: Fill MCP tool blind spots
- [ ] One test per blind spot in `tests/capsem-mcp/`, invoked via the real `capsem-mcp` binary
- [ ] Assertions must check behavior, not just absence of `isError`

### T2: Fill service endpoint blind spots
- [ ] One test per blind spot in the appropriate `tests/capsem-e2e/`, `tests/capsem-lifecycle/`, or `tests/capsem-service/` module
- [ ] Prefer existing fixtures; do not spin up a second service harness

### T3: Gateway layering decision
- [ ] Decide: (a) new `tests/capsem-gateway-e2e/` suite against real service, or (b) document the layering and leave gateway mocked
- [ ] Implement the chosen option
- [ ] If (a): at minimum one smoke test per gateway-proxied route that hits a real VM

### T4: Testing gate
- [ ] `just test` -- all green
- [ ] `just run "capsem-doctor"` -- VM smoke passes
- [ ] Coverage matrix shows zero blind spots

### T5: Changelog + commit
- [ ] `CHANGELOG.md` entry under `## [Unreleased]` -- test-only, group under Changed or new Tests section
- [ ] Commit(s) per milestone: T0 matrix, T1 MCP fills, T2 service fills, T3 gateway

## Notes

- 2026-04-18: Sprint drafted during the next-gen -> main merge push. Deferred out of the merge window -- do after main lands.
- Resume doc "Known drift flagged but NOT addressed" items (capsem_stop, capsem restart, capsem history, capsem_service_logs) are out of scope; they are drift cleanup, not coverage gaps.

## Discoveries

_(fill in as work progresses)_
