---
name: dev-testing-vm
description: In-VM diagnostics and test fixtures for Capsem. Use when working with capsem-doctor, adding new in-VM tests, debugging test failures inside the guest, inspecting session databases, or updating the test fixture. Covers the full capsem-doctor test suite, how to run subsets, how to add new VM tests, session inspection, and fixture management.
---

# In-VM Testing

## capsem-doctor

The diagnostic suite runs inside the guest VM via pytest. Tests live in `guest/artifacts/diagnostics/` and are baked into the rootfs.

### Running diagnostics

```bash
just run "capsem-doctor"              # Full suite (~10s total)
just run "capsem-doctor -k sandbox"   # Only sandbox tests
just run "capsem-doctor -k network"   # Only network tests
just run "capsem-doctor -x"           # Stop on first failure
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
4. Rebuild rootfs with `just build-assets` to bake new test files into the image
5. For fast iteration during development, tests in `diagnostics/` are also repacked into the initrd by `just run`, so `just run "capsem-doctor"` picks up changes without a full rootfs rebuild
6. Verify: `just run "capsem-doctor -k <your_test>"`

## Session inspection

After running a VM session, inspect the telemetry database:

```bash
just inspect-session              # Latest session
just inspect-session <session-id> # Specific session
just inspect-session --list       # List recent sessions
just inspect-session -n 10        # Show 10 preview rows per table
```

Checks: all 6 tables exist (net_events, model_calls, tool_calls, tool_responses, mcp_calls, fs_events), row counts, orphaned tool_calls, AI-provider consistency.

## Verifying telemetry pipelines

Each pipeline can be tested with a targeted VM command:

- **fs_events**: `just run 'touch /root/test.txt && sleep 1'` then `just inspect-session`
- **net_events**: `just run 'curl -s https://api.anthropic.com/ && sleep 1'`
- **model_calls/tool_calls**: boot interactively, run `claude -p "what is 2+2"`
- **mcp_calls**: boot interactively, run `claude -p "use fetch to get https://example.com"`

If events are missing: check boot logs for daemon startup, vsock connection acceptance, and whether the VM lived long enough for the debouncer to flush (add `sleep 1`).

## Test fixture

The fixture (`data/fixtures/test.db`) is a real session DB shared by frontend mock mode and Rust roundtrip tests. No synthetic data.

### Updating the fixture

```bash
# 1. Run integration test to generate a rich session
python3 scripts/integration_test.py --binary target/debug/capsem --assets assets

# 2. Inspect completeness
just inspect-session <session-id>

# 3. Update (scrubs API keys, copies to both data/ and frontend/)
just update-fixture ~/.capsem/sessions/<id>/session.db

# 4. Verify
cargo test --workspace
```

The fixture must contain: both allowed and denied net_events, created/modified/deleted fs_events, model_calls with cost > 0, tool_calls with origin populated.
