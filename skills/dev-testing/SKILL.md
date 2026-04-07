---
name: dev-testing
description: Capsem testing policy and workflow. Use whenever running tests, writing new tests, or verifying changes work. Covers the three test tiers (unit, smoke, full), TDD red-green-refactor, adversarial security testing, coverage policy, and the mandatory end-to-end VM validation. For VM-specific tests see dev-testing-vm, for hypervisor tests see dev-testing-hypervisor, for frontend tests see dev-testing-frontend.
---

# Testing

## Test tiers

Three tiers, fast to thorough. Every change must pass all three before it ships.

| Command | What | VM? |
|---------|------|-----|
| `just test` | Everything: unit tests (llvm-cov, warnings-as-errors for service crates) + cross-compile + frontend + all Python integration tests + injection + benchmarks | Yes |
| `just smoke` | Quick end-to-end: repack + sign + boot + capsem-doctor + MCP + service integration (~30s) | Yes |

`just test` is the single source of truth. There is no "fast" tier that skips integration tests -- that's how the "Connection refused" bug shipped while tests said green. Individual `test-*` recipes exist for targeted debugging but `just test` is the gate.

## TDD workflow

Write tests first:
1. Write failing tests that capture expected behavior
2. Verify they fail for the right reason
3. Write minimal implementation to pass them
4. Refactor

Without a failing test first, it's easy to write tests that pass by accident or don't actually verify the behavior you intended.

## Adversarial testing

Capsem is a security product. Every security-relevant feature needs tests that actively try to break invariants. Think like an attacker:
- Can a corp-blocked domain be snuck through another provider's list?
- Does an overlapping wildcard in allow+block always deny?
- Does malformed input (empty strings, unicode, huge payloads, invalid JSON) get rejected?
- Can path traversal escape the VirtioFS sandbox?
- Can a guest process modify its own binaries?

Stress-test boundary conditions. Write tests for the attacks you'd attempt yourself.

## Test fixture anti-pattern: masking races with polling

If all test fixtures wait/poll before asserting, the tests will never catch server-side race conditions. For every endpoint that talks to a VM socket, write at least one test that calls it IMMEDIATELY after provision (no `wait_exec_ready`, no `ready_vm` fixture). The server must handle readiness internally.

**Pattern to avoid** (masks the bug -- server never needs wait logic because client always waits):
```
fixture calls provision -> fixture polls wait_exec_ready -> test calls exec
```

