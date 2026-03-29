---
name: dev-start
description: Quick-start guide for new Capsem developers. Use when someone asks "how do I get started", "how to set up", "first time setup", or "bootstrap". Points to the bootstrap script and full docs. For detailed environment troubleshooting, use /dev-setup instead.
---

# Developer Quick Start

## Fastest path

```bash
git clone <repo> && cd capsem
bash scripts/bootstrap.sh      # checks tools, installs deps, runs doctor
just build-assets               # builds kernel + rootfs (~10 min, needs Docker/Podman)
just run "echo hello"           # verify VM boots
```

## What bootstrap.sh does

1. Checks all required tools: Rust, just, Node 24+, pnpm, Python 3.11+, uv, Docker/Podman
2. Prints install commands for anything missing
3. Runs `uv sync` (Python deps) and `pnpm install` (frontend deps)
4. Runs `just doctor` (writes `.dev-setup` sentinel)

## After bootstrap

All just recipes (`run`, `test`, `dev`, etc.) check for `.dev-setup` and auto-run doctor if missing. You can't accidentally skip setup.

## Full documentation

- **Detailed setup + troubleshooting**: [Development Guide](https://capsem.org/development/getting-started/) or `/dev-setup` skill
- **Just recipe reference**: `/dev-just`
- **Testing workflow**: `/dev-testing`

## Container runtime

Docker or Podman with 4GB+ RAM (8GB recommended). On macOS both run inside a Linux VM -- the default 2GB is too small. See `/dev-setup` for configuration.
