# Sprint: Release Hardening

## Tasks

- [x] S06-001 — keep publication fresh while resuming qualification
- [x] S06-002 — verify every reused asset lane and reconcile contracts
- [ ] S06-003 — reuse and bound VM image products and caches
- [ ] S06-004 — normalize Docker runtime cache identity
- [x] S02-001 — restore deterministic cross-platform CI
- [ ] S02-002 — unify channel state and exact install inputs
- [ ] S02-003 — eliminate the two-VM IPC qualification timeout and preserve evidence
- [ ] S03-001 — run every nightly lane before the aggregate verdict
- [ ] S04-001 — produce causal installed transition evidence
- [ ] S05-001 — prove atomic channel activation and rollback
- [ ] S05-002 — qualify and verify stable, then nightly
- [ ] Changelog entries at user-visible milestones
- [ ] Exact-source complete qualification

## Notes

- Existing branch commits `59f8dc09` and `fa4cad4d` are user-owned starting
  work. Audit and extend them; do not rewrite or discard them.
- Sprinty S01 was deprecated before implementation because its immutable gates
  named two nonexistent test paths and combined three independent reuse risks.
  S06 is its corrected replacement; no work or evidence was lost.
- Explicit `--from` on a public release used to carry publication prerequisites
  from graph shape alone. Fixed in `023e43ac`: public release
  dispatch rejects it; only recursively verified candidate qualification may
  auto-resume.
- The local/nightly asset-rebuild contract was reconciled in `d7241933`: local
  construction must reuse exact identity-and-receipt hits while downstream
  assembly and release proof still run; nightly profile publication rebuilds.
- S06-003 treats a valid warm VM-image hit as mandatory. Its proof must detect a
  fallback rebuild, not merely accept either reuse or reconstruction.
- S06-003 also keeps the install receipt in every retained prefix while copying
  it into the shared warm cache. Moving that authority away would let Docker
  reclamation erase the exact source/helper images a resumable journal needs.
- The first real cold install-image proof reached construction, then exposed a
  Docker formatting assumption: its Go-template renderer aligned the requested
  tab-separated runtime fields with spaces. S06-004 normalizes one exact
  single-line three-field identity before hashing and rejects ambiguous shapes;
  the cold/warm S06-003 proof must be repeated after that fix.
- S02-001 removes the Linux-only collector paths that failed macOS ARM CI after
  nextest had run 728 of 1,644 tests. All 15 collector tests now launch through
  `PATH`, so the first failure cannot hide incompatible `true`, `sleep`, or
  `echo` assumptions.
- Install CI keeps missing evidence fatal when the install gate itself fails,
  but an input-resolution failure before that step now remains the primary
  error instead of acquiring a second `upload-artifact` failure.
- The S02 gate exposed one S06 boundary regression before release: the storage
  controller's new shared retention import was unavailable under its deliberate
  `uv --no-project` entrypoint. The controller now resolves checked-in shared
  code explicitly, with a direct standalone regression test, and the script
  debt ratchet dropped from 1,539 to 1,538 lines.
- Stable binary run `32640139313` built, signed, notarized, and installed its
  packages, then failed closed in the pairing gate after 339 compatibility
  tests: the first file write in `test_two_vms_isolated` timed out at the
  service IPC boundary after both VMs reported exec-ready. The workflow then
  searched only workspace evidence paths even though the gate had retained its
  exact qualification prefix, so it uploaded none. S02-003 owns evidence
  routing, root-cause repair, and repeated co-work parallel proof.
- Current install CI directly reads the explicitly retired stable graph.
- Nightly scheduler currently short-circuits after the first failed profile.
- Asset reuse initially never hit because preflight discarded
  `target/ironbank-assets`, and a matching identity checked only existence.
  Preflight now preserves isolated lane roots and exact byte receipts are
  required at build and packed-initrd boundaries.

## Coverage Ledger

- Unit/contract: S06 reuse/cache cohort plus install, asset, prefix, storage,
  config, and Citadel contracts green (484 passed, 2 platform skips); S02 CI
  contracts green (15 Rust collector tests, 132 install/evidence tests, and 48
  storage/Citadel tests)
- Functional: mandatory zero-construction warm hit and prefix salvage/lend path
  green through production functions; the real cold run exposed S06-004 before
  smoke, so cold/warm proof remains pending
- Adversarial: public continuation; mutated/partial/extra/escaping asset output;
  corrupt, stale, non-finite, symlinked, over-bound, and partial-reclaim state green
- E2E/VM: stable pairing reached the two-VM cohort and failed closed at the
  30-second IPC boundary; root cause and repetition are S02-003
- Ironbank: pending
- Telemetry/evidence: the failed pairing prefix was retained, but the workflow
  did not export or upload it; S02-003 must make failure evidence unavoidable
- Performance: zero redundant construction asserted; wall-clock cold/warm proof pending
- Missing/deferred: physical macOS and public Cloudflare boundaries remain final
  owning gates; broad service-main decomposition is outside this sprint.
