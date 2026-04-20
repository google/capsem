# MASTER: capsem-service main.rs split (follow-up)

> **Blocked.** Do not start until both blockers in `plan.md` clear.

See `plan.md` for full context, blocker rationale, and constraints.

## Status table

| Sub | Topic | Status | Owner | Blocked by |
|-----|-------|--------|-------|------------|
| T1 | `registry.rs` extraction | Not started | — | T2 (needs `ServiceState` accessible) -- can also run before T2 if `PersistentRegistry` is moved standalone first |
| T2 | `ServiceState` -> `lib.rs` | Not started | — | T1 (mechanical), Python suite stabilization |
| T3 | `routes/files.rs` | Not started | — | T2 |
| T4 | `routes/images.rs` | Not started | — | T2 |
| T5 | `routes/mcp.rs` | Not started | — | T2, **`sprints/mcp-endpoint-coverage/` completion** |
| T6 | `routes/history.rs` | Not started | — | T2 |
| T7 | `process.rs` (spawn dedup) | Not started | — | T2 + user decision on Path A vs Path B (see plan.md) |

## Phase groupings

- **Phase 1 -- Foundations.** T1, T2. Land before any route module moves.
- **Phase 2 -- Route modules.** T3, T4, T6 in parallel-safe order. Each is
  one PR.
- **Phase 3 -- Blocked work.** T5 (waits on mcp-endpoint-coverage), T7
  (waits on Path A/B decision).

## Just recipes

```bash
just test                           # full workspace gate (REQUIRED green)
just run "capsem-doctor"            # VM smoke (REQUIRED green)
cargo test -p capsem-service        # crate-only fast loop
cargo llvm-cov -p capsem-service --summary-only   # per-file coverage
```

## When to start

When BOTH of these are true:

1. `just test` is green on `next-gen` for at least one consecutive run
   without retries.
2. `sprints/mcp-endpoint-coverage/` either completes or its tracker
   explicitly hands off the MCP handler surface to this sprint.

When you start, create a `tracker.md` in this directory for the active
sub-sprint and update this MASTER's status table as sub-sprints land.
