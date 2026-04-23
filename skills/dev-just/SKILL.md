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
| `just doctor fix` | Doctor + auto-fix all fixable issues in dependency order |
| `just shell` | Daily driver: cross-compile + repack initrd + build + sign + boot temp VM + shell (~10s) |
| `just exec "CMD"` | Run CMD in a fresh temp VM (auto-provisioned and destroyed) |
| `just run-service` | Start capsem-service daemon (builds, signs, launches or reuses) |
| `just ui` | Tauri dev with hot reload (service + Astro dev server on :5173 in Tauri webview) |
| `just dev-frontend` | Frontend-only dev server on :5173 (no Tauri, no VM, mock data) |
| `just build-ui [release]` | **Frontend build + `cargo build -p capsem-app` in lockstep.** Use after any frontend change when running the Tauri binary directly. |
| `just run-ui -- [args]` | `build-ui` then launch `./target/debug/capsem-app` with args (e.g. `--connect <id>`) |
| `just build-assets [arch]` | Full VM asset rebuild via capsem-builder (kernel + rootfs). Default: both arches. |
| `just smoke` | Fast path: audit + doctor --fast + injection + integration + parallel pytest groups (~30s) |
| `just test` | ALL tests: unit (warnings-as-errors) + cov + cross-compile + frontend + python + injection + integration + bench + install e2e |
| `just test-gateway` | Gateway unit + Python mock-UDS tests (no VM needed) |
| `just test-gateway-e2e` | Gateway E2E tests (real service + VMs) |
| `just test-install` | Install e2e in Docker + systemd (real .deb, dpkg -i, pytest) |
| `just coverage` | HTML coverage report across all Rust crates (opens `target/llvm-cov/html/index.html`) |
| `just cross-compile [arch]` | Full Linux build in container (agent + deb) |
| `just bench` | In-VM benchmarks (disk I/O, rootfs, CLI startup, HTTP) + host lifecycle benchmarks |
| `just inspect-session [args]` | Session DB integrity + event summary |
| `just list-sessions` | Table of recent sessions with event counts |
| `just query-session "SQL" [id]` | Run SQL against a session DB (latest with a DB by default) |
| `just update-fixture <src>` | Copy + scrub real session DB as test fixture |
| `just update-prices` | Refresh model pricing JSON |
| `just update-deps` | `cargo update` + `pnpm update` |
| `just logs` | Tail `~/.capsem/run/service.log` |
| `just sandbox-logs <id>` | View process + serial logs for a specific sandbox |
| `just build-host-image` | Build/refresh the `capsem-host-builder` Docker image |
| `just install` | Build release .pkg/.deb + install it locally (postinstall handles codesign, PATH, service registration) |
| `just release [tag]` | Wait for CI to build + publish a pushed tag |
| `just cut-release` | Run test, bump version, stamp changelog, tag, push, wait for CI |
| `just clean` | Remove all build artifacts |
| `just clean all` | clean + Docker prune (full reset) |

## When to use which

| What changed | Command |
|-------------|---------|
| Rust host code | `just smoke` (E2E) or `just test` (full) |
| Guest binary (agent, net-proxy, mcp-server) | `just smoke` (auto-repacks initrd) |
| `capsem-init` | `just smoke` (auto-repacks) |
| In-VM diagnostics (`guest/artifacts/diagnostics/`) | `just smoke` |
| Guest config (`guest/config/`) or rootfs packages | `just build-assets` then `just shell` |
| Frontend components | `just ui` (iterate) then `just test` (validate) |
| Frontend standalone (no VM) | `just dev-frontend` |
| Tauri binary (not dev) | `just build-ui` then `just run-ui` |
| Telemetry pipelines | `just exec "<cmd>"` then `just inspect-session` |
| Gateway code | `just test-gateway` (unit) or `just test-gateway-e2e` (real VMs) |
| Service HTTP API / CLI / MCP | `just smoke` (parallel pytest groups cover all three) |
| Install / postinst / systemd flow | `just test-install` |
| Pre-release | `just test` |
| Ship | `just cut-release` |

## Dependency chains

