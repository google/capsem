---
title: Build System
description: Architecture of capsem-builder, the config-driven build system for Capsem VM images.
sidebar:
  order: 30
---

capsem-builder is a Python CLI that reads TOML configs from `guest/config/`, validates them through Pydantic models, renders Jinja2 Dockerfiles, and produces per-architecture VM assets. It also generates the `defaults.json` consumed by the Rust binary at compile time.

## Architecture

```mermaid
flowchart TD
  subgraph Input["Source of Truth"]
    TOML["guest/config/*.toml\n(AI providers, packages,\nsecurity, VM resources)"]
  end

  subgraph Validation["Validation Layer"]
    Config["config.py\nTOML loader"]
    Models["models.py\nPydantic models\n(PackageManager, InstallConfig,\nAiProviderConfig, ...)"]
    Validate["validate.py\nLinter (E001-E402, W001-W012)"]
  end

  subgraph Generation["Code Generation"]
    Context["docker.py\n_rootfs_context()\n_kernel_context()"]
    Jinja["Jinja2 Templates\nDockerfile.rootfs.j2\nDockerfile.kernel.j2"]
    Defaults["config.py\ngenerate_defaults_json()"]
  end

  subgraph Output["Build Outputs"]
    Docker["Docker Build"]
    Assets["assets/{arch}/\nvmlinuz, initrd.img,\nrootfs.squashfs"]
    JSON["config/defaults.json\n(consumed by Rust)"]
    BOM["manifest.json\n+ B3SUMS"]
  end

  TOML --> Config
  Config --> Models
  Models --> Validate
  Models --> Context
  Models --> Defaults
  Context --> Jinja
  Jinja --> Docker
  Docker --> Assets
  Assets --> BOM
  Defaults --> JSON
```

### Data flow

TOML configs are the single source of truth. The data flows through four layers:

1. **TOML configs** (`guest/config/`) -- user-facing, declarative definitions for AI providers, packages, security policy, and VM resources.
2. **Pydantic models** (`models.py`) -- type-safe validation with enums (`PackageManager`: apt, uv, pip, npm, curl), frozen models, and cross-field validators.
3. **Context dicts** (`docker.py`) -- template variables assembled from the validated config. Each template type (`rootfs`, `kernel`) has its own context builder that collects packages by manager type.
4. **Jinja2 templates** -- Dockerfile output parameterized per architecture.

Three outputs are produced:

1. **defaults.json** -- settings interchange consumed by Rust via `include_str!`, validated against `settings-schema.json`.
2. **Rendered Dockerfiles** -- Jinja2 templates (`Dockerfile.rootfs.j2`, `Dockerfile.kernel.j2`) parameterized per architecture.
3. **manifest.json** -- bill-of-materials with package versions, BLAKE3 hashes, and vulnerability findings.

## TOML Config Structure

All config lives under `guest/config/`. Each file maps to a Pydantic model.

| File | Model | Purpose | Key Fields |
|------|-------|---------|------------|
| `build.toml` | `BuildConfig` | Architectures, compression | `compression`, `compression_level`, `architectures.*` |
| `manifest.toml` | `ImageManifestConfig` | Image identity and changelog | `name`, `version`, `description`, `changelog` |
| `ai/*.toml` | `AiProviderConfig` | AI provider definitions | `api_key`, `network.domains`, `install` (manager: npm/curl), `cli`, `files` |
| `packages/apt.toml` | `PackageSetConfig` | Apt package set | `manager`, `install_cmd`, `packages`, `network` |
| `packages/python.toml` | `PackageSetConfig` | Python package set | `manager`, `install_cmd`, `packages` |
| `mcp/*.toml` | `McpServerConfig` | MCP server definitions | `transport`, `command`, `url`, `args`, `env` |
| `security/web.toml` | `WebSecurityConfig` | Domain allow/block policy | `allow_read`, `allow_write`, `custom_allow`, `search`, `registry`, `repository` |
| `vm/resources.toml` | `VmResourcesConfig` | CPU, RAM, disk limits | `cpu_count`, `ram_gb`, `scratch_disk_size_gb` |
| `vm/environment.toml` | `VmEnvironmentConfig` | Shell, PATH, TLS | `shell.term`, `shell.home`, `shell.path`, `tls.ca_bundle` |
| `kernel/defconfig.*` | (raw) | Kernel configs per arch | Linux kernel defconfig files |

