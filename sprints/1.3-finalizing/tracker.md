# Sprint: 1.3 Finalizing

## Status

Closed on branch `release/1.3-cleanup-pr-v2`.

The original broad checklist was superseded by the focused
`snapshot-restore/` execution sprint after we discovered that the cleanup
snapshot had accidentally dropped real profile/admin/TUI/Linux/benchmark work.
The detailed implementation and proof ledger now lives in:

- `snapshot-restore/MASTER.md`
- `snapshot-restore/tracker.md`
- `snapshot-restore/S0-loss-inventory.md`

## Closure Checklist

- [x] Snapshot restore S0-S6 completed and committed.
- [x] Parent sprint reconciled to snapshot restore outcomes.
- [x] Old policy-v2/domain/MCP decision rails remain burned.
- [x] Old setup/provider onboarding and settings-owned credential/provider
  rails remain burned.
- [x] Profile-first configuration contract is restored: VMs execute immutable
  profile ids; profiles own assets, rules, detection, MCP, plugins, defaults,
  availability, identity, and VM behavior.
- [x] Settings are UI/application preferences only.
- [x] Corp config owns constraints, reporting, and negative-priority rules over
  profiles.
- [x] Service/gateway route contract is explicit and profile-addressed for
  authoring routes; retired and fallback routes fail closed.
- [x] Security decisions run through typed `SecurityEvent` +
  `SecurityRuleSet`/CEL.
- [x] Default rules are visible real rules in the same rule set, not a second
  engine.
- [x] Plugin behavior is plugin-owned runtime/config behavior, not rule-invoked
  hidden policy.
- [x] Credential brokerage is opaque plugin/runtime evidence with BLAKE3
  references; raw host credential injection/settings writeback remains burned.
- [x] `capsem-admin` typed profile/asset/manifest/rule validation rail is
  restored.
- [x] Profile-derived EROFS/LZ4HC asset build/verify/materialize rail is
  restored.
- [x] `capsem shell`/TUI restore is complete for the current route/profile
  contract.
- [x] Local deterministic HTTP/MCP/model/DNS benchmark and release proof
  fixtures replaced public-service dependencies.
- [x] Current benchmark evidence is recorded in docs and the snapshot tracker.
- [x] Current docs, skills, and changelog describe implemented 1.3 behavior
  only.
- [x] Full local smoke passed.
- [x] Package/install build handoff passed: `just install` built
  `packages/Capsem-1.0.1780977620.pkg`; macOS GUI installer click-through is
  human-driven.
- [x] Branch pushed to `origin/release/1.3-cleanup-pr-v2`.

## Verification Ledger

- Unit/contract: current S6 proof includes `cargo test -p capsem-core
  net::policy_config:: -- --nocapture` with 375 passing tests, plus focused
  profile/security/default/plugin/config tests recorded in
  `snapshot-restore/tracker.md`.
- Functional API: route conformance and service/gateway tests are recorded in
  T1/S6 evidence; explicit-route and body-limit tests use real routes.
- Adversarial: retired route/old policy/settings/provider/credential rails are
  covered by old-rail regression tests and `test_security_rails_retired.py`.
- E2E/VM: `just smoke` booted the profile-selected EROFS/LZ4HC VM, ran doctor,
  integration, injection, state transition, and resume-path suites.
- Session DB/ledger: integration proof records denied network events, DB
  rollups, JSONL process log validity, and snapshot rows through accepted
  runtime paths.
- Frontend/TUI: `pnpm -C frontend check` passed; `cargo test -p capsem-tui`
  passed with 54 tests; TUI clippy passed.
- Performance: S4/S5 benchmark gates record EROFS/storage, DB writer, local
  MITM, DNS, MCP, security-action, plugin, and CEL/security-event latency.
- Install/package: `just install` built the real macOS package and handed off
  to the GUI Installer after package assembly.
- Final checks: `cargo fmt --check`, `git diff --check`, and targeted
  `cargo check -p capsem-admin -p capsem-core -p capsem-service
  -p capsem-gateway -p capsem-tui` passed after S6.

## Accepted Handoff

- Linux runtime KVM/DAX execution is an explicit Linux-team/CI handoff. The
  Linux-team KVM/filesystem/EROFS/LZ4HC work is restored and respected, but the
  local macOS environment cannot execute the Linux runtime validation lane.

## Commits

- `0e414b08 bench: close security corpus gates`
- `8d635399 chore: close 1.3 verification gate`