```
shell            -> _check-assets + _pack-initrd + _ensure-service (_sign + build)
ui               -> _ensure-setup + _pnpm-install + run-service
run-service      -> _check-assets + _pack-initrd + _ensure-service
exec             -> run-service
build-assets     -> _install-tools + _clean-stale (inline: doctor, capsem-builder kernel + rootfs)
build-ui         -> _pnpm-install (pnpm build + cargo build -p capsem-app)
smoke            -> _install-tools + _pnpm-install + _check-assets + _pack-initrd + _ensure-service
test             -> _install-tools + _clean-stale + _pnpm-install + _generate-settings
                    + _check-assets + _pack-initrd
bench            -> _ensure-setup + _check-assets + _pack-initrd + _ensure-service
test-gateway-e2e -> _check-assets + _pack-initrd + _sign
test-install     -> _build-host
install          -> _pnpm-install + _stamp-version + _check-assets + _pack-initrd
cut-release      -> test + _stamp-version
```

`_`-prefixed recipes are internal (hidden from `just --list`).

## Docker disk management

Docker builds (`build-assets`, `cross-compile`, `test-install`) accumulate images, build cache, and stopped containers inside the Colima VM. The `_docker-gc` helper runs automatically after each of these recipes to prevent unbounded disk growth:

- Removes stopped containers
- Prunes unused images older than 72h
- Prunes build cache older than 72h
- Runs `fstrim` on the Colima VM disk to release freed space back to macOS

The Colima VM uses a Virtualization.framework raw disk that only grows, never shrinks on its own. Without `fstrim`, Docker prune frees space inside the VM but macOS never gets it back. This is why `_docker-gc` always trims after pruning.

For a full manual reset: `just clean all` (removes all build artifacts + aggressive Docker prune).

## Tauri gotcha: frontend is embedded at cargo build time

`tauri::generate_context!()` reads `tauri.conf.json` `frontendDist: ../../frontend/dist` and **bakes every file under that directory into the Rust binary** during `cargo build`. Consequences:

- Rebuilding only the frontend (`pnpm run build`) has **zero effect** on a running `./target/**/capsem-app` -- the binary still carries the old bundle.
- After any edit to `frontend/**`, you must `cargo build -p capsem-app` for the change to reach the Tauri app.
- `just ui` (`cargo tauri dev`) sidesteps this by serving `http://localhost:5173` directly -- no embedding happens in dev mode.
- For manual launches, always go through `just build-ui` / `just run-ui`, never raw `pnpm run build` followed by re-running an already-compiled binary.

Symptom you'll see when you forget: edits to Svelte/CSS don't appear in the window, but `http://localhost:5173` in a browser shows the new version. That's the embed-vs-live split.

## Build log

All build infrastructure (runner, code signing, generation scripts) logs to `target/build.log`. This is a unified diagnostic log -- never write to stdout from build scripts. The runner (`scripts/run_signed.sh`) and `_generate-settings` both append here.

When debugging build issues, check `target/build.log` first. When writing new build scripts or recipes, always log to this file, never stdout (which contaminates binary output like `mcp-export`).

## First-time setup

```bash
just doctor        # Check tools (colored output, shows fixable issues)
just doctor fix    # Auto-fix missing targets, cargo tools, config files
just build-assets  # Build kernel + rootfs (~10 min, needs docker)
just shell         # Boot a temp VM and drop into a shell
```

Or use bootstrap which does all of this:

```bash
sh scripts/bootstrap.sh   # Installs deps + runs doctor fix
```

## Daily dev

`just shell` is the daily driver. It cross-compiles the guest agent, repacks the initrd, builds the host binary, codesigns, boots the VM, and drops into a shell. For a one-shot command use `just exec "CMD"`. For UI iteration use `just ui` (Tauri dev with hot reload).

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

`just cross-compile [arch]` is a debug/verification tool that builds everything in a container: agent binaries, frontend, and the full Tauri `.deb`. It's not in the daily `just shell` path -- `_pack-initrd` calls `cross_compile_agent()` directly for agent-only builds.

Guest binaries target `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`. Per-arch named volumes (`capsem-agent-target-{arch}`) cache build artifacts separately to prevent cache clobbering.
