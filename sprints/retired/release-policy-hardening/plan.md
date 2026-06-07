# Release Policy Hardening Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert the swarm review findings into release-blocking fixes and
verification for the next `1.1.1778456247` release.

**Architecture:** Keep fixes split by subsystem so artifact packaging, image
manifest compatibility, UI policy settings, hook runtime, docs, and service
helper packaging can be reviewed and tested independently. Every track must
name the exact test that proves the release claim it makes.

**Tech Stack:** Rust workspace, Tauri 2, Astro/Svelte 5 frontend, GitHub
Actions release workflow, Python image builder, minisign, Docker install tests.

---

## Track Order

1. T7 is the pre-sprint transfer board. Every `swarm-findings/*.md` has now
   been read, FD01-FD14 subtasks exist in `T7-active-review-followups.md`, and
   each P0/P1 finding has an owner row in T0-T13. Keep its downstream
   blocker checkboxes open as T8-T13 resolve or deliberately defer them.
2. T0.1 manifest contract first. It decides whether package manifests are final
   release manifests or signed asset-compatibility snapshots.
3. T1.1 release manifest metadata preservation before any T0.2 package
   manifest mutation.
4. T0.2/T0.3 package manifest work and T5.1 helper packaging can run in
   parallel after those prerequisites.
5. T1 image/manifest compatibility protects both new and older binaries from
   resolving the wrong asset release.
6. T5 helper packaging can run alongside T0 because it touches Linux package
   contents and process helper discovery.
7. T3 hook runtime hardening can run independently, with focused Rust tests.
8. T2 UI policy settings can run independently, but requires visual
   verification before sign-off.
9. T8 policy integration E2E records that configured external hook dispatch is
   deferred for `1.1.1778456247`, implements the non-hook Policy V2 settings/reload
   proof path, and leaves the focused VM run as T10/T11 evidence.
10. T4 docs follow the code fixes so docs describe the exact shipped surface.
11. T6 telemetry/session tooling follows the runtime work so audit views match
   the policy behavior that actually ships.
12. T9 release metadata/changelog runs after implementation decisions are final
    and owns the exact `1.1.1778456247` version.
13. T10 focused verification runs after each fixed track has targeted tests.
14. T11 local release candidate gate runs only after T10 is green, then builds
    and installs the package locally with Elie + Codex sign-off.
15. T12 CI green release landing runs only after T11 is signed off.
16. T13 kernel/netfilter recovery gate runs immediately when full-suite
    regression appears and must return full `just test` green before any new
    sprint scope starts.

## Cross-Track Verification Gate

- [x] `git diff --check`
- [x] T7 pre-sprint proof: FD01-FD14 are read, owner rows exist in T0-T13, and
  no P0/P1 finding remains only in a finding doc.
- [x] `cargo test -p capsem-core asset_manager -- --nocapture`
- [x] `cargo test -p capsem-core policy_hook -- --nocapture`
  (23 passed; rerun escalated because the raw TCP streaming test binds a
  localhost listener).
- [x] `cargo test -p capsem-core policy_hook_spec -- --nocapture`
  (6 passed).
- [x] `cargo test -p capsem-core mcp_frame -- --nocapture` (50 passed).
- [x] `cargo test -p capsem-core mcp_endpoint -- --nocapture` (9 passed).
- [x] `cargo test -p capsem-logger mcp_call -- --nocapture` (15 passed).
- [x] `cargo bench -p capsem-core --bench policy_v2 -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2`
  (completed; benchmark setup assertions passed).
- [ ] `cargo test -p capsem-service policy_hook -- --nocapture`
- [x] `cargo test -p capsem-service cleanup -- --nocapture` (3 passed).
- [x] `cargo test -p capsem-service mcp_refresh_surfaces_process_failure -- --nocapture`
  (1 passed after sandbox escalation for temporary UDS bind).
- [x] `cargo test -p capsem-logger` (98 unit tests + 126 roundtrip tests).
- [x] `cargo test -p capsem-gateway all_non_root_paths_require_auth -- --nocapture`
  (1 passed).
- [x] `uv run pytest tests/test_docker.py::TestGenerateChecksums tests/test_gen_manifest.py tests/capsem-build-chain/test_manifest_regen.py -q`
- [ ] `uv run pytest tests/capsem-build-chain/test_create_hash_assets.py tests/capsem-install/test_asset_download.py tests/capsem-install/test_installed_layout.py -q`
- [x] `uv run pytest tests/test_repack_deb.py tests/capsem-install/test_installed_layout.py -q`
  (15 passed, 6 skipped).
- [x] `cd frontend && pnpm run check && pnpm run build`
  (verified with the repository pnpm store setting; full frontend Vitest suite
  also passed after T6 SQL coverage: 18 files, 383 tests).
- [x] T8 frontend reload proof:
  `pnpm -C frontend test -- src/lib/__tests__/settings-store.test.ts src/lib/__tests__/settings-page-reload-banner.test.ts src/lib/__tests__/api.test.ts src/lib/__tests__/settings-export.test.ts src/lib/models/__tests__/settings-model.test.ts src/lib/__tests__/policy-rules-section.test.ts`
  (19 files, 388 tests).
- [ ] `cargo test -p capsem-app`
- [ ] Visual verification: `just ui`, Settings -> Policy add/edit rename/delete/import/generated rule flow.
- [x] T8 policy integration implementation: configured external hook dispatch
  deferred, hook controls/writes hidden or rejected, reload-failure UI tested,
  and non-hook Policy V2 live reload/timeline E2E path added.
- [x] T8 focused VM proof: non-hook Policy V2 live reload/timeline E2E proof
  passed in T10 and the final T11 `just test` policy/MITM suites passed.
