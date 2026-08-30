---
name: dev-start
description: Quick start for new Capsem developers. Use for "how do I get started" or "first time setup"; for environment troubleshooting use dev-setup instead.
---

# Developer Quick Start

## Fastest path

```bash
git clone <repo> && cd capsem
./bootstrap.sh                  # interactive, prompts [Y/n] before each install
./bootstrap.sh --yes            # non-interactive (CI / unattended setup)
just exec "echo hello"           # verify VM boots (build-assets runs as part of bootstrap)
```

`bootstrap.sh` is the deliberately public **repo-root** bootstrap entrypoint.

## What bootstrap.sh does

Three phases. Default answer at every prompt is **Yes** — press Enter to install, type `n` to skip.

1. **Hard prereqs** (you must have): `bash`, `git`, `curl`. Auto-installed: `rustup` (sh.rustup.rs), `just` (just.systems → `~/.local/bin`).
2. **Dependencies**: on Linux, distro-native build/Tauri packages, `cpio`, verified Node 24, pnpm 10, Docker + Buildx, Bubblewrap, and immediate Docker/KVM access; on macOS, `flock`, `colima` + `docker` + `docker-buildx`, Tart, and sshpass through Homebrew. Both platforms install `uv`, locked Python/frontend dependencies, and then prove the runtime.
3. **Doctor `--fix`** (`build_system/scripts/doctor/doctor-common.sh --fix`): installs Rust targets and the exact config-owned Cargo tools (`cargo-nextest`, `cargo-llvm-cov`, `cargo-audit`, `b3sum`, `cargo-tauri`, and `cargo-sbom`); builds VM assets and packs the initrd.

The VM asset rail materializes its digest-pinned OBOM tools inside its own
architecture-matched helper; do not install a parallel global cdxgen.

`--yes` flag and non-tty input both auto-accept every prompt.

## After bootstrap

All just recipes (`run`, `test`, `dev`, etc.) check for `.dev-setup` and auto-run doctor if missing. You can't accidentally skip setup.

## Full documentation

- **Detailed setup + troubleshooting**: [Development Guide](https://capsem.org/development/getting-started/) or `/dev-setup` skill
- **Just recipe reference**: `/dev-just`
- **Testing workflow**: `/dev-testing`

## Container runtime

Docker (via Colima on macOS) with 12GB+ RAM (16GB recommended -- the Tauri install-test build OOMs below 12GB). On Linux, bootstrap installs and starts native Docker, repairs current-session Docker/KVM access, and verifies the Bubblewrap host-gate boundary. See `/dev-setup` for configuration.
