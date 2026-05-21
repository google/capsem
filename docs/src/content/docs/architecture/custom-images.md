---
title: Custom Images
description: Build custom Capsem VM images with your own AI providers, packages, and security policies.
sidebar:
  order: 40
---

Capsem images are defined by signed Profile V2 payloads. Organizations create
profiles with their own packages, tools, MCP servers, VM assets, policy packs,
and detection packs, then use `capsem-admin` to derive build plans, verify
assets, generate manifests, and sign the catalog.

## Quick Start

```bash
python -m pip install capsem
capsem-admin profile init corp-dev --out profiles/corp-dev.profile.toml
capsem-admin profile validate profiles/corp-dev.profile.toml --json
capsem-admin image build profiles/corp-dev.profile.toml --arch all --json
capsem-admin image verify profiles/corp-dev.profile.toml --assets-dir assets/ --json
capsem-admin manifest generate --profiles profiles/ --base-url https://profiles.example.com/catalog/ --out manifest.json
```

The generated build workspace still contains TOML files consumed by the Docker
templates, but those files are derived artifacts. The profile is the source of
truth.

## Generated Build Workspace

```
build/corp-dev-image/
    config/
        build.toml              Architectures, compression, base images
        ai/
            anthropic.toml      Provider: API key, domains, CLI install, config files
            google.toml
            openai.toml
        packages/
            apt.toml            System packages
            python.toml         Python packages + PyPI registry
        mcp/
            capsem.toml         MCP server definitions
        security/
            web.toml            Domain allow/block policy
        vm/
            resources.toml      CPU, RAM, disk, session limits
            environment.toml    Shell, bashrc, TLS config
        kernel/
            defconfig.arm64     Kernel config per architecture
            defconfig.x86_64
    artifacts/
        capsem-init             PID 1 init script
        capsem-bashrc           Shell configuration
        banner.txt              Login banner
        diagnostics/            In-VM test suite
```

## Configuration Reference

### AI Providers

Each file in `config/ai/` defines one provider. The filename is the provider identifier.

```toml
# config/ai/anthropic.toml
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

[anthropic.files.settings_json]
path = "/root/.claude/settings.json"
content = '{"permissions":{"defaultMode":"bypassPermissions"}}'
```

Add a custom provider by editing the profile package/tool/provider contract,
then validate the profile:

```bash
capsem-admin profile validate profiles/corp-dev.profile.toml --json
capsem-admin image plan profiles/corp-dev.profile.toml --json
```

### Package Sets

Each file in `config/packages/` defines packages for one manager.

```toml
# config/packages/apt.toml
[apt]
name = "System Packages"
manager = "apt"
packages = [
    "coreutils", "util-linux", "git", "curl",
    "python3", "python3-pip", "python3-venv",
]
```

```toml
# config/packages/python.toml
[python]
name = "Python Packages"
manager = "uv"
install_cmd = "uv pip install --system --break-system-packages"
packages = ["numpy", "pandas", "requests", "pytest"]

[python.network]
name = "PyPI"
domains = ["pypi.org", "files.pythonhosted.org"]
allow_get = true
```

### MCP Servers

```toml
# config/mcp/capsem.toml
[capsem]
name = "Capsem"
description = "Built-in file and snapshot tools"
transport = "stdio"
command = "/run/capsem-mcp-server"
builtin = true
enabled = true
```

### Security Policy

`config/security/web.toml` controls network access inside the VM.

```toml
[web]
allow_read = false      # GET/HEAD for unknown domains
allow_write = false     # POST/PUT for unknown domains
custom_allow = []       # additional allowed domain patterns
custom_block = []       # blocked patterns (override allow)

[web.search.google]
name = "Google"
enabled = true
domains = ["www.google.com", "google.com"]
allow_get = true

[web.registry.npm]
name = "npm"
enabled = true
domains = ["registry.npmjs.org", "*.npmjs.org"]
allow_get = true

[web.repository.github]
name = "GitHub"
enabled = true
domains = ["github.com", "*.github.com", "*.githubusercontent.com"]
allow_get = true
allow_post = true
```

### Build Configuration

`config/build.toml` defines per-architecture build parameters. Each architecture is self-contained.

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

[build.architectures.x86_64]
base_image = "debian:bookworm-slim"
docker_platform = "linux/amd64"
rust_target = "x86_64-unknown-linux-musl"
kernel_branch = "6.6"
kernel_image = "arch/x86_64/boot/bzImage"
defconfig = "kernel/defconfig.x86_64"
node_major = 24
```

### VM Resources

```toml
# config/vm/resources.toml
[resources]
cpu_count = 4
ram_gb = 4
scratch_disk_size_gb = 16
retention_days = 30
max_sessions = 100
```

### VM Environment

```toml
# config/vm/environment.toml
[environment.shell]
term = "xterm-256color"
home = "/root"
path = "/opt/ai-clis/bin:/root/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

[environment.shell.bashrc]
path = "/root/.bashrc"
content = '''
PS1='\[\033[1;32m\]capsem\[\033[0m\]:\[\033[1;34m\]\w\[\033[0m\]\$ '
alias pip='uv pip'
alias claude='claude --dangerously-skip-permissions'
alias gemini='gemini --yolo'
'''