**Required test pattern** (catches the bug -- if server doesn't wait, test fails):
```
test calls provision -> test immediately calls exec -> server handles wait
```

See `tests/capsem-service/test_svc_exec_ready.py` for the regression tests that enforce this.

## Where tests live

- Rust unit: `#[cfg(test)] mod tests` in each module
- Rust integration: `crates/capsem-core/tests/`
- In-VM diagnostics: `guest/artifacts/diagnostics/test_*.py` (see dev-testing-vm)
- Hypervisor: KVM + Apple VZ tests (see dev-testing-hypervisor)
- Frontend: `frontend/src/lib/__tests__/` (see dev-testing-frontend)
- Python (builder): `tests/test_*.py`
- Python integration (service daemon): `tests/capsem-*/` directories, each with its own conftest.py and pytest marker

## Integration test suites

All Python integration tests live under `tests/capsem-*/` and use pytest markers. Each suite has a dedicated `just` recipe.

| Suite | Directory | Marker | VM? | What it tests |
|-------|-----------|--------|-----|---------------|
| Service API | `capsem-service/` | `integration` | Yes | HTTP endpoints: provision, list, info, exec, logs, file I/O, delete |
| CLI | `capsem-cli/` | `integration` | Yes | CLI subcommands via subprocess |
| MCP | `capsem-mcp/` | `mcp` | Yes | MCP server black-box (stdio, tool routing) |
| Session DB | `capsem-session/` | `session` | Yes | Telemetry: net/model/tool/mcp/fs/snapshot events |
| Snapshots | `capsem-snapshots/` | `snapshot` | Yes | Auto/manual snapshots, revert |
| Isolation | `capsem-isolation/` | `isolation` | Yes | Multi-VM filesystem + network isolation |
| Security | `capsem-security/` | `security` | Yes | Binary perms, codesigning, asset integrity, env blocklist |
| Config | `capsem-config/` | `config` | Yes | Limits, resource bounds, hot-reload |
| Bootstrap | `capsem-bootstrap/` | `bootstrap` | No | Setup flow, dev tools, asset checks |
| Stress | `capsem-stress/` | `stress` | Yes | 5 concurrent VMs, rapid create/delete |
| Build chain | `capsem-build-chain/` | `build_chain` | Yes | cargo build -> codesign -> pack -> manifest -> boot |
| Guest | `capsem-guest/` | `guest` | Yes | Network, services, filesystem, env inside guest |
| Cleanup | `capsem-cleanup/` | `cleanup` | Yes | Process killed, socket removed, session dir removed |
| Codesign | `capsem-codesign/` | `codesign` | No | All binaries signed, entitlements present (FAIL not skip) |
| Serial | `capsem-serial/` | `serial` | Yes | Console logs, boot timing < 30s |
| Session lifecycle | `capsem-session-lifecycle/` | `session_lifecycle` | Yes | DB exists, schema, events, survives shutdown |
| Config runtime | `capsem-config-runtime/` | `config_runtime` | Yes | CPU/RAM applied in guest, blocked domains |
| Recipes | `capsem-recipes/` | `recipe` | No | just run-service, just doctor, cargo build |
| Recovery | `capsem-recovery/` | `recovery` | Yes | Stale socket/instances, orphaned process, double service |
| Rootfs artifacts | `capsem-rootfs-artifacts/` | `rootfs` | No | Artifact files, build context, doctor consistency |
| Session exhaustive | `capsem-session-exhaustive/` | `session_exhaustive` | Yes | Per-table data validation, cross-table FK integrity |
| Install | `capsem-install/` | `install` | No | Native installer: layout, auto-launch, service install, setup wizard, update, uninstall, lifecycle, reinstall, error paths |

Composite recipe: `just test-vm` runs build-chain + guest + cleanup + codesign + serial + session-lifecycle + config-runtime + recovery. `just test-install` runs the install suite in Docker with systemd. `just test` runs everything.

## Coverage

- Rust: `cargo llvm-cov` via `just test`
- Python: `--cov-fail-under=90`
- `codecov.yml` maps components to code paths. Update it when files or directories are added, moved, or renamed.

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
| `capsem_inspect_schema` | Get session.db table schema |
| `capsem_inspect` | Run SQL against session.db (telemetry) |
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
3. **Inspect**: `capsem_read_file` to check config files, logs; `capsem_inspect` to query telemetry tables
4. **Iterate**: fix code on host, rebuild (`just build`), create a new VM to test again
5. **Cleanup**: `capsem_delete` when done

### When to use MCP tools vs just recipes

| Scenario | Use |
|----------|-----|
| Quick check: "does this command work in the guest?" | `capsem_run` |
| Read a guest file to understand state | `capsem_read_file` |
| Verify telemetry was recorded correctly | `capsem_inspect` with SQL query |
| Full regression suite | `just test` |
| Build + boot + validate in one shot | `just smoke` |
| Benchmark performance | `just bench` |

MCP tools are for fast, targeted checks during development. Just recipes are for comprehensive validation before committing.

### Common debug queries

```sql
-- Check network events for a domain
SELECT * FROM net_events WHERE domain LIKE '%example%' ORDER BY timestamp DESC LIMIT 10;

-- Verify MCP tool calls were logged
SELECT server_name, tool_name, decision, duration_ms FROM mcp_calls ORDER BY timestamp DESC;

-- Check model API calls
SELECT provider, model, status_code, duration_ms FROM model_calls ORDER BY timestamp DESC;

-- File system events
SELECT operation, path, success FROM fs_events ORDER BY timestamp DESC LIMIT 20;
```

## End-to-end validation is not optional

After any change touching guest binaries, network policy, telemetry, MCP, or VM lifecycle:

1. `just run "capsem-doctor"` -- verifies sandbox integrity inside the VM
2. After telemetry/logging changes: run a real session and verify with `just inspect-session` that all 6 tables (net_events, model_calls, tool_calls, tool_responses, mcp_calls, fs_events) are populated correctly

## When tests fail

Never dismiss a test failure as "pre-existing" or "unrelated." Every failure must be investigated. Follow the dev-debugging workflow:

1. **Do not change the test to make it pass.** The test is evidence. Changing the assertion to match broken behavior destroys that evidence.
2. **Reproduce and diagnose first.** Understand *why* it fails before writing any fix. See the dev-debugging skill for the full methodology: reproduce with a test, diagnose root cause, then fix comprehensively.
3. **Fix the code, not the test.** If the test is genuinely wrong (not the code), explain in detail why the test's expectation is incorrect before changing it.

## Platform gating tests

`cargo test --test platform_gating` scans all `.rs` files under `crates/` for macOS-only and Linux-only symbols (`libc::clonefile`, `AppleVzHypervisor`, `KvmHypervisor`, `FICLONE`, etc.) and verifies they appear inside `#[cfg(target_os = "...")]` blocks. This catches ungated platform APIs before they reach CI. Run this test when adding any platform-specific code.

## Testable design

Extract logic into `capsem-core` -- never embed business logic in the app layer where it's coupled to Tauri. If you can't test something without booting a VM or launching the GUI, it belongs in core.
