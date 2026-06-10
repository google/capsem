---
title: Customizing VM Images
description: How to edit guest configuration, rebuild images, and test your changes.
sidebar:
  order: 15
---

The VM image is defined by TOML configs in `guest/config/`. To change what's
installed in the VM -- packages, guest tools, MCP server binaries, network
mechanics, or VM resources -- edit these configs and rebuild. Enforcement,
detection, provider access, and credentials are profile/corp/plugin runtime
truth, not image-build truth.

## The config directory

```
guest/
    config/
        build.toml              Build settings (base image, compression, kernel branch)
        manifest.toml           Package metadata
        ai/
            anthropic.toml      Claude Code tool metadata
            google.toml         Gemini CLI tool metadata
            openai.toml         Codex tool metadata
        packages/
            apt.toml            System packages (coreutils, git, curl, python3, ...)
            python.toml         Python packages (numpy, requests, pytest, ...)
        mcp/
            capsem.toml         Built-in MCP server
        security/
            web.toml            Network mechanics
        vm/
            resources.toml      CPU, RAM, disk limits
            environment.toml    Shell config, bashrc, PATH, TLS
        kernel/
            defconfig.arm64     Kernel config (arm64)
            defconfig.x86_64    Kernel config (x86_64)
    artifacts/
        banner.txt              Login banner (ASCII art shown at session start)
        tips.txt                Random tips (one shown per login)
        capsem-bashrc           Shell configuration (PS1, aliases, banner/tips display)
        capsem-init             PID 1 init script
        capsem-doctor           In-VM diagnostic suite
        capsem-bench            In-VM benchmarks
        diagnostics/            Test scripts for capsem-doctor
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

### Add a guest AI CLI

Guest AI CLI metadata can install a tool into the rootfs, but it does not grant
network access or inject credentials. Add network/provider behavior through
profile/corp enforcement rules and the credential broker plugin.

### Change network policy

Keep `guest/config/security/web.toml` for network mechanics such as upstream
ports. Add allow/block behavior as profile or corp security rules:

```toml
[profiles.rules.allow_corp_http]
name = "allow_corp_http"
action = "allow"
match = 'http.host.matches("(^|.*\\.)your-corp\\.com$")'

[profiles.rules.block_banned_domain]
name = "block_banned_domain"
action = "block"
match = 'http.host.matches("(^|.*\\.)banned-domain\\.com$")'
```

### Customize login tips

Edit `guest/artifacts/tips.txt` -- one tip per line, `#` lines are ignored. A random tip is shown each time a user opens a session:

```
pip install and uv pip install work out of the box.
npm install -g works -- packages go to your scratch disk.
Run capsem-doctor to verify sandbox integrity.
Your custom tip here.
```

### Customize the login banner

Edit `guest/artifacts/banner.txt` -- shown at the top of every new session, before the AI tool status and tips.

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
just build-rootfs arm64 code

# 4. Boot and verify
just run "capsem-doctor"
```

If you changed kernel config, rebuild everything:

```bash
just build-assets code
just run "capsem-doctor"
```

### What triggers a full rebuild?

| What you changed | Rebuild command |
|-----------------|----------------|
| `packages/*.toml` | `just build-rootfs <arch> code` |
| `ai/*.toml` | `just build-rootfs <arch> code` |
| `mcp/*.toml` | `just build-rootfs <arch> code` |
| `security/web.toml` | No rebuild -- network mechanics are resolved with the active profile |
| `vm/resources.toml` | No rebuild -- profile VM defaults are resolved at VM creation |
| `vm/environment.toml` | No rebuild -- profile/guest environment defaults are resolved at VM creation |
| `kernel/defconfig.*` | `just build-kernel <arch> code` |
| `build.toml` | `just build-assets code [arch]` (full rebuild) |
| `guest/artifacts/tips.txt` | `just build-rootfs <arch> code` (baked into rootfs) |
| `guest/artifacts/banner.txt` | `just build-rootfs <arch> code` (baked into rootfs) |
| `guest/artifacts/capsem-bashrc` | `just build-rootfs <arch> code` (baked into rootfs) |
| `guest/artifacts/capsem-init` | `just run` (repacks initrd automatically) |

Settings-only changes (security, resources, environment) take effect on the next `just run` without any rebuild -- capsem-builder generates `defaults.json` which the host reads at boot.

## Builder CLI reference

```bash
uv run capsem-builder validate guest/           # lint all configs
uv run capsem-builder inspect guest/            # show resolved config summary
uv run capsem-builder build guest/ --arch arm64 # build for arm64
uv run capsem-builder build guest/ --dry-run    # preview Dockerfiles
uv run capsem-builder doctor --profile code --config-root config # check prerequisites and profile
```

## Further reading

- [Build System Architecture](/architecture/build-system/) -- how capsem-builder works internally (Pydantic models, Jinja2 templates, Docker pipeline)
- [Custom Images Reference](/architecture/custom-images/) -- full config schema, corporate deployment, install methods, manifest format
- [Life of a Build](./stack) -- how image assets flow into the boot pipeline
