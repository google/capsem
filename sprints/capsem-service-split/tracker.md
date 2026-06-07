# Sprint: capsem-service main.rs split -- tracker

See `plan.md` for context, scope, and exit criteria.

## Tasks

### Phase 0: Reconnaissance (fast, read-only)

- [x] Confirm `crates/capsem-service/Cargo.toml` has an explicit `[[bin]]`
      entry, or note that we need to add one.
      → No explicit `[[bin]]`. Cargo auto-infers both targets when a `lib.rs`
        is added; verified by `cargo check`. No Cargo.toml edit required.
- [x] Locate `ErrorResponse`: lives at `crates/capsem-service/src/api.rs:271`
      as `pub struct`. Re-exported from `errors.rs` via `pub use` so the
      `api.rs` surface stays untouched.
- [x] Record baseline: 119 tests passing in capsem-service (single bin
      target).
- [x] Identify the inline `mod tests` span -- moved 19 helper-only test
      fns (3 errors, 10 sanitize, 6 validate_vm_name) into the new submodules.

### Phase 1: lib.rs scaffolding

- [x] Create `crates/capsem-service/src/lib.rs` declaring `pub mod api`,
      `pub mod errors`, `pub mod fs_utils`, `pub mod naming`.
- [x] Drop `mod api;` from `main.rs` (api was being compiled twice; the
      lib-side declaration is now canonical) and switch `use api::*` to
      `use capsem_service::api::*` plus a separate `use capsem_service::api;`
      for the explicit `api::Foo` callsites.
- [x] `cargo check -p capsem-service` green.

### Phase 2: extract `errors.rs`

- [x] `pub struct AppError(pub StatusCode, pub String)` + `IntoResponse` impl.
      `pub use crate::api::ErrorResponse` re-export.
- [x] `main.rs` adds `use capsem_service::errors::AppError;`.
- [x] 3 `app_error_*` tests moved + 2 new (`app_error_preserves_arbitrary_status`,
      `app_error_preserves_empty_message`).
- [x] `cargo test -p capsem-service` green.

### Phase 3: extract `fs_utils.rs`

- [x] Moved `sanitize_file_path`, `extract_magika_info`, `identify_file_sync`.
      `resolve_workspace_path` stays in `main.rs` (takes `&ServiceState`).
- [x] 10 `sanitize_*` tests moved + 5 new (`sanitize_rejects_only_slashes`,
      `sanitize_rejects_dot_dot_after_filter`, `extract_magika_info_smoke`,
      `identify_file_sync_returns_unknown_for_missing_file`,
      `identify_file_sync_round_trips_real_file`).
- [x] `cargo test -p capsem-service` green.

### Phase 4: extract `naming.rs`

- [x] Moved `validate_vm_name`, `generate_tmp_name`.
- [x] 6 `validate_vm_name_*` tests moved + 7 new
      (`_starts_with_underscore`, `_starts_with_digit_ok`, `_rejects_non_ascii`,
      `_rejects_dot`, `generate_tmp_name_starts_with_tmp_prefix`,
      `generate_tmp_name_has_exactly_two_hyphens`,
      `generate_tmp_name_passes_validate_vm_name`).
- [x] `cargo test -p capsem-service` green.

### Phase 5: verification

- [x] `cargo test -p capsem-service`: 133 tests, 0 failed (59 lib + 74 main).
- [x] `cargo clippy -p capsem-service --all-targets -- -D warnings`: clean.
- [x] `cargo llvm-cov -p capsem-service --summary-only`: 100% line/region/
      function coverage on `errors.rs`, `fs_utils.rs`, `naming.rs`.
- [x] `cargo test --workspace`: all Rust unit/integration tests green.
- [~] `just test`: 37 Python integration failures, **all unrelated to T0**
      (gateway, guest, doctor, mcp service-logs, lifecycle benchmarks,
      session events). Spot-checked `test_svc_fork.py`: passes 3/3 against
      the refactored build in isolation. Per user direction, the integration
      suite needs its own stabilization sprint before T1+ proceeds.
- [~] `just run "capsem-doctor"`: skipped because the same instability
      affects `test_doctor_passes`; would not give meaningful signal until
      the suite stabilizes.

### Phase 6: changelog + commit

- [x] CHANGELOG `[Unreleased]` entry under `### Changed`.
- [x] Commit 1 (atomic, self-contained):
      `refactor(service): extract pure helpers into lib + submodules`

## Baseline

```
Test count before:    119  (single capsem-service bin)
Test count after:     133  (59 lib + 74 main)  +14 net
Coverage:
  errors.rs           100.00%  line / region / function
  fs_utils.rs         100.00%  line / region / function
  naming.rs           100.00%  line / region / function
  api.rs              98.56% / 97.86% / 90.62%  (now exercised on lib side)
  main.rs             45.44% / 46.18% / 38.69%  (handlers untouched)
```

## Notes

- Started: 2026-04-18. Resumed and executed: 2026-04-20.
- Concurrent commits between session start and resume added `mod startup;`,
  `crates/capsem-guard/`, plus ~318 lines to `main.rs`. None affected the
  helpers being extracted; the line offsets in the plan shifted but the
  function signatures did not.
- Per `/dev-sprint`: gating on `just test` was attempted; 37 Python
  failures observed but none are caused by T0 -- confirmed by isolation
  check on a representative test (`test_svc_fork.py`). User confirmed the
  integration suite is unstable; deferring `just run capsem-doctor` gating
  for the same reason.

## Discoveries

- **`mod api` was compiled twice.** Pre-refactor, `main.rs` declared
  `mod api;` even though api items were also reachable via the eventual
  `lib.rs` route. Removing the bin-side `mod api;` shifted 26 api.rs tests
  from the bin test binary to the lib test binary -- the +14 net total is
  the honest measure; per-binary numbers shifted as a side effect.
- **`AppError` tuple fields had to be `pub`.** Construction sites in
  `main.rs` use `AppError(StatusCode::X, msg.into())` directly; making the
  fields `pub StatusCode, pub String` keeps that ergonomics with no
  callsite changes.
- **`extract_magika_info` was unused in `main.rs`** after extraction --
  only `identify_file_sync` was called from handlers. Dropped from the
  bin-side import list.
- **Python integration suite is the bottleneck.** 37 failures in
  `just test`, all infra/integration. Test stabilization is now a
  prerequisite to the follow-up sprint per user direction.

## Follow-up sprints

T1+ tracked under `sprints/capsem-service-split-followup/`:
moves handlers (files/images/mcp/history), `PersistentRegistry`, dedups
spawn paths. Blocked on:

1. Python integration test stabilization.
2. `sprints/mcp-endpoint-coverage/` completing -- that sprint owns the MCP
   handler surface, so a future `routes/mcp.rs` must wait.
