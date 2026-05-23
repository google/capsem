# S11 - Status, Debug, Provenance

## Goal

Make wrong settings, profile resolution, profile catalog state, and VM asset
binding visible.

This is release-blocking for the Profile V2 bedrock. Operators must be able to
answer "what profile/rule/engine decision caused this?" through supported
status/debug/log surfaces, not by inspecting raw SQLite tables or test-only
fixtures. Full S12 OpenTelemetry export can follow, but shipped status/debug
truth cannot lie or omit the core profile/security provenance.

## Tasks

- Harden `capsem status`.
- Harden `capsem logs` and debug/report projections for canonical
  resolved-event identity, event family, profile id/revision, VM id, user id,
  rule id/pack id, final action, detection finding ids, and engine provenance.
- [x] Add debug report sections for service settings, profile roots, selected
  profiles, VM-effective settings, derived rules, locks, MCP/tools/skills, and
  policy assembly.
- [x] Add "why is this here?" explanations for effective values and generated rules.
- [ ] Add generated-rule ownership details in status/debug
      (`owner_setting_path`, `owner_setting_label`, editable/managed state).
- [ ] Add manifest profile catalog status: profile ids, installed revisions,
      current catalog revision, lifecycle status, binary compatibility, and
      payload verification state. Lifecycle status uses the canonical
      `ProfileRevisionStatus` enum (`active`, `deprecated`, `revoked`) and
      renders the exact enum value plus user-facing explanation. There is no
      `removed` status; absent revisions are reported as absent/unknown.
- [ ] Add selected/resolved package/tool contract and VM asset readiness.
- [ ] Add persistent VM pin rendering: profile id/revision, package contract
      hash, pinned asset hashes, and drift/deprecated/revoked warnings.
- [ ] Add VM live health rendering sourced from S12 typed metrics snapshots:
      model call count, providers, models, token totals, estimated cost,
      detection finding counts/severity, ask/policy counters, and stale/partial
      metrics state. Running VM status must use the live accumulator; stopped
      VM detail may use the one-shot cold `session.db` fallback.
- [ ] Add chain-of-trust rendering for profile-backed VMs: manifest identity,
      profile payload verification, package/tool contract, asset verification,
      and VM pin status.
- [ ] Add forward-only pin rendering so invalid registry entries missing a
      profile pin or pinned asset identity fail closed and are reported as
      invalid state, never as a compatible legacy VM.
- Test status/debug against active service and VM-effective state.

## Implemented Slice

- `SettingsProfilesDebugSnapshot` summarizes service settings without secret
  credential values.
- `/debug/report` now includes `[settings_profiles]` with service defaults,
  profile roots, manifest source/path/URL, asset directory, image roots, asset
  download endpoint, telemetry endpoint, remote decision endpoint, credential
  IDs, profiles and lock/source state, selected/effective profile, VM settings,
  MCP server IDs, skills state, and derived/raw rule counts.
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
  profile, stale manifest rollback, interrupted download, unauthorized signing
  key, invalid VM pin, and missing asset diagnostics must be covered.
- E2E/VM: debug report explains launched session profile revision and pinned
  verified assets.
- Telemetry: report includes audit-relevant settings/profile/rule summaries at
  model level plus profile catalog/update, VM pin state, and S12 live health
  summaries for model/provider/cost/detection counters.
- Performance: report generation remains bounded.
