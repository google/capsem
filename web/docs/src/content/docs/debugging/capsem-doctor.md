---
title: Capsem Doctor
description: In-VM diagnostic suite for verifying sandbox integrity, network isolation, and runtime environment.
sidebar:
  order: 1
---

capsem-doctor is a pytest-based diagnostic suite that runs inside the guest VM. It verifies every security invariant, network isolation property, and runtime configuration that Capsem guarantees. Tests are baked into the rootfs via `Dockerfile.rootfs` and repacked into the initrd on every `just run`, so changes to test files take effect immediately without a full rootfs rebuild.

## Running Diagnostics

| Command | What it does |
|---------|-------------|
| `just exec "capsem-doctor"` | Repack initrd, build, sign, boot VM, run all tests, shut down (~10s) |
| `capsem-doctor` | Run all tests (inside a running VM) |
| `capsem-doctor -k sandbox` | Run only sandbox tests |
| `capsem-doctor -k "network and not throughput"` | Run network tests excluding throughput |
| `capsem-doctor -x` | Stop on first failure |

## Test Categories

| File | Tests | What it verifies |
|------|-------|------------------|
| `test_sandbox.py` | 42 | Clock sync, filesystem isolation (EROFS immutability, overlay config, runtime-only writes, writable mounts), guest binary security (read-only, executable), no setuid/setgid, kernel hardening (no modules, no /dev/mem, no /dev/port, no /proc/kcore, no debugfs, no IPv6, no kallsyms, seccomp available), kernel cmdline hardening (ro, init_on_alloc, slab_nomerge, page_alloc.shuffle), network isolation (dummy0, DNS proxy, iptables redirect, net-proxy running, allowed/denied domains, no real NICs), process integrity (pty-agent, dns-proxy present, legacy dnsmasq absent, no systemd/sshd/cron), swap mode validation, loopback interface |
| `test_network.py` | 39 | Layered L1-L7 network verification: L1 guest plumbing (dummy0 IP, capsem-dns-proxy UDP/TCP listeners, DNS redirect to :1053, upstream DNS answers and NXDOMAIN propagation, HTTPS iptables redirect), L2 net-proxy (TCP 10443 listener, 443 redirect, vsock byte delivery), L3 TLS handshake (MITM proxy termination, Capsem CA cert verification), L4 HTTP over MITM (curl with skip-verify, verbose diagnostics), L5 CA trust chain (cert file exists, system bundle, curl without -k, Python urllib TLS, CA env vars), L6 policy enforcement (denied domains, POST to random domains, AI provider blocking, HTTP port 80 blocked, non-standard ports, direct IP), L7 proxy download throughput |
| `test_environment.py` | 18 | Env vars (TERM, HOME, PATH, VIRTUAL_ENV), shell is bash, exact configured kernel version, architecture, mount points (/proc, /sys, /dev, /dev/pts), filesystem layout (overlay root, writable /root, writable /tmp, VirtioFS kernel support), boot performance, XSS rejection in timing data |
| `test_runtimes.py` | 5 | What the minimal runtime ships: python3 and pip3 versions, hermetic `pip install` of a local wheel into the session venv, hermetic `apt-get install` of a local `.deb`, apt's `_apt` sandbox reading the TLS trust bundle, Python execution with file I/O. Application toolchains (node, uv, git, AI CLIs) come from OCI images and are proven by `tests/images/` |
| `test_workflows.py` | 4 | File I/O patterns: text write/read, Python JSON roundtrip, shell pipes, large file (10MB) write and verify |
| `test_virtiofs.py` | 9 | VirtioFS storage mode (skipped in block mode): VirtioFS root mount, ext4 loopback overlay upper, loop device active on rootfs.img, workspace write/read/large file/subdirectory, system overlay writable, pip install through overlay, file delete and recreate |
| `test_mcp.py` | 25 | Guest MCP endpoint: binary exists, JSON-RPC initialize handshake, tools/list (fetch_http, grep_http, http_headers with descriptions, input schemas, annotations), tool invocation (allowed/blocked domains, real content verification, subpath fetch, raw HTML mode, grep pattern matching, pagination, headers), error handling (unknown tool, missing URL, invalid URL), retired snapshot tools and CLI absent |
| `test_injection.py` | 9 | Data-driven injection verification from host manifest: env vars present in login shell with correct values, no empty env vars, boot files exist with correct permissions and non-empty content, .git-credentials format and permissions, .gitconfig credential helper, GH_TOKEN env var |

## Test Infrastructure

### conftest.py

The shared test configuration in `conftest.py` provides:

- **Auto-skip outside the VM**: `pytest_ignore_collect` checks `os.geteuid() == 0` and `os.access("/root", os.W_OK)`. Tests are silently skipped when run on the host or in CI.
- **`run(cmd, timeout=10)`**: Shell command helper returning `CompletedProcess`. All tests use this instead of calling `subprocess` directly.
- **`output_dir` fixture**: Returns `/root/tests` (created automatically via `autouse` fixture). Tests that write temp files use this shared directory.

### Layered Testing

`test_network.py` orders tests from L1 (guest plumbing) through L7 (throughput) so that a failure at a lower layer immediately pinpoints the root cause. If L2 (net-proxy TCP) fails, there is no point debugging L4 (HTTP over MITM) -- the proxy is not listening. This structure eliminates cascading false failures.

### Parametrization

Several tests use `@pytest.mark.parametrize` to cover lists of items with a single test function:

- **Domain lists**: `test_dns_all_resolve_to_local` checks 5 domains, `test_ai_provider_domain_blocked` checks 2 AI providers
- **Env vars**: `test_ca_env_var_set` checks 3 CA-related environment variables
- **Runtimes**: `test_runtime_version` checks python3 and pip3
- **Writable paths**: `test_writable_mounts` checks 5 paths

The `test_sandbox.py` file also uses a fixture-based parametrization pattern for guest binary paths, yielding each existing binary path to `test_guest_binary_not_writable` and `test_guest_binary_executable`.

## Adding New Tests

1. Add test functions to the appropriate `guest/artifacts/diagnostics/test_<category>.py` file, or create a new `test_<category>.py`.
2. Use `from conftest import run` for shell commands and the `output_dir` fixture for temp files.
3. Tests auto-skip outside the capsem VM -- conftest checks for root user with writable `/root`.
4. Run `just exec "capsem-doctor"` to test. Initrd repacking picks up modified `diagnostics/` files automatically.
5. For new rootfs-level changes (packages, configs), run `just build-assets` instead.
