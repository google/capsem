# Release Hardening Sprint

## Why

The orthogonal release architecture is directionally correct, but publication is
blocked by red cross-platform CI, inconsistent channel-state handling, incomplete
reuse proof, failure-short-circuiting nightly orchestration, causal gaps in update
proof, and no executable failed-deployment rollback contract.

This sprint hardens those rails before another production release. The active
execution ledger is the current worktree's gitignored Sprinty sprint; this file
is the checked-in durable summary required by the development-sprint contract.

## Decisions

- No production publication through a known red required gate.
- Complete candidate qualification must reuse work that actually ran for the
  exact source lineage; reuse is recorded as `carried`, never converted to a
  new `ok`. A valid retained product is a required warm hit, not an optional
  optimization; rebuilding is allowed only after identity/receipt validation
  rejects it or an explicit release rule requires a fresh build. Public release
  dispatch never carries its short publication graph:
  qualification acceptance, remote-main validation, mutable channel fetch,
  immutable source publication, and dispatch are fresh on every attempt.
- Reuse covers journals, asset lanes, install images, and real VM-backed products.
  Every class needs an exact identity plus a live-product receipt revalidated
  before carry.
- VM-image and asset caches use config-owned size, age, and count bounds. Active
  and structurally resumable qualification lineages are protected from eviction.
- Cache corruption, incomplete reclaim, missing receipts, or stale identity causes
  deterministic rebuild or refusal, never a nominal cache hit.
- Cache identity probes normalize only transport formatting; malformed,
  multiline, missing, or extra identity fields still fail closed.
- One typed channel resolver owns published, absent, retired, unreachable, and
  invalid state for release preflight, ordinary CI, and monitoring.
- Nightly still has exactly the two public release commands. A checked-in
  scheduler invokes every selected profile command and then binaries before
  aggregating the verdict.
- Transition proof is causal structured evidence shared by local qualification,
  staging, and release CI; log substrings and elapsed time are not verdicts.
- Deployment validates an immutable preview before activation and restores the
  exact prior distribution when activation verification fails.
- Every fixed-port test fixture is lifecycle-bound to the process that owns its
  exclusion lock, so launcher death cannot leave an unlocked socket owner.
- Broad service decomposition and unrelated feature/performance work are outside
  the release-critical implementation path.

## Ordered Work

1. Make public release dispatch refuse continuation while candidate auto-resume
   selects only a recursively proven exact-source lineage and retained prefix.
2. Make asset identities input-complete, verify every reused output, and
   reconcile the normative local/nightly rebuild rules.
3. Add VM-backed reuse receipts and bounded cache retention with adversarial
   corruption, reclaim, and pressure coverage. Prove that a valid warm VM-image
   hit cannot silently fall through to reconstruction, including against the
   real Docker runtime identity renderer.
4. Restore portable benchmark CI and reliable early-failure evidence.
5. Preserve retained-prefix diagnostics on every pairing failure, bind shared
   mock fixtures to their launchers, eliminate the two-VM IPC timeout at its
   root cause, and repeat the co-work parallel cohort.
6. Unify channel-state resolution and exact CI install-content selection.
7. Make nightly scheduling outcome-complete without nesting gate commands.
8. Extract causal transition transport/evidence from the release-only glow-up
   script and run it through the shared modules.
9. Add preview activation and byte-exact rollback proof.
10. Require green CI, focused stability repetitions, staging fault injection,
   one complete exact-source qualification, stable publication, then nightly.

## Proof Matrix

| Slice | Unit/contract | Functional | Adversarial | E2E/VM / Ironbank | Telemetry/evidence | Performance |
|---|---|---|---|---|---|---|
| Reuse and bounded caches | Resume, identity, GC, receipt tests | Cold/warm gate | Corrupt/stale/missing/pressure cases | Real VM product reused then rebuilt on damage | Journal `carried` lineage and receipt identity | Warm run removes redundant asset/VM construction without weakening steps |
| CI and channel state | Collector and state-enum tests | CI install selection | HTML, 404, retired digest drift, mixed cohort | Exact installed profile pairing | Primary failure artifact | Not applicable |
| Two-VM pairing | MCP lifecycle and state tests | Repeated isolated writes | Process loss, delayed result, later-test teardown | Co-work parallel compatibility cohort | Retained service/process/serial evidence for the exact failed prefix | No unexplained 30-second gap |
| Nightly scheduler | Outcome matrix | All public commands execute | Each lane fails independently | Downstream release job correlation | Structured lane summary | Bounded total scheduler time |
| Transition proof | Evidence-schema tests | Installed update path | Unheard request, unrelated error, tamper, incompatibility | Ironbank installed transition | Exact served/fetched/installed/rejected/preserved identities | Bounded polling deadline |
| Deployment | Distribution/rollback contracts | Preview then activate | Failure at every deployment edge | Public install/channel verification | Public digests and prior-state identity | Propagation attempts remain bounded |

## Done

- Every Sprinty item is resolved with commit-backed gate evidence.
- Main CI and Live Channel Watch have trustworthy green signals for published
  channels.
- The exact committed source has one complete recursively verified qualification
  journal; reused steps retain provenance and every retained VM/cache product is
  current and within policy.
- Stable packages and staged stable profiles activate and pass public install,
  boot, Doctor, Winterfell, Ironbank, update, tamper, and preservation proof.
- The repaired nightly scheduler builds both profile families and binaries, and
  nightly becomes public only as a complete verified graph.
