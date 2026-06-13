---
name: build-images
description: Building Capsem VM images from profile-owned inputs. Use when working with profile package files, Docker templates, kernel builds, rootfs builds, capsem-admin image builds, or the capsem-builder backend. Covers the profile-derived build rail, multi-arch assets, build ledgers, OBOMs, Dockerfile templates, and backend internals.
---

# Building VM Images

## Overview

Capsem image builds are profile-led.

- `config/profiles/<profile_id>/profile.toml` is the profile ledger.
- Profile sibling files own packages, MCP declarations, rule files, detection
  files, tips, build-time hooks, and packaged guest root seed files.
- `capsem-admin` validates and materializes profile-owned inputs into the
  backend build workspace.
- The Python `capsem-builder` backend renders Docker templates and emits
  assets, build ledgers, and OBOMs. Do not add product truth directly to the
  backend image-spec path.

## Source Layout

Read `config/README.md` before changing this layout.

```
config/
  settings/               UI/application preferences and generated UI schema
  corp/                   Corporate source contracts and rule files
  docker/                 Dockerfile/build templates
  profiles/<profile_id>/
    profile.toml          Source ledger; no hash/size pins
    enforcement.toml      Profile enforcement rules
    detection.yaml        Profile Sigma detections
    mcp.json              Profile MCP declarations
    apt-packages.txt      Profile apt package input
    python-requirements.txt
    npm-packages.txt
    build.sh              Profile image build hook
    tips.txt              Profile guest tips
    root/                 Guest / seed, projected by capsem-init
target/config/            Generated runtime config with resolved pins
guest/artifacts/          Core guest payloads: init, doctor, diagnostics, bench
assets/                   Generated VM assets
packages/                 Generated native packages
```

The materialized backend workspace may contain generated package-set files and
profile build scripts. Treat those as implementation details, not authoring
surfaces.

`capsem-admin` is a tool, not a config authority. It validates, materializes,
builds, and checks the profile/corp/settings contracts; it must not grow
scaffolding commands that invent profile, MCP, AI provider, package, or rule
truth outside `config/profiles`, `config/corp`, and `config/settings`.

## CLI commands

```bash
just build-assets code [arch]                # Profile-derived asset rebuild
just build-kernel arm64 code                 # Kernel slice
just build-rootfs arm64 code                 # Rootfs slice
uv run capsem-builder audit                  # Parse trivy/grype vulnerability output
```

Prefer admin/just recipes over direct `capsem-builder build` calls unless the
task is explicitly inside the backend.

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
    rootfs.erofs         Root filesystem
    initrd.img           Initial ramdisk (repacked by just run)
