---
name: build-images
description: Building Capsem VM images and profile-owned assets. Use when working with profile asset builds, Dockerfiles, kernel builds, rootfs builds, capsem-admin image/manifest commands, or the Python builder backend. Covers the profile-ledger image contract, generated runtime config, Docker build templates, multi-arch support, build ledgers, and release install gates.
---

# Building VM Images

## Overview

The product contract is profile-led:

- `config/profiles/<profile_id>/profile.toml` is the profile ledger.
- Profile sibling files own packages, MCP declarations, rule files, detection
  files, tips, manual installer scripts, and packaged guest root seed files.
- `target/config/` is generated runtime config produced by the same admin/just
  rail used by CI and release.
- `assets/` and `packages/` are generated outputs.

`capsem-admin` owns profile validation, asset/manifest materialization, and the
package-facing build contract. The Python `capsem-builder` code is a backend
implementation detail. Do not add new product truth to `guest/config`; it is a
legacy input surface being burned during the repository ontology cleanup.

## Source Layout

```
config/
  host/                   Host/corp/settings source contracts
  docker/                 Dockerfile/build templates
  profiles/<profile_id>/
    profile.toml          Profile ledger
    enforcement.toml      Profile enforcement rules
    detection.yaml        Profile Sigma detections
    mcp.json              Profile MCP declarations
    apt-packages.txt      Profile apt package input
    python-requirements.txt
    npm-packages.txt
    install.sh            Profile manual installer input
    tips.txt              Profile guest tips
    root/                 Guest / seed, projected by capsem-init
target/config/            Generated runtime config
guest/artifacts/          Core guest payloads: init, doctor, diagnostics, bench
assets/                   Generated VM assets
packages/                 Generated native packages
```

Current transition note: some code still references `guest/config`. Treat that
as cleanup target, not a place to add new behavior.

## CLI commands

```bash
just build-assets code [arch]                # Profile-derived asset rebuild
just build-kernel arm64 code                 # Kernel slice
just build-rootfs arm64 code                 # Rootfs slice
uv run capsem-builder audit                  # Parse trivy/grype vulnerability output
```

Prefer admin/just recipes over calling `capsem-builder build` directly. Direct
builder calls are only acceptable when the task is explicitly inside the backend
and the sprint records that the path is not release proof.

## Building assets

Full rebuild (kernel + rootfs):
```bash
just build-assets code    # Runs doctor + profile-derived admin build
```

Individual templates:
```bash
just build-kernel arm64 code
just build-rootfs arm64 code
```

## Per-arch asset layout

```
assets/
  manifest.json          Version, checksums, asset list
  B3SUMS                 BLAKE3 checksums
  arm64/
    vmlinuz              Kernel
    rootfs.erofs         Root filesystem
    initrd.img           Initial ramdisk (repacked by just run)
```

## Build Ledger

Each per-arch build emits `build-ledger.log` JSONL with hashes for rendered
Dockerfiles, build contexts, rootfs tar, final EROFS, kernel assets, tool
version output, compression settings, git revision, and project version. CI
uploads the ledger as an artifact. As profile file hashing lands, the ledger
must also record profile and profile-owned payload hashes.

## Adding packages to the VM

1. Add the package to the profile-owned package file, for example
   `config/profiles/code/apt-packages.txt`,
   `python-requirements.txt`, or `npm-packages.txt`.
2. Make sure `profile.toml` references and hash-pins the file once the profile
   hash schema is active.
3. Run the admin/profile validation path.
4. Run `just build-assets code` to rebuild the rootfs.
5. Verify with `capsem-doctor` inside a booted VM.

Do not edit generated Dockerfiles. Docker build templates live under
`config/docker/`.

## Adding a guest CLI/tool

There are no image-owned AI providers. A CLI/tool exists only if the active
profile declares the package/manual installer and any required guest root seed
files.

1. Add install input to the profile package files or profile-owned `install.sh`.
2. Add config files under `config/profiles/<profile_id>/root/` so they project
   into the VM at boot.
3. Add MCP declarations to profile-owned `mcp.json` when relevant.
4. Add network/model/security behavior through profile/corp rules, not builder
   provider config.
5. Let the credential broker plugin capture/materialize credentials at runtime;
   do not add settings-owned boot secrets.
6. Rebuild with `just build-assets code` and verify with `capsem-doctor`.

## Dockerfile templates

Template location:
- `Dockerfile.rootfs.j2` -- rootfs image (apt packages, Python packages, AI CLIs, diagnostics)
- `Dockerfile.kernel.j2` -- kernel build (defconfig, modules, vmlinuz extraction)

Templates use Jinja2 with variables from the admin-resolved image spec. The
builder backend renders them from `config/docker/`; include template hashes in
build ledgers.

---

# Builder Internals (for modifying the builder itself)

## Transition Architecture

The target flow is:

1. **Profile ledger** (`config/profiles/<id>/profile.toml`) and hash-pinned
   sibling files.
2. **capsem-admin** validates/materializes profile-owned inputs.
3. **Image backend spec** carries only resolved build inputs.
4. **Python builder backend** renders Docker templates and emits assets plus
   build ledgers.

### Key files

