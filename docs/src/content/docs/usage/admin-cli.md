---
title: Admin CLI
description: Install and use capsem-admin for profile, image, manifest, policy, and detection contracts.
sidebar:
  order: 10
---

`capsem-admin` is the corporate administration CLI. It validates public
Capsem contracts through typed Pydantic models, emits JSON Schema artifacts,
derives images from profiles, and checks signed profile catalogs.

## Install

Corporate admins install the release package from PyPI:

```bash
python -m pip install capsem
capsem-admin --version
```

Developers use the editable repo environment:

```bash
uv sync
uv run capsem-admin --version
uv run capsem-admin profile validate schemas/fixtures/profile-v2-valid.json
```

Bootstrap runs the same editable proof after `uv sync`, so local development
uses the same entrypoint shape as the packaged CLI.

## Command Groups

| Group | Purpose |
|---|---|
| `settings` | Create, validate, and inspect `capsem.service-settings.v2`. |
| `profile` | Create and validate Profile V2 payloads. |
| `image` | Derive build plans, build workspaces, verify image assets, and emit SBOMs from profiles. |
| `manifest` | Generate, check, sign, and verify profile catalog manifests. |
| `policy` | Validate and export schemas for profile-owned enforcement policy packs. |
| `detection` | Validate Sigma-backed detection packs, compile Detection IR, and check event fixtures. |

## Settings And Profiles

```bash
capsem-admin settings init --out service.toml
capsem-admin settings schema
capsem-admin settings validate service.toml --json
capsem-admin settings doctor service.toml --json

capsem-admin profile init corp-dev --out corp-dev.profile.toml
capsem-admin profile schema
capsem-admin profile validate corp-dev.profile.toml --json
```

## Image And Manifest

```bash
capsem-admin image plan corp-dev.profile.toml --json
capsem-admin image build corp-dev.profile.toml --arch all --json
capsem-admin image verify corp-dev.profile.toml --assets-dir assets/ --json
capsem-admin image sbom corp-dev.profile.toml --assets-dir assets/ --out-dir sboms/

capsem-admin manifest generate --profiles profiles/ --base-url https://profiles.example.com/catalog/ --out manifest.json
capsem-admin manifest check manifest.json --fast --json
capsem-admin manifest check manifest.json --download --download-dir downloaded/ --pubkey profile-sign.pub --json
capsem-admin manifest sign manifest.json --key manifest-sign.key --out manifest.json.minisig
capsem-admin manifest verify-signature manifest.json --signature manifest.json.minisig --pubkey manifest-sign.pub --json
```

`--arch all` is the default for image build and verification workflows. Use
`--arch arm64` or `--arch x86_64` only for local debugging or CI shards.

## Policy And Detection

```bash
capsem-admin policy schema
capsem-admin policy validate corp-policy.toml --json

capsem-admin detection schema
capsem-admin detection validate corp-detections.yml --json
capsem-admin detection compile corp-detections.yml --out detection.ir.json --json
capsem-admin detection check corp-detections.yml --events events.jsonl --json
```

Policy packs are enforcement contracts. Detection packs are finding contracts.
Detection packs may embed Sigma YAML, but Sigma is validated with pySigma and
compiled into Capsem Detection IR before runtime consumption.

## JSON Boundaries

Admin commands do not rely on raw JSON dict manipulation at command
boundaries. Public inputs enter through Pydantic validation such as
`model_validate_json()` or `TypeAdapter.validate_json()`, and public JSON
outputs leave through Pydantic dump helpers such as `model_dump_json()` or
`TypeAdapter.dump_json()`.
