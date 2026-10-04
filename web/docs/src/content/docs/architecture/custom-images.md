---
title: Custom Images
description: Customize what runs in a Capsem session with OCI images, and the VM runtime itself with the image contract.
sidebar:
  order: 40
---

Capsem separates the VM from what runs in it. The **runtime** is one minimal
kernel, initrd, and rootfs per architecture, built from `config/docker/image/`
and `guest/artifacts/`; it carries only Capsem's own guest machinery.
**Applications** -- agents, toolchains, internal tools -- are OCI images, run
as the session's workload with `--image`. Organizations customize Capsem by
publishing OCI images, and only rarely by rebuilding the runtime. Provider
access and credentials remain runtime rule/plugin truth, not image truth.

## Quick Start

A custom application image, on the Capsem base:

```bash
docker buildx build --platform linux/arm64,linux/amd64 \
  --build-arg BASE=ghcr.io/<owner>/<base-image>@sha256:<digest> \
  -t registry.internal.corp/capsem/corp-dev:1 --push images/corp-dev
capsem create -n work --image registry.internal.corp/capsem/corp-dev:1
```

A custom VM runtime:

```bash
just build-assets arm64
cargo run -p capsem-admin -- manifest generate cache/target/assets --version 1.3.corp.1 --json
```

## Directory Structure

```
images/
    catalog.toml              Official image catalog descriptions
    base/                     Capsem base image every official image builds FROM
    dev/  claude-code/  codex-cli/  agy/  claude-desktop/
                              One Dockerfile per official image
    ci/                       Catalog and rootfs publication helpers
config/
    settings/
        settings.toml             UI/application preferences only
        schema.generated.json     Settings shape for UI and validation
        ui-metadata.toml          UI rendering metadata
    corp/
        corp.toml                 Corp locks and reporting endpoints
        enforcement.toml          Corp enforcement rules
        detection.yaml            Corp Sigma detection rules
    docker/
        image/
            build.toml            Kernel, architectures, EROFS, runtime_apt_packages
            manifest.toml         Image identity and changelog
            kernel/               Defconfigs and patches
            vm/  security/        Guest environment and web domain lists
        Dockerfile.rootfs-dependencies.j2
        Dockerfile.rootfs.j2
        Dockerfile.kernel.j2
guest/
    artifacts/                    capsem-init, bashrc, tips, diagnostics, capsem-bench
```

## Configuration Reference

### Guest Tools

Images may install guest tools, but provider access, credentials, rules, and
tool configuration are not image-owned. Provider/network control is corp
rule truth. Credentials are captured and materialized by the credential broker
plugin at runtime, and logged only as BLAKE3 references.

### Application Images

An application image is an ordinary OCI image. The official ones under
`images/` build `FROM` the Capsem base by digest, so its layers are stored and
pulled once, and run as the unprivileged `capsem` user:

```dockerfile
# images/corp-dev/Dockerfile
ARG BASE
FROM ${BASE}
USER root
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential postgresql-client && \
    rm -rf /var/lib/apt/lists/*
USER capsem
CMD ["bash"]
```

`capsem create --image` accepts a catalog name, `docker://IMAGE`, or
`registry/repository:tag`. The service pulls, verifies, and stages the image,
and records the session's `repository@digest`. See the
[CLI reference](/usage/cli/) for the flags and the image policy.

The official images are published by `images.yaml` to ghcr.io with their OBOM,
EROFS rootfs, and build provenance, and are appended to the channel's image
catalog; a catalog name resolves to the newest version this host can run,
pinned by digest.

### Runtime Packages

The runtime's whole package set is `runtime_apt_packages` in
`config/docker/image/build.toml`. Add a package there only when Capsem's own
guest machinery needs it -- the container launcher, `capsem-init`,
`capsem-doctor`, `capsem-bench`, or the network diagnostics. Anything an
application or agent needs belongs in an OCI image.

