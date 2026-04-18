---
title: Just Recipes
description: Complete reference for all just recipes -- the single entry point for building, testing, and shipping Capsem.
sidebar:
  order: 10
---

[just](https://just.systems) is the task runner. Every build, test, and release workflow goes through the justfile. Run `just --list` to see all public recipes.

## Daily development

| Recipe | What it does | Time |
|--------|-------------|------|
| `just run` | Cross-compile guest + repack initrd + build host + codesign + boot VM | ~10s |
| `just run "CMD"` | Same, but run CMD inside the VM and exit | ~10s |
| `just dev` | Hot-reloading Tauri app (frontend + Rust, full desktop app) | continuous |
| `just ui` | Frontend-only dev server with mock data (no VM needed) | continuous |

`just run` is the daily driver. It only rebuilds what changed -- if you edited Rust code, it recompiles; if you changed a guest script, it repacks the initrd. See [Life of a Build](./stack) for the full pipeline.

## Testing

| Recipe | What it does | Boots VM? |
|--------|-------------|-----------|
| `just test` | Unit tests (llvm-cov) + agent cross-compile + frontend check + Python schema tests | No |
| `just run "capsem-doctor"` | In-VM diagnostic suite (VirtioFS, networking, binaries, permissions) | Yes |
| `just full-test` | All of the above + injection test + integration test + benchmarks | Yes (3x) |
| `just test-injection` | Boot VM with generated configs, verify all injection paths | Yes |
| `just bench` | In-VM benchmarks (disk I/O, rootfs read, CLI startup, HTTP latency) | Yes |

Three-tier testing policy:
1. `just test` -- catches compile errors, regressions, type issues
2. `just run "capsem-doctor"` -- catches VirtioFS, networking, and guest binary issues
3. `just full-test` -- full validation before release

## VM image builds

| Recipe | What it does | Time |
|--------|-------------|------|
| `just build-assets` | Full rebuild: kernel + rootfs via capsem-builder (needs Docker) | ~10 min |
| `just build-kernel [arch]` | Kernel only (default: arm64) | ~5 min |
| `just build-rootfs [arch]` | Rootfs only (default: arm64) | ~8 min |
| `just cross-compile [arch]` | Full Linux build in container: agent binaries + deb + AppImage | ~15 min |
| `just full-run "CMD"` | `build-assets` then `run` (full rebuild + boot) | ~10 min |

You only need `just build-assets` on first setup or when `guest/config/` changes (new packages, rootfs changes). Day-to-day, `just run` repacks the initrd without rebuilding images.

## Session inspection

| Recipe | What it does |
|--------|-------------|
| `just inspect-session [id]` | Session DB integrity check + event summary (latest by default) |
| `just list-sessions` | Table of recent sessions with event counts per table |
| `just query-session "SQL" [id]` | Run raw SQL against a session DB |
| `just update-fixture <path>` | Copy + scrub a real session DB as test fixture |

## Dependency management

| Recipe | What it does |
|--------|-------------|
| `just audit` | Check for known vulnerabilities in Rust + npm deps |
| `just update-deps` | `cargo update` + `pnpm update` to latest compatible versions |
| `just update-prices` | Refresh model pricing JSON from upstream |
| `just doctor` | Check tools, colored output, structured recap (exits 1 if failures) |
| `just doctor-fix` | Doctor + auto-fix all fixable issues in dependency order |

## Release

| Recipe | What it does |
|--------|-------------|
| `just cut-release` | Run tests, bump version, stamp changelog, tag, push, wait for CI |
| `just release [tag]` | Wait for CI to build + publish an existing tag |
| `just install` | Full validation (doctor + full-test), for pre-release checks |

## Cleanup

| Recipe | What it does |
|--------|-------------|
| `just clean` | Remove Rust + frontend build artifacts |
| `just clean-all` | Deep clean: build artifacts + container images + docker cache |

## Dependency chains

Recipes automatically pull in their prerequisites. You never need to run setup steps manually.

```
run            -> audit -> _ensure-setup (auto-runs doctor on first use)
               -> _check-assets + _generate-settings + _pack-initrd -> _sign -> _compile -> _frontend

test           -> _install-tools + audit + _generate-settings

full-test      -> test + _check-assets + _pack-initrd + _sign

build-assets   -> doctor + _install-tools + audit

dev            -> _ensure-setup + _pnpm-install

install        -> doctor + full-test
```

`_`-prefixed recipes are internal (hidden from `just --list`). Key internal recipes:

| Recipe | What it does |
|--------|-------------|
| `_ensure-setup` | Checks for `.dev-setup` sentinel, runs `doctor` if missing |
| `_install-tools` | Auto-installs Rust targets, components, and cargo tools |
| `_pack-initrd` | Cross-compiles guest agent + repacks initrd with latest binaries |
| `_sign` | Codesigns the binary with virtualization entitlement |
| `_check-assets` | Verifies VM assets exist, tells you to run `build-assets` if not |
| `_generate-settings` | Exports MCP tool defs + generates schema/defaults/mock data |
| `_frontend` | `pnpm build` (Astro + Svelte) |
| `_compile` | `cargo build -p capsem` |
