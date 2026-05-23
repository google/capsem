---
title: Profile Format
description: Profile V2 payload fields, validation, status, assets, and VM pinning.
sidebar:
  order: 1
---

Profiles are the source of truth for VM assumptions. They describe what tools,
packages, MCP servers, skills, providers, rules, detections, and VM assets a
VM may rely on.

Profile payloads are validated by the committed JSON Schema Draft 2020-12
artifact `schemas/capsem.profile.v2.schema.json` and the matching Pydantic v2
models used by `capsem-admin`. Admin tooling reads TOML as input, immediately
validates through the Pydantic JSON model boundary, and writes JSON through
Pydantic serializers. Do not hand-roll raw JSON mutation for profile payloads.

## Minimal Shape

```toml
schema = "capsem.profile.v2"
id = "corp-coding"
revision = "2026.0523.1"
name = "Corp Coding"
profile_type = "corp"
ui = "coding"

[editable]
skills = true
mcp_servers = true
ai_providers = false
rules = false
detections = false
vm = false

[packages]
apt = ["git=1:2.39.*", "ripgrep"]
python = ["pydantic>=2"]

[vm.assets.arm64]
kernel = { url = "https://profiles.example.com/corp-coding/arm64/vmlinuz", hash = "blake3:..." }
initrd = { url = "https://profiles.example.com/corp-coding/arm64/initrd.img", hash = "blake3:..." }
rootfs = { url = "https://profiles.example.com/corp-coding/arm64/rootfs.ext4", hash = "blake3:..." }
```

The profile id is stable. The revision is immutable. A new payload requires a
new revision.

## Standard MCP Format

Profiles use the industry-standard `mcpServers` map. Capsem-only governance
lives under each server's `capsem` key:

```toml
[mcpServers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[mcpServers.github.capsem]
allowed_tools = ["search_repositories", "get_file_contents"]
editable = false
```

Legacy `[mcp.connectors]` is rejected.

## Assets And Pins

Each architecture declares the VM assets it needs. The service downloads assets
only when that profile is selected or first used, verifies hashes/signatures,
and records the VM pin at creation time:

- profile id
- profile revision
- profile payload hash
- package contract hash
- per-asset hashes

A VM with no explicit profile pin is corrupted. A VM with a pinned deprecated
revision may continue with warnings. A VM pinned to a revoked revision must be
surfaced as revoked and handled by the runtime contract; new launches are
blocked.

## Validation Failures

Common profile failures:

| Failure | Result |
|---|---|
| Unknown field | Rejected by schema/Pydantic. |
| Wrong `schema` value | Rejected. |
| `extends_profile_id` without `extends_profile_revision` | Rejected. |
| Missing arch asset declaration | Rejected for that build/launch path. |
| Invalid package version contract | Rejected before image build. |
| Manual catch-all rule at priority `1000` | Rejected. |
| User/base profile using corp priority `-1000..-1` | Rejected. |

Use:

```bash
capsem-admin profile schema
capsem-admin profile validate profiles/corp-coding.profile.toml --json
```