```toml
[build.rootfs]
runtime_apt_packages = [
    "runc",
    "umoci",
    "python3",
    "iptables",
    "iproute2",
    "ca-certificates",
    "curl",
    "procps",
    "python3-pytest",
    "python3-rich",
    "python3-venv",
    "auditd",
    "e2fsprogs",
]
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

Backend build parameters -- rootfs compression levels, Docker platforms,
kernel image paths, defconfig paths -- live in `config/docker/image/` and the
Docker templates. They are image mechanics owned by the runtime build rail,
not runtime policy.

## CLI Reference

| Command | What it does |
|---------|-------------|
| `just build-assets [arch]` | Build the runtime's kernel, initrd, and rootfs |
| `capsem-admin image build` | Build the runtime's kernel/rootfs assets directly |
| `capsem-admin manifest generate` | Generate manifest and B3SUMS for assets |
| `capsem-admin manifest corporate` | Author a corporate channel manifest from an official one plus a corporation-built runtime |

## Manifest

Every repository build produces `cache/target/assets/manifest.json` (format 2) -- a single top-level file covering every arch. It records BLAKE3 hashes and file sizes for each asset and ties asset versions to compatible binary versions:

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

The runtime boots only when the asset hashes match. `min_binary`/`min_assets`
gate which binary and asset versions are compatible with each other. The asset
version recorded here (`assets.current`) is the runtime revision a release
publishes; a release lane generates it as
`<workspace version>-<first 12 hex of the source commit>`.

Nothing checked in hand-authors asset hashes. Evidence belongs in asset
manifests, OBOMs, and build ledgers.

## Corporate Deployment

### Admin Provisioning Trust Chain

Corporate provisioning is corp-config driven. Do not put signing keys,
catalog channels, build knobs, or release-process metadata inside `corp.toml`;
it should only describe runtime behavior.

The release and runtime evidence chain is:

| Layer | Owns |
|-------|------|
| Release artifacts | SBOM and provenance attestations |
| Corp config | Corp locks, endpoints, enforcement files, detection files, and `refresh_policy` |
| Channel manifest `runtime` | Kernel, initrd, and rootfs URLs, sizes, SHA-256 and BLAKE3, OBOM evidence |
| Application images | OCI images pinned by `repository@digest` |

At runtime Capsem verifies BLAKE3 hashes and refresh policy before booting the
runtime. A missing, stale, or mismatched runtime asset must fail closed.

A corporation that builds its own runtime publishes it under its own HTTPS base
and authors its channel with `capsem-admin manifest corporate`, which takes
the official manifest for packages plus `--runtime-manifest` (the
corporation-built runtime) and `--runtime-base` (the HTTPS base that must own
every runtime image and evidence URL).

Example corp payload:

```toml
refresh_policy = "24h"

[corp_rule_files]
enforcement = "corp/enforcement.toml"
sigma = "corp/detection.yaml"
sigma_output_endpoint = "https://siem.example.invalid/capsem/sigma"
open_telemetry = "https://otel.example.invalid"
remote_enforcement = "https://security.example.invalid/capsem/enforcement"
```

### Workflow

1. Put the tools your users need in an OCI image, built `FROM` the Capsem base.
2. Push it to a registry your hosts can reach, and allow that registry in
   corp enforcement rules.
3. Edit corp security rules to allow, ask, or block network/model/MCP
   boundaries.
4. Keep credentials brokered at runtime; do not bake them into an image.
5. Create sessions with `capsem create --image <reference>`.
6. Rebuild the runtime only when Capsem's own guest machinery must change:
   edit `config/docker/image/`, build with `just build-assets`, generate the
   manifest with `capsem-admin manifest generate`, and publish it through
   `capsem-admin manifest corporate`.

### Lockdown Example

Block external search and allow only internal registries:

Edit the corp enforcement rule file:

```toml
[corp.rules.allow_internal_registry]
name = "allow_internal_registry"
action = "allow"
match = 'http.host.matches("(^|.*\\.)internal\\.corp\\.com$")'

[corp.rules.block_external_search]
name = "block_external_search"
action = "block"
match = 'http.host.matches("(^|.*\\.)(google\\.com|bing\\.com|duckduckgo\\.com)$")'
```

## Install Inputs

Install application tooling in the OCI image's Dockerfile, with whatever
package manager the image uses. The runtime installs only
`runtime_apt_packages`.

The build ledger records the runtime's declared inputs for debugging. The
CI/release asset rail publishes the CycloneDX OBOM, which records the installed
base-image component names and versions after the rootfs is produced.

:::caution[/root is runtime overlay state]
Anything installed under `/root/` in the runtime rootfs is hidden at runtime by
the tmpfs overlay. Install runtime files at a stable system path and verify
with `capsem-doctor`.
:::

## Troubleshooting

| Diagnostic | Cause | Fix |
|-----------|-------|-----|
| `error[E001] Missing required file: build.toml` | Image contract not found | Check `--config-root` points at the directory holding `docker/image/` |
| `error[E300] Missing kernel defconfig` | Kernel config for declared arch doesn't exist | Add `config/docker/image/kernel/defconfig.{arch}` |
| `warn[W003] Potential secret` | Hardcoded key in image config | Remove it; credentials must be brokered at runtime |
| Build fails: "container runtime not found" | No Docker | Install Docker (`brew install colima docker` on macOS, `sudo apt install docker.io` on Linux) |
| Build fails: exit 137 (OOM), exit 143, or ENOSPC | Container runtime is below the release-gate memory/disk floor | Run `colima stop && colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8 --disk 128` |
| Build fails: "Release file not valid yet" | Container VM clock drift | Builder handles this automatically via `Acquire::Check-Valid-Until=false` |
| Tool not found in a session | It is installed in neither the runtime nor the session's image | Add it to the session's OCI image |
