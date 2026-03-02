# Debug & Diagnostics Skill

Use this skill when debugging capsem issues, verifying telemetry pipelines, or validating guest VM behavior.

## Quick Verification Checklist

When debugging or verifying a feature works end-to-end:

1. **Build & boot**: `just run "<command>"` (fast path, ~10s) or `just run` (interactive)
2. **Check session DB**: `just inspect-session` (latest) or `just inspect-session <id>`
3. **Run in-VM diagnostics**: `just run "capsem-doctor"`

## Verifying Telemetry Pipelines

### Filesystem Events (fs_events)

Boot a short-lived VM that creates a file and waits for the debouncer to flush:

```bash
just run 'touch /root/test_file.txt && echo hello > /root/test_file.txt && sleep 1 && echo done'
```

Then check the session:

```bash
just inspect-session
```

Look for `fs_events` rows. Expect:
- Boot config files (.claude/settings.json, .gemini/*) from BootConfig writes
- Your test file (test_file.txt) with correct size
- Actions: `created`, `modified`, `removed`

If fs_events is 0, investigate:
- Guest: is `capsem-fs-watch` running? Check boot logs for `[capsem-fs-watch] starting`
- Host: is the vsock accept for port 5005 succeeding? Check for `[capsem-agent] fs-watch connected`
- Debouncer: did the VM shut down too fast? Add `sleep 1` before exit to let the 100ms debouncer flush

### Network Events (net_events)

Boot with a command that makes an HTTPS request:

```bash
just run 'curl -s https://api.anthropic.com/ && sleep 1 && echo done'
```

Check the session for `net_events` rows with domain, decision, status_code, etc.

### Model Calls & Tool Calls (model_calls, tool_calls, tool_responses)

These require a real AI session. Boot interactively and run an AI CLI:

```bash
just run
# Inside VM: claude -p "what is 2+2"
```

Then check the session for model_calls (provider, model, tokens, cost) and tool_calls/tool_responses pairing.

### MCP Calls (mcp_calls)

These require an AI agent to invoke MCP tools. Boot interactively:

```bash
just run
# Inside VM: claude -p "use the fetch tool to get https://example.com"
```

Check session for mcp_calls rows with server_name, method, tool_name, decision.

## In-VM Diagnostics (capsem-doctor)

Run the full diagnostic suite inside the VM:

```bash
just run "capsem-doctor"              # Full suite
just run "capsem-doctor -k sandbox"   # Only sandbox tests
just run "capsem-doctor -x"           # Stop on first failure
```

Test categories:
- `test_sandbox.py` -- security boundaries (rootfs, permissions, kernel hardening, network isolation)
- `test_network.py` -- MITM proxy, TLS trust chain, port blocking
- `test_environment.py` -- VM config (env vars, arch, mounts)
- `test_runtimes.py` -- dev tools (Python, Node, git)
- `test_utilities.py` -- unix tool availability (~36 utilities)
- `test_workflows.py` -- file I/O patterns (JSON roundtrip, large files)
- `test_ai_cli.py` -- AI CLI sandboxing (claude, gemini, codex)
- `test_mcp.py` -- MCP gateway (tool routing, domain blocking)

## Session Inspection

```bash
just inspect-session              # Latest session
just inspect-session <session-id> # Specific session
just inspect-session --list       # List recent sessions
just inspect-session -n 10        # Show 10 preview rows per table
```

The script checks:
- All expected tables exist (net_events, model_calls, tool_calls, tool_responses, mcp_calls, fs_events)
- Row counts per table
- Orphaned tool_calls without matching tool_responses
- AI-provider net_events vs model_calls consistency
- Preview of recent rows per table

## Updating the Test Fixture

The test fixture (`data/fixtures/test.db`) must come from a real session that exercises all telemetry pipelines. **Never insert synthetic data.**

### Proper workflow:

1. **Run the integration test** to generate a rich session:
   ```bash
   python3 scripts/integration_test.py --binary target/debug/capsem --assets assets
   ```
   This exercises: fs_events (create/modify/delete), net_events (allowed + denied), mcp_calls, model_calls (with cost), tool_calls (with origin).

2. **Inspect the session** to verify completeness:
   ```bash
   just inspect-session <session-id>
   ```

3. **Verify the session has everything the fixture tests need**:
   ```bash
   sqlite3 ~/.capsem/sessions/<id>/session.db "
     SELECT decision, COUNT(*) FROM net_events GROUP BY decision;
     SELECT action, COUNT(*) FROM fs_events GROUP BY action;
     SELECT COUNT(*) FROM model_calls WHERE estimated_cost_usd > 0;
     SELECT COUNT(*) FROM tool_calls WHERE origin IS NOT NULL;
   "
   ```
   Required: denied net_events, deleted fs_events, positive costs, origin column.

4. **Update the fixture**:
   ```bash
   just update-fixture ~/.capsem/sessions/<id>/session.db
   ```

5. **Run tests** to verify the fixture satisfies all assertions:
   ```bash
   cargo test --workspace
   ```

### What the fixture must contain:
- `net_events` with both `allowed` and `denied` decisions
- `fs_events` with `created`, `modified`, and `deleted` actions
- `model_calls` with `estimated_cost_usd > 0`
- `tool_calls` with `origin` column populated
- `tool_calls` schema includes `origin TEXT` and `mcp_call_id INTEGER` columns

## Common Debugging Patterns

### "Events not showing up in session DB"

1. Check boot logs -- did the relevant daemon start? (`capsem-fs-watch`, `capsem-net-proxy`, etc.)
2. Check vsock connections -- did the host accept the connection? Look for `connected (port XXXX)` in logs
3. Check timing -- does the VM live long enough for debounced events to flush? Add `sleep 1`
4. Check the session DB directly: `sqlite3 ~/.capsem/sessions/<id>/session.db "SELECT * FROM fs_events"`

### "Guest binary not picking up changes"

- Changed `capsem-init`, agent, or repacked binary? -> `just run` (auto-repacks initrd)
- Changed rootfs (Dockerfile, bashrc, diagnostics)? -> `just build-assets`
- Binary on rootfs vs initrd: initrd copies take priority (capsem-init checks `/binary` before rootfs path)

### "Cross-compile failure"

- Check `.cargo/config.toml` for `aarch64-unknown-linux-musl` linker config
- Watch for platform-specific types (e.g., `libc::ioctl` request param differs macOS vs Linux -- use `as _`)
- Run `cargo build --release --target aarch64-unknown-linux-musl -p capsem-agent` to isolate the failure
