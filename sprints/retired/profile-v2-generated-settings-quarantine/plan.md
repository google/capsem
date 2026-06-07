# Profile V2 Generated Settings Quarantine

## Goal

Make `config/defaults.json` unambiguously a generated builder/frontend mock
artifact, not a runtime settings authority. The runtime Profile V2 source of
truth remains `settings_profiles` plus per-VM `vm-effective-settings.toml`.

## Scope

- Add source guard tests that fail when docs/comments reintroduce the old
  `defaults.json`/`policy_config` authority model.
- Assert Rust runtime crates do not read or embed generated frontend settings
  artifacts.
- Update stale comments in the Python generator, frontend types, generated mock
  headers, and MITM hooks.
- Leave the generated file in git for frontend fixture determinism; this sprint
  quarantines its meaning rather than changing the build pipeline.

## Done

- Guard tests cover the builder/frontend labels and Rust runtime artifact usage.
- Focused Python tests pass.
- Generated mock output is refreshed if generator headers change.
- Existing Profile V2 rescue tracker records the extra hygiene pass.

## Testing Proof Matrix

- Unit/contract: Python guard tests for generated settings authority.
- Functional: `_generate-settings` or equivalent stale-file test proves generated
  mock headers remain reproducible.
- Adversarial: runtime source scan rejects `defaults.json`/`settings-schema.json`
  consumption in Rust crates.
- E2E/VM: not required; no runtime behavior change.
- Telemetry: not touched.
- Performance: not touched.
