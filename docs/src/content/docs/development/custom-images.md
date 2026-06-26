---
title: Customizing VM Images
description: How to edit profile-owned image inputs, rebuild images, and test your changes.
sidebar:
  order: 15
---

The VM image is defined by a profile. To change what is installed in the VM,
edit the profile-owned package files, root seed, MCP config, or install script
under `config/profiles/<profile_id>/`, then rebuild through `capsem-admin`.
Enforcement, detection, provider access, plugins, credentials, VM resources,
and UI settings are profile/corp/settings runtime truth, not backend image
workspace truth.

## The config directory

```
config/
    profiles/
        code/
            profile.toml              Profile ledger
            apt-packages.txt          System packages
            python-requirements.txt   Python packages
            npm-packages.txt          Node CLI packages
            build.sh                  Profile image build hook
            mcp.json                  Profile MCP config
            enforcement.toml          Profile enforcement rules
            detection.yaml            Profile Sigma detection rules
            tips.txt                  Login tips
            root/                     Files projected into the guest rootfs
            root.manifest.json        Hashes for files under root/
guest/
    artifacts/
        capsem-init                   PID 1 init script
        capsem-doctor                 In-VM diagnostic suite
        capsem-bench                  In-VM benchmarks
        diagnostics/                  Test scripts for capsem-doctor
config/docker/
    Dockerfile.rootfs.j2              Backend rootfs template
    Dockerfile.kernel.j2              Backend kernel template
```

## Common changes

### Add a system package

Edit `config/profiles/code/apt-packages.txt`:

```text
your-package
```

### Add a Python package

Edit `config/profiles/code/python-requirements.txt`:

```text
your-package
```

### Add a guest AI CLI

Add the package to `config/profiles/code/npm-packages.txt` or the build hook to
`config/profiles/code/build.sh`. This installs the binary into the base image;
it does not grant network access or inject credentials. Add provider behavior
through profile/corp enforcement rules and the credential broker plugin.

### Change network policy

Add allow/block behavior as profile or corp security rules:

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

Edit `config/profiles/code/tips.txt` -- one tip per line, `#` lines are ignored. A random tip is shown each time a user opens a session:

```
pip install and uv pip install work out of the box.
npm install -g works -- packages go to your scratch disk.
Run capsem-doctor to verify sandbox integrity.
Your custom tip here.
```

### Change VM resources

VM resources are profile/runtime configuration, not rootfs build configuration.
Change the VM defaults through the profile/runtime API or profile-owned VM
defaults when that profile schema is active:

```toml
[resources]
cpu_count = 8
ram_gb = 8
scratch_disk_size_gb = 32
```

## Rebuild and test

After editing profile files:

```bash
# 1. Validate your changes (fast, catches typos)
cargo run -p capsem-admin -- profile check config/profiles/code/profile.toml --config-root config

# 2. Rebuild the rootfs (kernel rebuild only needed if you changed backend kernel inputs)
just build-rootfs arm64 code

# 3. Boot and verify
just exec "capsem-doctor"
```

If you changed kernel config, rebuild everything:

```bash
just build-assets code
just exec "capsem-doctor"
```

### What triggers a full rebuild?

| What you changed | Rebuild command |
|-----------------|----------------|
| `config/profiles/code/apt-packages.txt` | `just build-rootfs <arch> code` |
| `config/profiles/code/python-requirements.txt` | `just build-rootfs <arch> code` |
| `config/profiles/code/npm-packages.txt` | `just build-rootfs <arch> code` |
| `config/profiles/code/build.sh` | `just build-rootfs <arch> code` |
| `config/profiles/code/root/**` | `just build-rootfs <arch> code` |
| `config/profiles/code/mcp.json` | No rootfs rebuild unless it changes projected root seed files |
| `config/profiles/code/enforcement.toml` | No rootfs rebuild |
| `config/profiles/code/detection.yaml` | No rootfs rebuild |
| `kernel/defconfig.*` | `just build-kernel <arch> code` |
| backend build spec/templates | `just build-assets code [arch]` (full rebuild) |
| `config/profiles/code/tips.txt` | `just build-rootfs <arch> code` |
| `guest/artifacts/capsem-init` | `just run` (repacks initrd automatically) |

Settings-only changes take effect through the settings/profile route path and
do not rebuild the rootfs.

## Builder CLI reference

```bash
cargo run -p capsem-admin -- profile check config/profiles/code/profile.toml --config-root config
cargo run -p capsem-admin -- image build --profile config/profiles/code/profile.toml --config-root config --arch arm64
```

## Further reading

- [Build System Architecture](/architecture/build-system/) -- how capsem-builder works internally (Pydantic models, Jinja2 templates, Docker pipeline)
- [Custom Images Reference](/architecture/custom-images/) -- full config schema, corporate deployment, install methods, manifest format
- [Life of a Build](./stack) -- how image assets flow into the boot pipeline
