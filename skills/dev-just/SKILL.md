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
| `just build-assets [arch]` | Full VM asset rebuild through capsem-admin/profile materialization and the private Python builder backend. Default: both arches. |
| `just smoke` | Hermetic smoke gate: audit + doctor + injection + integration + parallel pytest groups |
| `just test` | Canonical release gate: unit + coverage + VM suites + both Linux packages + Docker/systemd install + exact `.pkg` install/glow-up in a clean Tart Mac |
| `just test-gateway` | Gateway unit + mock-UDS tests (no VM needed) |
| `just test-gateway-e2e` | Gateway E2E tests (real service + VMs) |
| `just test-install` | Install e2e in Docker + systemd, then hermetic local release glow-up from generated stable/nightly channels |
| `just coverage` | HTML coverage report across all Rust crates (opens `target/llvm-cov/html/index.html`) |
| `just cross-compile [arch]` | Full Linux build in container (agent + deb) |
| `just benchmark` | Standard artifact-recording benchmark suite across host-native, in-VM, lifecycle/fork/parallel, and Security Engine lanes |
| `just bench` | Alias for `just benchmark` |
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
| `just prepare-release` | Run local full gate, stamp version/changelog, and commit an untagged candidate |
| `just qualify-release` | Run and wait for the remote canonical Linux gate on exact published `HEAD` |
| `just cut-release` | Verify exact-SHA remote qualification, then create the local immutable tag |
| `just clean` | Remove all build artifacts |
| `just clean all` | clean + Docker prune (full reset) |

## When to use which

| What changed | Command |
|-------------|---------|
| Rust host code | `just smoke` (E2E) or `just test` (full) |
| Guest binary (agent, net-proxy, mcp-server) | `just smoke` (auto-repacks initrd) |
| `capsem-init` | `just smoke` (auto-repacks) |
| In-VM diagnostics (`guest/artifacts/diagnostics/`) | `just smoke` |
| Profile payloads (`config/profiles/<id>/`) or rootfs packages | `just build-assets` then `just shell` |
| Frontend components | `just ui` (iterate) then `just test` (validate) |
| Frontend standalone (no VM) | `just dev-frontend` |
| Tauri binary (not dev) | `just build-ui` then `just run-ui` |
| Telemetry pipelines | `just exec "<cmd>"` then `just inspect-session` |
| Gateway code | `just test-gateway` (unit) or `just test-gateway-e2e` (real VMs) |
| Service HTTP API / CLI / MCP | `just smoke` (parallel pytest groups cover all three) |
| Linux install / postinst / systemd / release glow-up flow | `just test-install` |
| macOS package / postinstall / installed-product glow-up flow | `python3 scripts/macos_release_glowup.py` |
| Pre-release | `just test` |
| Prepare candidate | `just prepare-release`, then manually push only `main` |
| Qualify candidate | `just qualify-release` (no tag or publication) |
| Ship qualified candidate | `just cut-release`, push the tag, then manually dispatch `release.yaml` for that exact tag/channel |

## Dependency chains

```
shell            -> _check-assets + _pack-initrd + _ensure-service (_sign + build)
ui               -> _ensure-setup + _pnpm-install + run-service
run-service      -> _check-assets + _pack-initrd + _ensure-service
exec             -> run-service
build-assets     -> _install-tools + _clean-stale (inline: doctor, capsem-admin image build)
build-ui         -> _frontend-dist (pnpm build + cargo build -p capsem-app)
smoke            -> _install-tools + _frontend-dist + _check-assets + _pack-initrd + _ensure-service
test             -> _install-tools + _clean-stale + _frontend-dist + _generate-settings
                    + _check-assets + _pack-initrd
bench            -> _ensure-setup + _check-assets + _pack-initrd + _ensure-service
test-gateway-e2e -> _check-assets + _pack-initrd + _sign
test-install     -> Docker package install + generated local stable/nightly glow-up
scripts/macos_release_glowup.py -> production package + clean Tart install + physical-host exact-payload VZ boot
install          -> _pnpm-install + _stamp-version + _check-assets + _pack-initrd
prepare-release  -> test + _stamp-version (commit only, no tag)
qualify-release  -> exact origin/main SHA + release-qualification.yaml
cut-release      -> exact successful qualification + local tag (no stamp/commit)
```

`_`-prefixed recipes are internal (hidden from `just --list`).

