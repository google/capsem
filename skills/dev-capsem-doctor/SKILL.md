---
name: dev-capsem-doctor
description: The capsem-doctor in-VM diagnostic suite. Use when writing, running, or extending capsem-doctor tests, adding new diagnostic categories, debugging VM sandbox issues, or understanding what capsem-doctor validates. Covers all 11 test categories, how to run subsets, the conftest infrastructure, and how to add new tests.
---

# capsem-doctor

capsem-doctor is a pytest-based diagnostic suite that runs inside the guest VM. It verifies sandbox integrity, network isolation, runtime environment, and AI agent functionality. It's the smoke test gate -- every change must pass it before shipping.

## Running

```bash
just run "capsem-doctor"              # Full suite (~10s total including VM boot)
just run "capsem-doctor -k sandbox"   # Only sandbox tests
just run "capsem-doctor -k network"   # Only network tests
just run "capsem-doctor -x"           # Stop on first failure
just run "capsem-doctor -v"           # Extra verbose
```

## Test categories (11 files)

| File | What it validates |
|------|-------------------|
| `test_sandbox.py` | Read-only rootfs, binary permissions (chmod 555), no setuid/setgid, kernel hardening (no modules, no debugfs, no IPv6, no swap, no kallsyms), process integrity (pty-agent, dnsmasq running; no systemd, sshd, cron), network isolation (dummy0, fake DNS, iptables, no real NICs) |
| `test_network.py` | MITM CA in system store + certifi, curl without -k works, Python urllib HTTPS, CA env vars set (SSL_CERT_FILE, REQUESTS_CA_BUNDLE, NODE_EXTRA_CA_CERTS), HTTP/80 blocked, non-443 ports blocked, direct IP blocked, multi-domain DNS faking, AI provider domains reachable |
| `test_environment.py` | TERM/HOME/PATH env vars correct, shell is bash, kernel version, aarch64 arch, mount points (/proc, /sys, /dev, /dev/pts), tmpfs verification |
| `test_runtimes.py` | Python3, Node.js, npm, pip3, git version checks; Python file I/O; Node file I/O; git init+commit workflow |
| `test_utilities.py` | ~36 unix utilities available (coreutils, text processing, network, system tools, capsem-bench) |
| `test_workflows.py` | Text write/read, JSON roundtrip (Python + Node), shell pipes, large file (10MB) |
| `test_ai_cli.py` | claude, gemini, codex installed and executable without crashing |
| `test_virtiofs.py` | VirtioFS root mount, ext4 loopback upper, loop device active, workspace write/read/large file/subdir, system overlay writable, pip install works, file delete+recreate (skipped in block mode) |
| `test_mcp.py` | MCP gateway tool routing, domain blocking via MCP |
| `test_injection.py` | Security injection tests |
| `conftest.py` | Test infrastructure (auto-skip outside VM, `run()` helper, output dir fixture) |

## Infrastructure (conftest.py)

```python
# Auto-skip if not in capsem VM (checks root + writable /root)
def pytest_ignore_collect(collection_path, config):
    if os.geteuid() != 0 or not os.access("/root", os.W_OK):
        return True

# Shell command runner
def run(cmd, timeout=10):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout)

# Shared output directory: /root/tests
@pytest.fixture
def output_dir():
    return TESTS_OUTPUT_DIR
```

## Adding a new test

1. Add test functions to the appropriate `guest/artifacts/diagnostics/test_*.py` file, or create `test_<category>.py`
2. Use `from conftest import run` for shell commands, `output_dir` fixture for temp files
3. Tests auto-skip outside the capsem VM (no special guards needed)
4. `just run "capsem-doctor"` picks up changes immediately (diagnostics repacked into initrd)
5. For rootfs-baked changes: `just build-assets` then `just run "capsem-doctor"`

## Where tests live on disk

- **Source**: `guest/artifacts/diagnostics/test_*.py` (in the repo)
- **In rootfs**: `/usr/local/lib/capsem-tests/test_*.py` (baked by Dockerfile.rootfs)
- **In initrd**: overrides rootfs copies via `_pack-initrd` (fast iteration)

## Writing good diagnostic tests

- Test one thing per function. Name clearly: `test_readonly_rootfs`, `test_ca_in_certifi`
- Use `run()` for shell commands, check `.returncode` and `.stdout`/`.stderr`
- Set reasonable timeouts (default 10s). Network tests may need longer.
- Think adversarially: test what should be blocked, not just what should work
- For VirtioFS tests, skip gracefully in block mode: `pytest.mark.skipif`
