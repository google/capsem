---
name: build-images
description: Building Capsem VM images with capsem-builder. Use when working with guest image configuration, Dockerfiles, kernel builds, rootfs builds, the builder CLI, or guest config TOML files. Covers the config-driven build system, guest config layout, Dockerfile templates, multi-arch support, and the builder CLI commands.
---

# Building VM Images

## Overview

capsem-builder is a config-driven build system. It reads TOML configs from `guest/config/`, renders Jinja2 Dockerfile templates, and builds kernel + rootfs via Docker/Podman. Assets output to `assets/{arch}/`.

## Guest config layout

```
guest/config/
  build.toml              Architectures, compression, base images
  manifest.toml           Image name, version, changelog
  ai/*.toml               AI provider configs (Claude, Gemini, Codex)
  packages/*.toml         Package sets (apt, python)
  mcp/*.toml              MCP server configs
  security/web.toml       Web security (allow/block domains)
  vm/resources.toml       CPU, RAM, disk
  vm/environment.toml     Shell, TLS, env vars
  kernel/*.defconfig      Kernel defconfigs per architecture
```

All configs use Pydantic models for validation. Run `uv run capsem-builder validate guest/` to lint.

## CLI commands

```bash
uv run capsem-builder doctor guest/          # Check build prerequisites
uv run capsem-builder validate guest/        # Lint all configs (E001-E302, W001-W012)
uv run capsem-builder build guest/ --dry-run # Preview rendered Dockerfiles
uv run capsem-builder build guest/ --arch arm64 --template rootfs  # Build rootfs
uv run capsem-builder build guest/ --arch arm64 --template kernel  # Build kernel
uv run capsem-builder inspect guest/         # Show config summary
uv run capsem-builder new my-image/ --from guest/  # Scaffold new image from base
uv run capsem-builder audit                  # Parse trivy/grype vulnerability output
```

## Building assets

Full rebuild (kernel + rootfs):
```bash
just build-assets    # Runs doctor + validate + build for host arch
```

Individual templates:
```bash
just build-kernel arm64
just build-rootfs arm64
```

## Per-arch asset layout

```
assets/
  manifest.json          Version, checksums, asset list
  B3SUMS                 BLAKE3 checksums
  arm64/
    vmlinuz              Kernel
    rootfs.squashfs      Root filesystem
    initrd.img           Initial ramdisk (repacked by just run)
```

## Adding packages to the VM

1. Edit the appropriate config in `guest/config/packages/` (apt or python TOML)
2. Run `uv run capsem-builder validate guest/` to check
3. Run `just build-assets` to rebuild the rootfs
4. Verify: `just run "capsem-doctor"`

Do not edit Dockerfiles directly -- they are rendered from Jinja2 templates in `src/capsem/builder/templates/`.

## Adding a new AI provider

1. Create `guest/config/ai/<provider>.toml` with provider config
2. Add domain entries to `guest/config/security/web.toml` if needed
3. Validate: `uv run capsem-builder validate guest/`
4. Rebuild: `just build-assets`

## Dockerfile templates

Templates live in `src/capsem/builder/templates/`:
- `Dockerfile.rootfs.j2` -- rootfs image (apt packages, Python packages, AI CLIs, diagnostics)
- `Dockerfile.kernel.j2` -- kernel build (defconfig, modules, vmlinuz extraction)

Templates use Jinja2 with variables from the merged guest config. Preview with `--dry-run`.
