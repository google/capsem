# Sprint: capsem-service main.rs split

## Why now

`crates/capsem-service/src/main.rs` is 4,331 lines. It contains the entire
service daemon: persistent-VM registry, `ServiceState`, IPC helpers, file-API
path security, Magika wrappers, ~40 axum route handlers, router construction,
`#[tokio::main]`, **and** an inline `#[cfg(test)] mod tests { ... }` buried at
line 2882. Only two files in the crate: `main.rs` and `api.rs`.

This shape bites us three ways:

1. **Handlers can't be unit-tested without spinning up the full service.**
   The crate has no `lib.rs`, so `crates/capsem-service/tests/` -- the
   conventional place for integration tests -- can't exist. Every behavioral
   test has to talk to a running daemon over a real UDS, which is what
   `tests/capsem-service/*.py` already does. The feedback loop is slow and
   CI-heavy, and simple things like "does `sanitize_file_path` reject this
   input" have to go through the whole stack.
2. **Helpers that are obviously pure (`sanitize_file_path`, Magika adapters,
   `validate_vm_name`, `generate_tmp_name`, `AppError`) are invisible to
   readers outside the file** because the file is too long to skim. New
   handlers tend to re-implement path sanitization rather than discover the
   existing helper.
3. **Test density is 2 tests per 100 lines** and all of them sit in the single
   inline module. There's no locality between a handler and its tests -- the
   reader has to scroll or grep.

Clean numbers from today's audit:
- capsem-service crate: 112 test fns / 4,999 lines, 2 tests/100 LOC.
- `main.rs` alone: 4,331 lines, 86 tests, all in one `mod tests`.
- Handlers that have NO direct Rust unit test (only in-process Python e2e, or
  nothing at all): `handle_list_files`, `handle_download_file`,
  `handle_upload_file`, `handle_fork`, `handle_service_logs`,
  `handle_reload_config`, all of `/settings/*`, all of `/setup/*`, all of
  `/mcp/*`, all of `/history/*`.

## What this sprint does

**Tight first step**, not the full split. Goal: unblock unit-testing of pure
helpers and establish the crate shape (`lib.rs` + submodules), **without**
moving any handler yet. Moving handlers requires touching `ServiceState`
visibility + untangling shared state + re-homing tests; that's a follow-up.

Do in this sprint:

1. Add `crates/capsem-service/src/lib.rs` and declare the crate as both a
   library and a binary in `Cargo.toml`. `main.rs` stays the binary entry
   point; it uses `lib.rs` items via `use capsem_service::*` or local
   `mod` re-declarations (pick one -- see Decisions).
2. Extract three pure-helper clusters to small submodules of the library:
   - `src/errors.rs` -- `AppError`, `ErrorResponse`, the `IntoResponse` impl.
   - `src/fs_utils.rs` -- `sanitize_file_path`, `extract_magika_info`,
     `identify_file_sync`. Leave `resolve_workspace_path` behind for now
     because it takes `&ServiceState` and would force `ServiceState` out of
     `main.rs` too.
   - `src/naming.rs` -- `validate_vm_name`, `generate_tmp_name`.
3. Update `main.rs` to import from the new modules. Delete the original
   definitions.
4. Move the portion of the inline `#[cfg(test)] mod tests` that exercises the
   extracted helpers into the new submodules' own `#[cfg(test)]` blocks.
   Leave handler/state tests in `main.rs` for this sprint.
5. Add a handful of NEW unit tests for each submodule -- the split is not
   the goal, better tests are. Target each submodule reaching ~90% line
   coverage on what lives in it.
6. Run `cargo test -p capsem-service` green; run `just test` gate.

Do **NOT** do in this sprint:

- Move `ServiceState` or `PersistentRegistry` out of `main.rs`.
- Move any `handle_*` function.
- Change any handler signature or behavior.
- Touch `crates/capsem-service/src/api.rs` (separate concern).
- Add Python integration tests -- that's the
  `sprints/mcp-endpoint-coverage/` sprint's territory. Do not conflict.

## Key decisions

### lib.rs vs. module declarations in main.rs

Two ways to add submodules to a crate that has a `[[bin]]` target:

- **A. Canonical library + binary.** `src/lib.rs` declares `pub mod errors;
  pub mod fs_utils; pub mod naming;`. `src/main.rs` becomes a thin
  `#[tokio::main] async fn main()` that imports from `capsem_service::*`.
  Benefits: external tests work (`crates/capsem-service/tests/`); rustdoc
  generates. Cost: `main.rs` still has 4k lines of handlers, so most of the
  crate is double-compiled (once for `lib`, once for `bin`). Not actually
  a cost until handlers move, and we're not moving handlers this sprint --
  so `main.rs` stays bin-only and `lib.rs` only re-exports the new small
  submodules.
