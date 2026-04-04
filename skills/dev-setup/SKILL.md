---
name: dev-setup
description: Setting up a Capsem development environment from scratch. Use when onboarding a new developer, setting up a new machine, or troubleshooting environment issues. Covers prerequisites, first-time setup, tool installation, VM asset builds, container runtime configuration (Colima/Docker memory and CPU requirements), and verification steps.
---

# Developer Setup

## Prerequisites

- **macOS 13+** (Ventura or later) -- required for Virtualization.framework
- **Apple Silicon** (arm64) -- primary target. Intel Macs are not supported for VM features.
- **Docker (via Colima on macOS)** -- needed for `just build-assets` (kernel + rootfs builds)

## Required tools

Run `just doctor` to check all of these:

| Tool | Purpose | Install |
|------|---------|---------|
| Rust (stable) | Host + guest binaries | `rustup` |
| just | Task runner | `cargo install just` |
| pnpm | Frontend package manager | `npm i -g pnpm` |
| Node.js 24+ | Frontend build | `nvm` or `brew install node` |
| uv | Python package manager | `curl -LsSf https://astral.sh/uv/install.sh \| sh` |
| Docker (via Colima on macOS) | VM image builds | `brew install colima docker` (macOS) or `sudo apt install docker.io` (Linux) |
| Docker BuildKit (buildx) | Cross-arch container builds | `brew install docker-buildx` (macOS) or `sudo apt install docker-buildx-plugin` (Linux) |

Rust targets (auto-installed by `just doctor-fix`):
- `aarch64-unknown-linux-musl` -- guest binaries (arm64)
- `x86_64-unknown-linux-musl` -- guest binaries (x86_64)

Cargo tools (auto-installed by `just doctor-fix`):
- `cargo-llvm-cov` -- coverage
- `cargo-audit` -- vulnerability scanner
- `cargo-tauri` -- Tauri CLI
- `b3sum` -- BLAKE3 checksums

## Container runtime setup

On macOS, Docker runs inside a Colima VM. The default memory allocation may be too small -- the rootfs build runs apt installs, npm installs, and curl-based CLI installers concurrently, which can OOM-kill the build (exit code 137).

**Minimum**: 4GB RAM. **Recommended**: 8GB RAM, 8 CPUs.

### Colima (macOS)

```bash
# First-time setup
brew install colima docker
colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8

# Restart with new resources
colima stop
colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8

# Verify
docker info | grep -E 'Total Memory|CPUs'
```

### Linux

Docker runs natively on Linux -- no Colima or memory tuning needed.

```bash
sudo apt install docker.io
```

`just doctor` checks these resources automatically and fails if below minimum.

## First-time setup

```bash
# 1. Clone and enter
git clone <repo> && cd capsem

# 2. Bootstrap (installs deps + runs doctor with auto-fix)
sh scripts/bootstrap.sh

# 3. Build VM assets (kernel + rootfs, ~10 min, needs Docker)
just build-assets

# 4. Boot the VM to verify everything works
just run "echo hello from capsem"
```

Or step by step:

```bash
just doctor          # Check tools (colored output, structured recap)
just doctor-fix      # Auto-fix missing targets, cargo tools, config files
just build-assets    # Build kernel + rootfs (~10 min)
just run "echo hi"   # Verify VM boots
```

If step 4 prints "hello from capsem" and exits cleanly, you're set.

## Daily workflow

```bash
just run              # Build + boot VM interactively (~10s)
just run "CMD"        # Build + boot + run command + exit
just test             # Unit tests + cross-compile + frontend check
just ui               # Frontend dev server (mock mode, no VM)
just dev              # Full Tauri app with hot-reload
```

See `/dev-just` for the complete recipe reference.

## API keys (optional, needed for integration tests)

Create `~/.capsem/user.toml`:
```toml
[providers.anthropic]
api_key = "sk-ant-..."

[providers.google]
api_key = "AIza..."
```

Needed for: `just test` (integration tests exercise real AI API calls), interactive AI sessions inside the VM.

## Claude Code permissions

To avoid repeated permission prompts when using `just` and `capsem` commands, add these to your Claude Code settings. Run `/update-config` or edit `.claude/settings.local.json`:

```json
{
  "permissions": {
    "allow": [
      "Bash(just *)",
      "Bash(uv run *)",
      "Bash(cargo *)",
      "Bash(pnpm *)",
      "Bash(cd frontend && pnpm *)",
      "Bash(npx *)",
      "Bash(python3 scripts/*)",
      "Bash(rustup *)"
    ]
  }
}
```

This allows:
- `just *` -- all recipes (run, test, build-assets, query-session, list-sessions, doctor, etc.)
- `uv run *` -- capsem-builder CLI and Python scripts
- `cargo *` -- Rust builds, tests, checks
- `pnpm *` -- frontend package management and builds
- `npx *` -- skills CLI and other npx tools
- `python3 scripts/*` -- project scripts (check_session, list_sessions, etc.)
- `rustup *` -- target/component management

## Codesigning

The app binary must be codesigned with `com.apple.security.virtualization` entitlement or
Virtualization.framework calls crash. The justfile handles this automatically via `_sign` recipe.

**Prerequisites** (macOS only):
- Xcode Command Line Tools: `xcode-select --install`
- `entitlements.plist` must exist in the repo root (checked into git)

**Verification**: `just doctor` includes a signing test that compiles a tiny binary, signs it with
the entitlements, and verifies the operation succeeds. Run `just doctor` after initial setup to
confirm signing works.

