---
title: Customizing VM Images
description: How to edit guest configuration, rebuild images, and test your changes.
sidebar:
  order: 15
---

The VM image is defined by TOML configs in `guest/config/`. To change what's installed in the VM -- packages, AI providers, MCP servers, security policy -- you edit these configs and rebuild.

## The config directory

```
guest/config/
    build.toml              Build settings (base image, compression, kernel branch)
    manifest.toml           Package metadata
    ai/
        anthropic.toml      Claude Code provider
        google.toml         Gemini CLI provider
        openai.toml         Codex provider
    packages/
        apt.toml            System packages (coreutils, git, curl, python3, ...)
        python.toml         Python packages (numpy, requests, pytest, ...)
    mcp/
        capsem.toml         Built-in MCP server
    security/
        web.toml            Domain allow/block policy
    vm/
        resources.toml      CPU, RAM, disk limits
        environment.toml    Shell config, bashrc, PATH, TLS
    kernel/
        defconfig.arm64     Kernel config (arm64)
        defconfig.x86_64    Kernel config (x86_64)
```

## Common changes

### Add a system package

Edit `guest/config/packages/apt.toml`:

```toml
[apt]
packages = [
    # ... existing packages ...
    "your-package",
]
```

### Add a Python package

Edit `guest/config/packages/python.toml`:

```toml
[python]
packages = ["numpy", "pandas", "requests", "pytest", "your-package"]
```

### Add an AI provider

Create `guest/config/ai/your-provider.toml`:

```toml
[your_provider]
name = "Your Provider"
description = "Your LLM provider"
enabled = true

[your_provider.api_key]
name = "API Key"
env_vars = ["YOUR_PROVIDER_API_KEY"]
prefix = "sk-"
docs_url = "https://your-provider.com/keys"

[your_provider.network]
domains = ["api.your-provider.com"]
allow_get = true
allow_post = true

[your_provider.install]
manager = "npm"
prefix = "/opt/ai-clis"
packages = ["your-provider-cli"]
```

### Change network policy

Edit `guest/config/security/web.toml` to allow or block domains:

```toml
[web]
custom_allow = ["*.your-corp.com"]
custom_block = ["*.banned-domain.com"]
```

### Change VM resources

Edit `guest/config/vm/resources.toml`:

```toml
[resources]
cpu_count = 8
ram_gb = 8
scratch_disk_size_gb = 32
```

## Rebuild and test

After editing configs:

```bash
# 1. Validate your changes (fast, catches typos)
uv run capsem-builder validate guest/

# 2. Preview the generated Dockerfile without building
uv run capsem-builder build guest/ --dry-run

# 3. Rebuild the rootfs (kernel rebuild only needed if you changed defconfig)
just build-rootfs

# 4. Boot and verify
just run "capsem-doctor"
```

If you changed kernel config, rebuild everything:

```bash
just build-assets
just run "capsem-doctor"
```

### What triggers a full rebuild?

| What you changed | Rebuild command |
|-----------------|----------------|
| `packages/*.toml` | `just build-rootfs` |
| `ai/*.toml` | `just build-rootfs` |
| `mcp/*.toml` | `just build-rootfs` |
| `security/web.toml` | No rebuild -- applied at boot via settings |
| `vm/resources.toml` | No rebuild -- applied at boot via settings |
| `vm/environment.toml` | No rebuild -- applied at boot via settings |
| `kernel/defconfig.*` | `just build-kernel` |
| `build.toml` | `just build-assets` (full rebuild) |
| `guest/artifacts/capsem-init` | `just run` (repacks initrd automatically) |

Settings-only changes (security, resources, environment) take effect on the next `just run` without any rebuild -- capsem-builder generates `defaults.json` which the host reads at boot.

## Builder CLI reference

```bash
uv run capsem-builder validate guest/           # lint all configs
uv run capsem-builder inspect guest/            # show resolved config summary
uv run capsem-builder build guest/ --arch arm64 # build for arm64
uv run capsem-builder build guest/ --dry-run    # preview Dockerfiles
uv run capsem-builder doctor guest/             # check prerequisites
```

## Further reading

- [Build System Architecture](/architecture/build-system/) -- how capsem-builder works internally (Pydantic models, Jinja2 templates, Docker pipeline)
- [Custom Images Reference](/architecture/custom-images/) -- full config schema, corporate deployment, install methods, manifest format
- [Life of a Build](./stack) -- how image assets flow into the boot pipeline
