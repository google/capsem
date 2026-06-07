---
title: Custom Profiles And Images
description: Get from custom controls/images to a VM pinned to a signed profile.
sidebar:
  order: 4
---

Use profiles when you want Capsem to run with your own images, package
contracts, MCP tools, AI-provider controls, enforcement rules, or detections.

## Fast Path

1. Install `capsem-admin`.
2. Create a profile with your controls.
3. Build or reference profile-owned VM assets.
4. Validate the profile and image inventory.
5. Generate/check/sign a profile catalog.
6. Configure Capsem to use that catalog.
7. Select the profile and create a VM.

```bash
uv tool install capsem-admin
capsem-admin profile init corp-coding --out profiles/corp-coding.profile.toml
capsem-admin profile validate profiles/corp-coding.profile.toml --json
capsem-admin image build profiles/corp-coding.profile.toml --json
capsem-admin manifest generate --profiles profiles/ --out manifest.json
capsem profile catalog
capsem profile install corp-coding
capsem run --profile-id corp-coding
```

The service downloads assets on first use and records the VM profile/revision/
asset pin. Updating the profile later does not silently migrate existing VMs.

