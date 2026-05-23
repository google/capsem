---
title: Service Settings
description: Service-scoped settings, schema validation, telemetry, profile catalogs, and corp directives.
sidebar:
  order: 2
---

Service Settings V2 configure the host service and desktop control plane.
Profiles configure VM/session behavior. Keep that boundary sharp: service
settings choose roots, catalogs, assets, credentials, telemetry, and extension
endpoints; profiles choose packages, VM assets, MCP servers, enforcement, and
detection.

The schema id is `capsem.service-settings.v2`. The JSON Schema artifact is
`schemas/capsem.service-settings.v2.schema.json`.

## Commands

```bash
capsem-admin settings schema
capsem-admin settings validate service.toml
capsem-admin settings validate service.toml --json
capsem-admin settings doctor service.toml --json
```

`capsem-admin` parses TOML once, then validates through the same Pydantic model
used for JSON. JSON input uses `model_validate_json()` or
`TypeAdapter.validate_json()`. JSON output uses `model_dump_json()`.

## Example

```toml
version = 1

[app]
auto_launch = true
google_config_path = "/Users/example/.config/gcloud/application_default_credentials.json"

[app.appearance]
theme = "dark"
accent = "blue"

[profiles]
base_dirs = ["/Library/Application Support/Capsem/profiles/base"]
corp_dirs = ["/Library/Application Support/Capsem/profiles/corp"]
user_dirs = ["/Users/example/.capsem/profiles"]
default_profile = "everyday-work"
allow_user_profiles = true
allow_user_fork = true
allow_user_delete = false

[assets]
assets_dir = "/var/lib/capsem/assets"
image_roots = ["/var/lib/capsem/images"]
download_base_url = "https://assets.example.com/capsem/"

[credentials]
backend = "toml"

[credentials.items."openai.api_key"]
description = "OpenAI API key reference"
value = "env:OPENAI_API_KEY"

[telemetry]
enabled = true
endpoint = "https://otel.example.com/v1/traces"
batch_max_events = 64
flush_interval_ms = 1000
redact_secrets = true
retry_attempts = 2
failure_mode = "drop"

[telemetry.headers]
x-capsem-tenant = "example"

[remote_policy]
enabled = false
timeout_ms = 1500
failure_mode = "fail-closed"

[profile_catalog]
manifest_url = "https://profiles.example.com/capsem/manifest.json"
profile_payload_pubkey = "RWQprofilepayloadpubkey"
check_interval_secs = 300

[[corp_directives]]
operation = "lock"
path = "security.capabilities.network_egress"
value = "ask"
reason = "Corp network egress must stay interactive."
```

## Sections

| Section | Purpose |
|---|---|
| `app` | Host app behavior and appearance defaults. |
| `profiles` | Built-in, corp, and user profile roots plus default profile behavior. |
| `assets` | Service asset/cache locations, image roots, and optional download base URL. |
| `credentials` | Credential backend and named credential references. |
| `telemetry` | Export endpoint, headers, batching, retry, redaction, and failure mode. |
| `remote_policy` | Reserved remote enforcement endpoint shape. S13 owns shipped remote decisions. |
| `profile_catalog` | Signed profile catalog URL, profile payload public key, and background check interval. |
| `corp_directives` | Corp-applied profile overrides after profile inheritance. |

## Validation Rules

- Unknown fields are rejected.
- `profiles.base_dirs` must contain at least one directory.
- Profile ids use lowercase letters, numbers, and hyphens.
- Telemetry requires `telemetry.endpoint` when enabled.
- Remote policy requires `remote_policy.endpoint` when enabled.
- Profile catalogs require both `manifest_url` and `profile_payload_pubkey`.
- `http://` catalog URLs are allowed only for loopback development hosts.
- `corp_directives` with `add`, `replace`, or `lock` require `value`.
- `corp_directives` with `remove` or `forbid` must not carry `value`.

## Fixtures

The service-settings contract is tested with shared fixtures:

```text
schemas/fixtures/service-settings-v2-minimal.json
schemas/fixtures/service-settings-v2-complete.json
schemas/fixtures/service-settings-v2-defaults.json
schemas/fixtures/service-settings-v2-invalid-unknown-field.json
schemas/fixtures/service-settings-v2-invalid-profile-roots.json
schemas/fixtures/service-settings-v2-invalid-telemetry.json
schemas/fixtures/service-settings-v2-invalid-remote-policy.json
schemas/fixtures/service-settings-v2-invalid-profile-catalog.json
```

Python and Rust both validate the same valid and invalid shapes. This keeps
settings at the same standard as profiles instead of treating them as loose
configuration JSON.
