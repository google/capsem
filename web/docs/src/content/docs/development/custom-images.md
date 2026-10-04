---
title: Customizing VM Images
description: How to change the official OCI images or the VM runtime, rebuild, and test your changes.
sidebar:
  order: 15
---

A session is a VM runtime running an application. To change what an agent or
developer gets in a session, edit an OCI image under `images/`. To change the
VM itself -- Capsem's own guest machinery -- edit the runtime inputs under
`config/docker/image/` and `guest/artifacts/`, then rebuild with
`just build-assets`. Enforcement, detection, provider access, plugins,
credentials, VM resources, and UI settings are corp/settings runtime truth,
not image truth.

## The source directories

```
images/
    catalog.toml                      Official image catalog descriptions
    base/                             Capsem base image (every official image builds FROM it)
    dev/  claude-code/  codex-cli/  agy/  claude-desktop/
                                      One Dockerfile per official image
    smoke.sh                          Per-image smoke check
config/docker/
    image/
        build.toml                    Kernel, architectures, EROFS, runtime_apt_packages
        manifest.toml                 Image identity and changelog
        kernel/                       Defconfigs and patches
        vm/environment.toml           Guest shell environment
        security/web.toml             Web domain lists
    Dockerfile.rootfs-dependencies.j2 Runtime package layer template
    Dockerfile.rootfs.j2              Runtime rootfs assembly template
    Dockerfile.kernel.j2              Kernel template
guest/
    artifacts/
        capsem-init                   PID 1 init script
        capsem-doctor                 In-VM diagnostic suite
        capsem-bench                  In-VM benchmarks
        diagnostics/                  Test scripts for capsem-doctor
        tips.txt                      Login tips
```

## Common changes

### Add a tool for agents or developers

Add it to the relevant OCI image under `images/` -- `images/base/` when every
official image needs it, otherwise the one image that does:

```dockerfile
USER root
RUN apt-get update && apt-get install -y --no-install-recommends your-package && \
    rm -rf /var/lib/apt/lists/*
USER capsem
```

This installs the binary into the image; it does not grant network access or
inject credentials. Add provider behavior through corp enforcement rules and
the credential broker plugin.

### Add a guest AI CLI

Add a new directory under `images/` with a `Dockerfile` that builds
`FROM ${BASE}`, list it in `images/catalog.toml`, and add it to the publication
matrix in `.github/workflows/images.yaml`; `tests/images/test_image_ci.py`
holds the three to the same set of names.

### Add a runtime package

Only when Capsem's own guest machinery needs it (the container launcher,
`capsem-init`, `capsem-doctor`, `capsem-bench`, network diagnostics), add it to
`runtime_apt_packages` in `config/docker/image/build.toml`. Everything else
belongs in an OCI image.

### Change network policy

Add allow/block behavior as corp security rules:

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
Run capsem-doctor to verify sandbox integrity.
Your custom tip here.
```

### Change VM resources

VM resources are runtime configuration, not rootfs build configuration. Set
them per session (`capsem create --ram <GB> --cpu <CORES>`) or through
settings.

## Rebuild and test

After editing an OCI image, push it to a registry the service can pull from and
run it in a session:

```bash
docker buildx build --push --build-arg BASE=<base image by digest> \
  -t registry.internal.corp/capsem/corp-dev:test images/corp-dev
capsem run --image registry.internal.corp/capsem/corp-dev:test sh -c 'your-package --version'
```

`tests/images/test_official_images_boot.py` boots the official images the same
way.

After editing runtime inputs:

```bash
# Rebuild the runtime for the host architecture
just build-assets arm64

# Boot and verify
just exec "capsem-doctor"
```

### What triggers a rebuild?

| What you changed | Rebuild command |
|-----------------|----------------|
| `images/<name>/**` | Rebuild that image (and its dependents, for `images/base/`) |
| `config/docker/image/build.toml` `runtime_apt_packages` | `just _build-rootfs <arch>` |
| `guest/artifacts/diagnostics/**`, `tips.txt`, `capsem-bashrc` | `just _build-rootfs <arch>` |
| `config/docker/image/kernel/**` | `just _build-kernel <arch>` |
| backend build spec/templates | `just build-assets [arch]` (full rebuild) |
| `guest/artifacts/capsem-init` | `just shell` (repacks initrd automatically) |
| corp enforcement/detection rules | No rebuild |

Settings-only changes take effect through the settings route path and do not
rebuild the rootfs.

## Builder CLI reference

```bash
just build-assets [arch]
cargo run -p capsem-admin -- image build --config-root config --arch arm64
```

## Further reading

- [Build System Architecture](/architecture/build-system/) -- how capsem-builder works internally (Pydantic models, Jinja2 templates, Docker pipeline)
- [Custom Images Reference](/architecture/custom-images/) -- OCI application images, the runtime package set, corporate deployment, manifest format
- [Life of a Build](./stack) -- how image assets flow into the boot pipeline
