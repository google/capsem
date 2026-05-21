---
name: admin-settings
description: Capsem service settings schema and admin workflow. Use this whenever the user edits ServiceSettingsV2, service.toml, settings defaults, profile roots, asset locations, telemetry settings, credential references, capsem-admin settings commands, or asks whether settings have the same typed quality as profiles.
---

# Admin Settings

Use this skill for operator-facing service settings. Service settings describe
where Capsem finds profiles and assets, how it chooses defaults, and how service
runtime configuration is validated before VMs are created.

## Ground Rules

- Keep settings as strongly typed as profiles. Python must use Pydantic v2
  models and JSON/TOML helpers, not raw JSON manipulation.
- Keep settings separate from Profile V2. Settings point to profile catalogs,
  asset roots, telemetry, credentials, and service defaults; profiles define VM
  behavior and corporate controls.
- Preserve round-trip guarantees. Human-authored TOML and generated JSON must
  reparse to the same typed model.
- Do not load legacy settings/defaults as a fallback authority.

## First Files To Read

- `src/capsem/builder/service_settings.py`
- `schemas/capsem.service-settings.v2.schema.json`
- `schemas/fixtures/service-settings-v2-defaults.json`
- `docs/src/content/docs/architecture/settings.md`
- `docs/src/content/docs/usage/admin-cli.md`

## Admin CLI Surface

Use these commands when working on service settings:

```bash
uv run capsem-admin settings schema
uv run capsem-admin settings init --format toml
uv run capsem-admin settings validate service.toml
uv run capsem-admin settings doctor service.toml
```

## Testing Checklist

- Prove TOML -> typed model -> JSON and JSON -> typed model -> TOML parity.
- Prove invalid types, unknown fields, and missing required values fail closed.
- Prove Rust/Python defaults do not drift when runtime code consumes settings.
- Prove docs and generated schema mention any new operator-facing field.

Useful focused gates:

```bash
uv run python -m pytest tests/test_service_settings.py tests/test_admin_cli.py -q
cargo test -p capsem-core settings_profiles:: --lib
```