[environment.tls]
ca_bundle = "/etc/ssl/certs/ca-certificates.crt"
```

The `PATH` is set by the host at boot via the settings registry -- do not set PATH in the bashrc (it creates duplicates and hides bugs). The aliases enable auto-approve modes for AI CLIs since the VM is already sandboxed.

## CLI Reference

| Command | What it does |
|---------|-------------|
| `capsem-admin profile init <id> --out <profile>` | Create a valid Profile V2 draft |
| `capsem-admin profile validate <profile> --json` | Validate profile JSON/TOML |
| `capsem-admin image build <profile>` | Build all architectures from a Profile V2 payload |
| `capsem-admin image build <profile> --arch arm64` | Single architecture |
| `capsem-admin image build <profile> --dry-run --json` | Preview without building |
| `capsem-admin image verify <profile> --assets-dir assets/ --json` | Verify local assets, hashes, and package/tool inventory |
| `capsem-admin image sbom <profile> --assets-dir assets/ --out-dir sboms/` | Emit guest-image SPDX SBOMs |
| `capsem-admin manifest generate --profiles profiles/ --out manifest.json` | Build a profile catalog manifest |
| `capsem-admin manifest check manifest.json --download --pubkey profile-sign.pub --json` | Download and verify profile/assets/signatures |
| `capsem-admin policy validate <policy-pack> --json` | Validate enforcement policy packs |
| `capsem-admin detection compile <detection-pack> --out detection.ir.json --json` | Validate Sigma with pySigma and compile Detection IR |

## Manifest

Every build produces `assets/manifest.json` (format 2) -- a single top-level file covering every arch. It records BLAKE3 hashes and file sizes for each asset and ties asset versions to compatible binary versions:

```json
{
  "format": 2,
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
            "rootfs.squashfs": {"hash": "<64-char blake3>", "size": 454230016}
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

### Workflow

1. `capsem-admin profile init corp-image --out profiles/corp-image.profile.toml` -- create a typed draft.
2. Remove unwanted providers, MCP servers, packages, policy packs, or detection packs from the profile.
3. Add internal providers and package/tool requirements to the profile.
4. Validate: `capsem-admin profile validate profiles/corp-image.profile.toml --json`.
5. Build: `capsem-admin image build profiles/corp-image.profile.toml --arch all --json`.
6. Verify: `capsem-admin image verify profiles/corp-image.profile.toml --assets-dir assets/ --json`.
7. Generate and sign the profile catalog manifest.

### Lockdown Example

Create a corp profile draft, then keep only the approved providers and security
packs:

```bash
capsem-admin profile init corp-image --out profiles/corp-image.profile.toml
capsem-admin profile validate profiles/corp-image.profile.toml --json
capsem-admin policy validate corp-policy.toml --json
capsem-admin detection compile corp-detections.yml --out detection.ir.json --json
```

Policy packs carry blocking rules:

```toml
[web]
allow_read = false
allow_write = false
custom_allow = ["*.internal.corp.com"]
custom_block = []

[web.search.google]
name = "Google"
enabled = false

[web.registry.npm]
name = "Internal npm"
enabled = true
domains = ["npm.internal.corp.com"]
allow_get = true
```

## Install Methods

AI providers support two install methods via the `[provider.install]` section:

### npm (default for most CLIs)

```toml
[provider.install]
manager = "npm"
prefix = "/opt/ai-clis"
packages = ["@google/gemini-cli"]
```

All npm packages across providers are batched into a single `npm install -g --prefix /opt/ai-clis` command. The prefix directory is writable at runtime via the overlayfs upper layer, allowing CLIs to self-update.

### curl (native binary installers)

```toml
[provider.install]
manager = "curl"
packages = ["https://claude.ai/install.sh"]
```

Each URL gets its own `RUN curl -fsSL <url> | bash` step. Binaries are automatically copied from `~/.local/bin/` to `/usr/local/bin/` (chmod 555) because `/root` is a tmpfs at runtime.

:::caution[/root is ephemeral]
Anything installed under `/root/` during the Docker build is hidden at runtime by the tmpfs overlay. If your installer puts binaries in `~/.local/bin/` or `~/.claude/bin/`, the template automatically copies them to `/usr/local/bin/`. If you add a custom curl-based installer, verify where it puts its binaries and ensure they're copied to a system path.
:::

## Troubleshooting

| Diagnostic | Cause | Fix |
|-----------|-------|-----|
| `error[E001] missing required field` | TOML config missing a schema field | Check file:line in error, compare against examples above |
| `error[E304] defconfig missing` | Kernel config for declared arch doesn't exist | Add `config/kernel/defconfig.{arch}` |
| `warn[W001] no npm registry` | npm packages declared but no registry in web.toml | Add npm registry entry to security policy |
| `warn[W005] API key in config` | Hardcoded key in TOML | Use `~/.capsem/user.toml` for personal keys |
| Build fails: "container runtime not found" | No Docker | Install Docker (`brew install colima docker` on macOS, `sudo apt install docker.io` on Linux) |
| Build fails: exit 137 (OOM) or exit 143 (SIGTERM mid-build) | Container runtime VM out of memory -- Tauri install-test cold build needs >12GB | Bump Colima to 16GB: `colima stop && colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8` |
| Build fails: "Release file not valid yet" | Container VM clock drift | Builder handles this automatically via `Acquire::Check-Valid-Until=false` |
| CLI not found at runtime | Installer put binary in `/root/` which is tmpfs | Copy binary to `/usr/local/bin/` in the Dockerfile template |
