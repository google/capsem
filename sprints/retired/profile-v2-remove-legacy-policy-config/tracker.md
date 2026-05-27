# Sprint: Profile V2 Remove Legacy Policy Config

## Tasks

- [x] Create sprint plan and tracker.
- [x] Add no-legacy compile/source guards.
- [x] Move Policy V2 rule/CEL implementation out of `policy_config`.
- [x] Delete `net::policy_config` module from the public tree.
- [x] Replace service/setup uses of legacy settings files with Profile V2 APIs.
- [x] Migrate tests, scripts, install fixtures, support bundle, and docs off v1 config files.
- [x] Update changelog.
- [x] Run focused Rust/Python verification.
- [ ] Commit functional milestone.

## Notes

- User explicitly rejected keeping legacy settings/defaults loading in
  `policy_config`; this sprint removes the surface instead of renaming it.
- `net::policy_config` is no longer public or present on disk; only guard tests
  contain the old token so future runtime imports fail loudly.
- Setup corp provisioning now installs Profile V2 corp profile TOML through
  `settings_profiles::install_corp_profile_toml`.
- Support bundles, uninstall/reinstall tests, install recipe preservation, and
  setup wizard tests now use `service.toml` plus profile roots.
- Legacy v1 config examples/fixtures were removed or rewritten as Profile V2
  profile fixtures.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-core policy_v2_ --lib -- --nocapture`;
  `cargo test -p capsem-process mcp_runtime -- --nocapture`; `cargo test -p
  capsem-core host_config --lib -- --nocapture`.
- Functional: `cargo test -p capsem-service settings -- --nocapture`; `cargo
  test -p capsem setup -- --nocapture`; `cargo test -p capsem support_bundle
  -- --nocapture`; `cargo test -p capsem uninstall -- --nocapture`.
- Adversarial: no-legacy guard tests assert deleted module paths/imports, and
  runtime policy state ignores a planted v1 file while consuming
  `vm-effective-settings.toml`.
- E2E/VM: not run in this milestone; scripts and install fixtures were updated
  but full `just smoke`/`just test` remains the later gate.
- Telemetry: covered by existing Policy V2 MITM/DNS/model tests in the
  `policy_v2_` focused run.
- Performance: not in scope.
- Missing/deferred: full VM smoke/install matrix not run in this turn.
