# Sprint: capsem-service main.rs split (T1+ follow-up)

> **Status: blocked, do not start.**
> Two prerequisites must land before this sprint executes -- see "Blockers"
> below.

## Why now (and why not yet)

T0 (`sprints/capsem-service-split/`) landed in commit `4bc7827`. It added
`crates/capsem-service/src/lib.rs` and extracted three pure-helper clusters
(`errors.rs`, `fs_utils.rs`, `naming.rs`) without touching handlers,
`ServiceState`, or `PersistentRegistry`. `main.rs` is still ~4.3k lines,
still owns every `handle_*`, and still owns the persistent registry +
service state.

The T0 plan flagged this follow-up as out of scope for two reasons:

1. Moving handlers requires `ServiceState`'s currently-private fields to
   become `pub(crate)` -- that's a real surface change, not a mechanical
   refactor.
2. The MCP-handler subset can't move while `sprints/mcp-endpoint-coverage/`
   is paused mid-flight against that same surface.

The user added a third reason on 2026-04-20: the Python integration test
suite is unstable (`just test` shows 37 failures unrelated to T0's
refactor). The follow-up sprint requires those tests as a regression net
when handlers move; without them, "no behavior change" is unverifiable.

## Blockers (resolve in this order)

1. **Stabilize the Python integration suite.** `just test` must run green
   end-to-end. T0 verified that the failures are not caused by the T0
   refactor (spot-checked with `tests/capsem-service/test_svc_fork.py`
   passing 3/3 in isolation), but the suite as a whole needs its own
   stabilization sprint before this one starts. Probable causes to
   investigate: parallel-execution races in `pytest -n`, gateway/guest
   start-up flakes, leaked test sandboxes between runs. Tracked separately.
2. **`sprints/mcp-endpoint-coverage/` must complete.** That sprint owns
   the MCP handler surface and is mid-flight. Doing T5 (`mcp.rs` extraction
   below) before it finishes will fight merge conflicts on every test it
   adds. When mcp-endpoint-coverage lands, it should be the consumer of
   `routes/mcp.rs`, not the cause of its creation.

## What this sprint does

Convert `crates/capsem-service/src/main.rs` from a 4.3k-line monolith into
a thin entry point that delegates to route-grouped modules under
`src/routes/`. Cluster handlers by domain, not alphabetically. Move
`PersistentRegistry` and `ServiceState` into the library so handlers can
live anywhere in the crate.

Sub-sprints (each its own `T<N>-<name>.md` per `/dev-sprint`'s meta-sprint
convention):

| Sub | Topic | Files added | Notes |
|-----|-------|-------------|-------|
| T1 | `registry.rs` | `src/registry.rs` -- `PersistentVmEntry`, `PersistentRegistryData`, `PersistentRegistry` and its tests | Smallest, isolated. Good first move. |
| T2 | `ServiceState` to lib | edits `lib.rs` and `main.rs` | Surface change: many fields become `pub(crate)`. Nothing else can move until this is done. |
| T3 | `routes/files.rs` | `src/routes/files.rs` (handle_list_files, handle_download_file, handle_upload_file) and `resolve_workspace_path` | First handler-relocation; sets the route-module pattern. |
| T4 | `routes/images.rs` | `handle_fork` (and any peer image-management handlers) | Smaller surface than files. |
| T5 | `routes/mcp.rs` | `handle_mcp_servers`, `handle_mcp_tools`, `handle_mcp_policy`, `handle_mcp_refresh`, `handle_mcp_approve`, `handle_mcp_call` | **Blocked on `sprints/mcp-endpoint-coverage/`.** |
| T6 | `routes/history.rs` | `handle_history`, `handle_history_processes`, `handle_history_counts`, `handle_history_transcript` | Smaller, isolated. |
| T7 | `process.rs` | shared `spawn_capsem_process` + `wait_for_process_exit` helpers | **NOT a routine refactor** -- see "Process dedup" below. |

## Hard constraints (carry forward from T0)

- **No behavior changes** to any handler unless the sub-sprint's plan.md
  explicitly calls them out and the user signs off.
- **Stage files explicitly** -- no `git add -A`, no auto-staging of
  unrelated working-tree edits (see T0's `git status` lessons).
- **One sub-sprint per commit minimum.** Larger sub-sprints (T3, T5) may
  warrant 2-3 commits each at functional milestones.
- **`api.rs` content stays untouched.** If a handler needs a new DTO,
  decide whether it belongs in `api.rs` (public on-the-wire shape) or in
  the route module (internal). Default to route-local.

## Process dedup -- decide first, code second

The user proposed "deduplicate the function for spawning capsem-process".
There are currently two spawn sites:

- `provision_sandbox` (`main.rs` line ~398): creates a NEW VM, inserts into
  `state.instances`, sets `persistent` flag from caller, etc.
- `resume_sandbox` (`main.rs` line ~587): looks up an existing persistent
  VM in the registry, re-spawns capsem-process against its existing
  session_dir.

These are NOT semantically identical -- they share boilerplate but diverge
on intent. Two paths forward, **pick one before T7 starts:**

- **Path A: Refactor.** Extract a shared `spawn_capsem_process(args) ->
  Result<Child>` helper that takes the boilerplate (binary path, args,
  child setup) and leave the surrounding orchestration in place. Both
  callers stay structurally distinct; only the spawn block dedups.
  Risk: low. Behavior: unchanged.
- **Path B: Consolidate.** Force both flows through one canonical helper
  that owns instance insertion + cleanup-on-exit. Picking the canonical
  behavior means picking either provision's or resume's semantics for the
  parts that differ. Risk: medium. Behavior: changes for whichever site
  loses; document in CHANGELOG `### Changed`.

Default: **Path A.** Path B only if a concrete bug motivates it.

`wait_for_process_exit` is already a single function (`main.rs:2088`); no
dedup work needed.

## Out of scope even for this follow-up

- Replacing the bin-side inline tests with `crates/capsem-service/tests/`
  integration tests. That's a third sprint -- best done after handlers
  have stabilized in their new homes.
- Splitting `api.rs` itself.
- Touching `crates/capsem-mcp/`, `crates/capsem-process/`, or any other
  crate.

## Definition of done

- [ ] Every sub-sprint listed above has its own `T<N>-<name>.md` and a
      committed implementation.
- [ ] `crates/capsem-service/src/main.rs` is under 500 lines and contains
      only: `main()`, `Args`, router construction, top-level signal
      wiring.
- [ ] `cargo test -p capsem-service` >= 133 (T0 baseline) plus new tests
      added per sub-sprint.
- [ ] `just test` green end-to-end.
- [ ] `just run "capsem-doctor"` green.
- [ ] CHANGELOG has one `### Changed` entry per sub-sprint, in the commit
      it describes.
