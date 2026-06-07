---
title: capsem-admin Internals
description: Developer reference for the Python admin package, Pydantic boundaries, tests, and release packaging.
sidebar:
  order: 17
---

`capsem-admin` is the Python administration package for profiles, service
settings, image plans, image verification, manifests, enforcement packs, and
detection packs. Enterprise admins use the released PyPI package. Developers
use the workspace editable install from bootstrap.

## Development Install

```bash
uv sync
uv run capsem-admin --version
```

Do not validate local development changes against the released PyPI package.
Bootstrap uses the editable workspace package so CLI changes, Pydantic models,
schema generation, and tests all exercise the code in this repo.

## Package Layout

| Path | Purpose |
|---|---|
| `src/capsem/admin/cli.py` | Public `capsem-admin` command tree and JSON reports. |
| `src/capsem/builder/service_settings.py` | Service Settings V2 Pydantic model and schema output. |
| `src/capsem/builder/profiles.py` | Profile V2 model, schema, TOML/JSON validation, and profile helpers. |
| `src/capsem/builder/image_plan.py` | Profile-derived image planning. |
| `src/capsem/builder/image_workspace.py` | Build workspace generation. |
| `src/capsem/builder/image_verify.py` | Asset, package, and image inventory verification. |
| `src/capsem/builder/image_sbom.py` | Guest-image SPDX SBOM generation. |
| `src/capsem/builder/manifest*.py` | Manifest generation, signing, versioning, and check/download verification. |
| `src/capsem/builder/security_packs.py` | Enforcement/detection pack validation and compilation helpers. |
| `src/capsem/builder/doctor.py` | Admin/build prerequisite checks. |

## Model Boundary

All user-authored JSON crosses a Pydantic boundary:

```python
ProfileV2.model_validate_json(payload)
ServiceSettingsV2.model_validate_json(payload)
TypeAdapter(SomeReport).validate_json(payload)
model.model_dump_json()
```

TOML is parsed once, serialized through the Pydantic adapter, and then
validated through the same model. Do not add raw nested `json.loads()` /
`json.dumps()` manipulation for profiles, settings, manifests, image reports,
or rule packs.

## Schemas And Fixtures

Schema artifacts are generated from models:

```text
schemas/capsem.profile.v2.schema.json
schemas/capsem.service-settings.v2.schema.json
schemas/capsem.detection-pack.v1.schema.json
schemas/capsem.detection.ir.v1.schema.json
```

Valid and invalid fixtures live under `schemas/fixtures/` and are shared with
Rust tests. Add fixtures before changing a public field, enum, or validation
rule.

## Focused Tests

Use focused tests while developing:

```bash
uv run python -m pytest tests/test_service_settings.py -q
uv run python -m pytest tests/test_profiles.py -q
uv run python -m pytest tests/test_admin_cli.py -q
uv run python -m pytest tests/test_image_verify.py -q
uv run python -m pytest tests/test_security_packs.py -q
uv run python -m compileall src/capsem
```

Rust parity tests cover the same public contracts:

```bash
cargo test -p capsem-core service_settings
cargo test -p capsem-core profile_schema
cargo test -p capsem-security-engine
```

## Adding A Command

1. Add or extend a Pydantic model first.
2. Add valid and invalid fixtures.
3. Add the CLI handler in `src/capsem/admin/cli.py`.
4. Emit structured JSON reports through Pydantic `model_dump_json()`.
5. Add Python tests for text and `--json` output.
6. Add Rust parity tests when the command touches a runtime contract.
7. Update the enterprise docs in [capsem-admin](/configuration/capsem-admin/).

## Release Handoff

Release packaging must ship the same admin package that generated the schemas
and assets. The S18 gate verifies both paths:

- packaged enterprise use from PyPI;
- developer bootstrap use from the editable workspace.

The two paths must agree on schemas, defaults, validation errors, and JSON
report shapes.
