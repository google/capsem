# S07d - Service Settings Schema And Admin Contract

## Status

Not started. Inserted during the 2026-05-19 regroup after S08 exposed that
Profile V2 now has a stronger formal contract than service settings.

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
  Pydantic model validation/dump tests; schema generation/stability tests.
- Functional: `capsem-admin settings validate/schema/doctor` against valid and
  invalid TOML/JSON fixtures.
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
- `capsem-admin` has settings validation/schema/doctor coverage or a committed
  implementation stub with failing tests that S07b consumes immediately.
- Docs explain service settings versus profiles.
- Tracker/MASTER/S07b are synchronized so admin tooling consumes the formal
  settings contract instead of raw dicts.
