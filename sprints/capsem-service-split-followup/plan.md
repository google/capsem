# Sprint: capsem-service main.rs split (T1+ follow-up)

> **Status: blocked.** See Blockers. This plan is written to be executed
> cold -- start here, do not re-audit.

## Context: what T0 landed, what's left

### Already done (commit `4bc7827`, do NOT redo)

T0 added `crates/capsem-service/src/lib.rs` and extracted three pure-helper
clusters. Current crate shape:

```
crates/capsem-service/src/
  lib.rs        12 lines   pub mod api/errors/fs_utils/naming
  api.rs       678 lines   on-the-wire DTOs (UNTOUCHED)
  errors.rs     87 lines   AppError + IntoResponse, re-exports ErrorResponse
  fs_utils.rs  212 lines   sanitize_file_path, Magika helpers
  naming.rs    240 lines   generate_tmp_name (collision-aware), validate_vm_name
  startup.rs   170 lines   startup lock / singleton
  main.rs    4,855 lines   <-- still owns ServiceState + PersistentRegistry +
                               every axum handler + spawn paths + router
```

Current baseline (commit on `next-gen` HEAD):
- `cargo test -p capsem-service`: 144 passing (62 lib + 82 main).
- Coverage on `errors.rs`/`fs_utils.rs`/`naming.rs`: 100% line/region/fn.
- `cargo check -p capsem-service`: green. Clippy: clean.

### What T0 deliberately left behind

- `ServiceState`, `PersistentRegistry`, `PersistentVmEntry`,
  `PersistentRegistryData`, `InstanceInfo`, `ProvisionOptions` -- all in
  `main.rs`.
- Every `handle_*` axum handler.
- `resolve_workspace_path` (borrows `&ServiceState`).
- `provision_sandbox` + `resume_sandbox` (two spawn-capsem-process sites).
- IPC helpers (`send_ipc_command`, `wait_for_process_exit` at ~line 2088).
- The inline `#[cfg(test)] mod tests` block in `main.rs` (handler +
  state-bound tests).

### Things evolved between T0 and this plan

- **`generate_tmp_name` now takes an `existing` iterator** and produces
  `<adj>-<noun>-tmp` (suffix, not `tmp-` prefix). This was a post-T0
  change to avoid first-word collisions between concurrent temp VMs.
  Callsites in `main.rs:1193` and `main.rs:2511` are already updated --
  don't revert them when moving code. Tests in `naming.rs` cover the new
  collision-avoidance path.

## Blockers (resolve in order before starting)

1. **Python integration suite stable.** T0 observed 37 failures in
   `just test`, all unrelated to T0 (verified by isolating
   `tests/capsem-service/test_svc_fork.py` -- passes 3/3 against the
   refactored build). The follow-up needs a working integration net to
   prove "no behavior change" on handler moves. Likely causes to probe:
   pytest-xdist parallel races, leaked sandboxes between tests,
   gateway/guest startup flakes. Tracked separately.
2. **`sprints/mcp-endpoint-coverage/` complete.** It owns the MCP handler
   surface and is paused mid-flight. Touching MCP handlers before it
   finishes will collide with its test additions.

When both are resolved, start with T1.

## Sub-sprints

Meta-sprint layout per `/dev-sprint`: each sub-sprint is its own
`T<N>-<name>.md` inside `sprints/capsem-service-split-followup/`, with its
own tracker (create `tracker.md` for the active one only, per
`/dev-sprint` convention). Update `MASTER.md`'s status table as each
lands.

### T1 -- `src/registry.rs` (extract `PersistentRegistry`)

**Scope.** Move `PersistentVmEntry`, `PersistentRegistryData`, `PersistentRegistry`
+ its impl to a new `src/registry.rs`. These are in `main.rs` around lines
65-146 (verify -- offsets drift with every concurrent commit).

`PersistentRegistry` does NOT depend on `ServiceState`. It's the smallest,
most isolated move and should land first. Depends on nothing beyond what
T0 already provides.

**Tests to move.** In the inline `main.rs` mod tests block, search for
`persistent_registry_*` and move each into `registry::tests`. Should be
~5 tests.

**New tests to add.** Target ~90% line coverage on the module:
- `register`/`unregister` atomic save (file-on-disk after each op).
- Corrupt JSON on load (existing behavior: silently empty).
- `get` vs `get_mut` against a missing key.
- `contains` true/false.

**Commit.** `refactor(service): move PersistentRegistry into registry module`.

### T2 -- `ServiceState` -> `lib.rs`

**Scope.** Move `ServiceState`, `InstanceInfo`, `ProvisionOptions` into
the library (likely `src/state.rs` or `pub mod state` in `lib.rs`).
Mark fields `pub(crate)`. Keep `impl ServiceState` with the handler-facing
methods (`provision_sandbox`, `resume_sandbox`, `resolve_asset_paths`,
etc.) in `main.rs` for now -- T2 only moves the type definitions, not
the giant impl blocks.

**Why this order.** T1 went first because it's the smallest mechanical
move. T2 is the surface-changing one. Every `routes/*.rs` below needs
`ServiceState` to be reachable from the library, which means fields are
`pub(crate)` and the struct itself is `pub` at the crate root.

**Tests.** No handler moves yet. Existing `ServiceState`-using tests
continue to work because of `pub(crate)`. No new tests in this sub-sprint;
it's pure plumbing.

**Commit.** `refactor(service): move ServiceState into lib`.

### T3 -- `src/routes/files.rs`

**Scope.** Create `src/routes.rs` declaring `pub mod files;` (and pre-declare
siblings as they come). Move:
- `handle_list_files` (main.rs ~946)
- `handle_download_file` (~1006)
- `handle_upload_file` (~1055)
- `resolve_workspace_path` (~766)
- Helpers: `FileListQuery`, `FileContentQuery`, `default_file_depth`,
  `list_dir_recursive` (~849).

