# Next-Gen Platform Tracker

Restructured 2026-04-03 after codebase audit. S1 consolidates all foundation work (service, process, CLI, MCP, shell, snapshots, CLI parity, docs).

## Phase A: Foundation (In progress)

| Sprint | Name | Status | What shipped |
|--------|------|--------|-------------|
| S1 | Service + Process + CLI + MCP + CLI Parity | In progress | Core platform shipped (daemon, process, CLI, MCP 11 tools, shell, snapshots). Now: CLI parity with Docker, MCP parity, skills, docs, tests, foundation gaps |

## Phase B: TCP HTTP API

| Sprint | Name | Status | Depends On |
|--------|------|--------|-----------|
| S2 | Localhost HTTP API | Not started | S1 |
| S3 | SSH key auth | Not started | - |
| S4 | Remote HTTP API (mTLS) | Not started | S2, S3 |

## Phase C: Quiescence + Branch/Rewind

| Sprint | Name | Status | Depends On |
|--------|------|--------|-----------|
| S5 | Quiescence protocol | Not started | S1 |
| S6 | Branch + Rewind | Not started | S1, S5 |

## Phase D: Suspend/Resume

| Sprint | Name | Status | Depends On |
|--------|------|--------|-----------|
| S7 | Apple VZ suspend/resume | Not started | S5 |
| S8 | Auto-nap + pressure monitoring | Not started | S7 |
| S9 | Linux suspend/resume | Deferred | S5 |

## Phase E: SSH + IDE

| Sprint | Name | Status | Depends On |
|--------|------|--------|-----------|
| S10 | MITM SSH + guest SSH server | Not started | S4 |
| S11 | IDE integration | Not started | S10 |

## Phase F: Menu Bar + Auto-start

| Sprint | Name | Status | Depends On |
|--------|------|--------|-----------|
| S12 | Menu bar | Not started | S1 |
| S13 | Auto-start + notifications | Not started | S12 |

## Phase G: Frontend Rebuild

| Sprint | Name | Status | Depends On |
|--------|------|--------|-----------|
| S14 | Frontend foundation | Not started | S2 |
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
S1 (in progress -- foundation + CLI/MCP parity + docs)
  |
  +---> S2 (localhost HTTP) ---> S4 (mTLS remote) ---> S10 (SSH) ---> S11 (IDE)
  |                                    ^                                  |
  +---> S3 (SSH key auth) ------------+                                   v
  |                                                                     S17 (enterprise)
  +---> S5 (quiescence) ---> S6 (branch/rewind)
  |         |
  |         +---> S7 (suspend) ---> S8 (auto-nap)
  |         |
  |         +---> S9 (Linux suspend, deferred)
  |
  +---> S12 (menu bar) ---> S13 (auto-start)
  |
  +---> S2 ---> S14 (frontend) ---> S15 (views)
                    |                    |
                    +---> S16 (search)   +---> S18 (renderer) ---> S19 (preview)
```

Parallel tracks after S1: S2+S3, S5, S12 can all start independently.

## Spike (Complete)

**Checkpoint/Restore** (2026-03-30, branch spike/checkpoint-restore): Apple VZ save/restore proven (730ms round-trip, 54MB for 2GB VM). Quiescence via fsfreeze validated. Informs S9-S13.

## Reference

- Original plan: `sprints/next-gen/plan.md`
- Spike results: `sprints/next-gen/spike-checkpoint/results.md`
- Testing master: `sprints/testing/MASTER.md` (25 test sub-sprints, all done)
