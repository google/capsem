---
name: dev-just
description: Capsem development toolchain -- all just recipes, what they do, when to use which, and dependency chains. Use when you need to know how to build, run, test, or ship Capsem, or when deciding which just command to run for a given change. This is the toolchain reference.
---

# Capsem Toolchain

All workflows use `just` (not make). The justfile is the single entry point.

## Quick reference

| Command | What it does |
|---------|-------------|
| `just doctor` | Check all required tools, colored output, structured recap |
| `just doctor-fix` | Doctor + auto-fix all fixable issues in dependency order |
| `just ui` | Tauri dev with hot reload (frontend + Rust) |
| `just dev-frontend` | Frontend-only dev server on :5173 (no Tauri, no VM) |
| `just build-ui [release]` | **Frontend build + `cargo build -p capsem-ui` in lockstep.** Use after any frontend change when running the Tauri binary directly. |
| `just run-ui -- [args]` | `build-ui` then launch `./target/debug/capsem-ui` with args (e.g. `--connect <id>`). |
| `just run` | Cross-compile + repack initrd + build + sign + boot VM (~10s) |
| `just run "CMD"` | Same but run CMD instead of interactive shell |
| `just smoke` | test + repack + sign + boot + session DB validation (~30s) |
| `just test` | ALL tests: unit (warnings-as-errors) + cross-compile + frontend + all integration + injection + bench |
| `just cross-compile [arch]` | Full Linux build in container (agent + deb + AppImage) |
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
| `just install` | doctor + test + release .app + sign + /Applications |
| `just cut-release` | Bump version, stamp changelog, tag, push, wait for CI |
| `just clean` | Remove all build artifacts |
| `just clean-all` | clean + Docker prune (full reset) |
| **Integration test recipes** | |
| `just test-service` | Service HTTP API tests (provision, exec, logs, delete) |
| `just test-cli` | CLI integration tests via subprocess |
| `just test-mcp` | MCP black-box tests |
| `just test-session` | Session.db telemetry tests |
| `just test-snapshots` | Snapshot lifecycle tests |
| `just test-isolation` | Multi-VM isolation tests |
| `just test-security` | Security invariant tests |
| `just test-config` | Config obedience tests |
| `just test-bootstrap` | Setup/install flow tests (no VM) |
| `just test-stress` | 5-VM concurrency + rapid create/delete |
| `just test-build-chain` | Build chain E2E: cargo build -> codesign -> pack -> manifest -> boot |
| `just test-guest` | Guest validation: network, services, filesystem, env |
| `just test-cleanup` | VM cleanup: process killed, socket removed, no zombies |
| `just test-codesign` | Codesigning strict: all binaries signed (FAIL not skip) |
| `just test-serial` | Serial console logs + boot timing < 30s |
| `just test-session-lifecycle` | Session.db lifecycle: exists, schema, events, survives shutdown |
| `just test-config-runtime` | Config applied in guest: CPU, RAM, blocked domains |
| `just test-recipes` | Just recipe smoke tests (no VM) |
| `just test-recovery` | Recovery: stale sockets, orphaned processes, double service |
| `just test-rootfs` | Rootfs artifact validation (no VM) |
| `just test-session-exhaustive` | Exhaustive per-table session.db data + FK validation |
| `just test-vm` | All VM-requiring Phase 3 tests combined |

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
| Service HTTP API | `just test-service` |
| CLI subcommands | `just test-cli` |
| MCP server/gateway | `just test-mcp` |
| Session.db schema or writer | `just test-session` + `just test-session-lifecycle` + `just test-session-exhaustive` |
| VM lifecycle (create/delete) | `just test-cleanup` + `just test-recovery` |
| Network policy or proxy | `just test-guest` + `just test-config-runtime` |
| Codesigning or entitlements | `just test-codesign` |
| Build pipeline | `just test-build-chain` |
| Rootfs artifacts | `just test-rootfs` |
| Just recipes | `just test-recipes` |
| Pre-release | `just test` |
| Ship | `just cut-release` |

## Dependency chains

```
run            -> audit + _check-assets + _generate-settings + _pack-initrd -> _sign -> _compile -> _frontend
test           -> audit + _install-tools + _generate-settings + _check-assets + _pack-initrd
                  (Rust warnings-as-errors + llvm-cov + cross-compile + frontend + ALL Python tests
                   + injection + integration + benchmarks)
test-vm        -> test-build-chain + test-guest + test-cleanup + test-codesign + test-serial
                  + test-session-lifecycle + test-config-runtime + test-recovery
build-assets   -> doctor + _install-tools + audit (capsem-builder: kernel + rootfs)
install        -> doctor + test
cut-release    -> test
```

`_`-prefixed recipes are internal (hidden from `just --list`).

## Docker disk management

Docker builds (build-assets, cross-compile, test-install) accumulate images, build cache, and stopped containers inside the Colima VM. The `_docker-gc` recipe runs automatically after each of these recipes to prevent unbounded disk growth:

- Removes stopped containers
- Prunes unused images older than 72h
- Prunes build cache older than 72h
- Runs `fstrim` on the Colima VM disk to release freed space back to macOS

The Colima VM uses a Virtualization.framework raw disk that only grows, never shrinks on its own. Without `fstrim`, Docker prune frees space inside the VM but macOS never gets it back. This is why `_docker-gc` always trims after pruning.

For a full manual reset: `just clean-all` (removes all build artifacts + aggressive Docker prune).

## Tauri gotcha: frontend is embedded at cargo build time

`tauri::generate_context!()` reads `tauri.conf.json` `frontendDist: ../../frontend/dist` and **bakes every file under that directory into the Rust binary** during `cargo build`. Consequences:

- Rebuilding only the frontend (`pnpm run build`) has **zero effect** on a running `./target/**/capsem-ui` -- the binary still carries the old bundle.
- After any edit to `frontend/**`, you must `cargo build -p capsem-ui` for the change to reach the Tauri app.
- `just ui` (`cargo tauri dev`) sidesteps this by serving `http://localhost:5173` directly -- no embedding happens in dev mode.
- For manual launches, always go through `just build-ui` / `just run-ui`, never raw `pnpm run build` followed by re-running an already-compiled binary.

Symptom you'll see when you forget: edits to Svelte/CSS don't appear in the window, but `http://localhost:5173` in a browser shows the new version. That's the embed-vs-live split.

## Build log

All build infrastructure (runner, code signing, generation scripts) logs to `target/build.log`. This is a unified diagnostic log -- never write to stdout from build scripts. The runner (`scripts/run_signed.sh`) and `_generate-settings` both append here.

When debugging build issues, check `target/build.log` first. When writing new build scripts or recipes, always log to this file, never stdout (which contaminates binary output like `mcp-export`).

## First-time setup

```bash
just doctor        # Check tools (colored output, shows fixable issues)
just doctor-fix    # Auto-fix missing targets, cargo tools, config files
just build-assets  # Build kernel + rootfs (~10 min, needs docker)
just run           # Boot the VM
```

Or use bootstrap which does all of this:

```bash
sh scripts/bootstrap.sh   # Installs deps + runs doctor --fix
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
