# T10: Focused Verification

## Objective

Run the smallest meaningful proof for every fixed release-blocking track before
the full `just test` gate. This catches targeted failures while they are still
cheap to diagnose.

## Owned Files

- `sprints/release-policy-hardening/tracker.md`
- `sprints/release-policy-hardening/plan.md`
- `justfile`
- `.github/workflows/release.yaml`
- `tests/test_repack_deb.py`
- `tests/test_release_workflow.py`
- `tests/test_docker.py`
- `tests/test_gen_manifest.py`
- `tests/capsem-build-chain/*`
- `tests/capsem-install/*`
- `tests/capsem-session-lifecycle/*`
- `tests/capsem-session/*`
- `tests/capsem-session-exhaustive/*`
- `frontend/package.json`
- `frontend/vitest.config.ts`

## Findings

- [P0] Focused verification existed as a tracker row but had no owner doc.
- [P1] Several track docs require newly named tests that should be run before
  the full suite.
- [P1] Package payload and manifest checks must be explicit, not hidden inside
  post-release verification.
- [P2] Frontend visual verification requires browser/console evidence in
  addition to unit tests.

## Task List

### T10.1 Package and Install Proof

- [ ] Run `.pkg` payload expansion and manifest signature checks from T0.
- [ ] Run `.deb` payload checks for manifest files and MCP helpers.
- [ ] Run fresh Linux `.deb` install from empty `CAPSEM_HOME`.
- [ ] Run clean macOS `.pkg` install proof or record manual blocker evidence.

### T10.2 Asset and Manifest Proof

- [ ] Run manifest producer tests from T1.
- [ ] Run release workflow metadata preservation test.
- [ ] Run rootfs validation tests after T1/T5 changes.
- [ ] Run asset cleanup tests after `cleanup_unused_assets()` changes.

### T10.3 Frontend Policy Proof

- [ ] Run Svelte/TypeScript checks.
- [ ] Run model/store/export/component tests.
- [ ] Run coverage after policy UI tests land.
- [ ] Run Gate A with Elie: `just dev-frontend`, browser JS UI policy flow,
  screenshots/evidence, and zero console errors/warnings.
- [ ] Run `just ui` and Chrome DevTools MCP visual/console proof.
- [ ] Verify T2.8 runtime/image truth states or prove unsupported surfaces are
  hidden.

### T10.4 Runtime Security Proof

- [ ] Run policy hook tests.
- [ ] Run policy hook spec tests.
- [ ] Run MCP frame tests.
- [ ] Run policy benchmark smoke with matching-rule assertions.

### T10.5 Service, Telemetry, and Integration Proof

- [ ] Run service reload/cleanup/route tests.
- [ ] Run gateway route/auth tests.
- [ ] Run logger migration tests.
- [ ] Run session lifecycle/session/session-exhaustive tests.
- [ ] Run T8 production-path policy E2E or hidden-hook UI proof.
- [ ] Run Gate B with Elie: `just exec "echo cli-ok"`,
  `just exec "capsem-doctor"`, and the chosen T8 policy E2E/session proof.

### T10.6 Docs and Metadata Proof

- [ ] Run stale-term searches.
- [ ] Run docs and site builds.
- [ ] Run version synchronization checks from T9.

### T10.7 Evidence Capture

- [ ] Record command, pass/fail status, and follow-up owner in `tracker.md`.
- [ ] Do not advance to T11 until every P0/P1 focused proof passes or has an
  explicit release-blocking owner.

### T10.8 Evidence Ledger

- [ ] Add an evidence ledger section to `tracker.md` before running Gate A/B.
- [ ] For every focused proof, record command, expected proof, pass/fail,
  artifact/log/screenshot path, owner, and release-blocking follow-up.
- [ ] Store screenshots, console logs, package inspection output, and CLI
  transcripts as durable local files referenced by path from `tracker.md`.
- [ ] Do not rely on chat history as release evidence.

## Proof Matrix

| Category | Required proof |
|---|---|
| Unit/contract | each touched Rust/Python/frontend logic boundary has targeted tests. |
| Functional | package payloads, settings save, reload, timeline, and docs builds pass. |
| Adversarial | tampered manifests, malformed imports, hook bypass, env leaks, old DBs. |
| E2E/VM | install proof, policy E2E, Gate A JS UI, Gate B CLI/VM, visual Settings flow. |
| Telemetry | session DB/timeline checks prove policy visibility. |
| Performance | policy benchmark smoke and existing benchmark gates. |

## Verification

- [ ] `git diff --check`
- [ ] `cargo test -p capsem-core asset_manager -- --nocapture`
- [ ] `cargo test -p capsem-core policy_hook -- --nocapture`
- [ ] `cargo test -p capsem-core policy_hook_spec -- --nocapture`
- [ ] `cargo test -p capsem-core mcp_frame -- --nocapture`
- [ ] `cargo test -p capsem-service policy_hook -- --nocapture`
- [ ] `cargo test -p capsem-service cleanup -- --nocapture`
- [ ] `cargo test -p capsem-gateway all_non_root_paths_require_auth -- --nocapture`
- [ ] `cargo test -p capsem-app`
- [ ] `cargo test -p capsem-logger`
- [ ] `uv run pytest tests/test_repack_deb.py tests/capsem-install/test_installed_layout.py -q`
- [ ] `uv run pytest tests/test_release_workflow.py::test_create_release_preserves_binary_metadata -q`
- [ ] `uv run pytest tests/test_docker.py::TestGenerateChecksums tests/test_gen_manifest.py tests/capsem-build-chain/test_manifest_regen.py -q`
- [ ] `uv run pytest tests/capsem-session-lifecycle/test_db_exists.py tests/capsem-session-lifecycle/test_db_schema.py -q`
- [ ] `uv run pytest tests/capsem-session tests/capsem-session-exhaustive -q`
- [ ] `cd frontend && pnpm run check`
- [ ] `cd frontend && pnpm run test`
- [ ] `cd frontend && pnpm run build`
- [ ] `just dev-frontend`
- [ ] Gate A evidence recorded in `tracker.md`.
- [ ] `just exec "echo cli-ok"`
- [ ] `just exec "capsem-doctor"`
- [ ] Gate B evidence recorded in `tracker.md`.
- [ ] `pnpm -C docs run build`
- [ ] `pnpm -C site run build`
- [ ] `just test-install`

## Exit Criteria

- [ ] Every fixed track has targeted proof.
- [ ] Every failing targeted proof has an owner and is release-blocking.
- [ ] The sprint is ready to pay the full-suite cost in T11.
