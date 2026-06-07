# Sprint 1 Tracker: Service + Process + CLI + MCP + CLI Parity

## Core platform (commit 0c6cd8d, 2026-04-02)
- [x] Task 1: Refactor `capsem-core` (Move shared logic)
  - boot.rs, terminal.rs, registry.rs in capsem-core/src/vm/
- [x] Task 2: Create `capsem-process` crate (Sandbox boundary)
  - VM boot on main thread, UDS listener, job store, terminal I/O, auto-snapshot timer
- [x] Task 3: Create `capsem-service` crate (Orchestrator daemon)
  - UDS API (19 endpoints), instance tracking, stale PID cleanup, max_concurrent_vms
- [x] Task 4: Create `capsem` CLI crate (Thin wrapper)
  - 24+ commands (see full list below)
- [x] Task 5: Service spawns capsem-process on provision
  - Fork/exec with unique ID, session dir, asset paths
- [x] Task 6: UDS API for service + per-VM UDS in process
  - Service on ~/.capsem/run/service.sock, process on instances/{id}.sock
- [x] Task 7: Recovery loop (stale PID cleanup via kill(pid, 0))
  - Runs on every list/info/provision request, removes dead instances

### Shipped beyond original scope
- Exec path (CLI -> service -> process -> guest) with job correlation
- File read/write via IPC
- Config reload (POST /reload-config -> ReloadConfig IPC)
- Logs endpoint (GET /logs/{id})
- Inspect endpoint (POST /inspect/{id} -> SQL on session.db)
- Auto-remove flag on instances
- Multi-version asset manager
- Persistent VM registry (named VMs survive stop/resume)
- `capsem run` -- fire-and-forget in temp VM
- `capsem persist` -- convert ephemeral to persistent
- `capsem purge` -- bulk cleanup

## MCP polish (2026-04-04)
- [x] Enrich all tool descriptions (return format, defaults, timeouts)
- [x] Add `timeout` param to `capsem_exec` (maps to service `timeout_secs`)
- [x] Add `grep` param to `capsem_vm_logs` and `capsem_service_logs`
- [x] Extract `build_exec_body` and `grep_log_fields` as testable helpers
- [x] Fix stale test name (`tool_router_registers_all_nine` -> `all_tools`)
- [x] Add missing tools to router test
- [x] 57 unit tests passing (was 24)

## CLI parity (commit 5456ee4, 2026-04-05)
- [x] `capsem logs <id>` -- serial log viewer with `--tail`
- [x] `--env KEY=VALUE` on `create` -- env var injection via `-e`/`--env` (repeatable)
- [x] `--rm` on `start` -- not needed, ephemeral-by-default design is better
- [x] `--timeout <secs>` on `exec` -- `--timeout` flag, default 30s
- [x] `capsem version` -- CLI + service versions
- [x] `-q` / `--quiet` on `list` -- ID-only output for scripting
- [x] `capsem restart <name>` -- stop + resume for persistent VMs
- [ ] `capsem logs --follow` -- streaming log follow (not yet, only `--tail`)
- [ ] `capsem cp` -- bidirectional file copy (read_file/write_file exist via API/MCP but no `cp` CLI)
- [ ] `--disk <GB>` on `start` -- scratch disk size override
- [ ] `capsem stats <id>` -- resource usage display

### CLI parity: Phase 3 (stretch, deferred)
- [ ] `capsem top <id>`
- [ ] `capsem pause` / `capsem unpause` -- moved to vm-lifecycle sprint (S5+S7)
- [ ] `capsem suspend` / warm `capsem resume` -- moved to vm-lifecycle sprint (S5+S7)
- [ ] `capsem wait <id>`
- [ ] `capsem system info` / `capsem system df`
- [ ] `--filter` on `list`
- [ ] `--format` on `list`/`info`

## MCP parity (21 tools, up from 9)
- [x] `env` param on `capsem_create`
- [x] `autoRemove` -- ephemeral-by-default, no param needed
- [x] `capsem_version` tool
- [x] `capsem_purge` tool
- [x] `capsem_persist` tool
- [x] `capsem_run` tool
- [x] `capsem_resume` tool
- [x] Update tool descriptions for all tools
- [ ] `capsem_restart` tool (CLI has restart, MCP does not)
- [ ] `capsem_stats` tool
- [ ] `diskGb` param on `capsem_create`
- [ ] `name`/`status` filter params on `capsem_list`

## Fork images (separate sprint, 2026-04-06)
- [x] `capsem fork <id> <name>` -- fork running/stopped VM into reusable image
- [x] `capsem image list/inspect/delete` -- image CRUD
- [x] ImageRegistry in capsem-core, session schema v5
- [x] `capsem_fork`, `capsem_image_list`, `capsem_image_inspect`, `capsem_image_delete` MCP tools
- [x] Service endpoints: POST /fork/{id}, GET /images, GET/DELETE /images/{name}
- [x] Boot from image: `capsem create --image <name>`

## Native installer (separate sprint, 2026-04-06-07)
- [x] `capsem service install/uninstall/status` -- launchd integration
- [x] `capsem setup` -- interactive wizard (presets, corp config, credential detection)
- [x] `capsem update` -- self-update with asset vacuum
- [x] `capsem uninstall` -- full cleanup
- [x] `capsem completions` -- shell completions (bash, zsh, fish)
- [x] CLI auto-launches service on first command
- [x] Remote manifest fetch + background asset download
- [x] Corp config provisioning from URL or file path
- [x] Docker-based e2e install test harness

## Service API (19 endpoints)
POST /provision, GET /list, GET /info/{id}, GET /logs/{id}, POST /inspect/{id},
POST /exec/{id}, POST /write_file/{id}, POST /read_file/{id}, POST /stop/{id},
DELETE /delete/{id}, POST /resume/{name}, POST /persist/{id}, POST /purge,
POST /run, POST /reload-config, POST /fork/{id}, GET /images,
GET /images/{name}, DELETE /images/{name}

## CLI commands (24+)
create/start, fork, image (list/inspect/delete), resume/attach, stop, shell,
list/ls, status, exec, run, delete/rm, persist, purge, info, logs, restart,
version, doctor, service (install/uninstall/status), completions, uninstall,
update, setup

## Skills and documentation
- [ ] Update `/dev-mcp` skill -- new tools, params, patterns
- [ ] Update `/dev-capsem` skill -- new CLI commands in overview
- [ ] Update `/site-architecture` skill -- new endpoints, IPC, env plumbing
- [ ] Update `/dev-testing` skill -- testing patterns for new tools/commands
- [ ] Site doc: CLI reference page
- [ ] Site doc: MCP tools reference page
- [ ] Site doc: Docker migration guide

## Foundation gaps (deferred)
- [ ] /health endpoint with resource headroom
- [ ] flock() on state files
- [ ] Conventional commit enforcement (commitlint or CI regex check on PR title)

## Tray integration (from `sprints/tray/tracker.md`)
- [ ] capsem-service spawns `capsem-tray` as child process on startup (after gateway is ready with token + port files)
- [ ] Kill capsem-tray on service shutdown

## Notes
- S1 scope grew significantly beyond original plan (was: boot one VM, now: full platform)
- Fork images and native installer shipped as separate sprints but are S1-era work
- MCP grew from 9 tools (original) to 21 tools
- Service grew from 10 endpoints to 19 endpoints
- CLI grew from 9 commands to 24+ commands
- Phase 3 CLI parity items deferred -- stretch goals, not blocking S2
