# S07d - Service Settings Schema And Admin Contract

## Status

In progress. Inserted during the 2026-05-19 regroup after S08 exposed that
Profile V2 now has a stronger formal contract than service settings.

First slice landed on 2026-05-20:

- `src/capsem/builder/service_settings.py` adds Pydantic v2
  `ServiceSettingsV2` models and helpers for JSON validation, JSON dumping,
  TOML-to-Pydantic validation, and schema export.
- `schemas/capsem.service-settings.v2.schema.json` is generated from the
  Pydantic model and checked into the repo.
- `schemas/fixtures/service-settings-v2-*.json` contains minimal/complete valid
  fixtures plus invalid unknown-field, catalog, root, telemetry, remote-policy,
  credential, and asset-location fixtures.
- Verification:
  `uv run python -m pytest tests/test_service_settings.py -q` passed with
  11 tests; `cargo test -p capsem-core service_settings_json --lib` passed with
  2 tests.

Second slice landed on 2026-05-20:

- `pyproject.toml` now installs `capsem-admin` as the public Python admin CLI.
- `src/capsem/admin/cli.py` adds typed `capsem-admin settings schema`,
  `capsem-admin settings validate <settings.json|settings.toml> [--json]`, and
  `capsem-admin settings doctor <settings.json|settings.toml> [--json]`
  commands.
- JSON command reports are Pydantic models dumped through
  `model_dump_json(by_alias=True)` with `schema = capsem.service-settings.v2`.
- Verification: `uv run python -m pytest tests/test_admin_cli.py
  tests/test_service_settings.py -q` passed with 17 tests; `uv run
  capsem-admin settings validate schemas/fixtures/service-settings-v2-complete.json`
  and `uv run capsem-admin settings doctor
  schemas/fixtures/service-settings-v2-complete.json --json` both passed.

Third slice landed on 2026-05-20:

- Python `ServiceSettingsV2` now derives default user profile roots from the
  same `CAPSEM_HOME` / `$HOME/.capsem` contract as Rust instead of using a
  literal `~/.capsem/profiles` string.
- `schemas/fixtures/service-settings-v2-defaults.json` is a committed defaults
  contract fixture used by both Python and Rust.
- Verification: `uv run python -m pytest tests/test_service_settings.py
  tests/test_admin_cli.py -q` passed with 18 tests; `cargo test -p capsem-core
  service_settings --lib` passed with 21 service-settings tests.

## Goal

Bring service settings to the same production-quality contract level as Profile
V2 before `capsem-admin`, CLI, UI, docs, and release tooling build more public
surface on top of them.

Profiles remain VM/session-scoped product policy. Service settings remain
service/app-scoped control plane: profile roots, selected defaults, catalog
source, telemetry/export configuration, remote policy plugin configuration,
credential storage references, asset/cache locations, and service runtime
switches.

## Why This Sprint Exists

Profile payloads now have:

- a formal JSON Schema artifact;
- Rust validation against the committed schema;
- Pydantic v2 admin models;
- JSON entering through Pydantic validation and leaving through Pydantic dump;
- valid/invalid fixtures;
- docs/admin direction.

Service settings have strong Rust structs and TOML validation, but they are not
yet equally consumable by corp admins or `capsem-admin`. That is a release risk:
admins need to validate service settings with the same confidence they validate
profiles, and public tooling must not manipulate raw nested JSON/TOML blobs.

## Product Contract

- Service settings get a formal schema identity:
  `capsem.service-settings.v2`.
- The committed schema artifact is JSON Schema Draft 2020-12:
  `schemas/capsem.service-settings.v2.schema.json`.
- Python admin models use Pydantic v2 end to end. JSON input uses
  `model_validate_json()` or `TypeAdapter.validate_json()`. JSON output uses
  `model_dump_json()`. TOML input may be parsed once, then immediately
  validated into Pydantic models.
