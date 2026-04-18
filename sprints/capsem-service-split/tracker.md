# Sprint: capsem-service main.rs split -- tracker

See `plan.md` for context, scope, and exit criteria.

## Tasks

### Phase 0: Reconnaissance (fast, read-only)

- [ ] Confirm `crates/capsem-service/Cargo.toml` has an explicit `[[bin]]`
      entry, or note that we need to add one.
- [ ] Locate `ErrorResponse`: `rg "struct ErrorResponse" crates/capsem-service`.
      Plan needs it; it might be in `api.rs`, not `main.rs`.
- [ ] Record baseline: `cargo test -p capsem-service 2>&1 | tail -5` (test
      count) and `cargo llvm-cov -p capsem-service --summary-only` (per-file
      coverage). Paste into Notes.
- [ ] Identify the inline `mod tests` span at `main.rs:2882` -- read ~400
      lines from there and note which test fns touch ONLY the extracted
      helpers (candidates to move) vs. ServiceState/handlers (stay).

### Phase 1: lib.rs scaffolding

- [ ] Create `crates/capsem-service/src/lib.rs` with empty `pub mod`
      placeholders for the three submodules (add them one file at a time).
- [ ] Verify `cargo check -p capsem-service` still passes before moving any
      code.

### Phase 2: extract `errors.rs`

- [ ] Create `crates/capsem-service/src/errors.rs` with `pub struct AppError`,
      `pub struct ErrorResponse` (or re-export from `api.rs` if that's where
      it already lives), and the `IntoResponse` impl.
- [ ] Update `main.rs`: delete the moved definitions; add
      `use crate::errors::{AppError, ErrorResponse};` (or the `capsem_service::...`
      path -- try `crate::` first since `lib.rs` and `main.rs` are siblings).
- [ ] Add or move `#[cfg(test)]` coverage for `AppError::into_response` and
      the JSON shape.
- [ ] `cargo test -p capsem-service` green.

### Phase 3: extract `fs_utils.rs`

- [ ] Create `crates/capsem-service/src/fs_utils.rs` with `sanitize_file_path`,
      `extract_magika_info`, `identify_file_sync`. Leave `resolve_workspace_path`
      in `main.rs` -- it takes `&ServiceState` and moving it pulls state out
      of scope for this sprint.
- [ ] Move corresponding tests into the submodule's own `mod tests`.
- [ ] Add new tests: path collapsing consecutive slashes, traversal
      rejection, Magika smoke against a real small file, empty path after
      sanitization.
- [ ] `cargo test -p capsem-service` green.

### Phase 4: extract `naming.rs`

- [ ] Create `crates/capsem-service/src/naming.rs` with `validate_vm_name`
      and `generate_tmp_name`.
- [ ] Move corresponding tests.
- [ ] Add new tests: empty name, >64 chars, leading hyphen/underscore,
      non-ASCII, `generate_tmp_name` shape (starts with `tmp-`, contains
      exactly two hyphens).
- [ ] `cargo test -p capsem-service` green.

### Phase 5: verification

- [ ] `cargo test -p capsem-service` -- count must be >= baseline + new tests.
- [ ] `cargo llvm-cov -p capsem-service --summary-only` -- record new
      per-file coverage for the three new submodules.
- [ ] `just test` -- full workspace gate.
- [ ] `just run "capsem-doctor"` -- VM smoke.

### Phase 6: changelog + commit

- [ ] CHANGELOG `[Unreleased]` entry under `### Changed` for the refactor.
      If enough new tests, a second line under `### Added`.
- [ ] Commit 1 (atomic, self-contained):
      `refactor(service): extract pure helpers into lib + submodules`
      Staged files:
      `crates/capsem-service/Cargo.toml`,
      `crates/capsem-service/src/lib.rs`,
      `crates/capsem-service/src/errors.rs`,
      `crates/capsem-service/src/fs_utils.rs`,
      `crates/capsem-service/src/naming.rs`,
      `crates/capsem-service/src/main.rs`,
      `CHANGELOG.md`,
      `sprints/capsem-service-split/plan.md`,
      `sprints/capsem-service-split/tracker.md`.

## Baseline (fill in during Phase 0)

```
Test count before:    [TBD]
Lines in main.rs:     4331
Coverage main.rs:     [TBD]  (expect ~60-70% from existing inline tests)
```

## Notes

- Started: 2026-04-18. Deferred MCP-crate work because
  `sprints/mcp-endpoint-coverage/` is the owner of that surface.
- The same afternoon already landed two adjacent bug fixes: corrupt
  setup-state now warns; `DbReader::query_raw` now validates SELECT-only
  up front. Neither touches capsem-service, so this sprint starts clean.
- **Resume hint for a fresh session**: open `plan.md` first, then this
  tracker. Don't re-audit -- the coverage matrix and related-work context
  in the plan already captured what today found.
- Per `/dev-sprint`: this sprint ends with `just test` green. Do not skip.

## Discoveries

_(fill in as work progresses)_

## Follow-up sprints (captured in plan.md)

- Move handlers into route-grouped modules.
- Move `ServiceState` + `PersistentRegistry` into `lib.rs`.
- Move `send_ipc_command` + `wait_for_vm_ready` into `src/ipc.rs`.
- Replace in-process handler tests with `crates/capsem-service/tests/`
  integration tests now that `lib.rs` exists.