- **B. Submodules declared in `main.rs`.** `main.rs` keeps `mod errors;
  mod fs_utils; mod naming;` the way it currently keeps `mod api;`. No
  `lib.rs`. Cost: can't add `crates/capsem-service/tests/` integration
  tests; unit tests must live inline in each submodule.

**Pick A.** Precedent: `crates/capsem-core`, `capsem-logger`, `capsem-proto`
all ship as `lib + bin`. Picking A lets the follow-up sprint (`handlers to
submodules`) add `tests/handlers.rs` without a second Cargo.toml change.
This sprint writes the tiniest possible `lib.rs`.

### Where the extracted tests live

Existing inline `#[cfg(test)] mod tests` at `main.rs:2882` contains
assertions for `sanitize_file_path`, `validate_vm_name`, `AppError`, and
handler paths mixed together. Move ONLY the helper-specific tests into the
new submodules' own `#[cfg(test)]` blocks; leave handler tests alone.

Search pattern: `grep -n "fn.*sanitize\|fn.*validate_vm\|fn.*generate_tmp\|fn.*app_error\|fn.*magika" crates/capsem-service/src/main.rs` to find candidates. Expect ~15-25 test fns to move.

### MCP sprint coordination

The `sprints/mcp-endpoint-coverage/` sprint is active (paused mid-flight). It
is test-only: adds Python tests under `tests/capsem-mcp/` and
`tests/capsem-e2e/`. It does NOT edit `crates/capsem-service/src/main.rs`.

**Conflict risk:** zero direct. Merge risk: zero because the MCP sprint
doesn't touch the Rust source. Proceed without waiting.

## Files to create

```
crates/capsem-service/src/lib.rs         NEW -- declares pub mod errors; etc.
crates/capsem-service/src/errors.rs      NEW -- AppError + IntoResponse + ErrorResponse
crates/capsem-service/src/fs_utils.rs    NEW -- sanitize_file_path + Magika helpers
crates/capsem-service/src/naming.rs      NEW -- validate_vm_name + generate_tmp_name
crates/capsem-service/Cargo.toml         MODIFIED -- may need [[bin]] explicit + [lib] target
crates/capsem-service/src/main.rs        MODIFIED -- delete moved items, add `use capsem_service::...`
sprints/capsem-service-split/plan.md     NEW (this file)
sprints/capsem-service-split/tracker.md  NEW
```

## Dependencies and ordering

1. Create `lib.rs` with empty exports; verify `cargo check -p capsem-service`.
2. Verify `Cargo.toml` either already supports `[lib]` implicitly or add
   `[lib] path = "src/lib.rs"` explicitly. (Cargo infers `lib.rs` by default
   even when `[[bin]]` is present, so this should be a no-op.)
3. Create `errors.rs` with `AppError` moved in; update `main.rs` to
   `use capsem_service::errors::AppError;` (or `use crate::errors::...` if
   `main.rs` also declares `mod errors` as a sibling of lib -- test both).
4. Run `cargo check -p capsem-service` and `cargo test -p capsem-service`.
5. Repeat for `fs_utils.rs`, then `naming.rs`.
6. Move the helper-specific tests from the inline `mod tests` block in
   `main.rs` into each submodule's own `#[cfg(test)] mod tests`.
7. Run `cargo test -p capsem-service` -- must still be 112+ tests, all green.
8. Add 5-10 new tests per submodule aiming at untested branches (path
   collapsing in `sanitize_file_path`, length-limit rejection in
   `validate_vm_name`, etc.).
9. Run `cargo llvm-cov -p capsem-service --summary-only` -- record before
   vs. after per-file coverage.
10. `just test` full gate.
11. CHANGELOG entry + commit (may be 1-2 commits; see below).

### Commit boundaries

Following `/dev-sprint`, commit at functional milestones. Proposed split:

- **Commit 1**: `refactor(service): extract pure helpers into lib + submodules`
  - Adds `lib.rs`, `errors.rs`, `fs_utils.rs`, `naming.rs`
  - Removes the original definitions from `main.rs`
  - Moves corresponding tests
  - CHANGELOG entry under `### Changed`
  - Sprint plan + tracker committed together
- **Commit 2** (optional): `test(service): expand coverage of extracted helpers`
  - New unit tests in each submodule
  - CHANGELOG entry under `### Added`

If commit 2 is small (<30 LOC of tests) just fold into commit 1.

## Definition of done

- [ ] `crates/capsem-service/src/lib.rs` exists and compiles.
- [ ] `errors.rs`, `fs_utils.rs`, `naming.rs` each contain their cluster and
      have a `#[cfg(test)] mod tests` block exercising every public function.
