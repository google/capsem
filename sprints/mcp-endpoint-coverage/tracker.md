# Sprint: MCP + Service Endpoint Coverage -- tracker

See `plan.md` for context and exit criteria.

## Tasks

### T0: Audit -- produce coverage matrix
- [x] List every `#[tool(name = "...")]` in `crates/capsem-mcp/src/main.rs`
- [x] List every HTTP handler registered in `crates/capsem-service/` (axum routes)
- [x] For each, find the test(s) that invoke it end-to-end -- write to `coverage-matrix.md`
- [x] Mark rows with zero real coverage as blind spots

### T1: Fill MCP tool blind spots
- [x] `capsem_version`, `capsem_vm_logs`, `capsem_mcp_servers`, `capsem_mcp_tools` -- `tests/capsem-mcp/test_meta.py`
- [x] `capsem_suspend` (happy path + ephemeral rejection), `capsem_persist`, `capsem_purge` -- `tests/capsem-mcp/test_state_transitions.py`. Suspend round-trip now passes after the IPC fix landed (see discovery below); the earlier xfail marker is removed.
- [x] `capsem_run` -- `tests/capsem-mcp/test_run.py`
- [x] `capsem_service_logs` -- `tests/capsem-mcp/test_service_logs.py`
- [x] `capsem_mcp_call` -- `tests/capsem-mcp/test_mcp_call.py` (error paths; full happy path needs a downstream MCP server in the fixture, tracked as follow-up)

### T2: Fill service endpoint blind spots
- [x] `/version`, `/stats`, `/service-logs`, `/reload-config` -- `tests/capsem-service/test_svc_core.py`
- [x] `/history/{id}`, `/history/{id}/processes`, `/history/{id}/counts`, `/history/{id}/transcript` -- `tests/capsem-service/test_svc_history.py`
- [x] `/files/{id}`, `/files/{id}/content` (GET + POST) -- `tests/capsem-service/test_svc_files.py`
- [x] `/fork/{id}` -- `tests/capsem-service/test_svc_fork.py`
- [x] `/settings`, `/settings/presets`, `/settings/presets/{id}`, `/settings/lint`, `/settings/validate-key` -- `tests/capsem-service/test_svc_settings.py`
- [x] `/setup/state`, `/setup/detect`, `/setup/complete`, `/setup/assets`, `/setup/corp-config` -- `tests/capsem-service/test_svc_setup.py`
- [x] `/mcp/servers`, `/mcp/tools`, `/mcp/policy`, `/mcp/tools/refresh`, `/mcp/tools/{name}/approve`, `/mcp/tools/{name}/call` -- `tests/capsem-service/test_svc_mcp_api.py`. `/mcp/tools/{name}/call` happy path against a downstream aggregator remains a follow-up (same gap as `test_mcp_call.py`)

### T3: Gateway layering decision
- [x] Decide: picked (b) document the layering. See "Gateway layering" below.
- [x] Implement: added scope-setting docstring to `tests/capsem-gateway/conftest.py` pointing to `tests/capsem-service/`, `tests/capsem-mcp/`, and `tests/capsem-e2e/` for downstream correctness
- [x] (a) not pursued -- reason in "Gateway layering"

### T4: Testing gate
- [x] `uv run pytest tests/capsem-mcp/ tests/capsem-service/ tests/capsem-lifecycle/` -- 2026-04-21: 192 passed, 4 skipped (after HOME isolation + settings/setup/mcp-api suites landed)
- [ ] `just test` -- full suite not yet re-run
- [ ] `just run "capsem-doctor"` -- VM smoke not yet re-run
- [x] Coverage matrix shows zero blind spots for the endpoints this sprint targets. Remaining partial entries: `/mcp/tools/{name}/approve` happy path and `/mcp/tools/{name}/call` downstream happy path (both require a populated aggregator, tracked as a follow-up).

### T5: Changelog + commit
- [x] `CHANGELOG.md` entries under `## [Unreleased]` for the two behavior-changing commits
  (HTTP status handling; service_logs routing; `/setup/assets/download` removal)
- [x] Commits grouped by category -- see Notes

## Notes

- 2026-04-18: Sprint drafted during the next-gen -> main merge push. Deferred out of the merge window -- do after main lands.
- Resume doc "Known drift flagged but NOT addressed" items (capsem_stop, capsem restart, capsem history, capsem_service_logs) are out of scope; they are drift cleanup, not coverage gaps.
- Work was executed on `next-gen` (not a feature branch off main as originally planned), because the test infrastructure required (`tests/capsem-mcp/`, `tests/helpers/service.py`) only exists on `next-gen`.

## Discoveries

- **Suspend round-trip was broken end-to-end.** Surfaced while writing coverage: both the new MCP suspend test and the pre-existing `tests/capsem-lifecycle/test_vm_lifecycle.py::TestSuspendResume::test_suspend_resume_round_trip` failed with `suspend timed out: VM did not confirm suspended state (process killed)`. Fix landed via the debug agent (changes in `crates/capsem-core/src/hypervisor/apple_vz/`, `crates/capsem-process/src/{ipc,vsock,main}.rs`, `crates/capsem-service/src/main.rs`, `crates/capsem-agent/src/main.rs`). Both suspend tests now pass; the xfail marker on `test_suspend_and_resume_persistent` is removed.

