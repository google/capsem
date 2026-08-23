# Sprint: Release Hardening

## Tasks

- [ ] S06-001 — keep publication fresh while resuming qualification
- [ ] S06-002 — verify every reused asset lane and reconcile contracts
- [ ] S06-003 — reuse and bound VM image products and caches
- [ ] S02-001 — restore deterministic cross-platform CI
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
- Explicit `--from` on a public release currently carries publication
  prerequisites from graph shape alone. Public release dispatch must reject it;
  only recursively verified candidate qualification may auto-resume.
- The normative release spec still says local assets always rebuild, while the
  new implementation carries identity-verified asset work. The contract must be
  reconciled before the reuse milestone can close.
- Current macOS CI failure is the Linux-only absolute collector command paths.
- Current install CI directly reads the explicitly retired stable graph.
- Nightly scheduler currently short-circuits after the first failed profile.

## Coverage Ledger

- Unit/contract: pending
- Functional: pending
- Adversarial: pending
- E2E/VM: pending
- Ironbank: pending
- Telemetry/evidence: pending
- Performance: pending
- Missing/deferred: physical macOS and public Cloudflare boundaries remain final
  owning gates; broad service-main decomposition is outside this sprint.
