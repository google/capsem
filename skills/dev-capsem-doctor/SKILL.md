---
name: dev-capsem-doctor
description: The capsem-doctor in-VM diagnostic suite. Use when writing, running, or extending doctor tests, or debugging VM sandbox issues.
---

# capsem-doctor

capsem-doctor is a pytest-based diagnostic suite that runs inside the guest VM. It verifies sandbox integrity, network isolation, runtime environment, and AI agent functionality. It's the smoke test gate -- every change must pass it before shipping.

Doctor is also an Ironbank input. When doctor is used to close a
release-critical VM/security/protocol/package-manager gate, load `/ironbank`
and assert the full ledger through `tests/ironbank/`: client result, DB rows,
structured logs, UDS/HTTP route output, counters, and UI-facing JSON. A doctor
exit code or "row exists" check is not enough.

## Running

```bash
just exec "capsem-doctor"              # Full suite (~10s total including VM boot)
just exec "capsem-doctor -k sandbox"   # Only sandbox tests
just exec "capsem-doctor -k network"   # Only network tests
just exec "capsem-doctor -x"           # Stop on first failure
just exec "capsem-doctor -v"           # Extra verbose
```

## Test categories

| File | What it validates |
|------|-------------------|
| `test_sandbox.py` | Read-only rootfs, binary permissions (chmod 555), no setuid/setgid, kernel hardening (no modules, no debugfs, no IPv6, no swap, no kallsyms), process integrity (pty-agent, dnsmasq running; no systemd, sshd, cron), network isolation (dummy0, fake DNS, iptables, no real NICs) |
| `test_network.py` | MITM CA in system store, curl without -k works, Python urllib HTTPS, CA env vars set (SSL_CERT_FILE, REQUESTS_CA_BUNDLE, NODE_EXTRA_CA_CERTS), HTTP/80 blocked, non-443 ports blocked, direct IP blocked, multi-domain DNS faking, AI provider domains reachable |
| `test_environment.py` | TERM/HOME/PATH env vars correct, shell is bash, kernel version, aarch64 arch, mount points (/proc, /sys, /dev, /dev/pts), tmpfs verification |
| `test_runtimes.py` | python3/pip3 versions; hermetic pip install into the venv; hermetic apt install of a local .deb; apt sandbox TLS trust; Python file I/O |
| `test_workflows.py` | Text write/read, Python JSON roundtrip, shell pipes, large file (10MB) |
| `test_virtiofs.py` | VirtioFS root mount, ext4 loopback upper, loop device active, workspace write/read/large file/subdir, system overlay writable, pip install works, file delete+recreate (skipped in block mode) |
| `test_mcp.py` | Guest MCP endpoint tool routing, domain blocking via MCP |
| `test_injection.py` | Security injection tests |
| `test_lifecycle.py` | capsem-sysutil lifecycle symlinks, read-only sysutil, VM identity env vars, hostname |
| `test_storage_write_probes.py` | Bounded create/read/delete probes on package-manager and workspace paths, `_apt` partial cache |
| `conftest.py` | Test infrastructure (auto-skip outside VM, output dir fixture); `run()` lives in `diagnostic_support.py` |

The runtime rootfs is minimal (Debian base plus runc, umoci, python3, iptables, iproute2,
curl, procps, pytest, rich, venv). Tests may only use those: node, uv, git, AI CLIs and pip
packages like certifi or fastmcp live in the OCI images under `images/`, proven by `tests/images/`.

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
4. `just exec "capsem-doctor"` picks up changes immediately (diagnostics repacked into initrd)
5. For rootfs-baked changes: `just _build-assets` then `just exec "capsem-doctor"`

## Where tests live on disk

- **Source**: `guest/artifacts/diagnostics/test_*.py` (in the repo)
- **In rootfs**: `/usr/local/lib/capsem-tests/test_*.py` (baked by Dockerfile.rootfs)
- **In initrd**: overrides rootfs copies via `_pack-initrd` (fast iteration)

## Writing good diagnostic tests

- Test one thing per function. Name clearly: `test_readonly_rootfs`, `test_mitm_ca_in_system_bundle`
- Use `run()` for shell commands, check `.returncode` and `.stdout`/`.stderr`
- Set reasonable timeouts (default 10s). Network tests may need longer.
- Think adversarially: test what should be blocked, not just what should work
- For VirtioFS tests, skip gracefully in block mode: `pytest.mark.skipif`
- For Ironbank/release gates, do not skip. If a package manager, protocol, or
  diagnostic dependency is unavailable, the product or harness is broken.
- Package-manager diagnostics must prove function, not installation presence:
  run the installed binary/module and verify deterministic behavior.