- Rust remains the runtime authority for service settings semantics. The schema
  and Pydantic models must be proven equivalent through golden fixtures and
  round-trip tests.
- No legacy settings shapes are accepted. Old defaults-json, v1 policy/config,
  and ad hoc builder settings are not compatibility surfaces.
- `capsem-admin` must support service settings as a first-class admin object,
  not as private helper data.

## Scope

- Add `schemas/capsem.service-settings.v2.schema.json`.
- Add valid and invalid fixtures under `schemas/fixtures/`, including:
  - minimal default-valid settings;
  - full defaults contract fixture shared by Python and Rust;
  - complete corp deployment settings;
  - invalid unknown field;
  - invalid profile catalog source;
  - invalid profile roots;
  - invalid telemetry/export config;
  - invalid remote policy plugin config;
  - invalid credential reference/value shape;
  - invalid asset/cache location shape.
- Add Pydantic v2 service-settings models in the future `capsem-admin` module
  boundary. If S07d lands before the package split, place them where S07b will
  move them without changing public semantics.
- Add admin CLI design/implementation hooks:
  - `capsem-admin settings validate <service.toml|service.json>`;
  - `capsem-admin settings schema`;
  - `capsem-admin settings doctor <service.toml>`;
  - JSON output for CI.
- Add Rust validation tests proving service settings fixtures load through
  `capsem-core::settings_profiles::ServiceSettings` and reject the invalid
  cases with stable error paths.
- Add Python tests proving Pydantic validates/dumps the same fixtures and error
  paths without raw JSON manipulation.
- Add drift tests that fail if the committed schema, Rust defaults, and Pydantic
  defaults disagree.
- Update corp/developer docs to explain what belongs in service settings versus
  what belongs in profiles.

## Explicit Non-Scope

- Do not change Profile V2 payload semantics.
- Do not implement the full `capsem-admin` profile/image/manifest workflow;
  that remains S07b.
- Do not add compatibility for old v1 settings/defaults.
- Do not redesign policy/detection rules here; that belongs to
  [S08a - Rule Abstraction And Detection Architecture](S08a-rule-abstraction-detection-architecture.md).

## Implementation Notes

Existing files to inspect before implementation:

- `crates/capsem-core/src/settings_profiles/mod.rs`
- `crates/capsem-core/src/settings_profiles/tests.rs`
- `src/capsem/builder/schema.py`
- `tests/test_settings_spec.py`
- `config/settings-schema.json`
- `docs/src/content/docs/architecture/settings-schema.md`

The existing `src/capsem/builder/schema.py` and `config/settings-schema.json`
may describe older/generated settings surfaces. S07d must decide explicitly
whether to replace, rename, or retire those artifacts. Do not silently treat
them as the new service-settings schema unless the models actually match the
runtime `ServiceSettings` contract.

## Coverage Ledger

- Unit/contract: Rust service-settings fixture load/reject tests; Python
  Pydantic model validation/dump tests; schema generation/stability tests;
  cross-runtime defaults fixture.
- Functional: `capsem-admin settings validate/schema/doctor` against valid and
  invalid TOML/JSON fixtures; installed console-script smoke for validate,
  schema, and doctor output.
- Adversarial: unknown fields, invalid URLs/paths, missing required catalog
  trust fields, malformed credential references, type mismatches, default drift.
- E2E/VM: not primary; one service-start fixture may be enough if runtime
  settings behavior changes.
- Telemetry/observability: service settings debug report/status must identify
  schema version and validation errors without leaking credentials.
- Performance: schema validation is admin/startup time only; no hot-path budget.

## Done Means

- Service settings have a committed schema artifact and fixtures.
- Rust and Pydantic validate the same shapes with stable errors.
- `capsem-admin` has settings validation/schema/doctor coverage that S07b
  consumes immediately.
- Docs explain service settings versus profiles.
- Tracker/MASTER/S07b are synchronized so admin tooling consumes the formal
  settings contract instead of raw dicts.
