---
name: dev-just
description: Capsem development toolchain -- all just recipes, what they do, when to use which, and dependency chains. Use when you need to know how to build, run, test, or ship Capsem, or when deciding which just command to run for a given change. This is the toolchain reference.
---

# Capsem Toolchain

All workflows use `just` (not make). The justfile is the single entry point.

## Quick reference

| Command | What it does |
|---------|-------------|
| `just doctor` | Check all required tools are installed |
| `just dev` | Hot-reload app (frontend + Rust, full Tauri) |
| `just ui` | Frontend-only dev server (mock mode, no VM) |
| `just run` | Cross-compile + repack initrd + build + sign + boot VM (~10s) |
| `just run "CMD"` | Same but run CMD instead of interactive shell |
| `just test` | Unit tests (llvm-cov) + cross-compile + frontend check |
| `just full-test` | test + capsem-doctor + integration + bench (3x VM boot) |
| `just build-assets` | Full VM asset rebuild via capsem-builder (kernel + rootfs) |
| `just build-kernel [arch]` | Kernel only (default: arm64) |
| `just build-rootfs [arch]` | Rootfs only (default: arm64) |
| `just bench` | In-VM benchmarks (disk I/O, rootfs, CLI startup, HTTP) |
| `just inspect-session [id]` | Session DB integrity + event summary |
| `just update-fixture <path>` | Copy + scrub real session DB as test fixture |
| `just update-prices` | Refresh model pricing JSON |
| `just install` | doctor + full-test + release .app + sign + /Applications |
| `just cut-release` | Bump version, stamp changelog, tag, push, wait for CI |
| `just clean` | Remove all build artifacts |

## When to use which

| What changed | Command |
|-------------|---------|
| Rust host code | `just run` or `just test` |
| Guest binary (agent, net-proxy, mcp-server) | `just run` (auto-repacks initrd) |
| `capsem-init` | `just run` (auto-repacks initrd) |
| In-VM diagnostics (`images/diagnostics/`) | `just run "capsem-doctor"` |
| Guest config (`guest/config/`) or rootfs packages | `just build-assets` then `just run` |
| Frontend components | `just ui` (iterate) then `just test` (validate) |
| Telemetry pipelines | `just run "<cmd>"` then `just inspect-session` |
| Pre-release | `just full-test` |
| Ship | `just cut-release` |

## Dependency chains

```
run            -> audit + _check-assets + _pack-initrd -> _sign -> _compile -> _frontend
test           -> audit + _install-tools
full-test      -> test + _check-assets + _pack-initrd + _sign
build-assets   -> doctor + _install-tools + audit (capsem-builder: kernel + rootfs)
install        -> doctor + full-test
```

`_`-prefixed recipes are internal (hidden from `just --list`).

## First-time setup

```bash
just doctor        # Verify tools
just build-assets  # Build kernel + rootfs (~10 min, needs docker/podman)
just run           # Boot the VM
```

## Daily dev

`just run` is the daily driver. It cross-compiles the guest agent, repacks the initrd, builds the host binary, codesigns, and boots the VM. Pass a command string to run non-interactively and exit.

## Builder CLI

The capsem-builder Python package provides config-driven image building:

```bash
uv run capsem-builder doctor guest/       # Check build prerequisites
uv run capsem-builder validate guest/     # Lint guest config
uv run capsem-builder build guest/ --dry-run   # Preview rendered Dockerfiles
uv run capsem-builder build guest/ --arch arm64 # Build for arm64
uv run capsem-builder inspect guest/      # Show config summary
```

## Cross-compilation

Guest binaries target `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`. Linker config is in `.cargo/config.toml` (uses `rust-lld`). Watch for platform-specific types -- e.g., `libc::ioctl` request param is `c_ulong` on macOS but `c_int` on Linux. Use `as _` to let the compiler infer.
