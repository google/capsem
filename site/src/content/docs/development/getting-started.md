---
title: Development Guide
description: Clone, build, and run Capsem from source.
sidebar:
  order: 1
---

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| macOS 13+ (Ventura) or Linux with KVM | Apple Silicon for macOS; `/dev/kvm` for Linux |
| Docker or Podman | 4GB RAM minimum, 8GB recommended |

## Required tools

| Tool | Purpose | Install |
|------|---------|---------|
| Rust (stable) | Host + guest binaries | [rustup.rs](https://rustup.rs) |
| just | Task runner | `cargo install just` |
| Node.js 24+ | Frontend build | `brew install node` or [nvm](https://github.com/nvm-sh/nvm) |
| pnpm | Frontend package manager | `npm i -g pnpm` |
| Python 3.11+ | Builder CLI, scripts | System Python or `brew install python` |
| uv | Python package manager | `curl -LsSf https://astral.sh/uv/install.sh \| sh` |

`just doctor` checks all of these and reports what's missing.

## Container runtime setup

On macOS, both Docker and Podman run inside a Linux VM. The default memory allocation (2GB for Podman) is too small -- the rootfs build runs apt installs, npm installs, and CLI installers concurrently, which can OOM-kill the build (exit code 137).

### Podman

```bash
brew install podman
podman machine init
podman machine set --memory 8192 --cpus 8
podman machine start
```

To fix an existing machine:

```bash
podman machine stop
podman machine set --memory 8192 --cpus 8
podman machine start
```

### Docker Desktop

Docker Desktop -> Settings -> Resources -> set Memory to 8GB, CPUs to 8.

## First-time setup

```bash
# 1. Clone and enter
git clone <repo> && cd capsem

# 2. Bootstrap (checks tools, installs deps)
bash scripts/bootstrap.sh

# 3. Build VM assets (kernel + rootfs, ~10 min, needs Docker/Podman)
just build-assets

# 4. Boot the VM
just run "echo hello from capsem"
```

If step 4 prints "hello from capsem" and exits cleanly, you're set.

Alternatively, skip the bootstrap script and run each step manually:

```bash
just doctor              # check all tools
uv sync                  # install Python deps
cd frontend && pnpm install && cd ..
just build-assets        # build kernel + rootfs
just run "echo hello"    # verify
```

`just doctor` writes a `.dev-setup` sentinel file on success. All recipes (`run`, `test`, `dev`, etc.) check for this file and auto-run doctor if it's missing, so new developers can't accidentally skip setup.

## Daily workflow

```bash
just run              # Cross-compile + repack + build + sign + boot (~10s)
just run "CMD"        # Boot + run command + exit
just test             # Unit tests + cross-compile + frontend check
just dev              # Hot-reloading Tauri app (frontend + Rust)
just ui               # Frontend-only dev server (mock mode, no VM)
```

See `just --list` for all targets.

## Cross-compilation

Guest agent binaries are cross-compiled to `aarch64-unknown-linux-musl` using `rust-lld` (from the `llvm-tools` rustup component). The linker config is in `.cargo/config.toml`:

```toml
[target.aarch64-unknown-linux-musl]
linker = "rust-lld"
```

If you see linker errors like `ld: unknown options: --as-needed`, the `llvm-tools` component is missing:

```bash
rustup component add llvm-tools
```

`just doctor` checks for this automatically.

## API keys (optional)

Needed for `just full-test` (integration tests exercise real AI API calls) and interactive AI sessions inside the VM.

Create `~/.capsem/user.toml`:

```toml
[providers.anthropic]
api_key = "sk-ant-..."

[providers.google]
api_key = "AIza..."
```

## Codesigning

The app binary must be codesigned with `com.apple.security.virtualization` entitlement or Virtualization.framework calls crash. The justfile handles this automatically via the `_sign` recipe. No manual setup needed.

## Troubleshooting

### `just doctor` fails

Install missing tools as indicated. Most are available via `brew` or `cargo install`. Run `just _install-tools` to auto-install Rust components and cargo tools.

### `just build-assets` fails with exit code 137

The container runtime VM ran out of memory. Increase to 8GB:
- Podman: `podman machine stop && podman machine set --memory 8192 && podman machine start`
- Docker: Docker Desktop -> Settings -> Resources -> Memory -> 8GB

### `just build-assets` fails with "Release file not valid yet"

The container VM's clock has drifted:
- Podman: `podman machine stop && podman machine start`
- Docker: restart Docker Desktop

### `just run` fails with "assets not found"

Run `just build-assets` first. Assets are gitignored and must be built locally.

### Cross-compile linker errors

```
ld: unknown options: --as-needed -Bstatic ...
```

The system `cc` is being used instead of `rust-lld`. Fix:

```bash
rustup component add llvm-tools
```

Verify `.cargo/config.toml` sets `linker = "rust-lld"` for both musl targets.

### VM boot hangs

- Check codesigning: `codesign -dvv target/debug/capsem 2>&1 | grep entitlements`
- Check assets exist: `ls assets/arm64/vmlinuz assets/arm64/rootfs.squashfs`
- Debug logs: `RUST_LOG=capsem=debug just run`
