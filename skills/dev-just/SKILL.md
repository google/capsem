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
| `just smoke` | test + repack + sign + boot + session DB validation (~30s) |
| `just test` | Unit tests (llvm-cov) + agent cross-compile + frontend check |
| `just cross-compile [arch]` | Full Linux build in container (agent + deb + AppImage) |
| `just full-test` | test + capsem-doctor + integration + bench (3x VM boot) |
| `just build-assets` | Full VM asset rebuild via capsem-builder (kernel + rootfs) |
| `just build-kernel [arch]` | Kernel only (default: arm64) |
| `just build-rootfs [arch]` | Rootfs only (default: arm64) |
| `just bench` | In-VM benchmarks (disk I/O, rootfs, CLI startup, HTTP) |
| `just inspect-session [id]` | Session DB integrity + event summary |
| `just list-sessions` | Table of recent sessions with event counts |
| `just query-session "SQL"` | Run SQL against latest session DB |
| `just query-session "SQL" <id>` | Run SQL against specific session DB |
| `just update-fixture <path>` | Copy + scrub real session DB as test fixture |
| `just update-prices` | Refresh model pricing JSON |
| `just install` | doctor + full-test + release .app + sign + /Applications |
| `just cut-release` | Bump version, stamp changelog, tag, push, wait for CI |
| `just clean` | Remove all build artifacts |

## When to use which

| What changed | Command |
|-------------|---------|
| Rust host code | `just smoke` (E2E) or `just test` (unit) |
| Guest binary (agent, net-proxy, mcp-server) | `just smoke` (auto-repacks) |
| `capsem-init` | `just smoke` (auto-repacks) |
| In-VM diagnostics (`guest/artifacts/diagnostics/`) | `just smoke` |
| Guest config (`guest/config/`) or rootfs packages | `just build-assets` then `just run` |
| Frontend components | `just ui` (iterate) then `just test` (validate) |
| Telemetry pipelines | `just run "<cmd>"` then `just inspect-session` |
| Pre-release | `just full-test` |
| Ship | `just cut-release` |

## Dependency chains

```
run            -> audit + _check-assets + _generate-settings + _pack-initrd -> _sign -> _compile -> _frontend
test           -> audit + _install-tools + _generate-settings
full-test      -> test + _check-assets + _pack-initrd + _sign
build-assets   -> doctor + _install-tools + audit (capsem-builder: kernel + rootfs)
install        -> doctor + full-test
```

`_`-prefixed recipes are internal (hidden from `just --list`).

## Build log

All build infrastructure (runner, code signing, generation scripts) logs to `target/build.log`. This is a unified diagnostic log -- never write to stdout from build scripts. The runner (`scripts/run_signed.sh`) and `_generate-settings` both append here.

When debugging build issues, check `target/build.log` first. When writing new build scripts or recipes, always log to this file, never stdout (which contaminates binary output like `mcp-export`).

## First-time setup

```bash
just doctor        # Verify tools
just build-assets  # Build kernel + rootfs (~10 min, needs docker)
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

On macOS, agent binaries are compiled inside a Linux container (docker) via `cross_compile_agent()` in `docker.py`. This avoids needing `rust-lld`, musl targets, or `llvm-tools` on the host. On Linux (CI), cargo builds natively.

`just cross-compile [arch]` is a debug/verification tool that builds everything in a container: agent binaries, frontend, and the full Tauri app (deb + AppImage). It's not in the daily `just run` path -- `_pack-initrd` calls `cross_compile_agent()` directly for agent-only builds.

Guest binaries target `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`. Per-arch named volumes (`capsem-agent-target-{arch}`) cache build artifacts separately to prevent cache clobbering.