Example `build.toml`:

```toml
[build]
compression = "zstd"
compression_level = 15

[build.architectures.arm64]
base_image = "debian:bookworm-slim"
docker_platform = "linux/arm64"
rust_target = "aarch64-unknown-linux-musl"
kernel_branch = "6.6"
kernel_image = "arch/arm64/boot/Image"
defconfig = "kernel/defconfig.arm64"
node_major = 24
```

Example AI provider (`ai/anthropic.toml`):

```toml
[anthropic]
name = "Anthropic"
description = "Claude Code AI agent"
enabled = true

[anthropic.api_key]
name = "Anthropic API Key"
env_vars = ["ANTHROPIC_API_KEY"]
prefix = "sk-ant-"
docs_url = "https://console.anthropic.com/settings/keys"

[anthropic.network]
domains = ["*.anthropic.com", "*.claude.com"]
allow_get = true
allow_post = true

[anthropic.install]
manager = "curl"
packages = ["https://claude.ai/install.sh"]
```

## Validation Pipeline

`capsem-builder validate` runs compiler-style diagnostics with error codes, severity levels, and file:line references. Errors block the build; warnings are informational.

### Error Codes

| Range | Category | Examples |
|-------|----------|----------|
| E001-E002 | TOML parsing | Missing `build.toml`, invalid TOML syntax |
| E003-E005 | Pydantic validation | Schema violations, empty package lists, invalid enum values |
| E006 | Domain validation | URLs in domain fields, ports, path components |
| E008 | Duplicate keys | Same key in multiple files within a directory |
| E009-E010 | File content | Non-absolute paths, invalid JSON in `.json` file settings |
| E100-E103 | Schema / JSON | Generated JSON fails schema validation |
| E200-E202 | Cross-language | Rust/Python conformance mismatches |
| E300-E305 | Artifacts | Missing defconfig, CA cert, capsem-init, diagnostics |
| E400-E402 | Docker | Dockerfile generation failures |

### Warning Codes

| Code | Description |
|------|-------------|
| W001 | Package sets configured but no registry in web security |
| W002 | Development packages (`-dev`, `-devel`) in package lists |
| W003 | Potential secrets detected in file content, headers, or env |
| W004 | Package set with no network config |
| W005 | Overlapping allow and block domain lists |
| W006 | Placeholder file content (TODO, FIXME) |
| W007 | Overly broad wildcard domains (`*`, `*.com`) |
| W008 | Duplicate env_vars across AI providers |
| W009 | Shell metacharacters in install_cmd |
| W010 | PATH missing essential directories (`/usr/bin`, `/bin`) |
| W011 | Wide-open network policy (both reads and writes, no block list) |
| W012 | Unknown Rust target (not a known musl target) |

Diagnostic output format:

```
error: [E006] config/ai/anthropic.toml: Invalid domain pattern 'https://api.anthropic.com'
warning: [W003] config/mcp/capsem.toml: Potential secret in mcp.capsem.headers.Authorization
```

## Multi-Architecture Support

Two architectures are supported. Each is self-contained in `build.toml` and produces an independent asset directory.

| Architecture | Hypervisor | Docker Platform | Rust Target | Kernel Image |
|-------------|------------|-----------------|-------------|--------------|
| arm64 | Apple VZ (macOS) / KVM (Linux) | `linux/arm64` | `aarch64-unknown-linux-musl` | `arch/arm64/boot/Image` |
| x86_64 | KVM | `linux/amd64` | `x86_64-unknown-linux-musl` | `arch/x86_64/boot/bzImage` |

Output layout:

```
assets/
  arm64/
    vmlinuz
    initrd.img
    rootfs.squashfs
    tool-versions.txt
  x86_64/
    vmlinuz
    initrd.img
    rootfs.squashfs
    tool-versions.txt
  manifest.json
  B3SUMS
```

## Build Pipeline

```mermaid
flowchart TD
  Load["Load TOML configs"] --> Validate["Validate (Pydantic + linter)"]
  Validate -->|errors| Abort["Abort with diagnostics"]
  Validate -->|clean| Arches["For each architecture"]
  Arches --> Cross["Cross-compile guest binaries\n(cargo build --target)"]
  Cross --> Render["Render Dockerfile.rootfs.j2"]
  Render --> Context["Assemble build context\n(CA cert, bashrc, diagnostics, binaries)"]
  Context --> Build["Docker build"]
  Build --> Export["Export container filesystem"]
  Export --> Squash["mksquashfs (zstd compression)"]
  Squash --> Versions["Extract tool versions"]
  Versions --> Checksums["Generate B3SUMS + manifest.json"]
```