**Linux developers**: codesign is not available and not needed on Linux. VM features (`just run`,
`just dev`, `just bench`) require macOS. You can use `just test`, `just build-assets`, and
`just audit` on Linux.

## Troubleshooting

### `just run` fails with codesign error
- Run `just doctor` -- it will diagnose the specific signing issue
- Ensure Xcode CLTools are installed: `xcode-select --install`
- Check entitlements file exists: `cat entitlements.plist`
- Try manual sign: `codesign --sign - --entitlements entitlements.plist --force target/debug/capsem`
- Check SIP status: `csrutil status`

### `just doctor` fails
Run `just doctor-fix` to auto-fix all fixable issues. Fixes run in dependency order (rustup targets before cargo tools before build-assets before pack-initrd). Non-fixable issues show install hints.

### `just build-assets` fails with exit code 137
The container runtime VM ran out of memory. Increase to 8GB:
- Colima: `colima stop && colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8`
- Linux: Docker runs natively, no memory tuning needed

### `just build-assets` fails with "Release file not valid yet"
The container VM's clock has drifted. The builder uses `Acquire::Check-Valid-Until=false` to work around this, but if you see this error on an old builder version:
- Colima: `colima stop && colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8` (resets clock)
- Docker Desktop: restart Docker Desktop

### `just build-assets` fails (other)
- Check Docker is running: `docker info`
- Check guest config is valid: `uv run capsem-builder validate guest/`
- On first run, Docker image pulls can be slow

### `just run` fails with "assets not found"
Run `just build-assets` first. Assets are gitignored and must be built locally.

### `cargo run` or `cargo test` crashes with signing error
- `.cargo/config.toml` must exist and be tracked in git -- it configures the custom runner (`scripts/run_signed.sh`) that signs binaries with Virtualization.framework entitlements before execution
- If missing: `git checkout .cargo/config.toml`
- The justfile `_sign` recipe signs separately, so `just run` works even without the cargo runner -- but direct `cargo run`/`cargo test` and IDE integrations will crash
- **Lesson:** bare `.gitignore` patterns (no `/` prefix) match at any depth. Always anchor with `/` when you mean root-only (e.g., `/config.toml` not `config.toml`), or you risk silently ignoring files in subdirectories like `.cargo/`

### Cross-compile errors
- Check `.cargo/config.toml` has linker config for musl targets
- Run `rustup target add aarch64-unknown-linux-musl x86_64-unknown-linux-musl`
- Platform-specific type issues: use `as _` for libc calls (see `/dev-rust-patterns`)

### Docker credential helper error (`docker-credential-osxkeychain not found`)
When Colima is installed standalone (without Docker Desktop), `~/.docker/config.json` may reference a credential helper that doesn't exist. The symptom is `docker run` failing to pull images with `exec: "docker-credential-osxkeychain": executable file not found`.

Fix: set `credsStore` to empty string in `~/.docker/config.json`:
```json
{ "credsStore": "" }
```

`just doctor` checks for this under "Container Runtime" and will flag the mismatch.

### VM boot hangs
- Check codesigning: `codesign -dvv target/debug/capsem 2>&1 | grep entitlements`
- Check assets exist: `ls assets/arm64/vmlinuz assets/arm64/rootfs.squashfs`
- Check kernel architecture matches host: wrong-arch kernel causes silent hang. `VmConfig::build()` now rejects mismatched kernels at config time.
- Try with debug logs: `RUST_LOG=capsem=debug just run`

## Doctor architecture

The doctor system is three bash scripts:

```
scripts/
  doctor-common.sh    # Entry point, cross-platform checks, fix registry, recap
  doctor-macos.sh     # macOS: Colima, Rosetta, codesigning, brew hints
  doctor-linux.sh     # Linux: KVM, apt/dnf hints
```

`just doctor` calls `doctor-common.sh`. `just doctor-fix` calls `doctor-common.sh --fix`.

### Fix registry

All fixable issues use an **ordered fix registry** defined at the top of `doctor-common.sh`. Each entry has an ID, command, and description. Checks call `fixable <id> <label>` to mark a fix as needed. Fixes run in registry order (dependency order), deduped by design.

Registry order (each depends on the ones above it):
1. `rustup-targets` -- cross-compile targets
2. `llvm-tools` -- rust-lld linker
3. `cargo-llvm-cov`, `cargo-audit`, `b3sum`, `cargo-tauri` -- cargo tools
4. `entitlements`, `cargo-config`, `run-signed` -- git checkout config files
5. `pnpm-install` -- frontend deps
6. `build-assets` -- VM kernel + rootfs (needs docker)
7. `pack-initrd` -- guest binaries (needs assets)

### Design rules

- **Fixable checks use `fixable <id> <label>`**, not raw `fail()`. This registers the fix in the ordered registry.
- **Non-fixable checks use `fail()` with an install hint.** System tools (node, docker, etc.) can't be auto-installed safely.
- **Platform-specific checks live in `doctor-macos.sh` / `doctor-linux.sh`.** Each defines `check_platform()` and `tool_hint()`.
- **Test, don't just check.** The codesigning section compiles and signs a test binary. `docker buildx version` tests functionality, not just file existence.
- **Bootstrap calls doctor.** `scripts/bootstrap.sh` checks bare minimums (bash, git, curl, rustup, just), installs Python/frontend deps, then runs `doctor-common.sh --fix`.
