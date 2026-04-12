# Next-Gen Platform Tracker

Restructured 2026-04-03 after codebase audit. S1 consolidates all foundation work (service, process, CLI, MCP, shell, snapshots, CLI parity, docs).

## Phase A: Foundation (Done)

| Sprint | Name | Status | What shipped |
|--------|------|--------|-------------|
| S1 | Service + Process + CLI + MCP + CLI Parity | Done | Daemon (19 API endpoints), process isolation, CLI (24+ commands), MCP (21 tools), shell, snapshots, persistent VMs, fork images, native installer (setup/update/uninstall/service install), env injection, corp config. Remaining stretch: `cp`, `stats`, `--disk`, `--follow`, `/health`, `flock`, skill/doc updates |

## Phase B: TCP HTTP API

| Sprint | Name | Status | Depends On |
|--------|------|--------|-----------|
| S2 | Localhost HTTP API | Not started | S1 |
| S3 | SSH key auth | Not started | - |
| S4 | Remote HTTP API (mTLS) | Not started | S2, S3 |

## Phase C: VM Lifecycle (Shutdown, Suspend, Identity)

| Sprint | Name | Status | Depends On |
|--------|------|--------|-----------|
| S5+S7 | VM Lifecycle (shutdown, identity, quiescence, Apple VZ suspend/resume) | In progress | S1 |
| S6 | Branch + Rewind | Not started | S5+S7 (quiescence) |
| S8 | Auto-nap + pressure monitoring | Not started | S5+S7 |
| S9 | Linux suspend/resume (KVM ioctls, GIC state, memory dump) | Deferred (~11-15 days) | S5+S7 |

S5+S7 combined into a single meta sprint: `sprints/done/vm-lifecycle/`. Covers guest system binaries (`shutdown`/`halt`/`poweroff`/`suspend`), VM identity injection, quiescence protocol (fsfreeze), Apple VZ pause/save/restore, agent reconnect after restore, and service orchestration. Guest-initiated lifecycle commands flow through the service's existing code paths.

## Phase E: SSH + IDE

| Sprint | Name | Status | Depends On |
|--------|------|--------|-----------|
| S10 | MITM SSH + IDE + file transfer (cp) | Not started | S4 |
| S11 | IDE integration | Not started | S10 |

## Phase F: Menu Bar + Auto-start

| Sprint | Name | Status | Depends On |
|--------|------|--------|-----------|
| S12 | Menu bar (standalone `capsem-tray`) | Not started | S1, gateway |
| S13 | Auto-start + notifications | Not started | S12 |

Sprint tracker: `sprints/tray/tracker.md`
Gateway dependency: `sprints/done/gateway/tracker.md`

## Phase G: Frontend Rebuild

| Sprint | Name | Status | Depends On |
|--------|------|--------|-----------|
| S14 | Frontend rebuild (Preline + gateway + tabs) | Not started | gateway (done) |
| S15 | Frontend views | Not started | S14 |
| S16 | Search (FTS5) | Not started | S14 |

## Phase H: Polish + Post-core

| Sprint | Name | Status | Depends On |
|--------|------|--------|-----------|
| S17 | Enterprise polish | Not started | S10 |
| S18 | Headless renderer | Not started | S14 |
| S19 | Renderer UI preview | Not started | S18 |

## Critical Path

```
S1 (done -- foundation + CLI/MCP parity + fork images + installer)
  |
  +---> S2 (localhost HTTP) ---> S4 (mTLS remote) ---> S10 (SSH) ---> S11 (IDE)
  |                                    ^                                  |
  +---> S3 (SSH key auth) ------------+                                   v
  |                                                                     S17 (enterprise)
  +---> S5+S7 (vm-lifecycle: shutdown, identity, quiescence, Apple VZ suspend/resume)
  |         |
  |         +---> S6 (branch/rewind)
  |         +---> S8 (auto-nap)
  |         +---> S9 (Linux suspend, deferred)
  |
  +---> S12 (menu bar) ---> S13 (auto-start)
  |
  +---> S2 ---> S14 (frontend) ---> S15 (views)
                    |                    |
                    +---> S16 (search)   +---> S18 (renderer) ---> S19 (preview)
```

Parallel tracks after S1: S2+S3, S5+S7, S12 can all start independently.

## Spike (Complete)

**Checkpoint/Restore** (2026-03-30, branch spike/checkpoint-restore): Apple VZ save/restore proven (730ms round-trip, 54MB for 2GB VM). Quiescence via fsfreeze validated. Informs S9-S13.

## Reference

- Original plan: `sprints/next-gen/plan.md`
- S1 detailed tracker: `sprints/next-gen/sprint-01/tracker.md`
- S5+S7 (vm-lifecycle) sprint: `sprints/done/vm-lifecycle/MASTER.md`
- Spike results: `sprints/next-gen/spike-checkpoint/results.md`
- Testing master: `sprints/done/testing/MASTER.md` (25 test sub-sprints, all done)
- Frontend rebuild: `sprints/frontend-rebuild/tracker.md` (S14 -- Preline + gateway + tabs)
- Fork images sprint: `sprints/done/fork-images/tracker.md`
- Native installer sprint: `sprints/native-installer/tracker.md`
