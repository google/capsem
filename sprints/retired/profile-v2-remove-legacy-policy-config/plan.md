# Profile V2 Remove Legacy Policy Config

## Goal

Delete the legacy `net::policy_config` surface instead of leaving it as a
settings/defaults compatibility island. Runtime and tests should depend on
Profile V2 settings/profile APIs and `net::policy_v2` only.

## Scope

- Move Policy V2 rule/CEL code out of `net::policy_config` into
  `net::policy_v2`.
- Remove the public `net::policy_config` module.
- Replace remaining runtime use of `load_settings_files`, `SettingsFile`,
  `MergedPolicies`, presets, and `user.toml`/`corp.toml` paths.
- Update setup/service/config tests to assert Profile V2 artifacts instead of
  legacy `user.toml`/`corp.toml` behavior.
- Add guards so new runtime code cannot depend on `policy_config`.

## Decisions

- No compatibility re-export for `policy_config`. If a call site still imports
  it, compilation or a guard must fail.
- Keep Policy V2 TOML rule parsing only if it is owned by `policy_v2` or
  Profile V2 profile parsing. No v1 settings-file wrapper should be required.
- Profile/corp setup work should use `settings_profiles` corp profile APIs,
  not legacy corp settings TOML.

## Files

- `crates/capsem-core/src/net/policy_v2/*`
- `crates/capsem-core/src/net/mod.rs`
- `crates/capsem-core/src/settings_profiles/*`
- `crates/capsem-service/src/main.rs`
- `crates/capsem-process/src/mcp_runtime.rs` tests
- `crates/capsem/src/setup.rs`, install/setup tests as needed
- `tests/capsem-*`
- `CHANGELOG.md`

## Done

- `rg policy_config crates/capsem-core/src crates/capsem-process/src crates/capsem-service/src crates/capsem/src tests` has no live code imports.
- `crates/capsem-core/src/net/policy_config/` is deleted or reduced to
  deleted-history only.
- Runtime settings/profile resolution uses Profile V2 only.
- Focused Rust/Python tests pass, and any full VM/install gaps are explicit.

## Testing Proof Matrix

- Unit/contract: Policy V2 parser/CEL tests under `net::policy_v2`, settings
  profile resolver tests, no-legacy guard tests.
- Functional: service settings/debug/update tests through Profile V2 endpoints.
- Adversarial: no fallback to legacy files when effective profile attachments
  are missing or legacy TOML exists.
- E2E/VM: full smoke/install gate after compile-focused removal is green.
- Telemetry: policy denial tests continue to assert Policy V2 fields.
- Performance: no expected hot-path behavior change; no benchmark planned.