`_ensure-service` honors `CAPSEM_HOME` / `CAPSEM_RUN_DIR` for isolated
smoke/test runs and assigns the gateway an ephemeral port in that mode. This
keeps test services from colliding with the user's installed gateway on the
default developer port.

## Docker disk management

Docker builds (`build-assets`, `cross-compile`, `test-install`) accumulate images, build cache, and stopped containers inside the Colima VM. The `_docker-gc` helper runs at owning outer recipe boundaries to prevent unbounded disk growth:

- Removes stopped containers
- Prunes dangling images older than 72h; it never prunes tagged images
- Prunes build cache older than 72h
- Runs `fstrim` on the Colima VM disk to release freed space back to macOS

Release-gate capacity is declared once in `config/storage-policy.toml`.
`scripts/ensure-docker-space.sh <rail>` accepts a named rail, never numeric
limits. The default policy requires 24 GiB free, preserves a 24 GiB BuildKit
cohort, supports existing 96 GiB Colima disks, and recommends 128 GiB for new
runtimes. Resource entries declare owners, last consumers, and release
boundaries; do not release an image or volume before its declared last
consumer. Docker and Tart actions append byte-accounted JSONL ledgers under
`target/storage/`. Review the resolved policy without mutation using:

```bash
uv run python scripts/docker-storage-policy.py show --rail assets --offline
```

On a red `just test`, Docker usage, image/container metadata, build logs, and
bounded IronBank failure logs are captured under `test-artifacts/` before the
next candidate can clean its staging directory. The same policy keeps at least
five recent failures, caps retention at 30 runs/14 days/8 GiB, and excludes
regenerable VM disk/kernel payloads.

The Docker daemon is shared by concurrent lanes and worktrees. Never add
`docker image prune -a` to an automatic path or call `_docker-gc` from an
internal primitive used in parallel. A cached image can be newly tagged while
retaining an old creation timestamp, so age-filtered `prune -a` can delete a
different lane's live image.

The Colima VM uses a Virtualization.framework raw disk that only grows, never shrinks on its own. Without `fstrim`, Docker prune frees space inside the VM but macOS never gets it back. This is why `_docker-gc` always trims after pruning.

For a full manual reset: `just clean all` (removes all build artifacts + aggressive Docker prune).

## Tauri gotcha: frontend is embedded at cargo build time

`tauri::generate_context!()` reads `tauri.conf.json` `frontendDist: ../../frontend/dist` and **bakes every file under that directory into the Rust binary** during `cargo build`. Consequences:

- Recipes that compile the full workspace (`just smoke`, `just test`) must build `_frontend-dist` before `cargo clippy --workspace --all-targets`; otherwise `capsem-app` fails during macro expansion if `frontend/dist` is missing.
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
sh bootstrap.sh   # Installs deps + runs doctor fix
```

## Daily dev

`just shell` is the daily driver. It cross-compiles the guest agent, repacks the initrd, builds the host binary, codesigns, boots the VM, and drops into a shell. For a one-shot command use `just exec "CMD"`. For UI iteration use `just ui` (Tauri dev with hot reload).

## Builder CLI

The capsem-builder Python package is the backend implementation. Product image
truth enters through `capsem-admin` and profile-owned config, not direct
builder authoring commands:

```bash
capsem-admin profile check --profile config/profiles/<profile-id>/profile.toml --config-root config
just build-assets              # Build profile-owned VM assets through the profile-derived build rail
just _materialize-config       # Materialize generated runtime profile config
```

The only public `capsem-builder` helper commands are backend support commands
used by just/CI: `doctor`, `validate-skills`, `agent`, and `audit`.
There is no public `capsem-builder build`, `validate`, `inspect`, `--dry-run`,
`mcp`, or render-only rail. If the product contract needs a new image input,
add it to the profile/corp/settings config model and the `capsem-admin`
validation path.

## Cross-compilation

On macOS, agent binaries are compiled inside a Linux container (docker) via `cross_compile_agent()` in `docker.py`. This avoids needing `rust-lld`, musl targets, or `llvm-tools` on the host. On Linux (CI), cargo builds natively.

`just cross-compile [arch]` is a debug/verification tool that builds everything in a container: agent binaries, frontend, and the full Tauri `.deb`. It's not in the daily `just shell` path -- `_pack-initrd` calls `cross_compile_agent()` directly for agent-only builds.

Guest binaries target `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`. Per-arch named volumes (`capsem-agent-target-{arch}`) cache build artifacts separately to prevent cache clobbering.