```

Rootfs EROFS settings are profile-derived. The approved release default
is EROFS with `lz4hc` compression level 12.

## Build Ledger

Each per-arch build emits `build-ledger.log` JSONL. The
`rootfs.config_inputs` record captures declared profile package inputs,
rendered rootfs package lists, profile root/build-script inputs, EROFS config,
git revision, and project version. Installed-package/component truth belongs in
the CycloneDX OBOM, not the build ledger.

## Profile Payload Pins

Profile sibling files are ledgered, but agents must not hand-edit their
`hash` or `size` fields in `profile.toml`. Payload pins are produced by the
profile-derived build rail. If editing `apt-packages.txt`, `python-requirements.txt`,
`npm-packages.txt`, `build.sh`, rules, MCP declarations, tips, or root seed
files makes `capsem-admin profile check` fail, run the supported admin pin
refresh command. If that command is missing or incomplete, implement it in
`capsem-admin` with tests before changing the payload. Do not "just fix the
hash" in TOML.

Generated runtime asset URLs/hashes belong in `target/config` after
`capsem-admin profile materialize`, not in checked-in source TOML.

## Adding packages to the VM

1. Edit the profile-owned package file, for example
   `config/profiles/code/apt-packages.txt`,
   `python-requirements.txt`, or `npm-packages.txt`.
2. Refresh payload pins through `capsem-admin`; if that path is missing, add it
   before proceeding.
3. Run the admin/profile validation path.
4. Run `just build-assets code` to rebuild the rootfs.
5. Verify with `capsem-doctor` inside a booted VM.

Do not edit generated Dockerfiles. Docker templates live under `config/docker/`.

## Adding a guest CLI/tool

There are no image-owned AI providers. A CLI/tool exists only if the active
profile declares the package/build hook and any required guest root seed files.

1. Add package input to the profile package files, or add build-time shell work
   to profile-owned `build.sh`.
2. Add config files under `config/profiles/<profile_id>/root/` so they project
   into the VM at boot.
3. Add MCP declarations to profile-owned `mcp.json` when relevant.
4. Add network/model/security behavior through profile/corp rules, not builder
   provider config.
5. Let the credential broker plugin capture/materialize credentials at runtime;
   do not add settings-owned boot secrets.
6. Rebuild with `just build-assets code` and verify with `capsem-doctor`.

`build.sh` is executed only while constructing the rootfs image. It is the
right place for official installer commands such as Claude, AGY, or Ollama
when they cannot be represented as apt/npm/Python package inputs. It must
install stable runtime binaries under system paths such as `/usr/local/bin`;
anything left only under `/root` can be hidden by the runtime overlay.

## Profile `build.sh` contract

Remember this rail when touching profile image contents:

- `config/profiles/<profile_id>/build.sh` is a profile-owned build hook.
- It runs inside the rootfs Docker build, before the EROFS image is produced.
- It does not run during `just install`, service startup, VM boot, or user
  session creation.
- It is for image construction work that cannot be cleanly expressed through
  `apt-packages.txt`, `python-requirements.txt`, or `npm-packages.txt`.
- It may install public runtime tools such as Claude, AGY, and Ollama into
  stable system paths.
- It is not a second profile format, provider registry, runtime settings file,
  credential injection path, or local developer repair script.
- It must not bake credentials, per-user state, corp policy, rules, MCP
  decisions, or runtime settings.
- The owning `profile.toml` must reference it through `[files.build]`; the
  descriptor hash/size is refreshed by the profile-derived build rail, never by hand.
- Changing `build.sh` changes future rootfs assets only. Rebuild assets through
  the profile-derived just/admin-tool rail before claiming a VM contains the
  change.
- The same profile materialization path must be used locally and in CI; no
  one-off Docker or installer path is release proof.
- Verification must be black-box: boot the rebuilt profile image, run the tool
  from the VM, and inspect the generated session evidence when the tool should
  produce network, model, MCP, file, process, or credential events.

Decision rule:

- Normal Debian package: use `apt-packages.txt`.
- Normal Python package: use `python-requirements.txt`.
- Normal npm package: use `npm-packages.txt`.
- Vendor shell installer, binary tarball, wrapper creation, or cleanup that must
  happen while baking the immutable rootfs: use `build.sh`.
- Anything that depends on user/corp/runtime state: do not use `build.sh`.

## Dockerfile templates

Templates live in `config/docker/`:
- `Dockerfile.rootfs.j2` -- rootfs image (apt packages, Python packages, optional npm/curl package sets, profile root/build hook, diagnostics)
- `Dockerfile.kernel.j2` -- kernel build (defconfig, modules, vmlinuz extraction)

Templates use Jinja2 with variables from the admin-materialized profile image
workspace. Do not add a second preview rail for product truth; if a build input
needs validation, add it to the normal profile/admin validation path.

---

# Builder Internals (for modifying the builder itself)

## Architecture: Profile -> admin materialization -> Pydantic -> context dict -> Jinja2 -> Dockerfile

The data flows through four layers:

1. **Profile ledger** (`config/profiles/<id>/profile.toml`) and admin-pinned
   sibling files.
2. **capsem-admin** validates and materializes a backend build workspace.
3. **Pydantic models** (`src/capsem/builder/models.py`) parse that workspace.
4. **Context dict** (`src/capsem/builder/docker.py`) feeds Jinja2 templates.
5. **Jinja2 templates** (`config/docker/`) produce Dockerfiles.

### Key files

| File | Role |
|------|------|
| `src/capsem/builder/models.py` | All Pydantic models (enums, configs, top-level `GuestImageConfig`) |
| `src/capsem/builder/config.py` | Backend loader for admin-materialized build workspaces |
| `src/capsem/builder/docker.py` | Context builders (`_rootfs_context`, `_kernel_context`), rendering, build execution |
| `config/docker/Dockerfile.rootfs.j2` | Rootfs Dockerfile template |
| `config/docker/Dockerfile.kernel.j2` | Kernel Dockerfile template |
| `src/capsem/builder/validate.py` | Validation rules (E001-E302, W001-W012) |
| `src/capsem/builder/cli.py` | Click CLI entry points |

### Context dict (rootfs template variables)

`_rootfs_context()` in `docker.py` builds the dict passed to `Dockerfile.rootfs.j2`:

```python
{
    "arch": ArchConfig,           # Per-arch settings (docker_platform, rust_target, etc.)
    "arch_name": str,             # "arm64" or "x86_64"
    "apt_packages": list[str],    # Materialized from profile apt-packages.txt
    "python_packages": list[str], # Materialized from profile python-requirements.txt
    "python_install_cmd": str,    # e.g. "uv pip install --system --break-system-packages"
    "npm_packages": list[str],    # Materialized from profile npm-packages.txt
    "profile_root_seed": bool,    # Whether profile-root/ is copied into the image
    "profile_build_script": bool, # Whether profile-build.sh is executed
    "npm_prefix": str,            # e.g. "/opt/ai-clis"
    "guest_binaries": list[str],  # ["capsem-pty-agent", "capsem-net-proxy", "capsem-mcp-server"]
}
```

### Kernel context dict

```python
{
    "arch": ArchConfig,
    "arch_name": str,
    "kernel_version": str,  # e.g. "6.6.130"
}
```

## How to: Change a shipped CLI

1. Prefer a profile package file (`apt-packages.txt`, `npm-packages.txt`, or
   `python-requirements.txt`) when the tool has a normal package manager.
2. Use profile-owned `build.sh` when the vendor ships an official shell
   installer. The build hook runs during rootfs construction only.
3. Make sure binaries end up in stable system paths such as `/usr/local/bin`.
4. Refresh profile file descriptor pins through `capsem-admin`; if the rail
   cannot express the change, implement it with tests first.
5. Add or update capsem-admin materialization tests and Docker context tests.
6. Rebuild: `just build-assets code` and verify with `capsem-doctor`.

Ollama is intentionally installed by `config/profiles/<id>/build.sh`, not by a
VM one-off command. That keeps Codex, Claude, AGY, and OpenAI-compatible local
testing available in every shipped profile image that declares the hook.

## How to: Add a new package to an existing set

1. Edit `config/profiles/<profile_id>/apt-packages.txt`,
   `python-requirements.txt`, or `npm-packages.txt`.
2. Refresh the matching descriptor pin through `capsem-admin`.
3. Validate through capsem-admin.
4. Rebuild: `just build-assets <profile_id>`.

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

## Backend Workspace Schema

The backend workspace is generated by `capsem-admin`; do not author it by
hand for product behavior. Its install inputs are package-set TOML files:

```toml
[npm]
name = "Node Packages"
manager = "npm"
install_cmd = "npm install -g --prefix /opt/ai-clis"
packages = ["@scope/package"]
```

Profiles own CLI/tool selection. If an installer cannot be represented as a
package set, put it in `config/profiles/<profile_id>/build.sh`, reference it
from `[files.build]` in `profile.toml`, refresh pins with `capsem-admin`, and
rebuild through the admin/just rail. Do not add a provider registry under
backend-generated image workspaces.

## Build pipeline (what `build_image()` does)

For rootfs:
1. Build guest agent binaries (`cross_compile_agent` -- on macOS delegates to `container_compile_agent` which builds inside a Linux container; on Linux compiles natively)
2. Assemble build context (`prepare_build_context`) -- copies CA cert, shell configs, diagnostics, agent binaries
3. Render Dockerfile from template
4. `docker build`
5. Export container filesystem as tar
6. Create EROFS from tar (`create_erofs` -- runs mkfs.erofs in a container)
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
