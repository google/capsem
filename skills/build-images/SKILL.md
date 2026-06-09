---
name: build-images
description: Building Capsem VM images with capsem-builder. Use when working with guest image configuration, Dockerfiles, kernel builds, rootfs builds, the builder CLI, or guest config TOML files. Covers the config-driven build system, guest config layout, Dockerfile templates, multi-arch support, the builder CLI commands, AND the internal architecture for modifying the builder itself (models, context flow, template variables, adding install managers).
---

# Building VM Images

## Overview

capsem-builder is a config-driven build system. It reads TOML configs from `guest/config/`, renders Jinja2 Dockerfile templates, and builds kernel + rootfs via Docker. Assets output to `assets/{arch}/`.

## Guest config layout

```
guest/config/
  build.toml              Architectures, compression, base images
  manifest.toml           Image name, version, changelog
  ai/*.toml               Guest AI CLI/tool metadata (not credential truth)
  packages/*.toml         Package sets (apt, python)
  mcp/*.toml              MCP server configs
  security/web.toml       Network mechanics (ports/capture)
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

## Adding packages to the VM

1. Edit the appropriate config in `guest/config/packages/` (apt or python TOML)
2. Run `uv run capsem-builder validate guest/` to check
3. Run `just build-assets code` to rebuild the rootfs
4. Verify: `just run "capsem-doctor"`

Do not edit Dockerfiles directly -- they are rendered from Jinja2 templates in `src/capsem/builder/templates/`.

## Adding a guest AI CLI/tool

1. Add guest tool install metadata under `guest/config/ai/<tool>.toml` only if
   the tool must be baked into the image.
2. Add network/provider behavior through profile/corp enforcement rules, not
   `guest/config/ai` or `security/web.toml`.
3. Let the credential broker plugin capture/materialize credentials at runtime;
   do not add settings-owned boot secrets.
4. Validate: `uv run capsem-builder validate guest/`
5. Rebuild: `just build-assets code`

## Dockerfile templates

Templates live in `src/capsem/builder/templates/`:
- `Dockerfile.rootfs.j2` -- rootfs image (apt packages, Python packages, AI CLIs, diagnostics)
- `Dockerfile.kernel.j2` -- kernel build (defconfig, modules, vmlinuz extraction)

Templates use Jinja2 with variables from the merged guest config. Preview with `--dry-run`.

---

# Builder Internals (for modifying the builder itself)

## Architecture: TOML -> Pydantic -> context dict -> Jinja2 -> Dockerfile

The data flows through four layers:

1. **TOML configs** (`guest/config/`) -- user-facing, declarative
2. **Pydantic models** (`src/capsem/builder/models.py`) -- validation + types
3. **Context dict** (`src/capsem/builder/docker.py`) -- template variables
4. **Jinja2 templates** (`src/capsem/builder/templates/`) -- Dockerfile output

### Key files

| File | Role |
|------|------|
| `src/capsem/builder/models.py` | All Pydantic models (enums, configs, top-level `GuestImageConfig`) |
| `src/capsem/builder/config.py` | TOML loader: walks `guest/config/`, returns `GuestImageConfig` |
| `src/capsem/builder/docker.py` | Context builders (`_rootfs_context`, `_kernel_context`), rendering, build execution |
| `src/capsem/builder/templates/Dockerfile.rootfs.j2` | Rootfs Dockerfile template |
| `src/capsem/builder/templates/Dockerfile.kernel.j2` | Kernel Dockerfile template |
| `src/capsem/builder/scaffold.py` | `_INSTALL_CMDS` dict + scaffolding for `capsem-builder new` |
| `src/capsem/builder/validate.py` | Validation rules (E001-E302, W001-W012) |
| `src/capsem/builder/cli.py` | Click CLI entry points |

### Context dict (rootfs template variables)

`_rootfs_context()` in `docker.py` builds the dict passed to `Dockerfile.rootfs.j2`:

```python
{
    "arch": ArchConfig,           # Per-arch settings (docker_platform, rust_target, etc.)
    "arch_name": str,             # "arm64" or "x86_64"
    "apt_packages": list[str],    # From packages/apt.toml
    "python_packages": list[str], # From packages/python.toml
    "python_install_cmd": str,    # e.g. "uv pip install --system --break-system-packages"
    "npm_packages": list[str],    # From ai/*.toml where install.manager == "npm"
    "npm_prefix": str,            # e.g. "/opt/ai-clis"
    "curl_installs": list[str],   # From ai/*.toml where install.manager == "curl"
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

## How to: Add a new install manager

Example: adding a `curl` manager so a CLI can be installed via `curl | bash` instead of npm.

### Step 1: Add enum value to `PackageManager`

In `src/capsem/builder/models.py`:

```python
class PackageManager(str, Enum):
    APT = "apt"
    UV = "uv"
    PIP = "pip"
    NPM = "npm"
    CURL = "curl"  # <-- new
```

### Step 2: Collect packages in `_rootfs_context()`

In `src/capsem/builder/docker.py`, add a new list and populate it from providers:

```python
curl_installs: list[str] = []
for provider in config.ai_providers.values():
    if provider.enabled and provider.install:
        if provider.install.manager == PackageManager.CURL:
            curl_installs.extend(provider.install.packages)
```

Add `"curl_installs": curl_installs` to the returned dict.

### Step 3: Add template block

In `src/capsem/builder/templates/Dockerfile.rootfs.j2`:

```jinja2
{% for url in curl_installs %}
# CLI installed via installer script
RUN curl -fsSL {{ url }} | bash
{% endfor %}
```

### Step 4: Add to scaffold

In `src/capsem/builder/scaffold.py`, add to `_INSTALL_CMDS`:

```python
"curl": "curl -fsSL",
```

### Step 5: Update the TOML config

For guest tool metadata in `guest/config/ai/<tool>.toml`:

```toml
[provider.install]
manager = "curl"
packages = ["https://example.com/install.sh"]
```

### Step 6: Update tests

- `tests/test_docker.py` -- context dict assertions (what's in npm_packages vs curl_installs)
- `tests/test_cli.py` -- Dockerfile rendering assertions (corporate config tests)

## How to: Change how an AI CLI is installed

1. Edit `guest/config/ai/<tool>.toml` -- change the install section
2. If changing install manager type, may need to update `_rootfs_context()` in `docker.py`
3. Check `extract_tool_versions()` in `docker.py` -- it hardcodes version-check paths
4. Update tests in `test_docker.py` and `test_cli.py`
5. Rebuild: `just build-assets code && just run "capsem-doctor"`

## How to: Add a new package to an existing set

1. Edit `guest/config/packages/apt.toml` or `guest/config/packages/python.toml`
2. Add the package name to the `packages` list
3. Validate: `uv run capsem-builder validate guest/`
4. Rebuild: `just build-assets code`

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

## Guest AI CLI/tool TOML schema

```toml
[tool_key]
name = "Tool Name"
description = "What this guest tool does"
enabled = true  # false to exclude from build

[tool_key.cli]
key = "cli-binary-name"      # e.g. "claude", "gemini", "codex"
name = "CLI Display Name"

[tool_key.install]
manager = "npm"               # "npm", "curl", "apt", "uv", "pip"
prefix = "/opt/ai-clis"       # Install prefix (npm only)
packages = ["@scope/package"] # Package names or URLs

[tool_key.files.some_config]
path = "/root/.config/file.json"
content = '{"key": "value"}'
```

Do not put credentials or allow/block domains here. Credentials are brokered at
runtime. Network access is enforced by profile/corp rules.

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
