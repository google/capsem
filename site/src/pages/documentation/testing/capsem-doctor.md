---
layout: ../../../layouts/Doc.astro
title: capsem-doctor
description: In-VM diagnostic suite that verifies sandbox integrity at every boot.
lastUpdated: "2026-03-11"
tags: ["testing", "diagnostics", "vm"]
---

`capsem-doctor` is a pytest-based diagnostic suite that runs inside the guest VM. It validates sandbox isolation, network policy, kernel hardening, runtime environment, and AI CLI configuration. Tests are baked into the rootfs and run automatically during CI.

## Running

```bash
# Boot VM, run all diagnostics, shut down (~10s)
just run "capsem-doctor"

# Inside a running VM
capsem-doctor              # all tests
capsem-doctor -k sandbox   # subset by keyword
capsem-doctor -x           # stop on first failure
```

## Test suites

### Sandbox security (`test_sandbox.py`)

Validates the VM's isolation model.

| Test | What it verifies |
|---|---|
| `test_clock_is_synchronized` | Guest clock within 60s of host |
| `test_squashfs_is_immutable` | `/dev/vda` is squashfs |
| `test_overlay_configured` | Root is overlay with lowerdir + upperdir |
| `test_overlay_writes_are_ephemeral` | Writes go to tmpfs, not squashfs |
| `test_writable_mounts` | `/root`, `/tmp`, `/run`, `/var/log`, `/var/tmp` are writable |
| `test_guest_binary_not_writable` | capsem binaries are chmod 555 |
| `test_guest_binary_executable` | capsem binaries are executable |
| `test_no_setuid_binaries` | No setuid files in rootfs |
| `test_no_setgid_binaries` | No setgid files in rootfs |
| `test_no_kernel_modules` | `modprobe` fails (MODULES=n) |
| `test_no_dev_mem` | `/dev/mem` absent |
| `test_no_dev_port` | `/dev/port` absent |
| `test_no_proc_kcore` | `/proc/kcore` not readable |
| `test_proc_modules_empty` | `/proc/modules` empty or absent |
| `test_no_debugfs` | debugfs not mounted |
| `test_no_ipv6` | IPv6 disabled |
| `test_no_kallsyms` | `/proc/kallsyms` empty or absent |
| `test_kernel_cmdline_has_ro` | `ro` in cmdline |
| `test_init_on_alloc` | `init_on_alloc=1` in cmdline |
| `test_slab_nomerge` | `slab_nomerge` in cmdline |
| `test_page_alloc_shuffle` | `page_alloc.shuffle=1` in cmdline |
| `test_seccomp_available` | Seccomp line in `/proc/self/status` |
| `test_swap_active` | Swap on scratch disk |
| `test_loopback_interface_up` | `lo` is UP |

**Network isolation:**

| Test | What it verifies |
|---|---|
| `test_dummy_interface_exists` | `dummy0` NIC present |
| `test_dns_resolves_to_local` | DNS resolves to 10.0.0.1 |
| `test_iptables_redirect` | Port 443 redirected to 10443 |
| `test_net_proxy_running` | `capsem-net-proxy` running |
| `test_allowed_domain` | Full HTTPS handshake to allowed domain |
| `test_denied_domain` | Denied domain returns 403 or refused |
| `test_no_real_nics` | Only `lo` and `dummy0` exist |

**Process integrity:**

| Test | What it verifies |
|---|---|
| `test_pty_agent_running` | `capsem-pty-agent` running |
| `test_dnsmasq_running` | `dnsmasq` running |
| `test_no_systemd` | No service manager |
| `test_no_sshd` | No remote access |
| `test_no_cron` | No scheduled tasks |

### Network & MITM proxy (`test_network.py`)

Tests ordered from low-level to high-level so failures pinpoint the exact broken layer.

| Test | What it verifies |
|---|---|
| `test_dummy0_has_ip` | dummy0 has 10.0.0.1 |
| `test_dnsmasq_responds` | DNS on 127.0.0.1:53 works |
| `test_dns_all_resolve_to_local` | All DNS queries -> 10.0.0.1 |
| `test_iptables_redirect_443_to_10443` | iptables REDIRECT rule |
| `test_net_proxy_listening` | TCP accepted on 127.0.0.1:10443 |
| `test_tcp_443_reaches_proxy` | 443 redirected to net-proxy |
| `test_vsock_bridge_delivers_bytes` | Raw bytes through proxy |
| `test_tls_handshake_completes` | TLS handshake via MITM |
| `test_tls_cert_from_capsem_ca` | Cert signed by Capsem CA |
| `test_curl_https_with_skip_verify` | curl -k gets HTTP response |
| `test_mitm_ca_cert_file_exists` | CA cert file present |
| `test_mitm_ca_in_system_bundle` | CA in system trust store |
| `test_certifi_includes_capsem_ca` | CA in Python certifi |
| `test_curl_allowed_domain_ca_trusted` | curl without -k succeeds |
| `test_python_urllib_https_trusted` | Python urllib TLS works |
| `test_ca_env_var_set` | SSL_CERT_FILE, REQUESTS_CA_BUNDLE, NODE_EXTRA_CA_CERTS set |
| `test_denied_domain_rejected` | Denied domain -> 403 |
| `test_post_to_random_domain_denied` | POST to non-allowed domain -> 403 |
| `test_ai_provider_domain_blocked` | AI provider domains blocked unless allowed |
| `test_http_port_80_not_proxied` | Port 80 not proxied |
| `test_non_standard_port_fails` | Non-443 ports fail |
| `test_direct_ip_no_route` | Direct IP has no route |
| `test_proxy_download_throughput` | 100MB download above minimum speed |

