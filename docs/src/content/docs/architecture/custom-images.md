---
title: Custom Images
description: Build custom Capsem VM images from profile-owned packages, rules, MCP config, and assets.
sidebar:
  order: 40
---

Capsem images are defined by profiles. Organizations create custom images by
shipping profile-owned package files, root seed files, MCP config, enforcement
rules, detection rules, plugin policy, and asset pins. Provider access and
credentials remain runtime rule/plugin truth, not image-builder truth.

## Quick Start

```bash
cargo run -p capsem-admin -- profile check config/profiles/code/profile.toml --config-root config
cargo run -p capsem-admin -- image build --profile config/profiles/code/profile.toml --config-root config --arch arm64
cargo run -p capsem-admin -- manifest generate assets --version 1.3.corp.1 --json
```

## Directory Structure

```
config/
    profiles/
        corp-code/
            profile.toml              Profile ledger
            apt-packages.txt          System packages
            python-requirements.txt   Python packages
            npm-packages.txt          Node CLI packages
            build.sh                  Profile image build hook
            mcp.json                  Profile MCP config
            enforcement.toml          Enforcement rules
            detection.yaml            Sigma detection rules
            tips.txt                  Login tips
            root/                     Guest root seed
            root.manifest.json        Root seed hashes
    corp.toml                         Corp locks and reporting endpoints
config/docker/
    Dockerfile.rootfs.j2
    Dockerfile.kernel.j2
```

## Configuration Reference

### Guest Tools

Images may install guest tools, but provider access, credentials, rules, and
tool configuration are not image-owned. Provider/network control is profile/corp
rule truth. Credentials are captured and materialized by the credential broker
plugin at runtime, and logged only as BLAKE3 references.

### Package Sets

Each profile-owned package file defines desired packages for one manager.

```text
# config/profiles/corp-code/apt-packages.txt
coreutils
util-linux
git
curl
python3
python3-pip
python3-venv
```

```text
# config/profiles/corp-code/python-requirements.txt
numpy
pandas
requests
pytest
```

### MCP Servers

```json
{
  "servers": [
    {
      "id": "capsem",
      "name": "Capsem",
      "transport": "stdio",
      "command": "/run/capsem-mcp-server",
      "enabled": true
    }
  ]
}
```

### Network Mechanics And Security Rules

```toml
[profiles.rules.allow_internal_registry]
name = "allow_internal_registry"
action = "allow"
match = 'http.host.matches("(^|.*\\.)registry\\.internal\\.corp$")'

[profiles.rules.block_external_search]
name = "block_external_search"
action = "block"
match = 'http.host.matches("(^|.*\\.)(google\\.com|bing\\.com|duckduckgo\\.com)$")'
```

### Build Configuration

Backend build parameters are resolved by the profile-derived build rail and Docker templates.
Each architecture is self-contained.

```toml
[build]
compression = "zstd"
compression_level = 15

[build.erofs]
enabled = true
compression = "lz4hc"
compression_level = 12

[build.architectures.arm64]
base_image = "debian:bookworm-slim"
docker_platform = "linux/arm64"
rust_target = "aarch64-unknown-linux-musl"
kernel_branch = "7.0"
kernel_image = "arch/arm64/boot/Image"
defconfig = "kernel/defconfig.arm64"
node_major = 24

[build.architectures.x86_64]
base_image = "debian:bookworm-slim"
docker_platform = "linux/amd64"
rust_target = "x86_64-unknown-linux-musl"
kernel_branch = "7.0"
kernel_image = "arch/x86_64/boot/bzImage"
defconfig = "kernel/defconfig.x86_64"
node_major = 24
```

## CLI Reference

| Command | What it does |
|---------|-------------|
| `capsem-admin profile check` | Validate profile ledger, file pins, rules, MCP, and root seed |
| `capsem-admin image build` | Build profile-derived kernel/rootfs assets |
| `capsem-admin manifest generate` | Generate manifest and B3SUMS for assets |
| `capsem-admin profile materialize` | Generate runtime `target/config` from profile and manifest |
| `capsem-builder doctor --profile code --config-root config` | Check build prerequisites and active profile |

## Manifest

Every build produces `assets/manifest.json` (format 2) -- a single top-level file covering every arch. It records BLAKE3 hashes and file sizes for each asset and ties asset versions to compatible binary versions:

```json
{
  "format": 2,
  "refresh_policy": "24h",
  "assets": {
    "current": "2026.0421.30",
    "releases": {
      "2026.0421.30": {
        "date": "2026-04-21",
        "deprecated": false,
        "min_binary": "1.0.0",
        "arches": {
          "arm64": {
            "vmlinuz":         {"hash": "<64-char blake3>", "size": 7797248},
            "initrd.img":      {"hash": "<64-char blake3>", "size": 2314963},
            "rootfs.erofs": {"hash": "<64-char blake3>", "size": 454230016}
          }
        }
      }
    }
  },
  "binaries": {
    "current": "1.0.1776688771",
    "releases": {
      "1.0.1776688771": {
        "date": "2026-04-21",
        "deprecated": false,
        "min_assets": "2026.0421.30"
      }
    }
  }
}
```

