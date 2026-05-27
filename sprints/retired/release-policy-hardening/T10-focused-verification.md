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
- `tests/test_release_workflow_policy.py`
- `tests/test_docker.py`
- `tests/test_gen_manifest.py`
- `tests/capsem-build-chain/*`
- `tests/capsem-install/*`
- `tests/capsem-session-lifecycle/*`
- `tests/capsem-session/*`
- `tests/capsem-session-exhaustive/*`
- `frontend/package.json`
- `frontend/vitest.config.ts`
- `bootstrap.sh`
- `scripts/doctor-common.sh`
- `scripts/doctor-macos.sh`
- `scripts/doctor-linux.sh`
- `scripts/sync-dev-assets.sh`
- `scripts/deb-postinst.sh`
- `scripts/pkg-scripts/postinstall`
- `docker/Dockerfile.install-test`
- `tests/test_package_scripts.py`

## Findings

- [P0] Focused verification existed as a tracker row but had no owner doc.
- [P1] Several track docs require newly named tests that should be run before
  the full suite.
- [P1] Package payload and manifest checks must be explicit, not hidden inside
  post-release verification.
- [P2] Frontend visual verification requires browser/console evidence in
  addition to unit tests.

## Swarm Transfer Tracker

| Source | Priority | Owner task | Required transfer point | Required proof |
|---|---:|---|---|---|
| FD01 ui-policy-settings | P0/P1 | T10.3 | Frontend proof must cover hook control visibility, callback matrix, reload-failure state, asset/image truth, create defaults, and generated/default mocks. | Gate A evidence plus frontend model/store/component tests recorded in tracker evidence ledger. |
| FD02 docs-release-metadata | P0/P1 | T10.6 | Docs/release metadata proof must include stale hook/updater/artifact terms, docs/site builds, and generated latest-release review. | T10.6 command rows and evidence ledger entries cover docs/site and release text checks. |
| FD03 sprint-consistency | P1 | T10.7 | T10 command list must become a complete focused-verification rollup or clearly defer to owner docs. | T10.7 normalizes invalid/future commands and records missing-test owner tasks. |
| FD04 core-policy-assets | P0/P1 | T10.2, T10.4 | Asset resolver, hook runtime, MCP notification bypass, audit rows, telemetry naming, and benchmark guardrails need focused proof. | Targeted cargo/pytest/bench commands exist, pass, and are recorded. |
| FD05 service-process | P0/P1 | T10.1, T10.5 | Helper packaging, reload, builtin refresh, env isolation, cleanup, and route/auth coverage require focused proof. | Package tests, service/process tests, gateway route tests, and integration proof recorded. |
| FD06 cli-updater-install | P0/P1 | T10.1, T10.2, T10.3, T10.6 | Clean package install, manifest verification, updater honesty, postinstall failure semantics, and version/update copy need proof. | Package payload, negative manifest, updater strategy, and UI/CLI copy proof recorded. |
| FD07 mcp-policy-boundary | P0/P1 | T10.4, T10.5 | MCP notification bypass, helper packages, env leaks, redirects, refresh, denial telemetry, and trace propagation need proof. | Unit/integration/E2E proofs recorded with no chat-only evidence. |
| FD08 telemetry-session | P1 | T10.5 | Timeline/triage, old/current DB checks, MCP/tool SQL, hook audit semantics, and reader/frontend SQL need proof. | T6 command set and session/timeline evidence recorded. |
| FD09 guest-image-builder | P1 | T10.2, T10.5 | Initrd/rootfs/manifest regeneration, same-day versions, cleanup, binary contract, and permissions need proof. | Image-builder/rootfs pytest commands and CI/rootfs dry-run evidence recorded. |
| FD10 ci-packaging | P0/P1 | T10.1, T10.2, T10.7 | Package payloads, manifest signatures, Linux/rootfs release-blocking behavior, metadata preservation, updater strategy, and provenance need proof. | Focused package/manifest/CI-policy checks recorded before T11. |
| FD11 verification-architecture | P0/P1 | T10.7, T10.8 | Implementation cannot start until swarm completeness is closed; invalid commands and missing tests must be normalized. | Evidence ledger marks future tests as to-be-created owner tasks until files exist. |
| FD12 manual-ui-cli-gates | P1 | T10.3, T10.5, T10.8 | Gate A/B must be blocking and durable, not chat-only. | Gate A/B rows include command/view, evidence path, owner, and sign-off/blocker. |
| FD14 swarm-transfer-closeout | P1 | T10.7 | Nonexistent-test commands must not appear as final runnable gates without owner creation tasks. | T10.7 `test -e`/`rg --files` review recorded before final gate. |

## Task List

### T10.1 Package and Install Proof

- [x] Run `.pkg` payload expansion and manifest signature checks from T0.
- [x] Run `.deb` payload checks for manifest files, dev pubkey, and MCP
  helpers.
- [x] Run fresh Linux `.deb` install from empty `CAPSEM_HOME`.
- [x] Run clean macOS `.pkg` install proof or record manual blocker evidence.
  Payload proof passed; clean install is not yet run because the current
  postinstall mutates `/Applications`, `/usr/local/share/capsem`, and the real
  `~/.capsem` for the console user, so it is not an isolated empty-home proof on
  this workstation.

### T10.2 Asset and Manifest Proof

- [x] Run manifest producer tests from T1.
- [x] Run release workflow metadata preservation test.
- [x] Run rootfs validation tests after T1/T5 changes.
- [x] Run asset cleanup tests after `cleanup_unused_assets()` changes.
- [x] Prove local dev asset manifests are signed before `run-service`/`exec`
  can launch a service.