The kernel build follows a parallel path:

```mermaid
flowchart TD
  KLoad["Load build.toml"] --> KResolve["Resolve kernel version\n(kernel.org LTS lookup)"]
  KResolve --> KRender["Render Dockerfile.kernel.j2"]
  KRender --> KBuild["Docker build\n(kernel compile + initrd)"]
  KBuild --> KExtract["Extract vmlinuz + initrd.img"]
```

Key implementation details:

- **Container runtime auto-detection.** Docker CLI.
- **CI cache integration.** Docker buildx with GitHub Actions cache (`type=gha`) when `GITHUB_ACTIONS` is set.
- **Kernel version resolution.** Fetches the latest stable version for the configured LTS branch from `kernel.org/releases.json`, falls back to a hardcoded version on network failure.
- **Cross-compilation.** Guest agent binaries are cross-compiled with `cargo build --target {rust_target}` using `rust-lld` as the linker (configured in `.cargo/config.toml`).
- **Clock skew resilience.** All `apt-get update` calls use `-o Acquire::Check-Valid-Until=false` to handle container VM clock drift.

## Container Runtime Requirements

On macOS, Docker runs inside a Colima VM with limited resources. The rootfs build runs apt, npm, and curl-based CLI installers concurrently, requiring substantial memory.

| Threshold | RAM | Notes |
|-----------|-----|-------|
| **Minimum** | 4 GB | Below this, builds OOM-kill (exit 137) |
| **Recommended** | 8 GB | Comfortable margin for all installers |
| **CI (GitHub Actions)** | 7 GB | Standard runner allocation |

```bash
# Colima (macOS): configure VM resources
colima stop
colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8

# Linux: Docker runs natively, no memory tuning needed
# sudo apt install docker.io
```

`just doctor` and `capsem-builder doctor` both check these resources automatically and fail if below minimum.

## Install Manager Types

AI providers declare how their CLI gets installed via `[provider.install]`. The builder supports multiple install strategies:

| Manager | Template Handling | Use Case | Example |
|---------|------------------|----------|---------|
| `npm` | Batched into single `npm install -g --prefix` | Node.js CLI tools | Gemini CLI, Codex |
| `curl` | Each URL gets its own `RUN curl -fsSL URL \| bash` | Native binary installers | Claude Code |
| `apt` | Package set (not per-provider) | System packages | coreutils, git, curl |
| `uv` | Package set (not per-provider) | Python packages | numpy, pytest |
| `pip` | Package set (not per-provider) | Python packages (fallback) | -- |

### The `/root` tmpfs constraint

At runtime, `/root` is a tmpfs overlay -- anything baked into the rootfs under `/root/` during the Docker build is hidden. This matters for CLI installers that put binaries in `~/.local/bin/` or `~/.claude/bin/`:

```dockerfile
# The installer puts claude at ~/.local/bin/claude, which is /root/.local/bin/
# inside the container. Since /root is tmpfs at runtime, copy to /usr/local/bin.
RUN curl -fsSL https://claude.ai/install.sh | bash && \
    for bin in /root/.local/bin/*; do \
        [ -f "$bin" ] && install -m 555 "$bin" /usr/local/bin/; \
    done
```

The `install -m 555` enforces the guest binary security invariant: all binaries are read-only, non-writable by the guest.

### Adding a new install manager

To add a new manager type (e.g., `cargo`):

1. Add the enum value to `PackageManager` in `models.py`
2. Collect packages in `_rootfs_context()` in `docker.py` -- create a new list variable
3. Pass it to the template context dict
4. Add a Jinja2 block in `Dockerfile.rootfs.j2`
5. Add to `_INSTALL_CMDS` in `scaffold.py`
6. Update tests in `test_docker.py` and `test_cli.py`

### Rootfs Dockerfile layer structure

The generated `Dockerfile.rootfs.j2` follows a specific ordering. Understanding this is important when adding new install steps -- the `/root` cleanup and binary permissions are load-bearing:

