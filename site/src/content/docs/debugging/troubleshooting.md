---
title: Troubleshooting
description: Common issues and solutions when running Capsem VMs.
sidebar:
  order: 2
---

## VM won't start

| Symptom | Cause | Fix |
|---------|-------|-----|
| `codesign: command not found` | Xcode CLTools not installed | `xcode-select --install` |
| Entitlement crash on launch | Binary not codesigned | `just doctor` to diagnose, then `just run` (signs automatically) |
| `CAPSEM_ASSETS_DIR` error | Assets not built | `just build-assets` (first time only) |
| `vmlinuz not found` | Missing kernel asset | `just build-kernel` |
| `rootfs.img not found` | Missing rootfs asset | `just build-rootfs` |

## Boot hangs or times out

| Symptom | Cause | Fix |
|---------|-------|-----|
| Stuck at "VsockConnected" | Agent crashed or missing | Rebuild initrd: `just run` repacks automatically |
| Boot > 1 second | Slow venv creation | Check `uv` is on PATH in rootfs; fallback to `python3 -m venv` is 10x slower |
| Network setup slow | DNS/iptables issue | Check `capsem-doctor -k network` for L1-L2 failures |

## Network issues inside VM

| Symptom | Cause | Fix |
|---------|-------|-----|
| `curl: (60) SSL certificate problem` | CA bundle not injected | Check `capsem-doctor -k "ca_env"` |
| Domain blocked unexpectedly | Not in allow list | Check `~/.capsem/user.toml` domain policy |
| All HTTPS fails | MITM proxy not running | Check `capsem-doctor -k "net_proxy"` for L2 status |
| Slow downloads | Expected for air-gapped proxy | All traffic routes through the MITM proxy by design |

## AI CLI issues

| Symptom | Cause | Fix |
|---------|-------|-----|
| `claude: command not found` | Not in PATH | Check `/opt/ai-clis/bin` is in PATH: `echo $PATH` |
| `disabled by policy` at boot | API key not configured | Add key to `~/.capsem/user.toml` |
| CLI hangs on first run | Waiting for network it can't reach | Check provider is in the domain allow list |

## Running diagnostics

When something goes wrong, `capsem-doctor` is the fastest way to pinpoint the issue:

```bash
just run "capsem-doctor"          # Full diagnostic suite (~10s)
just run "capsem-doctor -k sandbox"   # Just sandbox/security checks
just run "capsem-doctor -k network"   # Just network stack
just run "capsem-doctor -x"           # Stop on first failure
```

The test suite is layered L1-L7. Failures at lower layers explain failures at higher layers -- fix from the bottom up.

## Inspecting session data

Every VM session records telemetry to a SQLite database:

```bash
just inspect-session              # Most recent session
just inspect-session <id>         # Specific session
```

This shows MCP tool usage, network requests, boot timing, and snapshot operations. Useful for diagnosing slow operations or missing telemetry.
