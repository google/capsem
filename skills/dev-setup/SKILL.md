---
name: dev-setup
description: Setting up a Capsem development environment from scratch. Use when onboarding a new developer, setting up a new machine, or troubleshooting environment issues. Covers prerequisites, first-time setup, tool installation, VM asset builds, and verification steps.
---

# Developer Setup

## Prerequisites

- **macOS 13+** (Ventura or later) -- required for Virtualization.framework
- **Apple Silicon** (arm64) -- primary target. Intel Macs are not supported for VM features.
- **Docker or Podman** -- needed for `just build-assets` (kernel + rootfs builds)

## Required tools

Run `just doctor` to check all of these:

| Tool | Purpose | Install |
|------|---------|---------|
| Rust (stable) | Host + guest binaries | `rustup` |
| just | Task runner | `cargo install just` |
| pnpm | Frontend package manager | `npm i -g pnpm` |
| Node.js 24+ | Frontend build | `nvm` or `brew install node` |
| uv | Python package manager | `curl -LsSf https://astral.sh/uv/install.sh \| sh` |
| Docker/Podman | VM image builds | Docker Desktop or `brew install podman` |

Rust targets (auto-installed by `just test`):
- `aarch64-unknown-linux-musl` -- guest binaries (arm64)
- `x86_64-unknown-linux-musl` -- guest binaries (x86_64)

Cargo tools (auto-installed by `just test`):
- `cargo-nextest` -- test runner
- `cargo-llvm-cov` -- coverage

## First-time setup

```bash
# 1. Clone and enter
git clone <repo> && cd capsem

# 2. Check tools
just doctor

# 3. Install frontend deps
cd frontend && pnpm install && cd ..

# 4. Install Python deps
uv sync

# 5. Build VM assets (kernel + rootfs, ~10 min, needs Docker)
just build-assets

# 6. Boot the VM to verify everything works
just run "echo hello from capsem"
```

If step 6 prints "hello from capsem" and exits cleanly, you're set.

## Daily workflow

```bash
just run              # Build + boot VM interactively (~10s)
just run "CMD"        # Build + boot + run command + exit
just test             # Unit tests + cross-compile + frontend check
just ui               # Frontend dev server (mock mode, no VM)
just dev              # Full Tauri app with hot-reload
```

See `/dev-just` for the complete recipe reference.

## API keys (optional, needed for full-test)

Create `~/.capsem/user.toml`:
```toml
[providers.anthropic]
api_key = "sk-ant-..."

[providers.google]
api_key = "AIza..."
```

Needed for: `just full-test` (integration test exercises real AI API calls), interactive AI sessions inside the VM.

## Codesigning

The app binary must be codesigned with `com.apple.security.virtualization` entitlement or Virtualization.framework calls crash. The justfile handles this automatically via `_sign` recipe. No manual setup needed for development.

## Troubleshooting

### `just doctor` fails
Install missing tools as indicated. Most can be installed via `brew` or `cargo install`.

### `just build-assets` fails
- Check Docker/Podman is running: `docker info` or `podman info`
- Check guest config is valid: `uv run capsem-builder validate guest/`
- On first run, Docker image pulls can be slow

### `just run` fails with "assets not found"
Run `just build-assets` first. Assets are gitignored and must be built locally.

### Cross-compile errors
- Check `.cargo/config.toml` has linker config for musl targets
- Run `rustup target add aarch64-unknown-linux-musl x86_64-unknown-linux-musl`
- Platform-specific type issues: use `as _` for libc calls (see `/dev-rust-patterns`)

### VM boot hangs
- Check codesigning: `codesign -dvv target/debug/capsem 2>&1 | grep entitlements`
- Check assets exist: `ls assets/arm64/vmlinuz assets/arm64/rootfs.squashfs`
- Try with debug logs: `RUST_LOG=capsem=debug just run`