- **CI does not actually run the VM-requiring tests.** `.github/workflows/ci.yaml` runs the non-VM directories (`tests/capsem-bootstrap/`, `tests/capsem-codesign/`, `tests/capsem-rootfs-artifacts/`) but for everything else only does `pytest --collect-only -q`, which imports test modules but never executes them. That is why the suspend bug sat green in CI. This is the "merges green but production breaks" scenario plan.md describes; fixing CI to run these suites (with a macOS runner that has the `com.apple.security.virtualization` entitlement) is a separate sprint. **Flagged to user.**

- **`UdsClient::request` ignored HTTP status codes.** The MCP client read response bodies regardless of status; 400/502/503 JSON error bodies got deserialized as `Ok(value)`. `capsem_mcp_call` surfaced the error payload as a successful tool result with `isError:false`. Other tools only escaped this because `format_service_response` happens to catch an embedded `error` key in the body. Fixed in `fix(mcp): surface HTTP errors from capsem-service instead of treating them as tool success`.

- **`/service-logs` is NOT dead code.** Matrix issue #2 claimed it had no caller. It does: `frontend/src/lib/api.ts:278` uses it for the Service Logs view. Matrix has been mentally corrected; keep the endpoint.

- **`capsem_service_logs` bypassed the service by design (or by accident).** The MCP tool opened `$CAPSEM_RUN_DIR/service.log` directly instead of calling `/service-logs`, duplicating the read logic. Now routes through the endpoint; post-mortem reads when the service is dead must use `tail` on the log file.

- **`/setup/assets/download` is dead code.** Zero callers anywhere (no frontend, no CLI, no MCP tool). Handler was a stub. Removed in `refactor(service): remove dead /setup/assets/download stub endpoint`.

- **Commits 7--9 (settings, setup/onboarding, mcp-api) are held.** These endpoints read/write `$HOME/.capsem/`. An initial fix that added `env["HOME"] = tmp_dir` to `tests/helpers/service.py` and `tests/capsem-mcp/conftest.py::capsem_service` was reverted in the working tree during debug-agent handoff. Until the HOME-isolation design lands, writing tests for these endpoints would either read/write the developer's real config (wrong) or skip the meaningful assertions.

- **HOME isolation landed using both `CAPSEM_HOME` and `HOME`.** `capsem_core::paths::capsem_home_opt()` honors `CAPSEM_HOME` with priority over `$HOME/.capsem`, so that env var is the right override for write paths (settings, setup-state, corp.toml). `$HOME` on its own still controls read-only detection (`/setup/detect` reads `~/.gitconfig`, `~/.ssh`, `~/.anthropic`, `~/.claude`, `~/.gemini`, `~/.config/openai`, `gh auth token`, `~/.config/gcloud`). Setting only `CAPSEM_HOME` would leave detect reading the developer's real credentials during tests; setting only `HOME` would still resolve to `$HOME/.capsem` via the fallback and work for writes but fight the abstraction. Setting both in `tests/helpers/service.py::ServiceInstance.start` and `tests/capsem-mcp/conftest.py::_start_capsem_service` gives full isolation: MCP + service + lifecycle suites (192 passed, 4 skipped) all green.

- **`/setup/detect` env-var credentials leak through `os.environ.copy()`.** `/setup/detect` checks `GEMINI_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY` before falling back to file paths. Fixtures inherit the test-runner's shell env, so these presence flags reflect the dev's actual env regardless of `HOME`/`CAPSEM_HOME`. `test_svc_setup.py::test_detect_returns_summary_shape` therefore asserts shape + file-based presence only. If a future test needs a deterministic presence check, sanitize the env in the fixture first.

- **`config/integration-test-corp.toml` references `network.default_action`, which is not in `config/defaults.json`.** Corp-config install validates TOML syntax but silently accepts unknown setting IDs (they're written to `corp.toml` but never surface in the settings tree). Low-severity drift: either `network.default_action` was renamed/removed or this is a forward-looking placeholder. Worth a drift ticket; out of scope for this sprint.

## Gateway layering

**Decision: (b)** document `tests/capsem-gateway/` as layer-specific and point downstream-endpoint correctness at `tests/capsem-service/` + `tests/capsem-mcp/` + `tests/capsem-e2e/`. Reasoning:

- Gateway tests cover the TCP-to-UDS shell: routing, auth, CORS, terminal WebSocket handshake, SPA static serving, lifecycle. Those concerns are layer-local and well-served by the MockServiceHandler.
- Downstream endpoint correctness is already fully owned by `tests/capsem-service/` (every HTTP handler) and `tests/capsem-mcp/` (every `#[tool]` over the live `capsem-mcp -> capsem-service -> VM` stack). Adding a third "gateway + real service" layer would duplicate those assertions and multiply the VM boot cost by another suite.
- The real-service path through the gateway is already smoke-covered by `tests/capsem-e2e/` (`test_e2e_lifecycle.py`, `test_e2e_mcp.py`) -- the CLI hits the service through the same HTTP surface the gateway proxies.
- A scope-setting docstring at `tests/capsem-gateway/conftest.py` records the split so future contributors don't write service-correctness assertions against the mock.

Follow-up (out of scope): the CI-runs-VM-tests gap flagged in the Discoveries section also affects `tests/capsem-e2e/`, so option (a) would not actually close a CI blind spot today.