```mermaid
flowchart TD
  A["1. apt packages\n(system tools, runtimes)"] --> B["2. Node.js via nvm\n(for npm-based CLIs)"]
  B --> C["3. uv installer\n(Python package manager)"]
  C --> D["4. npm install\n(Gemini CLI, Codex)"]
  D --> E["5. CA certificate\n+ certifi patch"]
  E --> F["6. Guest binaries\n(COPY + chmod 555)"]
  F --> G["7. Shell config + diagnostics\n(bashrc, banner, tests)"]
  G --> H["8. Python packages\n(uv pip install)"]
  H --> I["9. Security hardening\n(strip setuid, rm EXTERNALLY-MANAGED)"]
  I --> J["10. rm -rf /root\n(clean HOME for tmpfs)"]
  J --> K["11. curl installers\n(Claude Code, copy to /usr/local/bin)"]
  K --> L["12. Switch apt to HTTPS"]

  style J fill:#f9f,stroke:#333
  style K fill:#bbf,stroke:#333
```

Step 10 and 11 ordering matters: curl installers run _after_ the `/root` cleanup so there's a clean HOME. Binaries are immediately copied to `/usr/local/bin/` since `/root` becomes tmpfs at boot.

## Manifest and BOM

Every build produces `manifest.json` at the asset root. The BOM records:

| Section | Source | Contents |
|---------|--------|----------|
| Packages (dpkg) | `dpkg-query` output | Name, version, architecture |
| Packages (pip) | `pip list --format json` | Name, version |
| Packages (npm) | `npm ls --json --global` | Name, version |
| Assets | `b3sum` output | Filename, BLAKE3 hash, size in bytes |
| Vulnerabilities | Trivy or Grype scan | CVE ID, severity, package, installed/fixed versions |

The `audit` subcommand parses vulnerability scanner output and fails on CRITICAL or HIGH findings.

## CLI Commands

| Command | Description | Key Options |
|---------|-------------|-------------|
| `build` | Render Dockerfiles or build images | `--arch`, `--dry-run`, `--json`, `--template`, `--output`, `--kernel-version` |
| `validate` | Lint and validate guest config | `--artifacts` (check built artifacts too) |
| `inspect` | Show config summary | `--json` |
| `audit` | Parse vulnerability scan results | `--scanner` (trivy/grype), `--input`, `--json` |
| `init` | Scaffold a minimal guest config directory | `--force` |
| `new` | Create a new image config from a base | `--from`, `--non-interactive`, `--force` |
| `add ai-provider` | Add an AI provider template | `--dir`, `--force` |
| `add packages` | Add a package set template | `--dir`, `--manager`, `--force` |
| `add mcp` | Add an MCP server template | `--dir`, `--transport`, `--force` |
| `mcp` | Start MCP stdio server for builder tools | (none) |
| `doctor` | Check build prerequisites | (none) |

Usage:

```bash
# Validate config
uv run capsem-builder validate guest

# Dry-run: render Dockerfiles without building
uv run capsem-builder build --dry-run --json

# Build rootfs for arm64 only
uv run capsem-builder build --arch arm64

# Build kernel for all architectures
uv run capsem-builder build --template kernel

# Scaffold a new image config
uv run capsem-builder new my-image --from guest
```

## Settings JSON Generation

The builder bridges Python config and Rust runtime through a JSON interchange layer.

```mermaid
flowchart LR
  TOML["guest/config/*.toml"] --> Py["generate_defaults_json()"]
  Py --> DJ["config/defaults.json"]
  DJ --> Rust["include_str! in Rust"]
  Py --> Schema["settings-schema.json"]
  Schema --> CV["Cross-language\nconformance tests"]
  DJ --> CV
```

`generate_defaults_json()` transforms a `GuestImageConfig` into the hierarchical JSON tree consumed by the Rust settings registry. This JSON defines every setting's name, description, type, default value, and metadata (env vars, domain rules, UI hints).

The schema is generated from `SettingsRoot.model_json_schema()` (Pydantic) and written to `config/settings-schema.json`. Cross-language conformance tests verify that:

1. The generated `defaults.json` validates against the JSON schema.
2. Rust's compiled-in defaults match the Python-generated output.
3. Every setting referenced in Rust code exists in the schema.

This ensures the Python build tooling and Rust runtime never drift.
