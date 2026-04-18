# Sprint: MCP + Service Endpoint Coverage

Prove that every MCP tool and every capsem-service HTTP endpoint is actually exercised end-to-end, not just declared or smoke-tested. Close the gap between "the test passes" and "the endpoint does the right thing under real inputs."

## Why now

Auditing what `just test` covers surfaced three asymmetries:

1. **`test_cli_parity.py`** is a static declaration check -- it fails on drift in the source (CLI subcommand vs `#[tool]` attribute) but says nothing about whether a tool actually works when invoked. It would pass on a `capsem_create` that always returned "not implemented."
2. **`tests/capsem-mcp/`** is real end-to-end: `McpSession` (conftest.py:37) talks to the real `capsem-mcp` binary over stdio, which talks to a real `capsem-service`, which spawns real VMs. Coverage here is real.
3. **`tests/capsem-gateway/`** is **mocked**: `conftest.py` defines `MockServiceHandler` with hardcoded `MOCK_VMS`. Gateway tests verify the gateway's routing / auth / CORS / lifecycle layer -- they do NOT prove that the service endpoints behind the gateway actually work. A gateway test can pass while the underlying `/provision` is broken.

The real service endpoint surface (e.g. `/provision`, `/suspend`, `/resume`, `/info`, `/fork`, `/persist`, `/purge`, `/delete`, `/list`) is exercised by `tests/capsem-e2e/`, `tests/capsem-lifecycle/`, and `tests/capsem-service/`. But no one has audited whether every endpoint is covered. Blind spots here are how "it merges green but production breaks" happens.

## Exit criteria

- [ ] Every MCP tool in `MCP_TO_CLI` that maps to a CLI (not `(None, reason)`) has at least one test in `tests/capsem-mcp/` that invokes it via the real MCP binary and asserts behavior (not just `isError: false`).
- [ ] Every capsem-service HTTP endpoint has at least one test in `tests/capsem-e2e/` or `tests/capsem-lifecycle/` or `tests/capsem-service/` that invokes it via the real UDS and asserts behavior.
- [ ] Gateway test suite either (a) adds a parallel "against-real-service" layer, or (b) is explicitly documented as "gateway-layer only; correctness of downstream endpoints is covered elsewhere" -- pick one and implement.

## Non-goals

- Performance / fuzz / stress coverage. Those are separate sprints.
- Rewriting the MCP or gateway implementation. This sprint is test-surface only.

## Key decisions

- **MCP tests stay stdio-over-real-binary.** Already correct; do not regress by mocking.
- **Do not replace gateway mocks blindly.** The gateway layer has legitimate unit-test value for routing/auth logic. But we need one separate "gateway + real service" smoke suite so regressions in that seam are caught.
- **Every blind spot becomes a failing test first, then green.** Per `/dev-testing` -- TDD. No "we added coverage" without a test that existed broken.

## Out of scope for now (follow-up sprints)

- `capsem_stop` cleanup (flagged in `test_cli_parity.py` as MCP-only drift).
- `capsem restart` + `capsem history` -- no MCP tool yet.
- `capsem service logs` -- CLI analog for `capsem_service_logs` tool.

These are drift candidates, not coverage gaps per se.

## Files to create / touch

- `sprints/mcp-endpoint-coverage/tracker.md` -- progress
- `sprints/mcp-endpoint-coverage/coverage-matrix.md` -- table of MCP tool + service endpoint -> test file (blind spots fall out of this)
- New tests under `tests/capsem-mcp/` and `tests/capsem-e2e/` (or `tests/capsem-lifecycle/`) for any gaps identified in the matrix
- Possibly `tests/capsem-gateway-e2e/` for a new "gateway over real service" suite, if we pick option (a)

## Dependencies / ordering

This sprint is **blocked on** landing next-gen to main. Runs on `main` afterward -- no merge pressure, can take the time to audit and backfill properly.
