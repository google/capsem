---
name: dev-testing-vm
description: In-VM diagnostics and test fixtures for Capsem. Use when working with capsem-doctor, adding new in-VM tests, debugging test failures inside the guest, inspecting session databases, or updating the test fixture. Covers the full capsem-doctor test suite, how to run subsets, how to add new VM tests, session inspection, and fixture management.
---

# In-VM Testing

## capsem-doctor

The diagnostic suite runs inside the guest VM via pytest. Tests live in `guest/artifacts/diagnostics/` and are baked into the rootfs.

### Running diagnostics

```bash
just exec "capsem-doctor"              # Full suite (~10s total)
just exec "capsem-doctor -k sandbox"   # Only sandbox tests
just exec "capsem-doctor -k network"   # Only network tests
just exec "capsem-doctor -x"           # Stop on first failure
```

### Test categories

| File | What it verifies |
|------|------------------|
| `test_sandbox.py` | Read-only rootfs, binary permissions, setuid/setgid, kernel hardening (no modules, no debugfs, no IPv6, no swap), process integrity, network isolation (dummy0, fake DNS, iptables) |
| `test_network.py` | MITM CA in system store + certifi, curl without -k, Python urllib HTTPS, CA env vars, HTTP/80 blocked, non-443 blocked, direct IP blocked, multi-domain DNS, AI provider domains |
| `test_environment.py` | TERM/HOME/PATH env vars, bash shell, kernel version, aarch64 arch, mount points, tmpfs |
| `test_runtimes.py` | Python3, Node.js, npm, pip3, git version checks, Python/Node file I/O, git workflow |
| `test_utilities.py` | ~36 unix utilities (coreutils, text processing, network, system tools) |
| `test_workflows.py` | Text write/read, JSON roundtrip, shell pipes, large file (10MB) |
| `test_ai_cli.py` | claude/gemini/codex installed and executable |
| `test_virtiofs.py` | VirtioFS mount, ext4 loopback, workspace I/O, pip install, file delete+recreate |

### Adding new in-VM tests

1. Add test functions to the appropriate `guest/artifacts/diagnostics/test_*.py` or create `test_<category>.py`
2. Use `from conftest import run` for shell commands, `output_dir` fixture for temp files
3. Tests auto-skip outside the capsem VM (conftest checks for root + writable /root)
4. Rebuild rootfs with `just _build-assets` to bake new test files into the image
5. For fast iteration during development, tests in `diagnostics/` are also repacked into the initrd by `just exec`, so `just exec "capsem-doctor"` picks up changes without a full rootfs rebuild
6. Verify: `just exec "capsem-doctor -k <your_test>"`

## Session inspection

After running a VM session, inspect the telemetry database:

```bash
python3 scripts/check_session.py              # Latest session
python3 scripts/check_session.py <session-id> # Specific session
python3 scripts/check_session.py --list       # List recent sessions
python3 scripts/check_session.py -n 10        # Show 10 preview rows per table
```

Checks: session ledgers exist (net_events, model_calls, tool_calls, tool_responses, fs_events, dns_events, security_rule_events), row counts, orphaned tool_calls, AI-provider consistency.

## Verifying telemetry pipelines

Each pipeline can be tested with a targeted VM command:

- **fs_events**: `just exec 'touch /root/test.txt && sleep 1'` then `python3 scripts/check_session.py`
- **net_events**: `just exec 'curl -s https://api.anthropic.com/ && sleep 1'`
- **model_calls/tool_calls**: boot interactively, run `claude -p "what is 2+2"`
- **MCP-origin tool_calls**: boot interactively, run `claude -p "use fetch to get https://example.com"` and query `tool_calls WHERE origin = 'mcp'`

If events are missing: check boot logs for daemon startup, vsock connection acceptance, and whether the VM lived long enough for the debouncer to flush (add `sleep 1`).

## Test fixture

The fixture (`data/fixtures/test.db`) is a real session DB shared by frontend mock mode and Rust roundtrip tests. No synthetic data.

### Updating the fixture

```bash
# 1. Run integration test to generate a rich session
python3 scripts/integration_test.py --binary target/debug/capsem --assets assets

# 2. Inspect completeness
python3 scripts/check_session.py <session-id>

# 3. Fixture refresh has no Just convenience command. Copy, checkpoint, and
# scrub the selected DB using the procedure in /dev-session-debug.

# 4. Verify
cargo test --workspace
```

The fixture must contain: both allowed and denied net_events, created/modified/deleted fs_events, model_calls with cost > 0, tool_calls with origin populated.
