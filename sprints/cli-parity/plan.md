# Sprint: CLI Parity with Docker

## What we're building

Capsem's CLI is functional but thin. Several capabilities already exist in the service API or boot protocol but aren't wired to CLI flags. Others are missing entirely. This sprint adds the most impactful Docker-equivalent commands and flags to make capsem feel complete for daily use and scripting.

## Why

Users coming from Docker expect `logs`, `cp`, `--env`, `--rm`, `--timeout`, and `version`. Without these, they fall back to raw API calls or awkward workarounds. The Tier 1 items are low-effort because the backend already supports them.

## Phases

### Phase 1: Wire existing API to CLI (Tier 1 -- quick wins)

These all have backend support. Just need clap args + HTTP client calls.

1. **`capsem logs <id>`** -- Wire `GET /logs/{id}` to CLI
   - Flags: `--follow` / `-f`, `--tail <N>`
   - File: `crates/capsem/src/main.rs`

2. **`capsem cp`** -- Wire `read_file` / `write_file` to CLI
   - Syntax: `capsem cp <id>:<path> <local>` and reverse
   - Handles binary via base64 or streaming
   - File: `crates/capsem/src/main.rs`

3. **`--env KEY=VALUE` / `-e` on `start`** -- Pass env vars through provision
   - Needs: extend `ProvisionRequest` with `env: HashMap<String, String>`
   - Service passes them to process, process sends `SetEnv` at boot
   - Files: `crates/capsem/src/main.rs`, `crates/capsem-service/src/api.rs`, `crates/capsem-service/src/main.rs`, `crates/capsem-process/src/main.rs`

4. **`--rm` on `start`** -- Expose existing `auto_remove` API field
   - File: `crates/capsem/src/main.rs`

5. **`--timeout <secs>` on `exec`** -- Expose existing `timeout_secs` API field
   - File: `crates/capsem/src/main.rs`

6. **`capsem version`** -- Show CLI version + query service version
   - Compile-time: `env!("CARGO_PKG_VERSION")`, git hash
   - Runtime: query service for its version + asset manifest version
   - File: `crates/capsem/src/main.rs`

### Phase 2: New capabilities (Tier 2)

7. **`capsem prune`** -- Clean terminated sessions
   - Walk `~/.capsem/run/sessions/`, check status, delete old ones
   - Flags: `--older-than <days>` (default: retention_days from config), `--force` (skip confirm), `--dry-run`
   - Files: `crates/capsem/src/main.rs`, possibly new API endpoint

8. **`-q` / `--quiet` on `list`** -- Print only IDs (one per line)
   - For scripting: `capsem stop $(capsem ls -q)`
   - File: `crates/capsem/src/main.rs`

9. **`--disk <GB>` on `start`** -- Scratch disk size override
   - Needs: extend `ProvisionRequest` with `scratch_disk_size_gb`
   - Plumb through service -> process -> boot
   - Files: same chain as `--env`

10. **`capsem restart <id>`** -- Stop + start with same config
    - Needs: store original config (ram, cpu, env, name) in instance metadata
    - Or: query info before delete, then re-provision
    - Files: `crates/capsem/src/main.rs`, possibly `crates/capsem-service/src/main.rs`

11. **`capsem stats <id>`** -- Resource usage
    - Approach: exec `cat /proc/meminfo && cat /proc/stat && df -h` inside guest, parse output
    - Or: host-side process stats via pid (less accurate but no guest round-trip)
    - File: `crates/capsem/src/main.rs`

### Phase 3: Nice-to-have (Tier 3, only if time permits)

12. `capsem top <id>` -- Sugar over `exec ps aux`
13. `capsem pause` / `capsem unpause` -- Apple VZ pause/resume
14. `capsem wait <id>` -- Block until stopped
15. `capsem system info` -- Host capabilities and config paths
16. `capsem system df` -- Disk usage breakdown
17. `--filter` on `list` -- Filter by status, name pattern
18. `--format` on `list`/`info` -- JSON/table/quiet output

### Phase 4: MCP parity

Keep capsem-mcp in sync with every CLI addition. Each Phase 1-3 item that adds a CLI flag or command needs a matching MCP tool or param.

18. **`env` param on `capsem_create`** -- Map to `ProvisionRequest.env`
    - Type: `Option<HashMap<String, String>>`, serde rename `env`
    - Ships with Phase 1 `--env` CLI work
    - File: `crates/capsem-mcp/src/main.rs`

19. **`autoRemove` param on `capsem_create`** -- Expose existing `auto_remove` field
    - Type: `Option<bool>`, serde/schemars rename `autoRemove`
    - Ships with Phase 1 `--rm` CLI work
    - File: `crates/capsem-mcp/src/main.rs`

20. **`tail` param on `capsem_vm_logs` / `capsem_service_logs`** -- Return last N lines
    - Type: `Option<u64>`
    - Apply after grep if both present
    - File: `crates/capsem-mcp/src/main.rs`

