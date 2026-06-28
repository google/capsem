---
name: dev-start
description: Quick-start guide for new Capsem developers. Use when someone asks "how do I get started", "how to set up", "first time setup", or "bootstrap". Points to the bootstrap script and full docs. For detailed environment troubleshooting, use /dev-setup instead.
---

# Developer Quick Start

## Fastest path

```bash
git clone <repo> && cd capsem
./bootstrap.sh                  # interactive, prompts [Y/n] before each install
./bootstrap.sh --yes            # non-interactive (CI / unattended setup)
just exec "echo hello"           # verify VM boots (build-assets runs as part of bootstrap)
```

`bootstrap.sh` lives at the **repo root**, not under `scripts/`.

## What bootstrap.sh does

Three phases. Default answer at every prompt is **Yes** — press Enter to install, type `n` to skip.

1. **Hard prereqs** (you must have): `bash`, `git`, `curl`. Auto-installed: `rustup` (sh.rustup.rs), `just` (just.systems → `~/.local/bin`).
2. **Dependencies**: `uv` (astral.sh), `uv sync`, `flock` (brew on macOS), container runtime on macOS (`colima` + `docker` + `docker-buildx` via brew, then `colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8`), `pnpm install` for the frontend.
3. **Doctor `--fix`** (`scripts/doctor-common.sh --fix`): installs Rust targets, `cargo-llvm-cov`, `cargo-audit`, `b3sum`, `cargo-tauri` (= `tauri-cli` crate), `cargo-sbom`; builds VM assets and packs the initrd.

Release-only local preflight also needs `cdxgen`. Install it with
`npm install -g @cyclonedx/cdxgen` before running
`bash scripts/check-release-workflow.sh` or local VM asset release dry runs.

`--yes` flag and non-tty input both auto-accept every prompt.

## After bootstrap

All just recipes (`run`, `test`, `dev`, etc.) check for `.dev-setup` and auto-run doctor if missing. You can't accidentally skip setup.

## Full documentation

- **Detailed setup + troubleshooting**: [Development Guide](https://capsem.org/development/getting-started/) or `/dev-setup` skill
- **Just recipe reference**: `/dev-just`
- **Testing workflow**: `/dev-testing`

## Container runtime

Docker (via Colima on macOS) with 12GB+ RAM (16GB recommended -- the Tauri install-test build OOMs below 12GB). On Linux, Docker runs natively. See `/dev-setup` for configuration.