### Environment (`test_environment.py`)

| Test | What it verifies |
|---|---|
| `test_term_is_xterm_256color` | TERM set correctly |
| `test_home_is_root` | HOME is /root |
| `test_path_includes_standard_dirs` | PATH has /usr/local/bin, /usr/bin |
| `test_python_venv_active` | Python venv activated |
| `test_shell_is_bash` | Bash installed |
| `test_kernel_is_linux_6` | Linux 6.x kernel |
| `test_architecture_is_aarch64` | ARM64 architecture |
| `test_proc_mounted` | /proc mounted |
| `test_sys_mounted` | /sys mounted |
| `test_dev_mounted` | /dev mounted |
| `test_dev_pts_mounted` | /dev/pts mounted |
| `test_root_is_ext4_scratch_disk` | /root on ext4 scratch disk |
| `test_root_scratch_disk_size` | Scratch disk >= 4GB |
| `test_tmp_is_writable` | /tmp writable |
| `test_rootfs_is_overlay` | Root is overlay mount |

### Runtimes (`test_runtimes.py`)

| Test | What it verifies |
|---|---|
| `test_runtime_version` | python3, node, npm, pip3, git respond to --version |
| `test_pip_install_works` | pip install succeeds |
| `test_uv_pip_install_works` | uv pip install succeeds |
| `test_npm_install_global_works` | npm -g install works |
| `test_npm_install_local_works` | npm local install works |
| `test_python_execution` | Python stdlib + file I/O |
| `test_node_execution` | Node.js file I/O |
| `test_git_workflow` | git init, commit, log |

### Utilities (`test_utilities.py`)

Verifies ~36 unix utilities are available: coreutils, text processing, network tools, system inspection.

### File I/O workflows (`test_workflows.py`)

| Test | What it verifies |
|---|---|
| `test_file_write_read` | Text write/read |
| `test_python_json_roundtrip` | Python JSON roundtrip |
| `test_node_file_roundtrip` | Node writes, Python reads |
| `test_pipe_workflow` | Shell pipe chains |
| `test_large_file_write` | 10MB file to tmpfs |

### AI CLI (`test_ai_cli.py`)

| Test | What it verifies |
|---|---|
| `test_ai_cli_installed` | claude/gemini/codex in PATH |
| `test_ai_cli_help` | --help runs without crash |
| `test_gemini_api_key_no_duplicate` | No GOOGLE_API_KEY alongside GEMINI_API_KEY |
| `test_gemini_settings_exist` | Gemini settings.json seeded |
| `test_gemini_projects_exist` | /root registered as project |
| `test_gemini_trusted_folders_exist` | /root is trusted |
| `test_google_ai_domain_allowed` | Google AI domain reachable |

### Boot injection (`test_injection.py`)

Verifies that env vars, boot files, and git credentials injected by the host arrived correctly inside the guest.

### MCP gateway (`test_mcp.py`)

| Test | What it verifies |
|---|---|
| `test_mcp_server_binary_exists` | capsem-mcp-server installed |
| `test_mcp_initialize` | JSON-RPC initialize handshake |
| `test_mcp_tools_list` | Three built-in HTTP tools |
| `test_mcp_fetch_http_allowed_domain` | fetch_http on allowed domain |
| `test_mcp_fetch_http_blocked_domain` | fetch_http on blocked domain returns error |
| `test_mcp_grep_http_finds_matches` | grep_http finds content |
| `test_mcp_http_headers_allowed_domain` | http_headers returns status |
| `test_fastmcp_available` | fastmcp Python package importable |

## Adding new tests

1. Add test functions to the appropriate `images/diagnostics/test_*.py` file, or create a new `test_<category>.py`
2. Use `from conftest import run` for shell commands, `output_dir` fixture for temp files
3. Tests auto-skip outside the capsem VM (conftest.py checks for root + writable /root)
4. Rebuild rootfs with `just build-assets` to pick up new test files
5. Verify with `just run "capsem-doctor"`
