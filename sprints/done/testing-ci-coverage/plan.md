# Sprint: Testing CI Coverage & Principled Architecture

## Why

Audit revealed that the testing infrastructure, while extensive (~3,700 tests), had critical
gaps in CI enforcement. Tests existed but didn't run where they mattered:

- 422 Rust tests in 6 crates never ran in CI (simple oversight -- no VZ deps blocking them)
- ~40 Python non-VM integration tests not in CI
- No Rust coverage floor (Python had 90%, Rust could silently regress to 0%)
- capsem-process was a 1,522-line monolith with 24 tests covering only data structures
- No test matrix documenting what runs where

## What we did

### 1. CI: Add 6 Rust crates (422 tests)

Added capsem-service, capsem, capsem-mcp, capsem-tray, capsem-process to the CI nextest
commands on both macOS and Linux. capsem-app gets a compile check (needs Tauri assets to link).

### 2. CI: Non-VM Python integration tests

capsem-bootstrap (21), capsem-codesign (7), capsem-rootfs-artifacts (6) now execute in CI.
All 25 capsem-* suites get --collect-only to catch import/syntax errors.

### 3. Rust coverage floor (70%)

--fail-under-lines 70 on CI and just test. Codecov unit upload changed to fail_ci_if_error: true.

### 4. capsem-process module decomposition

Split 1,522-line main.rs into 6 modules:
- helpers.rs (clone_fd, query_max_fs_event_id)
- job_store.rs (JobStore, JobResult, with_quiescence)
- vsock.rs (VsockOptions, setup_vsock, port routing)
- ipc.rs (handle_ipc_connection, IPC dispatch classification)
- terminal.rs (handle_terminal_socket, resize parsing)
- main.rs (Args, CLI env parsing, orchestrator)

Tests grew from 24 to 62.

### 5. Test matrix documentation

Added to skills/dev-testing/SKILL.md:
- Rust crate CI matrix (11 crates, which CI job runs each)
- Python integration suite tier map (25 suites, what runs in CI/smoke/full)
- Coverage targets table

## Files modified

- `.github/workflows/ci.yaml` -- CI pipeline expansion + coverage floor
- `codecov.yml` -- unchanged (already had component targets)
- `justfile` -- coverage floor in just test
- `crates/capsem-process/src/main.rs` -- slimmed to orchestrator
- `crates/capsem-process/src/helpers.rs` -- new module
- `crates/capsem-process/src/job_store.rs` -- new module
- `crates/capsem-process/src/vsock.rs` -- new module
- `crates/capsem-process/src/ipc.rs` -- new module
- `crates/capsem-process/src/terminal.rs` -- new module
- `skills/dev-testing/SKILL.md` -- test matrix section
- `CHANGELOG.md` -- entries added

## What "done" looks like

- cargo check --workspace clean
- cargo test -p capsem-process: 62 tests pass
- CI YAML valid
- CHANGELOG updated
- Test matrix in dev-testing skill matches CI reality

## Future work (not in this sprint)

- Thin integration suite expansion (capsem-recovery 4->10, capsem-stress 3->8, etc.)
- These need VM boot, so they're a separate effort
- Ratchet coverage floor upward as crate coverage improves
