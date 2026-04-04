# Sprint: CLI Parity with Docker

## Phase 1: Wire existing API to CLI
- [ ] `capsem logs <id>` -- serial log viewer with `--follow`, `--tail`
- [ ] `capsem cp` -- bidirectional file copy (id:path <-> local)
- [ ] `--env KEY=VALUE` on `start` -- env var injection via provision
- [ ] `--rm` on `start` -- auto-remove flag
- [ ] `--timeout <secs>` on `exec` -- configurable timeout
- [ ] `capsem version` -- CLI + service + asset versions
- [ ] Testing gate (Phase 1)
- [ ] Changelog + commit (Phase 1)

## Phase 2: New capabilities
- [ ] `-q` / `--quiet` on `list` -- ID-only output for scripting
- [ ] `capsem prune` -- clean terminated sessions
- [ ] `--disk <GB>` on `start` -- scratch disk size override
- [ ] `capsem restart <id>` -- stop + re-provision with same config
- [ ] `capsem stats <id>` -- resource usage display
- [ ] Testing gate (Phase 2)
- [ ] Changelog + commit (Phase 2)

## Phase 3: Nice-to-have (stretch)
- [ ] `capsem top <id>`
- [ ] `capsem pause` / `capsem unpause`
- [ ] `capsem wait <id>`
- [ ] `capsem system info` / `capsem system df`
- [ ] `--filter` on `list`
- [ ] `--format` on `list`/`info`

## Phase 4: MCP parity
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
- [ ] Testing gate (Phase 4)
- [ ] Changelog + commit (Phase 4)

## Phase 5: Skills and documentation
- [ ] Update `/dev-mcp` skill -- new tools, params, patterns
- [ ] Update `/dev-capsem` skill -- new CLI commands in overview
- [ ] Update `/site-architecture` skill -- new endpoints, IPC, env plumbing
- [ ] Update `/dev-testing` skill -- testing patterns for new tools/commands
- [ ] Site doc: CLI reference page (`site/src/content/docs/reference/cli.mdx`)
- [ ] Site doc: MCP tools reference page (`site/src/content/docs/reference/mcp-tools.mdx`)
- [ ] Site doc: Docker migration guide (`site/src/content/docs/guides/docker-migration.mdx`)

## Phase 6: Testing
- [ ] Unit tests for all new MCP params (serde roundtrips, edge cases, body construction)
- [ ] Unit tests for new CLI commands (clap parsing, flag combos)
- [ ] Integration tests: CLI -> service -> VM for each new command
- [ ] MCP tool router test -- update tool count as tools land
- [ ] Security edge cases for new params (path traversal in cp, env injection, huge disk)

## Notes
- Logs, cp, --rm, --timeout are pure CLI wiring -- backend exists
- --env needs ProvisionRequest extension + IPC plumbing
- Stats can start as exec-based (guest /proc), upgrade to host-side later
- MCP Phase 4 items should ship alongside their CLI counterparts, not after
- Phase 5 skills + docs can batch at the end once all features are stable
- Phase 6 tests follow each feature -- don't defer testing to the end
