# S11 - Status, Debug, Provenance

## Goal

Make wrong settings, profile resolution, profile catalog state, and VM asset
binding visible.

## Tasks

- Harden `capsem status`.
- [x] Add debug report sections for service settings, profile roots, selected
  profiles, VM-effective settings, derived rules, locks, MCP/tools/skills, and
  policy assembly.
- [x] Add "why is this here?" explanations for effective values and generated rules.
- [ ] Add generated-rule ownership details in status/debug
      (`owner_setting_path`, `owner_setting_label`, editable/managed state).
- [ ] Add manifest profile catalog status: profile ids, installed revisions,
      current catalog revision, lifecycle status, binary compatibility, and
      payload verification state.
- [ ] Add selected/resolved package/tool contract and VM asset readiness.
- [ ] Add persistent VM pin rendering: profile id/revision, package contract
      hash, pinned asset hashes, and drift/deprecated/revoked warnings.
- Test status/debug against active service and VM-effective state.

## Implemented Slice

- `SettingsProfilesDebugSnapshot` summarizes service settings without secret
  credential values.
- `/debug/report` now includes `[settings_profiles]` with service defaults,
  profile roots, manifest source/path/URL, asset directory, image roots, asset
  download endpoint, telemetry endpoint, remote decision endpoint, credential
  IDs, profiles and lock/source state, selected/effective profile, VM settings,
  MCP connector IDs, skills state, and derived/raw rule counts.
- If settings/profile resolution fails, the debug report records the load error
  instead of silently dropping the section.

Focused test command:

```sh
cargo test -p capsem-service debug_report::tests
```

Result: 5 debug report tests passed, including credential redaction and
settings/profile load-error rendering.

## Coverage Ledger

- Unit/contract: provenance summary rendering is partially covered; catalog,
  package/asset readiness, and VM pin rendering must add focused shape tests.
- Functional: debug report rendering tests are present.
- Adversarial: credential value redaction is covered; missing profile roots, bad
  profile load errors are covered; missing profile roots and locked setting
  rendering remain; generated-rule ownership rendering is pending; revoked
  profile and missing asset diagnostics must be covered.
- E2E/VM: debug report explains launched session profile revision and pinned
  verified assets.
- Telemetry: report includes audit-relevant settings/profile/rule summaries at
  model level plus profile catalog/update and VM pin state.
- Performance: report generation remains bounded.