- [ ] `main.rs` no longer defines `AppError`, `sanitize_file_path`,
      `extract_magika_info`, `identify_file_sync`, `validate_vm_name`, or
      `generate_tmp_name`.
- [ ] `cargo test -p capsem-service` reports >= the pre-sprint test count
      and 0 failures.
- [ ] `cargo llvm-cov -p capsem-service --summary-only` shows non-zero
      coverage for the new submodule files.
- [ ] `just test` passes (full workspace gate).
- [ ] `just run "capsem-doctor"` passes (VM smoke).
- [ ] CHANGELOG `[Unreleased]` has an entry reflecting the refactor.
- [ ] Tracker marked done.

## Out of scope (explicit follow-ups)

Captured here so the next sprint has a crisp starting point:

1. **Move `handle_*` route handlers into route-grouped modules**:
   `src/routes/lifecycle.rs` (provision/list/info/delete/stop/suspend/resume/
   purge/run/fork/persist), `routes/exec.rs`, `routes/files.rs`,
   `routes/settings.rs`, `routes/setup.rs`, `routes/mcp.rs`,
   `routes/history.rs`, `routes/inspect.rs`. Requires moving `ServiceState`
   to `lib.rs` first (or passing dependencies explicitly).
2. **Move `ServiceState` and `PersistentRegistry` into `lib.rs`.** Requires
   making many currently-private fields `pub(crate)`.
3. **Move IPC helpers (`send_ipc_command`, `wait_for_vm_ready`) into
   `src/ipc.rs`.** Small, mechanical.
4. **Replace in-process handler tests with integration tests under
   `crates/capsem-service/tests/`** once `lib.rs` exposes enough surface.

## Related work done today (context for resume)

These are already landed in the working tree (see `git status`):

- `crates/capsem-logger/src/reader.rs` -- `query_raw` / `query_raw_with_params`
  now validate SQL up front via `validate_select_only`. **Bug fix**: closed an
  in-memory gap and replaced cryptic SQLite errors with a clean message.
- `crates/capsem-core/src/setup_state.rs` -- `load_state` now warns on corrupt
  JSON instead of silently resetting setup progress. **Bug fix**.
- `crates/capsem/src/setup.rs` -- small DI refactor so `load_state`,
  `save_state`, and each `step_*` take `capsem_dir: &Path` explicitly. 11 new
  unit tests. Behavior unchanged.
- `codecov.yml` + `.github/workflows/ci.yaml` -- `capsem-ui`,
  `capsem-mcp-aggregator`, `capsem-mcp-builtin` added to coverage `-p` lists;
  `tooling` component includes MCP subprocess crates; new `systray`
  component.
- `crates/capsem-mcp/src/main.rs` -- `format_service_response` +
  `build_*_body` helpers extracted from inline handler matches. +26 tests.
- `crates/capsem-tray/src/gateway.rs` -- full HTTP-probe-based test harness
  for every verb. 36% -> 94%. New `new_with_base_url` constructor for DI.
- `crates/capsem-gateway/src/main.rs`, `crates/capsem-app/src/main.rs`,
  `crates/capsem-logger/src/reader.rs` -- various test additions.

**None** of that touches `crates/capsem-service/src/main.rs`, so this sprint
starts from a clean slate on that file.

## Risk ledger

- **Risk**: adding `lib.rs` to a crate that currently builds cleanly as
  bin-only may require an explicit `[[bin]]` entry in `Cargo.toml`; Cargo
  stops auto-inferring both targets once one is explicit.
  **Mitigation**: `crates/capsem-service/Cargo.toml` already has an explicit
  `[[bin]] name = "capsem-service" path = "src/main.rs"` (verify first). If
  not, add one.
- **Risk**: `AppError` currently uses `Json<ErrorResponse>` in
  `into_response`. The `ErrorResponse` type is defined in `api.rs` (check
  this). If so, `errors.rs` must re-export it or `api.rs` must move it.
  **Mitigation**: grep `rg "struct ErrorResponse" crates/capsem-service` to
  confirm where it lives before splitting.
- **Risk**: the inline `mod tests` at `main.rs:2882` may reference private
  fields of `ServiceState` that become inaccessible when test fns move to
  `errors.rs` etc. **Mitigation**: only move test fns that touch ONLY the
  extracted helper, not `ServiceState`. Leave the rest.
- **Risk**: `cargo llvm-cov` has flaked with exit 144 (OOM-kill?) when run
  across multiple large crates. **Mitigation**: run per-crate only; the
  sprint's coverage verification uses just `-p capsem-service`.