### T10.3 Frontend Policy Proof

- [x] Run Svelte/TypeScript checks.
- [x] Run model/store/export/component tests.
- [x] Run coverage after policy UI tests land.
- [x] Run Gate A with Elie: `just dev-frontend`, browser JS UI policy flow,
  screenshots/evidence, and zero console errors/warnings. `just dev-frontend`
  reached the app, but Settings required a live gateway and showed
  service-unavailable; completion proof came from `just ui`.
- [x] Run `just ui` and Chrome DevTools MCP visual/console proof. Captured
  Settings -> Policy generated-rule and staged-rule screenshots under
  `sprints/release-policy-hardening/evidence/`; browser-side warning/error
  capture stayed empty after navigation and staging.
- [x] Verify T2.8 runtime/image truth states or prove unsupported surfaces are
  hidden.

### T10.4 Runtime Security Proof

- [x] Run policy hook tests.
- [x] Run policy hook spec tests.
- [x] Run MCP frame tests.
- [x] Run policy benchmark smoke with matching-rule assertions.

### T10.5 Service, Telemetry, and Integration Proof

- [x] Run service reload/cleanup/route tests.
- [x] Run gateway route/auth tests.
- [x] Run logger migration tests.
- [x] Run session lifecycle/session/session-exhaustive tests.
- [x] Run T8 production-path policy E2E or hidden-hook UI proof.
- [x] Run Gate B command proof: `just exec "echo cli-ok"` and
  `just exec "capsem-doctor"` pass.

### T10.6 Docs and Metadata Proof

- [x] Run stale-term searches.
- [x] Run docs and site builds.
- [x] Run version synchronization checks from T9.
- [x] Run host doctor after adding `minisign` as a required local manifest
  signing tool.

### T10.7 Evidence Capture

- [x] Record command, pass/fail status, and follow-up owner in `tracker.md`.
- [x] Do not advance to T11 until every P0/P1 focused proof passes or has an
  explicit release-blocking owner.

### T10.8 Evidence Ledger

- [x] Add an evidence ledger section to `tracker.md` before running Gate A/B.
- [x] For every focused proof, record command, expected proof, pass/fail,
  artifact/log/screenshot path, owner, and release-blocking follow-up.
- [x] Store screenshots, console logs, package inspection output, and CLI
  transcripts as durable local files referenced by path from `tracker.md`.
- [x] Do not rely on chat history as release evidence.

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

- [x] `git diff --check`
- [x] `cargo test -p capsem-core asset_manager -- --nocapture`
- [x] `cargo test -p capsem-core policy_hook -- --nocapture`
- [x] `cargo test -p capsem-core policy_hook_spec -- --nocapture`
- [x] `cargo test -p capsem-core mcp_frame -- --nocapture`
- [x] `cargo test -p capsem-service policy_hook -- --nocapture`
- [x] `cargo test -p capsem-service cleanup -- --nocapture`
- [x] `cargo test -p capsem-gateway all_non_root_paths_require_auth -- --nocapture`
- [x] `cargo test -p capsem-app`
- [x] `cargo test -p capsem`
- [x] `cargo test -p capsem-logger`
- [x] `uv run pytest tests/test_repack_deb.py tests/capsem-install/test_installed_layout.py -q`
- [x] `uv run pytest tests/test_package_scripts.py -q`
- [x] `uv run pytest tests/capsem-install/test_asset_download.py -q`
- [x] `uv run pytest tests/test_release_workflow_policy.py::test_create_release_preserves_binary_metadata -q`
- [x] `uv run pytest tests/test_release_workflow_policy.py -q`
- [x] `uv run pytest tests/test_docker.py::TestGenerateChecksums tests/test_gen_manifest.py tests/capsem-build-chain/test_manifest_regen.py -q`
- [x] `uv run pytest tests/capsem-session-lifecycle/test_db_exists.py tests/capsem-session-lifecycle/test_db_schema.py -q`
- [x] `uv run pytest tests/capsem-session tests/capsem-session-exhaustive -q`
- [x] `cd frontend && pnpm run check`
- [x] `cd frontend && pnpm run test`
- [x] `cd frontend && pnpm run build`
- [x] `just dev-frontend`
- [x] `just ui`
- [x] `pnpm -C frontend exec vitest run --coverage`
- [x] Gate A evidence recorded in `tracker.md`.
- [x] `just exec "echo cli-ok"`
- [x] `just exec "capsem-doctor"`
- [x] `scripts/doctor-common.sh`
- [x] `bash scripts/sync-dev-assets.sh assets assets && minisign -Vm assets/manifest.json -x assets/manifest.json.minisig -p assets/manifest-sign.dev.pub`
- [x] Gate B command evidence recorded in `tracker.md`.
- [x] `pnpm -C docs run build`
- [x] `pnpm -C site run build`
- [x] `just test-install`
  (strict `.deb` install path passed: 33 passed, 31 skipped; no
  `apt-get install -f` retry path remains)

## Exit Criteria

- [x] Every fixed track has targeted proof.
- [x] Every failing targeted proof has an owner and is release-blocking.
- [x] The sprint is ready to pay the full-suite cost in T11.
- [x] T11 paid the full-suite cost on 2026-05-10; see
  `sprints/release-policy-hardening/evidence/T11-2026-05-10-full-release-gate.md`.