| File | Role |
|------|------|
| `crates/capsem-admin/` | Profile/image/manifest validation and materialization rail |
| `src/capsem/builder/models.py` | Backend image models while cleanup is in progress |
| `src/capsem/builder/config.py` | Legacy loader still being replaced by admin-resolved inputs |
| `src/capsem/builder/docker.py` | Context builders (`_rootfs_context`, `_kernel_context`), rendering, build execution |
| `config/docker/Dockerfile.rootfs.j2` | Target rootfs Dockerfile template location |
| `config/docker/Dockerfile.kernel.j2` | Target kernel Dockerfile template location |
| `src/capsem/builder/scaffold.py` | Legacy scaffolding targeted for deletion/rewrite |
| `src/capsem/builder/validate.py` | Validation rules (E001-E302, W001-W012) |
| `src/capsem/builder/cli.py` | Click CLI entry points |

### Context dict guardrail

`_rootfs_context()` should be moving toward resolved inputs:

- arch and kernel build settings;
- profile-resolved apt/Python/npm/manual install inputs;
- profile-resolved root seed metadata;
- core guest binaries and diagnostics;
- rootfs compression settings.

It must not own AI provider policy, MCP policy, credentials, VM settings, UI
settings, or security decisions.

### Kernel context dict

```python
{
    "arch": ArchConfig,
    "arch_name": str,
    "kernel_version": str,  # e.g. "6.6.130"
}
```

## Backend Internals

The older Python builder internals below are transition-only. During cleanup,
delete/rewrite product-authoring pieces instead of extending them:

- `AiProviderConfig`
- `McpServerConfig`
- web security/network policy config inside image config
- VM resource/settings ownership inside image config
- `capsem-builder init/new/add` product scaffolding
- `generate_defaults_json()` from guest image config

Keep backend-only concerns: arch config, resolved package install sets, kernel
defconfigs, rootfs compression, resolved root seed metadata, and tool-version
capture.

## Final Gate For Release-Candidate Image Work

Do not call image/config work release-ready until these pass:

1. `just build-assets code [arch]` through the admin/just rail.
2. `capsem-admin image verify` against the generated layout.
3. `capsem-doctor` in a booted VM.
4. Real package build and install with the chosen manifest override.
5. Service/UI readiness from installed state.
6. Linux CI/team KVM validation when KVM files changed; macOS cannot execute
   `hypervisor::kvm`.

## How to: Add a new guest binary

Guest binaries are compiled from `crates/capsem-agent/`. On macOS, `cross_compile_agent()` delegates to `container_compile_agent()` which builds inside a Linux container (docker). On Linux (CI), cargo builds natively.

1. Add the binary target in `crates/capsem-agent/Cargo.toml`
2. Add the binary name to `GUEST_BINARIES` list in `docker.py`
3. The template already loops `{% for binary in guest_binaries %}` to COPY + chmod 555

## Verifying Linux builds locally

`just cross-compile [arch]` builds everything in a container: agent binaries, frontend, and the full Tauri app (deb + AppImage). Useful for catching linuxdeploy and system dep issues before CI.

```bash
just cross-compile           # Build for host arch (arm64 on Apple Silicon)
just cross-compile x86_64    # Build x86_64 deb + AppImage
```

## Build pipeline (what `build_image()` does)

For rootfs:
1. Build guest agent binaries (`cross_compile_agent` -- on macOS delegates to `container_compile_agent` which builds inside a Linux container; on Linux compiles natively)
2. Assemble build context (`prepare_build_context`) -- copies CA cert, shell configs, diagnostics, agent binaries
3. Render Dockerfile from template
4. `docker build`
5. Export container filesystem as tar
6. Create EROFS from tar (`create_erofs` -- runs `mkfs.erofs` in a container)
7. Extract tool versions (`extract_tool_versions`)
8. Clean up container image

For kernel:
1. Resolve latest kernel version from kernel.org
2. Assemble build context (defconfig, capsem-init)
3. Render Dockerfile from template
4. `docker build`
5. Extract vmlinuz + initrd.img from image
6. Clean up

## Container runtime requirements

On macOS, Docker runs inside a Colima VM with limited resources.
The rootfs build runs apt, npm, and curl-based CLI installers concurrently --
the default RAM allocation may cause OOM kills (exit code 137).

**Minimum**: 12GB RAM. **Recommended**: 16GB RAM, 8 CPUs.

```bash
# Colima (macOS)
colima stop && colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8

# Linux: Docker runs natively, no memory tuning needed
# sudo apt install docker.io
```

`just doctor` and `capsem-builder doctor` both check these resources automatically.

The resource check lives in `src/capsem/builder/doctor.py`:
- `check_container_resources()` -- checks docker info
- Thresholds: `DOCKER_MIN_MEMORY_MB = 4096`, `DOCKER_RECOMMENDED_MEMORY_MB = 8192`

## Container image compatibility

The container builds use `rust:slim-bookworm` -- a minimal Debian image. Many common utilities (`file`, `less`, `vim`, etc.) are NOT available. Any shell commands run inside the container must use only coreutils (`ls`, `cp`, `cat`, `test`, etc.) or tools explicitly installed via `apt-get` in the same `RUN` step.

**Lesson learned**: using `file /output/binary` to verify compiled binaries failed because `file` is not in slim images. Replaced with `ls -l` which is always available and still confirms the copy succeeded. The real validation (existence + non-zero size) is done in Python after the container exits.

**Rule**: never assume a command exists in a slim container image. Stick to coreutils or install what you need explicitly.

## Clock skew workaround

All `apt-get update` calls use `-o Acquire::Check-Valid-Until=false` to handle container VM clock drift.
Without this, apt rejects Release files whose timestamp is in the future relative to the VM's clock.
This can occur with any container VM backend on macOS.

Files affected:
- `Dockerfile.kernel.j2` (line 11)
- `Dockerfile.rootfs.j2` (line 11)
- `docker.py` `create_erofs()` function
