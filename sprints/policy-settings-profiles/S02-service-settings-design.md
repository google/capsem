# S02 - Service Settings Design

## Goal

Design the typed service settings model before coding it.

## Decisions To Present For Review

- Rust struct layout.
- TOML file layout.
- Defaults.
- Validators and error messages.
- Credential storage fields.
- Profile root settings.
- Asset, manifest, and custom image location settings.
- Observability plugin settings.
- Remote policy plugin settings.
- UI descriptor strategy.

## Done

Closed. User has reviewed and approved the service settings shape as the design
baseline for implementation.

## Closed Design

The Rust-owned shape lives in `capsem-core::settings_profiles`:

- `ServiceSettings`
- `AppSettings`
- `ProfileRootSettings`
- `CredentialSettings`
- `AssetLocationSettings`
- `ManifestLocationSettings`
- `ManifestSource`
- `TelemetrySettings`
- `RemotePolicySettings`
- `service_setting_descriptors()`

Design decisions closed:

- Service settings are service/app scoped, not profile scoped.
- Profile roots, corp/user governance toggles, telemetry endpoint, remote policy
  endpoint, credential storage, manifest source, asset directory, image roots,
  and asset download endpoint are service settings.
- Manifest source supports installed defaults, local file, and remote URL.
- TOML credentials are acceptable for cutover; keychain remains stretch work in
  credential brokerage.
- UI descriptors are Rust-owned and generated/exposed from the typed model, not
  handwritten `config/defaults.json`.
- Debug/status must expose configured manifest/image/asset locations and
  telemetry/remote policy endpoints, with secrets redacted.

Implementation/open follow-ups move to later sprints:

- S03: finish loading, validation, and service integration.
- S06: resolve corp/general inheritance and precedence into VM-effective state.
- S11: keep status/debug provenance complete as runtime wiring lands.
- S12/S13: specify final telemetry and remote policy failure semantics.
- S15: harden generated descriptors for the final settings UI.

## Coverage Ledger

- Unit/contract: design review checklist closed.
- Functional: example TOML snippets and typed field list cover expected service
  settings.
- Adversarial: malformed and invalid examples are covered in S03 model tests.
- E2E/VM: not applicable.
- Telemetry: observability/remote policy fields covered.
- Performance: not applicable.
