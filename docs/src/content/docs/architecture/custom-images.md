---
title: Custom Images
description: Build custom Capsem VM images with your own AI providers, packages, and security policies.
sidebar:
  order: 40
---

Capsem images are defined declaratively using TOML configuration files. Organizations can create custom images with their own AI providers, pre-installed packages, MCP servers, and security policies.

## Quick Start

```bash
pip install capsem
capsem-builder init my-corp-image/
capsem-builder validate my-corp-image/
capsem-builder build my-corp-image/
```

## Directory Structure

```
my-corp-image/
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

Add a custom provider:

```bash
capsem-builder add ai-provider my-llm
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
| `capsem-builder build [DIR]` | Build all architectures |
| `capsem-builder build --arch arm64` | Single architecture |
| `capsem-builder build --dry-run` | Preview without building |
| `capsem-builder validate [DIR]` | Lint configs with diagnostics |
| `capsem-builder inspect [DIR]` | Render build manifest |
| `capsem-builder audit` | Vulnerability scan |
| `capsem-builder init NAME/` | Scaffold new image |
| `capsem-builder add ai-provider NAME` | Add provider template |
| `capsem-builder add packages NAME` | Add package set template |
| `capsem-builder add mcp NAME` | Add MCP server template |
| `capsem-builder doctor` | Check build prerequisites |

## Manifest

Every build produces `assets/{arch}/manifest.json` -- a complete bill of materials:

```json
{
  "version": "0.13.0",
  "build_timestamp": "2026-03-26T14:30:00Z",
  "architectures": [{
    "arch": "arm64",
    "assets": [
      {"filename": "rootfs.squashfs", "size": 888741888, "b3": "a1b2c3..."},
      {"filename": "vmlinuz", "size": 12582912, "b3": "d4e5f6..."}
    ],
    "packages": [
      {"name": "coreutils", "version": "9.1-1", "manager": "apt", "b3": "..."},
      {"name": "numpy", "version": "1.26.4", "manager": "pip", "b3": "..."}
    ],
    "vulnerabilities": [
      {"package": "curl", "cve": "CVE-2024-XXXX", "severity": "HIGH", "fixed_in": "7.88.2"}
    ]
  }]
}
```

## Corporate Deployment

### Workflow

1. `capsem-builder init corp-image/` -- scaffold from defaults
2. Remove unwanted providers: delete `config/ai/openai.toml`
3. Add internal providers: `capsem-builder add ai-provider internal-llm`
4. Edit security policy: lock down domains in `config/security/web.toml`
5. Add corporate packages: edit `config/packages/python.toml`
6. Validate: `capsem-builder validate corp-image/`
7. Build: `capsem-builder build corp-image/`
8. Distribute: ship the `assets/` directory

### Lockdown Example

Remove all AI providers except Anthropic, block external search, allow only internal registries:

```bash
capsem-builder init corp-image/
rm corp-image/config/ai/google.toml
rm corp-image/config/ai/openai.toml
```

Edit `corp-image/config/security/web.toml`:

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
| Build fails: exit code 137 | Container runtime VM out of memory | Increase to 8GB: `colima stop && colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8` |
| Build fails: "Release file not valid yet" | Container VM clock drift | Builder handles this automatically via `Acquire::Check-Valid-Until=false` |
| CLI not found at runtime | Installer put binary in `/root/` which is tmpfs | Copy binary to `/usr/local/bin/` in the Dockerfile template |
