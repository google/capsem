---
title: capsem-admin
description: Enterprise and developer workflows for profiles, images, manifests, enforcement, and detection.
sidebar:
  order: 3
---

`capsem-admin` is the typed administration package for Profile V2. Enterprise
admins install the released package from PyPI. Developers use the workspace
editable install created by bootstrap.

## Enterprise Install

```bash
uv tool install capsem-admin
capsem-admin --version
```

Use the PyPI package for corporate profile/image/catalog operations so the
schema and validation behavior match the release deployed to users.

## Development Install

The repo bootstrap uses the workspace package in editable mode:

```bash
uv sync
uv run capsem-admin --version
```

Do not test development changes against the released PyPI package.

## Core Commands

| Command | Purpose |
|---|---|
| `capsem-admin profile schema` | Emit the Profile V2 JSON Schema. |
| `capsem-admin profile validate <profile>` | Validate TOML/JSON through Pydantic models. |
| `capsem-admin image plan <profile>` | Derive an image plan from the profile source of truth. |
| `capsem-admin image build <profile>` | Build all supported arches by default. |
| `capsem-admin image build <profile> --arch arm64` | Build one arch. |
| `capsem-admin image verify <profile> --assets-dir assets/` | Verify image inventory, package contract, and assets. |
| `capsem-admin image sbom <profile> --assets-dir assets/ --out-dir sboms/` | Emit guest-image SPDX SBOMs. |
| `capsem-admin manifest generate --profiles profiles/ --out manifest.json` | Generate a signed-catalog candidate. |
| `capsem-admin manifest check manifest.json --fast` | Use HTTP HEAD checks for profile/assets. |
| `capsem-admin manifest check manifest.json --download` | Download and verify full bytes. |
| `capsem-admin enforcement validate <enforcement-pack>` | Validate enforcement packs. |
| `capsem-admin enforcement backtest <enforcement-pack> --events contexts.jsonl` | Backtest enforcement fixtures. |
| `capsem-admin detection validate <detection-pack>` | Validate detection-pack envelopes. |
| `capsem-admin detection compile <detection-pack>` | Validate Sigma and emit Detection IR. |
| `capsem-admin detection backtest <detection-pack> --events contexts.jsonl` | Backtest detection fixtures. |

## Pydantic Boundary

The admin package uses Pydantic models everywhere user-authored TOML/JSON
crosses a boundary:

- read JSON with `model_validate_json()` or `TypeAdapter.validate_json()`;
- write JSON with `model_dump_json()`;
- bridge TOML by parsing TOML, converting to the model input object, and
  immediately validating through the same model contract;
- emit schemas from the model layer, not from hand-written field lists.

This keeps validation errors stable and debuggable across profiles, service
settings, image plans, manifests, enforcement packs, and detection packs.
