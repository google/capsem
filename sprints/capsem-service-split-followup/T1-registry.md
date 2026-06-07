# T1 — `capsem-service::registry` (PersistentRegistry extraction)

Status: **Done**.

## What shipped

- New `crates/capsem-service/src/registry.rs` (329 lines, incl. 14 tests)
  holds `PersistentVmEntry`, `PersistentRegistryData`, `PersistentRegistry`
  and its impl (`load`, `save`, `register`, `unregister`, `get`, `get_mut`,
  `list`, `contains`).
- `lib.rs` gains `pub mod registry;` so the type is importable from the
  binary crate and from the forthcoming T2 `ServiceState` move.
- `main.rs` loses the three type definitions and impl (~83 lines), imports
  via `use capsem_service::registry::{PersistentRegistry, PersistentVmEntry};`,
  and drops its `Serialize` import (the only remaining `serde` user in
  `main.rs` is `Deserialize` on query structs).
- Seven registry-only tests moved to `registry::tests` (serde roundtrip,
  roundtrip on disk, duplicate rejection, unregister, `get_mut`, suspended
  flag roundtrip, suspended flag default-on-missing, resume-clears-suspended).
- Seven new tests lift coverage: corrupt-JSON on load, missing-file on
  load, `get` / `get_mut` / `contains` miss paths, `list` iterates all
  entries, `save` writes atomically via temp+rename.
- Moved tests switched from `std::env::temp_dir().join("...")` + manual
  cleanup to `tempfile::TempDir` (already a dev-dep) — eliminates the
  cross-run collision risk the old style had.

## Numbers

| | Before | After |
|---|---|---|
| `main.rs` lines | 4,855 | 4,563 |
| `registry.rs` lines | — | 329 |
| `cargo test -p capsem-service` | 144 pass (62 lib + 82 main) | 151 pass (76 lib + 75 main) |
| `registry.rs` line coverage | — | **100%** (207/207) |
| `registry.rs` region coverage | — | 98.53% |
| Clippy (`-- -D warnings`) | clean | clean |

Sprint floor was ≥ 90% line coverage on `registry.rs` — cleared by 10pp.

## Surprises / decisions worth noting

1. **Cross-crate visibility.** `main.rs` is a separate crate from the
   library (`capsem_service`), so `pub(crate)` on library items is not
   visible from `main.rs`. All methods on `PersistentRegistry` and every
   field on `PersistentVmEntry` had to be `pub`. Kept `PersistentRegistry::data`
   and `PersistentRegistryData::vms` `pub` because handler tests in
   `main.rs` inject entries directly via `reg.data.vms.insert(...)` — a
   seam helper would have cost more than it bought.
2. **Python-suite blocker treated as not applicable.** The sprint's
   `plan.md` lists "Python integration suite stable" as a prereq for T1.
   That blocker protects *behavior-changing* sub-sprints (T3–T7). T1
   moves no handlers and changes no wire-level behavior, so
   `cargo test -p capsem-service` green was the relevant gate. Recorded
   this reasoning in the sprint-plan context section and in the commit
   body so T2 doesn't re-question it.
3. **Test hygiene upgrade piggybacked on the move.** Every moved test
   previously raced on shared `/tmp/capsem-test-registry-*` paths. The
   `tempfile::TempDir` swap closes that race for free and is not a
   behavior change; noted in the CHANGELOG.

## Follow-on

- **T2 (next):** Move `ServiceState`, `InstanceInfo`, `ProvisionOptions`
  into the library. Now unblocked — `PersistentRegistry` is already
  library-resident, which was the key dependency.
- T3/T4/T6/T5/T7 remain blocked per `MASTER.md` on Python-suite stability
  (T3/T4/T6), `mcp-endpoint-coverage` completion (T5), and the Path A/B
  decision (T7).

## Commit

`refactor(service): move PersistentRegistry into registry module`
— touches `crates/capsem-service/src/{lib.rs,main.rs,registry.rs}`,
`CHANGELOG.md`, `sprints/capsem-service-split-followup/MASTER.md`,
`sprints/capsem-service-split-followup/T1-registry.md`.
