# Sprint: Release Hardening

## Tasks

- [x] S06-001 — keep publication fresh while resuming qualification
- [x] S06-002 — verify every reused asset lane and reconcile contracts
- [x] S06-003 — reuse and bound VM image products and caches
- [x] S06-004 — normalize Docker runtime cache identity
- [x] S02-001 — restore deterministic cross-platform CI
- [x] S02-002 — unify channel state and exact install inputs
- [x] S02-003 — eliminate the two-VM IPC qualification timeout and preserve evidence
- [x] S02-004 — bind every fixed-port mock server to its launcher lifecycle
- [x] S03-001 — run every nightly lane before the aggregate verdict
- [x] S04-001 — produce causal installed transition evidence
- [x] S05-001 — prove atomic channel activation and rollback
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
- The corrected cold run `20260823-150531-51b2e9-install-image` rebuilt and
  smoked exact child `sha256:bf6dec688d2afcb09871517ded2b1d166add3a670d3f27a71b0fde2921e27031`;
  the unchanged warm run `20260823-150806-92dd22-install-image` required and
  reused that same child in 35.8 seconds with no reconstruction.
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
- The exact four-worker co-work reproduction passed `test_two_vms_isolated`
  but found a separate deterministic lifecycle defect: a two-day-old orphaned
  `capsem-mock-server` retained port 3713 after its Python launcher died. The
  launcher-owned flock had already been released, so all later locked users
  acquired exclusion and then failed to bind. S02-004 binds the socket owner
  to both Python and Rust Doctor launchers with `capsem-guard`.
- S02-003 fixed both evidence-loss boundaries: session-scoped fixtures now
  preserve when any test on their worker failed, and the private qualification
  prefix exports `test-artifacts/` beside its gate journal before GitHub uploads
  them. The successful guest-write path no longer waits for a full session-DB
  flush after the ledger accepts its event, and a remaining failure names the
  VM and guest-completion stage. The gate-generated four-worker co-work cohort
  then passed all 340 tests with 52 expected skips in 8m54s, including the
  exact two-VM isolation case under load.
- The IPC milestone's final Citadel reconciliation also advanced the exact
  oversized-source debt ratchets for the service shell and its sibling tests;
  the source-shape guard is green at the measured post-fix line counts.
- Install CI no longer reads the explicitly retired stable graph. One typed
  resolver now verifies catalog authority and manifest bytes, then chooses the
  public URL only for a published graph or GitHub's latest immutable serialized
  channel source for a retired graph. Live Channel Watch validates published
  and retired catalog members, skips absent nightly, and fails closed on HTML,
  digest drift, malformed graphs, transport loss, or broken references.
- The live resolver classifies stable as retired at configured digest
  `e8ddf88034a3e73beb605811d5efe5e03c04e79d1ba4b656ff6ca837ef54640e`
  and nightly as absent. The retired CI path resolves immutable GitHub asset
  `517678454`: stable/current, both `code` and `co-work` 0.6.0 profiles for
  arm64 and x86_64, and no package cohort before the local binary build.
- Nightly orchestration now invokes one checked-in scheduler for the frozen
  commit. It runs `code`, `co-work`, then binaries serially through the public
  release commands, records structured start/completion and final aggregate
  events, and returns failure only after every lane has an outcome. The
  workflow's six-hour job timeout is the outer time bound; release commands
  retain ownership of their gate journals, channel locks, waits, and teardown.
- Asset reuse initially never hit because preflight discarded
  `target/ironbank-assets`, and a matching identity checked only existence.
  Preflight now preserves isolated lane roots and exact byte receipts are
  required at build and packed-initrd boundaries.
- Transition fixtures previously inherited HTTP conditional caching, so a
  same-second manifest promotion could return stale bytes. One root-confined
  handler now removes conditional validators, sends `Cache-Control: no-store`,
  and is shared by Linux and Tart.
- The updater now records exact candidate fetch, activation, and rejection
  events with previous/current installed-manifest identities and causal errors.
  The verifier ignores pre-marker rows and refuses unrelated errors, wrong
  digests, reordered events, cause confusion, or a rejection that changed the
  installed state.
- The macOS rail originally stopped after fresh activation and tamper rejection.
  It now activates a distinct valid profile-metadata graph through launchd,
  rejects both a corrupt artifact and a profile requiring Capsem 9999.0.0, and
  re-proves the same package, profile tree, manifest, metadata, service, and
  binary cohort. The three oversized owning scripts shrank from 398/610/341 to
  393/587/317 lines while the shared transition modules remain below the
  300-line ceiling.
- Channel publication now captures the exact canonical production deployment
  and prior public byte graph, validates every deploy-root public file on an
  immutable preview, and binds activation to both that snapshot and the
  Cloudflare action's deployment ID. Any attempted activation without a
  successful exact-byte verdict retries restoration of the prior deployment
  and revalidates the prior catalog/member graph; a failed preview never
  touches production. The reusable staging caller explicitly disables
  production activation.
- Exact qualification of `f57ff8e0` stopped before artifact and VM work after
  5,143 release contracts passed and two VM-cache tests inherited the gate's
  real `CAPSEM_GATE_SOURCE_CHECKOUT`. Those tests now isolate their temporary
  receipt lineage, while a separate positive regression proves that an active
  source-checkout receipt still pins its exact source and helper images. No
  release was attempted from the failed journal.

## Coverage Ledger

- Unit/contract: S06 reuse/cache cohort plus install, asset, prefix, storage,
  config, and Citadel contracts green (484 passed, 2 platform skips); S02 CI
  contracts green (15 Rust collector tests, 132 install/evidence tests, and 48
  storage/Citadel tests); typed channel/install/watch gates are green (92
  direct tests, 413 affected release contracts, and 55 source/Citadel guards);
  nightly scheduling is green across all eight lane failure combinations,
  launch failure, input rejection, and workflow enforcement (108 direct tests,
  79 source/Citadel guards, and 14 binary-release script contracts)
- Functional: mandatory zero-construction warm hit and prefix salvage/lend path
  green through production functions; real cold/warm image reuse is green on
  the identical exact child with a 2m30s cold and 35.8s warm run
- Adversarial: public continuation; mutated/partial/extra/escaping asset output;
  corrupt, stale, non-finite, symlinked, over-bound, and partial-reclaim state green
- E2E/VM: exact four-worker co-work compatibility is green (340 passed, 52
  skipped); two-VM writes complete without a 30-second IPC gap
- Ironbank: every IronBank test selected by the co-work compatibility cohort is
  green, including Doctor, MCP, model/client, credential, HTTP, DNS, package,
  file/process/snapshot, plugin, and profile-mutation ledgers; the release,
  update, and integrity slice is green (10 passed, 75 deselected)
- Telemetry/evidence: worker-wide failure detection, private-prefix export, and
  fatal workflow upload contracts make pairing evidence available outside the
  runner; focused preservation and release contracts are green; exact update
  audit events correlate served, fetched, activated/rejected, and preserved
  manifest identities across Linux and macOS adapters
- Deployment: preview/production snapshots are location-independent but
  byte-exact; the failure matrix distinguishes skipped activation from every
  upload/validation failure, while API errors, wrong canonical IDs, incomplete
  restoration, changed bodies, extra/missing paths, and transient rollback
  requests fail closed
- Performance: zero redundant construction asserted and observed; the warm
  install-image edge fell from 1m54s construction to a 1.2s validated hit
- Missing/deferred: the macOS transition implementation and host-side contracts
  are green on Linux, while execution on physical Apple Silicon and public
  stable/nightly activation remain owned by S05-002's exact release gate.
  Broad service-main decomposition is outside this sprint.
