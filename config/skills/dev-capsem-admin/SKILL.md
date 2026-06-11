---
name: dev-capsem-admin
description: Use when changing Capsem profiles, generated runtime config, profile payload pins, asset manifests, image workspaces, or any flow that must go through capsem-admin instead of hand-written shortcuts.
---

# Capsem Admin Rail

`capsem-admin` is the only supported rail for profile/config generation and
validation. Use it whenever a change touches profile identity, profile-owned
payloads, asset manifests, generated `target/config`, image workspaces, or
profile readiness proof.

## Ownership

- Source profiles live in `config/profiles/<profile_id>/`.
- A profile's source ledger is `config/profiles/<profile_id>/profile.toml`.
- Profile-owned payloads live beside that ledger and must be hash-pinned from
  `profile.toml`.
- Generated runtime config lives under `target/config/`.
- Never hand-patch generated runtime config.

## Required Commands

Create or clone a profile through admin:

```bash
cargo run -p capsem-admin -- profile init --output config/profiles/<id>/profile.toml --id <id> --name "<Name>" --description "<Description>" --from config/profiles/code/profile.toml
```

Validate a profile:

```bash
cargo run -p capsem-admin -- profile validate config/profiles/<id>/profile.toml --config-root config --json
```

Check profile payload pins and local file assets:

```bash
cargo run -p capsem-admin -- profile check config/profiles/<id>/profile.toml --config-root config --json
```

Materialize runtime config:

```bash
cargo run -p capsem-admin -- profile materialize --profile config/profiles/<id>/profile.toml --config-root config --output-root target/config --json
```

## Guardrails

- Do not copy a profile directory by hand as proof of multi-profile support.
- If `capsem-admin` cannot express the needed profile operation, extend
  `capsem-admin` with tests first.
- UI, TUI, CLI status, service status, and route tests must exercise real
  profile ids from profile routes, not a hardcoded `code` fallback.
- `target/config` must be reproducible from checked-in `config/` through this
  rail.