- [x] T6 timeline proof: layers `exec,mcp,net,fs,model,dns,hook,audit,snapshot`
  work against a fixture or real session DB.
- [ ] T9 metadata proof: version files, changelog, latest release, release
  page, and exact `1.1.1778456247` version agree.
- [x] T10 focused verification proof: all targeted track checks pass.
- [ ] T11 local release proof: preflight, `just test`, `just install`,
  installed CLI smoke, JS UI, desktop launch, and release hygiene pass.
  T11 full-suite, doctor, VM, restored-private preflight, host install,
  installed CLI, installed doctor, `just run-ui --` process proof, and
  installed app/tray relaunch proof are green. Elie Gate C/Gate D visual
  sign-off remains open before T12.
- [ ] T12 CI release proof: tag `v1.1.1778456247`, CI green, live assets verified,
  downloaded package install proof recorded.
- [x] T13 kernel/netfilter proof: guest `iptables` tables exist, redirect rules
  install successfully, focused network-policy/session tests pass, and full
  `just test` is green (re-verified locally on 2026-05-14).
- [x] Docker/systemd install e2e inside `just test` (33 passed, 31 skipped).
- [x] Final gate: `just test` (full local suite passed on 2026-05-11 and was
  re-verified on 2026-05-14).

## T4 Docs Proof, 2026-05-10

- [x] `rg -n "dnsmasq|vsock:?5003|DMG|\\.dmg|AppImage|image < 12MB|12MB" README.md docs/src/content/docs site/src`
  (no matches).
- [x] `rg -n "latest\\.json" README.md docs/src/content/docs site/src`
  (no matches).
- [x] `rg -n "external Policy Hook|hook endpoint|hook attempts|remote hook|configured hook|Policy Hook Spec0 callouts|policy_action IN \\('ask', 'deny'|policy_action = 'deny'" LATEST_RELEASE.md docs/src/content/docs/releases/1-0.md docs/src/content/docs/security/policy.md docs/src/content/docs/architecture/session-telemetry.md`
  (no matches).
- [x] `pnpm -C docs run build` (45 pages built).
- [x] `pnpm -C site run build` (1 page built after `pnpm -C site install`
  downloaded the missing `svelte` tarball).

## T5 Service/Process Proof, 2026-05-10

- [x] `cargo test -p capsem-core checked_in_artifact_matches_rust_export -- --nocapture`
  (1 passed).
- [x] `cargo test -p capsem-core stdio_child_base_env_allows_trace_and_execution_only -- --nocapture`
  (1 passed).
- [x] `cargo test -p capsem-process aggregator_parent_env_allows_execution_and_logging_only -- --nocapture`
  (1 passed).
- [x] `cargo test -p capsem-process mcp_runtime -- --nocapture` (4 passed).
- [x] `cargo test -p capsem-proto reload_config_result_roundtrip -- --nocapture`
  (1 passed).
- [x] `cargo test -p capsem-mcp-aggregator -- --nocapture` (compile passed).
- [x] `uv run pytest tests/test_package_scripts.py tests/test_repack_deb.py -q`
  (3 passed, 6 skipped).
- [x] `uv run pytest tests/capsem-install/test_installed_layout.py tests/capsem-install/test_smoke.py tests/capsem-install/test_reinstall.py -q`
  (17 passed, 3 skipped).
- [x] `uv run pytest tests/test_release_workflow_policy.py tests/capsem-rootfs-artifacts/ -q`
  (26 passed).
- [x] `uv run pytest tests/capsem-gateway/test_gw_auth.py tests/capsem-gateway/test_gw_proxy.py -q`
  (19 passed).
- [x] Generated package payload inspection remains T10/T11:
  `just cross-compile`, `.deb` `dpkg-deb --contents`, and `.pkg`
  `pkgutil --expand-full`.

## T6 Telemetry/Session Proof, 2026-05-10

- [x] `cargo test -p capsem-logger` (98 unit tests + 126 roundtrip tests).
- [x] `cargo test -p capsem-core policy_hook -- --nocapture`
  (23 passed after sandbox escalation for localhost TCP bind).
- [x] `cargo test -p capsem-service timeline_ -- --nocapture` (5 passed).
- [x] `cargo test -p capsem-service triage_ -- --nocapture` (1 passed).
- [x] `cargo test -p capsem-mcp timeline_tool_schema -- --nocapture`
  (1 passed).
- [x] `uv run pytest tests/capsem-session-lifecycle/test_db_exists.py tests/capsem-session-lifecycle/test_db_schema.py -q`
  (13 passed).
- [x] `uv run pytest tests/capsem-session tests/capsem-session-exhaustive -q`
  (52 passed, 1 skipped).
- [x] `uv run pytest tests/capsem-session/test_check_session_compat.py -q`
  (2 passed).
- [x] `pnpm -C frontend run check` (0 errors/warnings).
- [x] `pnpm -C frontend test -- src/lib/__tests__/sql-policy-fields.test.ts`
  (Vitest ran the frontend suite: 18 files, 383 tests passed).
- [x] `pnpm -C frontend test -- src/lib/__tests__/settings-store.test.ts src/lib/__tests__/settings-page-reload-banner.test.ts src/lib/__tests__/api.test.ts src/lib/__tests__/settings-export.test.ts src/lib/models/__tests__/settings-model.test.ts src/lib/__tests__/policy-rules-section.test.ts`
  (19 files, 388 tests passed).
- [x] `git diff --check`.
- [x] Real-session product-path timeline/triage proof remains T10/T11: T8 adds
  the timeline assertion to the focused E2E, and the final T11 full gate ran
  the session and policy/MITM suites.