The runtime boots only when the asset hashes match. `min_binary`/`min_assets` gate which binary and asset versions are compatible with each other.

## Corporate Deployment

### Admin Provisioning Trust Chain

Corporate provisioning is profile/corp driven. Do not put signing keys,
catalog channels, build knobs, or release-process metadata inside `corp.toml`
or `profile.toml`; those payloads should only describe runtime behavior.

The release and runtime evidence chain is:

| Layer | Owns |
|-------|------|
| Release artifacts | SBOM and provenance attestations |
| Corp config | Corp locks, endpoints, enforcement files, detection files, and `refresh_policy` |
| Profile config | VM defaults, rule files, MCP/profile metadata, asset URLs/hashes, and `refresh_policy` |
| Profile assets | Kernel, initrd, and rootfs bytes verified by BLAKE3 |

At runtime Capsem verifies BLAKE3 hashes and refresh policy before marking a
profile launchable. A missing, stale, or mismatched profile/asset contract must
fail closed.

Example profile payload:

```toml
id = "code"
name = "Code"
revision = "2026.06.08.7"
refresh_policy = "24h"

[assets]
format = "profile-assets.v1"
refresh_policy = "on_profile_refresh"

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "https://releases.capsem.dev/assets/arm64/rootfs.erofs"
hash = "blake3:..."
size = 12345678
```

Example corp payload:

```toml
refresh_policy = "24h"

[corp_rule_files]
enforcement = "corp/enforcement.toml"
sigma = "corp/detection.yaml"
sigma_output_endpoint = "https://siem.example.invalid/capsem/sigma"
open_telemetry = "https://otel.example.invalid/v1/traces"
remote_enforcement = "https://security.example.invalid/capsem/enforcement"
```

### Workflow

1. Copy `config/profiles/code/` to a new profile id.
2. Edit the new `profile.toml` name, description, icon, asset descriptors, and
   file pins.
3. Edit profile/corp security rules to allow, ask, or block network/model/MCP
   boundaries.
4. Add internal guest tools only if they must be baked into the image, using
   profile package files or `build.sh`.
5. Keep credentials brokered at runtime; do not add them to image config.
6. Validate with `capsem-admin profile check`.
7. Build with `capsem-admin image build`.
8. Generate the manifest with `capsem-admin manifest generate`.
9. Distribute the package plus selected manifest and profile assets.

### Lockdown Example

Block external search and allow only internal registries:

Edit the profile or corp enforcement rule file:

```toml
[profiles.rules.allow_internal_registry]
name = "allow_internal_registry"
action = "allow"
match = 'http.host.matches("(^|.*\\.)internal\\.corp\\.com$")'

[profiles.rules.block_external_search]
name = "block_external_search"
action = "block"
match = 'http.host.matches("(^|.*\\.)(google\\.com|bing\\.com|duckduckgo\\.com)$")'
```

## Install Inputs

Use profile-owned package files for normal package managers:

- `apt-packages.txt` for apt packages
- `python-requirements.txt` for Python packages
- `npm-packages.txt` for Node CLI packages
- `build.sh` for build-time installers that cannot be expressed as a package list

The build ledger records these declared inputs for debugging. The CI/release
asset rail publishes the CycloneDX OBOM, which records the installed base-image
component names and versions after the rootfs is produced.

:::caution[/root is runtime overlay state]
Anything installed under `/root/` during the Docker build can be hidden at
runtime by the tmpfs overlay. If a manual installer puts binaries in
`~/.local/bin/` or a tool-specific home directory, copy them to a stable system
path from `build.sh` and verify with `capsem-doctor`.
:::

## Troubleshooting

| Diagnostic | Cause | Fix |
|-----------|-------|-----|
| `error[E001] missing required field` | TOML config missing a schema field | Check file:line in error, compare against examples above |
| `error[E304] defconfig missing` | Kernel config for declared arch doesn't exist | Add `config/kernel/defconfig.{arch}` |
| `warn[W001] no npm registry` | npm packages declared but no registry config | Add a registry entry to the profile build config |
| `warn[W005] API key in config` | Hardcoded key in TOML | Remove it; credentials must be brokered at runtime |
| Build fails: "container runtime not found" | No Docker | Install Docker (`brew install colima docker` on macOS, `sudo apt install docker.io` on Linux) |
| Build fails: exit 137 (OOM) or exit 143 (SIGTERM mid-build) | Container runtime VM out of memory -- Tauri install-test cold build needs >12GB | Bump Colima to 16GB: `colima stop && colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8` |
| Build fails: "Release file not valid yet" | Container VM clock drift | Builder handles this automatically via `Acquire::Check-Valid-Until=false` |
| CLI not found at runtime | Installer put binary in `/root/` which is tmpfs | Copy binary to `/usr/local/bin/` in the Dockerfile template |
