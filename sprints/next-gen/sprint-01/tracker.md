# Sprint 1 Tracker: Service + Process + CLI + MCP + CLI Parity

## Core platform (commit 0c6cd8d, 2026-04-02)
- [x] Task 1: Refactor `capsem-core` (Move shared logic)
  - boot.rs, terminal.rs, registry.rs in capsem-core/src/vm/
- [x] Task 2: Create `capsem-process` crate (Sandbox boundary)
  - VM boot on main thread, UDS listener, job store, terminal I/O, auto-snapshot timer
- [x] Task 3: Create `capsem-service` crate (Orchestrator daemon)
  - UDS API (10 endpoints), instance tracking, stale PID cleanup, max_concurrent_vms
- [x] Task 4: Create `capsem` CLI crate (Thin wrapper)
  - 9 commands: start, stop, shell, list, status, exec, delete, info, doctor
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

## MCP polish (2026-04-04)
- [x] Enrich all 11 tool descriptions (return format, defaults, timeouts)
- [x] Add `timeout` param to `capsem_exec` (maps to service `timeout_secs`)
- [x] Add `grep` param to `capsem_vm_logs` and `capsem_service_logs`
- [x] Extract `build_exec_body` and `grep_log_fields` as testable helpers
- [x] Fix stale test name (`tool_router_registers_all_nine` -> `all_tools`)
- [x] Add missing tools to router test (vm_logs, service_logs -- was 9, now 11)
- [x] 57 unit tests passing (was 24)

## CLI parity: Phase 1 -- Wire existing API to CLI
- [ ] `capsem logs <id>` -- serial log viewer with `--follow`, `--tail`
- [ ] `capsem cp` -- bidirectional file copy (id:path <-> local)
- [ ] `--env KEY=VALUE` on `start` -- env var injection via provision
- [ ] `--rm` on `start` -- auto-remove flag
- [ ] `--timeout <secs>` on `exec` -- configurable timeout
- [ ] `capsem version` -- CLI + service + asset versions
- [ ] Testing gate (Phase 1)
- [ ] Changelog + commit (Phase 1)

## CLI parity: Phase 2 -- New capabilities
- [ ] `-q` / `--quiet` on `list` -- ID-only output for scripting
- [ ] `capsem prune` -- clean terminated sessions
- [ ] `--disk <GB>` on `start` -- scratch disk size override
- [ ] `capsem restart <id>` -- stop + re-provision with same config
- [ ] `capsem stats <id>` -- resource usage display
- [ ] Testing gate (Phase 2)
- [ ] Changelog + commit (Phase 2)

## CLI parity: Phase 3 -- Nice-to-have (stretch)
- [ ] `capsem top <id>`
- [ ] `capsem pause` / `capsem unpause`
- [ ] `capsem wait <id>`
- [ ] `capsem system info` / `capsem system df`
- [ ] `--filter` on `list`
- [ ] `--format` on `list`/`info`

## MCP parity (ships alongside CLI phases)
- [ ] `env` param on `capsem_create`
- [ ] `autoRemove` param on `capsem_create`
- [ ] `tail` param on `capsem_vm_logs` / `capsem_service_logs`
- [ ] `capsem_version` tool
- [ ] `diskGb` param on `capsem_create`
- [ ] `capsem_restart` tool
- [ ] `capsem_stats` tool
- [ ] `capsem_prune` tool
- [ ] `name`/`status` filter params on `capsem_list`
- [ ] Update tool descriptions for all new tools
- [ ] Testing gate (MCP parity)
- [ ] Changelog + commit (MCP parity)

## Skills and documentation
- [ ] Update `/dev-mcp` skill -- new tools, params, patterns
- [ ] Update `/dev-capsem` skill -- new CLI commands in overview
- [ ] Update `/site-architecture` skill -- new endpoints, IPC, env plumbing
- [ ] Update `/dev-testing` skill -- testing patterns for new tools/commands
- [ ] Site doc: CLI reference page (`site/src/content/docs/reference/cli.mdx`)
- [ ] Site doc: MCP tools reference page (`site/src/content/docs/reference/mcp-tools.mdx`)
- [ ] Site doc: Docker migration guide (`site/src/content/docs/guides/docker-migration.mdx`)

## Testing
- [ ] Unit tests for all new MCP params (serde roundtrips, edge cases, body construction)
- [ ] Unit tests for new CLI commands (clap parsing, flag combos)
- [ ] Integration tests: CLI -> service -> VM for each new command
- [ ] MCP tool router test -- update tool count as tools land
- [ ] Security edge cases for new params (path traversal in cp, env injection, huge disk)

## Foundation gaps (was deferred to S5)
- [ ] /health endpoint with resource headroom
- [ ] flock() on state files
