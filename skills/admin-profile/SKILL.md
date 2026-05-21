---
name: admin-profile
description: Capsem corporate profile authoring and validation. Use this whenever the user edits Profile V2 payloads, profile catalogs, section editability, root/corp/user profiles, built-in coding/everyday profiles, profile MCP servers, skills, packages, tools, VM assets, or asks how capsem-admin should create, validate, fork, or ship profiles.
---

# Admin Profile

Use this skill for operator-facing Profile V2 work. Profiles are the corporate
source of truth for VM assets, packages, tools, skills, MCP servers, UI mode,
security capabilities, and policy/detection packs.

## Ground Rules

- Treat Profile V2 as typed data. In Python, go through the Pydantic models and
  `model_validate_json()` / `model_dump_json()` boundaries rather than raw JSON
  dictionaries.
- Prefer TOML for humans and JSON Schema for tooling. JSON is an interchange
  output, not the hand-edited authoring format.
- Preserve the no-backward-compatibility stance. Do not reintroduce old
  guest-config or asset-manifest authority when profile payloads can derive it.
- Keep section editability explicit. `editable` gates decide which profile
  sections users can mutate in forks.
- Keep built-in `coding` and `everyday-work` profiles generated from the typed
  guest config until profile-native packages/tools fully replace that bridge.

## First Files To Read

- `src/capsem/builder/profiles.py`
- `schemas/capsem.profile.v2.schema.json`
- `config/profiles/base/*.profile.toml`
- `docs/src/content/docs/architecture/profiles.md`
- `docs/src/content/docs/usage/admin-cli.md`

## Admin CLI Surface

Use these commands when validating or generating profile work:

```bash
uv run capsem-admin profile schema
uv run capsem-admin profile validate config/profiles/base/coding.profile.toml
uv run capsem-admin profile init corp-dev --format toml
uv run capsem-admin profile init-builtins --guest-dir guest
uv run capsem-admin manifest generate --profiles config/profiles/base
```

## Testing Checklist

- Add or update Pydantic/Python tests for every profile field or default change.
- Add Rust fixture/schema parity tests when the runtime must parse the field.
- For editability changes, test direct mutation, fork propagation, and locked
  fork mutation rejection.
- For built-in profile changes, regenerate both `coding` and `everyday-work`
  and validate both.

Useful focused gates:

```bash
uv run python -m pytest tests/test_profiles.py tests/test_admin_cli.py -q
cargo test -p capsem-core --test profile_schema
cargo test -p capsem-core settings_profiles:: --lib
```
