# Sprint: Release Hardening

## Tasks

- [x] S06-001 — keep publication fresh while resuming qualification
- [x] S06-002 — verify every reused asset lane and reconcile contracts
- [ ] S06-003 — reuse and bound VM image products and caches
- [x] S02-001 — restore deterministic cross-platform CI
- [ ] S02-002 — unify channel state and exact install inputs
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
  green through production functions; real gate cold/warm run pending
- Adversarial: public continuation; mutated/partial/extra/escaping asset output;
  corrupt, stale, non-finite, symlinked, over-bound, and partial-reclaim state green
- E2E/VM: queued behind the pre-existing stable binary release machine lock
- Ironbank: pending
- Telemetry/evidence: pending
- Performance: zero redundant construction asserted; wall-clock cold/warm proof pending
- Missing/deferred: physical macOS and public Cloudflare boundaries remain final
  owning gates; broad service-main decomposition is outside this sprint.
