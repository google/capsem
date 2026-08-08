# Next-Gen Platform Roadmap

What's been shipped, what's left, and what order it goes in.

## Shipped

| What | Sprint | Key deliverables |
|------|--------|-----------------|
| Foundation (S1) | `done/cli-parity`, `done/fork-images`, `done/image-simplification` | Daemon (19 endpoints), process isolation, CLI (24+ commands), MCP (21 tools), shell, snapshots, persistent VMs, fork-as-sandbox, env injection |
| Native installer | `native-installer` (~85%) | Setup wizard, service install/uninstall, self-update, corp config, completions, uninstall. Remaining: binary swap + Docker e2e gate |
| VM lifecycle | `done/vm-lifecycle` | Guest shutdown/halt/poweroff/suspend, VM identity injection, quiescence (fsfreeze), Apple VZ pause/save/restore, agent reconnect, CLI + MCP tools |
| TCP gateway | `done/gateway` | capsem-gateway crate, Bearer auth, UDS proxy, /status aggregation, CORS, WebSocket terminal bridge, 95 tests |
| Menu bar tray | `tray` (code complete) | capsem-tray crate, gateway polling, Permanent/Temporary VM sections, context-sensitive actions, icon states, 47 tests. Needs integration smoke test. |
| Frontend rebuild | `frontend-rebuild` (sprints 01-05 done) | Preline + Svelte 5 + Tailwind v4 browser shell. Tab system, all views (terminal, logs, files, inspector, settings, stats, overview), gateway wiring. Remaining: polish/a11y (sprint 06) + CI/ship (sprint 07) |
| Testing | `done/testing`, `done/testing-ci-coverage` | 25 test sub-sprints (T0-T25), capsem-process modularization (24->62 tests), CI expansion (+422 tests), 70% coverage floor |
| Security | `done/symlink-security` | Guest I/O path hardening (validate + O_NOFOLLOW), snapshot symlink preservation |

## In progress

| What | Sprint | Effort | What's left |
|------|--------|--------|-------------|
| Native installer | `native-installer` | S | `run_update()` binary swap, Docker e2e verification, testing gate |
| Frontend polish + ship | `frontend-rebuild` (06-07) | S | Keyboard shortcuts, animations, a11y, responsive, CI pipeline, production build |
| Tray integration smoke | `tray` | XS | Manual verification with running gateway+service |

## Remaining roadmap

Ordered by dependency chain. Items without dependencies can run in parallel.

### Near-term (no new architecture)

| What | Sprint | Effort | Depends on | Notes |
|------|--------|--------|------------|-------|
| Install lifecycle | `install-lifecycle` | M | native-installer, tray | `just install` deploys all 6 binaries, graceful restart, post-install health check |
| Tray-UI integration | `tray-ui-integration` | S | tray, frontend | Tauri accepts `--connect`/`--new-named` from tray menu items |
| Tauri shell rewrite | `tauri-shell` | L | frontend, native-installer | New capsem-app from scratch, settings + MCP service endpoints, WebSocket events |
| Tray notifications | `tray-notifications` | M | tray | macOS native notifications, badge dot, pulse animation, ack flow |

### Mid-term (new capabilities)

| What | Sprint | Effort | Depends on | Notes |
|------|--------|--------|------------|-------|
| Branch + rewind | -- | M | vm-lifecycle (done) | Disk-only ops via quiescence + reflink copy. Named snapshots. |
| Auto-nap | -- | L | vm-lifecycle (done) | Memory pressure monitoring, idle detection, auto-suspend/resume |
| SSH key auth | -- | M | -- | SSH key loading, mTLS with SSH-derived certs |
| Remote HTTP API | -- | M | gateway (done), SSH key auth | mTLS listener on capsem-gateway for remote access |
| FTS5 search | -- | L | -- | Per-session FTS5 index, cross-session ATTACH search, UI search bar |

### Long-term (new subsystems)

| What | Sprint | Effort | Depends on | Notes |
|------|--------|--------|------------|-------|
| MITM SSH + IDE | -- | XL | remote HTTP API | Guest openssh, russh server, VM routing, session recording, VS Code extension, `capsem cp` |
| Forensics | `forensics` | XL | -- | API namespace restructure, process logs to SQLite, FTS5, forensic CLI, adversarial tests |
| Telemetry | `telemetry` | XL | -- | New crate (capsem-telemetry), OTLP export, per-VM instrumentation, service metrics |
| Linux suspend | -- | L | vm-lifecycle (done) | KVM ioctls for vCPU/GIC/memory snapshot, ~11-15 days |
| Enterprise polish | -- | L | SSH + IDE | Audit logging, OpenTelemetry, corp manifest enforcement, sandbox profiles |
| Headless renderer | -- | XL | frontend | Per-VM browser subprocess, MCP tools for navigation/screenshot/DOM/JS |
| Renderer UI preview | -- | M | headless renderer | Screenshot mode + sandboxed live preview in Tauri |

## Critical path

```
[shipped: foundation, gateway, vm-lifecycle, tray, frontend 01-05]
  |
  +-- frontend 06-07 (S) --> tauri-shell (L) --> tray-UI integration (S)
  |                                |
  |                                +--> install-lifecycle (M)
  |
  +-- SSH key auth (M) --> remote HTTP API (M) --> MITM SSH + IDE (XL) --> enterprise (L)
  |
  +-- branch + rewind (M) ----+
  |                            |
  +-- auto-nap (L) -----------+-- (both depend on vm-lifecycle, already done)
  |
  +-- FTS5 search (L) -- standalone, no deps
  +-- forensics (XL) -- standalone, no deps
  +-- telemetry (XL) -- standalone, no deps
```

## Reference

- Original plan: `sprints/next-gen/plan.md`
- S1 detailed tracker: `sprints/next-gen/sprint-01/tracker.md`
- Spike results: `sprints/next-gen/spike-checkpoint/results.md`

**Size key:** XS = ~half day, S = 1-2 days, M = 3-5 days, L = 1-2 weeks, XL = 2-4 weeks