21. **`capsem_version` tool** -- Return MCP server version, service version, asset version
    - Query service `/version` endpoint (add if missing)
    - Include compile-time `CARGO_PKG_VERSION` and git hash
    - File: `crates/capsem-mcp/src/main.rs`

22. **`diskGb` param on `capsem_create`** -- Scratch disk size override
    - Type: `Option<u64>`, serde/schemars rename `diskGb`
    - Ships with Phase 2 `--disk` CLI work
    - File: `crates/capsem-mcp/src/main.rs`

23. **`capsem_restart` tool** -- Stop + re-provision with same config
    - Query info, delete, re-create with same params
    - File: `crates/capsem-mcp/src/main.rs`

24. **`capsem_stats` tool** -- Resource usage for a VM
    - Exec `/proc` reads inside guest, parse and return structured JSON
    - File: `crates/capsem-mcp/src/main.rs`

25. **`capsem_prune` tool** -- Clean terminated sessions
    - Params: `olderThanDays: Option<u64>`, `dryRun: Option<bool>`
    - File: `crates/capsem-mcp/src/main.rs`

26. **`name`/`status` filter params on `capsem_list`** -- Filter VMs server-side or MCP-side
    - Type: `Option<String>` each
    - File: `crates/capsem-mcp/src/main.rs`

27. **Richer tool descriptions** -- Already done for current tools. Update descriptions for all new tools as they land.

### Phase 5: Skills and documentation

Update skills and create site documentation for all new CLI + MCP capabilities.

28. **Update `/dev-mcp` skill** -- Add new tools, params, grep/tail/timeout patterns, version tool
    - File: `skills/dev-mcp/SKILL.md`

29. **Update `/dev-capsem` skill** -- Reflect new CLI commands in overview
    - File: `skills/dev-capsem/SKILL.md`

30. **Update `/site-architecture` skill** -- New endpoints, IPC messages, env plumbing
    - File: `skills/site-architecture/SKILL.md`

31. **Update `/dev-testing` skill** -- Testing patterns for new MCP tools and CLI commands
    - File: `skills/dev-testing/SKILL.md`

32. **Site doc: CLI reference page** -- All commands, flags, examples
    - File: `site/src/content/docs/reference/cli.mdx` (new)

33. **Site doc: MCP tools reference page** -- All tools, params, return formats, examples
    - File: `site/src/content/docs/reference/mcp-tools.mdx` (new)

34. **Site doc: Docker migration guide** -- Map Docker commands to capsem equivalents
    - File: `site/src/content/docs/guides/docker-migration.mdx` (new)

### Phase 6: Testing

35. **Unit tests for all new MCP params** -- Serde roundtrips, edge cases, body construction
    - Follow existing pattern in `crates/capsem-mcp/src/main.rs` tests module
    - Every new param gets: roundtrip, default, edge case (zero/empty/huge)

36. **Unit tests for new CLI commands** -- Clap parsing, flag combinations
    - File: `crates/capsem/src/main.rs`

37. **Integration tests: CLI -> service -> VM** -- End-to-end for each new command
    - `logs`, `cp`, `--env`, `--rm`, `--timeout`, `version`, `prune`, `restart`, `stats`
    - Files: `tests/` directory

38. **MCP tool router test** -- Update tool count assertion as tools are added

39. **Security edge cases for new params** -- Path traversal in cp, injection in env values, huge disk sizes

## Key decisions

- **Phase 1 first, ship, then Phase 2.** Don't block quick wins on harder features.
- **`capsem stop` vs `capsem delete`**: Currently these are the same (SIGKILL + cleanup). Keep it that way for now -- `stop` is the user-facing name, `delete`/`rm` is the alias.
- **`cp` binary handling**: Use the existing `read_file`/`write_file` which are text-based. For binary files, base64-encode. Flag `--binary` or auto-detect.
- **`--env` plumbing**: Extend the provision request rather than adding a separate config step. Keeps it atomic.
- **No `docker build` equivalent**: capsem-builder is a separate Python tool. Out of scope.

## Files to modify

| File | Changes |
|------|---------|
| `crates/capsem/src/main.rs` | All new commands and flags |
| `crates/capsem-service/src/api.rs` | Extend DTOs (ProvisionRequest) |
| `crates/capsem-service/src/main.rs` | Handle new fields in provision, new endpoints |
| `crates/capsem-process/src/main.rs` | Accept env/disk from service |
| `crates/capsem-proto/src/ipc.rs` | Extend IPC messages if needed |

## What "done" looks like

- All Phase 1 commands work end-to-end
- Phase 2 commands work end-to-end
- `just test` passes
- `capsem doctor` passes
- CHANGELOG updated per commit
- Each phase is a separate commit (or 2-3 commits if a phase is large)
