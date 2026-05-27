# Sprint: cli-api-hardening

## Tasks
- [x] Create sprint plan/tracker
- [x] Change `create` to positional optional name (remove `-n`)
- [x] Change `shell` to positional optional session only (remove `-n`)
- [x] Remove CLI compatibility aliases (`attach`, `ls`, `rm`, `--image`)
- [x] Remove legacy service file routes (`/read_file`, `/write_file`)
- [x] Migrate frontend file read/write to `/files/{id}/content`
- [x] Migrate MCP file read/write to `/files/{id}/content`
- [x] Update CLI/API docs for strict surface
- [x] Migrate Python integration tests and gateway mock to canonical file routes
- [x] Run Rust validation suites for touched crates
- [ ] Run frontend API test suite (blocked: local `vitest` missing)

## Notes
- This is a strict no-backward-compat pass for initial release cleanup.
- `cargo test -p capsem -- --nocapture` still has pre-existing unrelated failures:
  - `setup::tests::corp_config_from_local_file_marks_step_done`
  - `client::tests::connect_await_startup_waits_for_late_binder`
- Targeted Python validation:
  - `uv run pytest -q tests/capsem-gateway/test_gw_proxy_advanced.py::TestProxyEndpointCoverage::test_post_read_file` passed
  - `uv run pytest -q tests/capsem-service/test_svc_file_io.py::TestFileIO::test_roundtrip` blocked in this environment (VM did not reach exec-ready)
  - `PYTHONPYCACHEPREFIX=/private/tmp/pycache python3 -m compileall ...` passed for updated test trees
- Frontend tests are blocked locally because `frontend/node_modules` (including `vitest`) is not installed in this environment.

## Coverage Ledger
- Unit/contract:
  - `cargo test -p capsem parse_ -- --nocapture` (62 passed)
  - `cargo test -p capsem-service -- --nocapture` (89 + 92 passed)
  - `cargo test -p capsem-mcp -- --nocapture` (100 passed)
- Functional: CLI parser + service/mcp suite coverage for changed endpoints and argument parsing
- Adversarial: explicit reject tests for `attach`, `ls`, `rm`, `--image`, `create -n`, `shell -n`
- E2E/VM: deferred
- Telemetry: n/a
- Performance: n/a
- Missing/deferred: full taxonomy migration (`sandbox`→`session`/`vm`) across all internals and Python integration test endpoint migration
- Missing/deferred: full taxonomy migration (`sandbox`→`session`/`vm`) across all internals
