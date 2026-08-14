# Sprint: S05-175 retired public graph

## Tasks

- [x] Reproduce hosted 404 and record exact public manifest digest.
- [x] Add RED retirement/config/admin/workflow contracts.
- [x] Implement config-owned exact retirement and `capsem-admin` authoring.
- [x] Run focused release, schema, Rust, lint, and Citadel gates.
- [x] Commit and push the milestone.
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
- Exact-SHA qualification `2be5c614bef3fd1349672814098bae4feb9c5b82`
  stopped in the fast release-contract step after 12m14s: 4,539 passed, 38
  skipped, with only duplicated workspace-version authority and the 300-line
  gate-module ceiling red. No artifact or VM work ran; its full-SHA prefix and
  journal remain as failure evidence.
- Exact-SHA qualification `ba9e862659612d8133addca3408b37381604ab76`
  passed fast/source, dual-architecture assets and boots, 4,385 Rust tests,
  5,624 broad Python tests, every functional cohort, and both Debian package
  builds. It stopped only at install image smoke after concurrent Rust coverage
  transiently gave build and smoke different source-derived tags. S05-176 owns
  the frozen-source/receipt fix; the retained prefix and run journal remain.

## Coverage ledger

- Unit/contract: final focused release/schema/Citadel cohort 206 passed; the
  broader release/source cohort passed 654; all 144 `capsem-admin` tests and
  all-target admin Clippy passed.
- Follow-up owning boundary/source cohort: 1,185 passed, 2 platform skips;
  `buildschema.py` is 275 lines and strict all-platform Ty remains green.
- Functional: the live stable catalog and payload select
  `bootstrap=true, retired=true`; hosted proof follows the new commit.
- Adversarial: malformed/open/duplicate config, catalog/payload digest drift,
  wrong channel, non-retired empty donors, and substituted bytes fail closed.
- E2E/VM: pending new-SHA complete gate.
- IronBank: pending new-SHA complete gate.
- Telemetry: not applicable; no runtime telemetry behavior changes.
- Performance: record complete-gate and hosted workflow durations.
