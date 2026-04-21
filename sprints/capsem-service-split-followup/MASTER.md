# MASTER: capsem-service main.rs split (follow-up)

> **Blocked.** Do not start until both blockers in `plan.md` clear.
> Enter through `plan.md`, which is self-contained for a fresh session.

## Status table

| Sub | Topic | Status | Depends on | Blocked by (external) |
|-----|-------|--------|------------|-----------------------|
| T1 | `src/registry.rs` (PersistentRegistry) | Not started | — | Python suite stable |
| T2 | `ServiceState` → lib | Not started | T1 | Python suite stable |
| T3 | `src/routes/files.rs` | Not started | T2 | Python suite stable |
| T4 | `src/routes/images.rs` (handle_fork) | Not started | T2 | Python suite stable |
| T6 | `src/routes/history.rs` | Not started | T2 | Python suite stable |
| T5 | `src/routes/mcp.rs` | Not started | T2 | **`sprints/mcp-endpoint-coverage/` complete** + Python suite stable |
| T7 | `src/process.rs` (spawn dedup) | Not started | T2 + Path A/B decision in `T7-process.md` | Python suite stable |

## Phase groupings

- **Phase 1 -- Foundations:** T1 → T2. Unlocks everything else.
- **Phase 2 -- Route modules:** T3, T4, T6 (any order; each an isolated
  PR). Runs after T2.
- **Phase 3 -- Blocked work:** T5 (waits on mcp-endpoint-coverage), T7
  (waits on Path A vs Path B written decision).

## Baseline at sprint scaffolding

- T0 shipped in commit `4bc7827`.
- `cargo test -p capsem-service`: 144 passing (62 lib + 82 main).
- `main.rs`: 4,855 lines (still monolithic for handlers).

## Just recipes

```bash
just test                                   # full workspace gate
just run "capsem-doctor"                    # VM smoke
cargo test -p capsem-service                # crate-only fast loop
cargo clippy -p capsem-service --all-targets -- -D warnings
cargo llvm-cov -p capsem-service --summary-only
```

## Start conditions

Start when **both** are true:

1. `just test` green end-to-end locally on `next-gen` for at least one
   consecutive run without retries.
2. `sprints/mcp-endpoint-coverage/` either complete (tracker shows done)
   or has explicitly handed off the MCP handler surface to this sprint.

When you start: create `tracker.md` (active sub-sprint) and
`T<N>-<name>.md` (sub-sprint plan) in this directory, then execute. Keep
`MASTER.md` status in sync as sub-sprints land.
