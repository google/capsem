# Sprint: CLI Parity

## Phase 1: Commit foundation + quick CLI wins
- [ ] Commit uncommitted UX refactor (create=detached, shell=interactive, persistent registry)
- [ ] `--timeout` on `exec` -- configurable timeout (default 30s)
- [ ] `capsem version` -- CLI + service version
- [ ] `-q` / `--quiet` on `list` -- ID-only output for scripting
- [ ] `--tail N` on `logs` -- last N lines
- [ ] Testing gate (Phase 1)
- [ ] Changelog + commit (Phase 1)

## Phase 2: restart command
- [ ] `capsem restart <name>` -- stop + resume for persistent VMs
- [ ] Testing gate (Phase 2)
- [ ] Changelog + commit (Phase 2)

## Phase 3: --env plumbing (cross-crate)
- [ ] `ProvisionRequest.env` -- service DTO extension
- [ ] Service passes `--env` to capsem-process spawn
- [ ] capsem-process accepts and injects env at boot
- [ ] CLI `--env KEY=VALUE` / `-e` on `create`
- [ ] MCP `env` param on `capsem_create`
- [ ] Integration test: env visible in guest
- [ ] Testing gate (Phase 3)
- [ ] Changelog + commit (Phase 3)

## Phase 4: MCP parity
- [ ] `tail` param on `capsem_vm_logs` / `capsem_service_logs`
- [ ] `capsem_version` tool
- [ ] Update tool descriptions for all tools
- [ ] Update tool router test (expected count)
- [ ] Testing gate (Phase 4)
- [ ] Changelog + commit (Phase 4)

## Phase 5: Skills update
- [ ] Update `/dev-capsem` skill -- detached create, new flags
- [ ] Update `/dev-mcp` skill -- new tools/params
- [ ] Update `/site-architecture` skill -- env plumbing, registry
- [ ] Update `/dev-testing` skill -- testing patterns

## Deferred to other sprints
- `capsem cp` -- deferred to S10 (SSH sprint), will use SCP for binary safety
- `--disk` on create -- scratch disk plumbing, separate sprint
- `--follow` on logs -- streaming endpoint, separate sprint
- `stats`, `top`, `pause/unpause`, `wait`, `--filter`, `--format` -- low priority
