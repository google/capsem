# Fast Debug with Capsem MCP Tools

Reference for /dev-testing: interactive VM debugging via the capsem MCP server, tool table, debug workflow, and common session-DB queries.

## Fast debug with capsem MCP tools

When the capsem MCP server is configured, Claude Code has direct VM control via MCP tools -- no shell commands or just recipes needed. This is the fastest way to test changes interactively because you stay in the conversation loop: create a VM, run commands, inspect results, fix code, repeat.

### The tools

| Tool | What it does |
|------|-------------|
| `capsem_create` | Spin up a fresh VM (returns VM id). Named VMs are persistent. |
| `capsem_run` | One-shot: boot temp VM, exec command, destroy, return output |
| `capsem_exec` | Run a command inside a running guest |
| `capsem_stop` | Stop VM (persistent: preserve state; ephemeral: destroy) |
| `capsem_resume` | Resume a stopped persistent VM |
| `capsem_read_file` | Read a file from the guest filesystem |
| `capsem_write_file` | Write a file into the guest |
| `capsem_list` | Show all VMs (running + stopped persistent) |
| `capsem_info` | VM details (config, status, persistent, PID) |
| `capsem_delete` | Destroy VM and wipe all state |
| `capsem_persist` | Convert running ephemeral VM to persistent |
| `capsem_purge` | Kill all temp VMs (all=true includes persistent) |
| `capsem_fork` | Fork a running/stopped VM into a reusable image |
| `capsem_image_list` | List all user images |
| `capsem_image_inspect` | Inspect a specific image's metadata |
| `capsem_image_delete` | Delete a user image |

### Debug workflow

**Quick one-shot** (no VM management): `capsem_run` with the command you want to test.

**Iterative debugging** (long-lived VM):
1. **Create**: `capsem_create` -- boots a fresh VM in ~10s
2. **Test**: `capsem_exec` with the command you want to verify (e.g., `capsem-doctor -k net`, `cat /etc/resolv.conf`, `curl https://example.com`)
3. **Inspect**: `capsem_read_file` to check config files/logs; typed stats, timeline, security, detection, and enforcement routes for telemetry
4. **Iterate**: fix code on host, rebuild (`just build`), create a new VM to test again
5. **Cleanup**: `capsem_delete` when done

### When to use MCP tools vs just recipes

| Scenario | Use |
|----------|-----|
| Quick check: "does this command work in the guest?" | `capsem_run` |
| Read a guest file to understand state | `capsem_read_file` |
| Verify telemetry was recorded correctly | typed stats/timeline/security routes or Ironbank direct ledger reads |
| Full regression suite | `just test` |
| Build + boot + validate in one shot | `just smoke` |
| Benchmark performance | `just test` |

MCP tools are for fast, targeted checks during development. Just recipes are for comprehensive validation before committing.

### Common debug queries

```sql
-- Check network events for a domain
SELECT * FROM net_events WHERE domain LIKE '%example%' ORDER BY timestamp DESC LIMIT 10;

-- Verify MCP-origin tool calls were logged
SELECT server_name, tool_name, decision, duration_ms
FROM tool_calls
WHERE origin = 'mcp'
ORDER BY timestamp DESC;

-- Check model API calls
SELECT provider, model, status_code, duration_ms FROM model_calls ORDER BY timestamp DESC;

-- File system events
SELECT operation, path, success FROM fs_events ORDER BY timestamp DESC LIMIT 20;
```
