# Sprint: CLI Parity with Docker

## What we're building

Finish the CLI and MCP parity gap: missing flags (`--timeout`, `--tail`, `--quiet`, `--env`), missing commands (`version`, `restart`), and MCP tool parity (`tail`, `version`, `env`). The UX refactor (create=detached, shell=interactive entry) is already done in the working tree.

## Why

Users expect `--env`, `--timeout`, `version`, and scripting-friendly output. The backend already supports most of these -- they just need wiring. The `--env` plumbing is the one cross-crate item that touches service, process, and guest boot.

## Phases

### Phase 1: Commit foundation + quick CLI wins

These are all CLI-only (no backend changes). Service already supports the underlying operations.

1. **Commit the UX refactor** -- 15 files, ~1200 lines of uncommitted work (create=detached, shell=interactive, persistent registry, MCP tools for stop/resume/persist/purge/run)

2. **`--timeout <secs>` on `exec`** -- Expose existing `timeout_secs` in ExecRequest
   - File: `crates/capsem/src/main.rs`

3. **`capsem version`** -- CLI version + service version query
   - File: `crates/capsem/src/main.rs`

4. **`-q` / `--quiet` on `list`** -- Print only IDs (one per line) for scripting
   - Enables: `capsem stop $(capsem ls -q)`
   - File: `crates/capsem/src/main.rs`

5. **`--tail <N>` on `logs`** -- Show last N lines (CLI post-processing)
   - File: `crates/capsem/src/main.rs`

### Phase 2: restart command

6. **`capsem restart <name>`** -- Stop + resume composition for persistent VMs
   - Error if target is ephemeral (destroyed on stop, can't restart)
   - File: `crates/capsem/src/main.rs`

### Phase 3: --env plumbing (cross-crate)

7. **`--env KEY=VALUE` / `-e` on `create`** -- Pass env vars through provision
   - Extend `ProvisionRequest` with `env: Option<HashMap<String, String>>`
   - Service passes `--env KEY=VALUE` args to capsem-process spawn
   - Process injects via `send_boot_config()` (already supports env, MAX_BOOT_ENV_VARS=128)
   - CLI: `-e KEY=VALUE` repeatable flag on `Create`
   - MCP: `env` param on `capsem_create`
   - Files: `crates/capsem-service/src/api.rs`, `crates/capsem-service/src/main.rs`, `crates/capsem-process/src/main.rs`, `crates/capsem/src/main.rs`, `crates/capsem-mcp/src/main.rs`
   - Test: create VM with `--env FOO=bar`, exec `echo $FOO`, verify "bar"

### Phase 4: MCP parity

8. **`tail` param on `capsem_vm_logs` / `capsem_service_logs`** -- Last N lines after grep
   - File: `crates/capsem-mcp/src/main.rs`

9. **`capsem_version` tool** -- MCP server + service version
   - File: `crates/capsem-mcp/src/main.rs`

10. **Update tool descriptions + router test** -- accuracy pass on all tools
    - File: `crates/capsem-mcp/src/main.rs`

### Phase 5: Skills and documentation

11. Update `/dev-capsem` -- detached create model, new flags
12. Update `/dev-mcp` -- new MCP tools/params
13. Update `/site-architecture` -- env plumbing, persistent registry
14. Update `/dev-testing` -- testing patterns

### Phase 6: Testing gate

- Unit tests: all new CLI parsing, DTO serde roundtrips, MCP param roundtrips
- Integration test: `--env` end-to-end
- `just test` passes
- CHANGELOG updated per commit

## Key decisions

- **`cp` deferred to SSH sprint (S10).** File transfer over the current string-based read_file/write_file API is not safe for binary. Will build on SCP when SSH server ships.
- **`--rm` dropped.** The ephemeral/persistent model already handles auto-removal. Unnamed VMs are ephemeral and destroyed on stop.
- **`--follow` on logs deferred.** Needs streaming endpoint or CLI-side polling. Separate effort.
- **`--disk` deferred.** Scratch disk plumbing through service->process->boot is a separate sprint.
- **Phase 1 first, ship, then Phase 2+3.** Don't block quick wins on --env plumbing.
- **MCP tools ship alongside CLI counterparts** (Phase 4 tracks MCP for Phase 1-3 features).

## Files to modify

| File | Changes |
|------|---------|
| `crates/capsem/src/main.rs` | --timeout, version, --quiet, --tail, restart, --env |
| `crates/capsem-service/src/api.rs` | ProvisionRequest.env |
| `crates/capsem-service/src/main.rs` | Pass env to process spawn |
| `crates/capsem-process/src/main.rs` | Accept --env, inject at boot |
| `crates/capsem-mcp/src/main.rs` | env param, tail param, version tool |

## What "done" looks like

- All Phase 1-4 items work end-to-end
- `just test` passes
- `capsem create -n test --env FOO=bar && capsem exec test 'echo $FOO'` prints "bar"
- `capsem list -q` prints IDs only
- `capsem version` shows CLI + service version
- CHANGELOG updated
- Skills updated
