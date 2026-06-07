---
title: Build A Profile
description: Worked flow for authoring, validating, building, and publishing a custom profile.
sidebar:
  order: 6
---

This flow creates a profile with its own package assumptions, controls, and VM
assets.

## One Path

```bash
capsem-admin profile init corp-coding --out profiles/corp-coding.profile.toml
capsem-admin profile validate profiles/corp-coding.profile.toml --json
capsem-admin image plan profiles/corp-coding.profile.toml --json
capsem-admin image build profiles/corp-coding.profile.toml --json
capsem-admin image verify profiles/corp-coding.profile.toml --assets-dir assets/ --json
capsem-admin manifest generate --profiles profiles/ --base-url https://profiles.example.com/catalog/ --out manifest.json
capsem-admin manifest check manifest.json --fast --json
capsem-admin manifest check manifest.json --download --json
```

Omit `--arch` to build all supported release architectures. Use
`--arch arm64` or another supported arch for focused development.

## Add Controls

- Put AI providers, MCP servers, skills, VM settings, enforcement packs, and
  detection packs in the profile.
- Use editable-section booleans to decide what users may change.
- Use package/tool contracts to describe the VM assumptions.
- Use per-arch asset declarations for kernel/initrd/rootfs.

## Publish And Use

1. Publish profile payloads and assets.
2. Sign and publish the catalog.
3. Configure service `profile_catalog`.
4. Run `capsem profile catalog`.
5. Select the profile in CLI or UI.
6. Create a VM; the service downloads/verifies assets and writes the VM pin.

