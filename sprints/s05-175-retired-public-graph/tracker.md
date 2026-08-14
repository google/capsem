# Sprint: S05-175 retired public graph

## Tasks

- [x] Reproduce hosted 404 and record exact public manifest digest.
- [x] Add RED retirement/config/admin/workflow contracts.
- [x] Implement config-owned exact retirement and `capsem-admin` authoring.
- [x] Run focused release, schema, Rust, lint, and Citadel gates.
- [ ] Commit and push the milestone.
- [ ] Qualify the new exact SHA.
- [ ] Re-run stable profile-first hosted release.
- [ ] Close Sprinty item with public evidence.

## Notes

- Run `31835485800` failed before any publication. Public stable remains the
  legacy graph with SHA-256
  `e8ddf88034a3e73beb605811d5efe5e03c04e79d1ba4b656ff6ca837ef54640e`.
- `channels.json` contains stable only; nightly is absent. There is no valid
  donor package cohort, so retirement must author an empty inactive same-channel
  source before profiles stage and the binary lane activates 0.6.

## Coverage ledger

- Unit/contract: final focused release/schema/Citadel cohort 206 passed; the
  broader release/source cohort passed 654; all 144 `capsem-admin` tests and
  all-target admin Clippy passed.
- Functional: the live stable catalog and payload select
  `bootstrap=true, retired=true`; hosted proof follows the new commit.
- Adversarial: malformed/open/duplicate config, catalog/payload digest drift,
  wrong channel, non-retired empty donors, and substituted bytes fail closed.
- E2E/VM: pending new-SHA complete gate.
- IronBank: pending new-SHA complete gate.
- Telemetry: not applicable; no runtime telemetry behavior changes.
- Performance: record complete-gate and hosted workflow durations.
