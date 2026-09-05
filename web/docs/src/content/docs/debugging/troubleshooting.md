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
| `CAPSEM_ASSETS_DIR` error | Assets not built | `just build-assets code` (first time only) |
| `vmlinuz not found` | Missing kernel asset | `just build-kernel <arch> code` |
| `rootfs.erofs not found` | Missing rootfs asset | `just build-rootfs <arch> code` |

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
| Domain blocked unexpectedly | Matching block/ask rule | Check the active profile/corp enforcement rules and the VM security ledger |
| All HTTPS fails | MITM proxy not running | Check `capsem-doctor -k "net_proxy"` for L2 status |
| Slow downloads | Expected for air-gapped proxy | All traffic routes through the MITM proxy by design |

## AI CLI issues

| Symptom | Cause | Fix |
|---------|-------|-----|
| `claude: command not found` | Not in PATH | Check `/opt/ai-clis/bin` is in PATH: `echo $PATH` |
| `disabled by policy` at boot | Profile/corp rule or broker state blocked materialization | Check profile rules, corp rules, and credential broker status |
| CLI hangs on first run | Waiting for network it can't reach | Check provider HTTP/DNS rules and brokered credential state |

## Disk full / Colima eating all disk space

Docker builds (kernel, rootfs, cross-compile, install tests) accumulate images and build cache inside the Colima VM. The VM disk only grows -- freed space isn't returned to macOS without `fstrim`.

The release gate reads `config/cache.toml`, where every disk, Docker, and Tart
cache declares its description, scope, maximum size, warm size, and prune
strategy. Cache/image release happens only after the declared last consumer.
`just test` preserves a bounded runtime report and IronBank logs under
`cache/target/tests/evidence/` on failure.

If an existing Colima disk cannot hold the configured Docker maximum plus the
active build, expand it before the complete gate:

```bash
colima stop
colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8 --disk 200

# Check current state
du -sh ~/.colima                            # host disk usage
docker system df
just cache stats
just cache stats --offline
just cache prune # exact preview; no mutation
```

`stats` inventories repository and native-runtime storage against each cache's
warm and maximum usage contract; `--offline` avoids Docker and Tart probes.
Pruning only selects policy-managed generations and preserves active leases and
structural files. Applied plans require an explicit reason and are journaled.

Gate-owned Rust builds share the stable Cargo target and use the pinned sccache
toolchain. Cargo keeps normal incremental output hot, while sccache can restore
compiled objects after that target is lost or deliberately cleaned.

Use `just cache clean all --apply --reason "intentional cold rebuild"` only
for an intentional cold rebuild. Do not use a broad
manual `docker system prune -af --volumes` during a release investigation; it
deletes the compiler caches and stopped-container evidence needed to diagnose
the failure.

## Running diagnostics

When something goes wrong, `capsem-doctor` is the fastest way to pinpoint the issue:

```bash
just exec "capsem-doctor"          # Full diagnostic suite (~10s)
just exec "capsem-doctor -k sandbox"   # Just sandbox/security checks
just exec "capsem-doctor -k network"   # Just network stack
just exec "capsem-doctor -x"           # Stop on first failure
```

The test suite is layered L1-L7. Failures at lower layers explain failures at higher layers -- fix from the bottom up.

## Inspecting session data

Every VM session records telemetry to a SQLite database:

```bash
just inspect-session              # Most recent session
just inspect-session <id>         # Specific session
```

This shows MCP tool usage, network requests, boot timing, and snapshot operations. Useful for diagnosing slow operations or missing telemetry.
