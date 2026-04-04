# Sprint: CLI Parity

## Phase 1: Commit foundation + quick CLI wins
- [x] Commit uncommitted UX refactor (create=detached, shell=interactive, persistent registry)
- [x] `--timeout` on `exec` -- configurable timeout (default 30s)
- [x] `capsem version` -- CLI + service version
- [x] `-q` / `--quiet` on `list` -- ID-only output for scripting
- [x] `--tail N` on `logs` -- last N lines
- [x] Testing gate (Phase 1) -- 53 CLI tests pass
- [x] Changelog + commit (Phase 1)

## Phase 2: restart command
- [x] `capsem restart <name>` -- stop + resume for persistent VMs
- [x] Testing gate (Phase 2) -- 53 CLI tests pass
- [x] Changelog + commit (Phase 2)

## Phase 3: --env plumbing (cross-crate)
- [x] `ProvisionRequest.env` -- service DTO extension
- [x] Service passes `--env` to capsem-process spawn
- [x] capsem-process accepts and injects env at boot
- [x] CLI `--env KEY=VALUE` / `-e` on `create`
- [x] MCP `env` param on `capsem_create`
- [x] Integration test: env visible in guest
- [x] Testing gate (Phase 3) -- 181 unit tests pass (53+67+61)
- [x] Changelog + commit (Phase 3)

## Phase 4: MCP parity
- [x] `tail` param on `capsem_vm_logs` / `capsem_service_logs`
- [x] `capsem_version` tool
- [x] Update tool descriptions for all tools
- [x] Update tool router test (expected count: 17)
- [x] Testing gate (Phase 4) -- 67 MCP tests pass
- [x] Changelog + commit (Phase 4)

## Phase 5: Skills update
- [x] Update `/dev-capsem` skill -- detached create, new flags
- [x] Update `/dev-mcp` skill -- new tools/params
- [x] Update `/site-architecture` skill -- env plumbing, registry
- [x] Update `/dev-testing` skill -- testing patterns (no changes needed)

## Deferred to other sprints
- `capsem cp` -- deferred to S10 (SSH sprint), will use SCP for binary safety
- `--disk` on create -- scratch disk plumbing, separate sprint
- `--follow` on logs -- streaming endpoint, separate sprint
- `stats`, `top`, `pause/unpause`, `wait`, `--filter`, `--format` -- low priority