This is the first route-relocation; sets the pattern for T4/T6. Keep
handlers as `pub async fn` taking `State<Arc<ServiceState>>`. The router
in `main.rs` stays; just swap the function paths.

**Tests to move.** Every `resolve_*`, `list_dir_*`, `download_*`,
`upload_*` in the inline mod tests (~10-15 tests).

**New tests.** Aim at untested branches in `resolve_workspace_path`
(parent-doesn't-exist, canonicalize error on workspace itself) and
upload size cap.

**Commit.** `refactor(service): extract files routes into routes/files.rs`.

### T4 -- `src/routes/images.rs`

**Scope.** `handle_fork` (~1096). Check for peers that logically belong
("image-like" handlers) but `handle_fork` may be it. Don't invent groupings.

**Tests.** Move the `handle_fork_*` tests (4+ in inline mod tests).

**Commit.** `refactor(service): extract fork into routes/images.rs`.

### T6 -- `src/routes/history.rs`

**Scope.** `handle_history` (~2001), `handle_history_processes` (~2024),
`handle_history_counts` (~2041), `handle_history_transcript` (~2061).

These reference types via `api::HistoryQuery`, `api::HistoryResponse`, etc.
-- the `use capsem_service::api;` in main.rs already works from route
modules.

**Tests.** No currently-inline history tests found -- add smoke tests for
each handler against a test `ServiceState` with an empty/non-empty db.

**Commit.** `refactor(service): extract history routes into routes/history.rs`.

### T5 -- `src/routes/mcp.rs` (BLOCKED on mcp-endpoint-coverage)

**Scope.** `handle_mcp_servers` (~1797), `handle_mcp_tools` (~1832),
`handle_mcp_policy` (~1852), `handle_mcp_refresh` (~1876),
`handle_mcp_approve` (~1892), `handle_mcp_call` (~1911).

**Do not start this sub-sprint until `sprints/mcp-endpoint-coverage/`
completes.** That sprint is actively adding tests against these handlers;
moving them underneath it causes merge pain.

### T7 -- `src/process.rs` (DECIDE Path A vs B before coding)

Two spawn sites today:
- `provision_sandbox` (main.rs:~398): creates NEW VM, inserts into
  `state.instances`, sets persistent flag, etc.
- `resume_sandbox` (main.rs:~587): looks up existing persistent VM in
  registry, re-spawns against its session_dir.

They share boilerplate (binary path resolution, Command construction,
child spawn, PID capture) but diverge on intent.

**Path A -- Refactor (default).** Extract
`spawn_capsem_process(binary, args, ...) -> Result<Child>` that owns only
the `Command` boilerplate. Callers keep their distinct orchestration.
Risk: low. Behavior: unchanged. Ship as `refactor(service): dedupe
capsem-process spawn boilerplate`.

**Path B -- Consolidate.** Force both flows through one canonical helper
that owns instance insertion + cleanup-on-exit. Picks a canonical
behavior for the parts that differ; whoever loses ships a behavior
change. Risk: medium. Behavior: changes one site; needs CHANGELOG
`### Changed` entry and clear PR description of what differed.

**Default: Path A.** Pick B only if a concrete bug motivates it. Do not
enter T7 without a written choice in `T7-process.md`.

## Hard constraints (inherit + extend from T0)

- **No behavior changes** to any handler unless a sub-sprint's plan.md
  explicitly calls it out with user sign-off.
- **`api.rs` content stays untouched.** New DTOs go in `api.rs` only if
  they're truly on-the-wire; otherwise define them inside the route
  module.
- **Stage files explicitly.** Do not `git add -A`. The working tree
  routinely contains unrelated in-flight edits from other work.
- **One sub-sprint per commit minimum.** T3 or T5 may warrant 2-3 commits
  at functional milestones.
- **Commit message + CHANGELOG discipline.** Each commit has a matching
  `[Unreleased]` CHANGELOG entry; author `Elie Bursztein <github@elie.net>`;
  no `Co-Authored-By` trailers.
- **Warnings are errors.** `#[deny(warnings)]` is on. Clippy clean per PR.

## Definition of done (whole follow-up)

- [ ] `crates/capsem-service/src/main.rs` under 500 lines, containing
      only: `main()`, `Args`, signal wiring, router construction.
- [ ] Every handler lives in `src/routes/<group>.rs`.
- [ ] `ServiceState`, `PersistentRegistry`, `spawn_capsem_process` all in
      lib.
- [ ] `cargo test -p capsem-service` ≥ 144 + new tests added per
      sub-sprint, 0 failed.
- [ ] `cargo llvm-cov -p capsem-service --summary-only`: ≥ T0 numbers on
      new modules.
- [ ] `just test` green end-to-end (enforced gate once Python suite
      stabilizes).
- [ ] `just run "capsem-doctor"` green.
- [ ] CHANGELOG reflects each sub-sprint in the commit it describes, not
      batched.

## Resume checklist for a fresh session

1. Read `sprints/capsem-service-split/plan.md` (T0 context) and
   `sprints/capsem-service-split/tracker.md` (Discoveries).
2. Read this file (plan.md) and `MASTER.md`.
3. Run `git log --oneline sprints/capsem-service-split-followup/` to see
   what's already been done in this sprint.
4. Check `MASTER.md` status table -- pick the earliest Not-Started
   sub-sprint whose blockers are clear.
5. Verify blockers are resolved:
   - `just test` runs clean locally on a fresh clone.
   - `sprints/mcp-endpoint-coverage/tracker.md` shows the sprint as done.
6. Create `T<N>-<name>.md` for the chosen sub-sprint + `tracker.md`, then
   execute.
