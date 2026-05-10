# Release Policy Hardening Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert the swarm review findings into release-blocking fixes and
verification for the next `1.1.xxx` release.

**Architecture:** Keep fixes split by subsystem so artifact packaging, image
manifest compatibility, UI policy settings, hook runtime, docs, and service
helper packaging can be reviewed and tested independently. Every track must
name the exact test that proves the release claim it makes.

**Tech Stack:** Rust workspace, Tauri 2, Astro/Svelte 5 frontend, GitHub
Actions release workflow, Python image builder, minisign, Docker install tests.

---

## Track Order

1. T0.1 manifest contract first. It decides whether package manifests are final
   release manifests or signed asset-compatibility snapshots.
2. T1.1 release manifest metadata preservation before any T0.2 package
   manifest mutation.
3. T0.2/T0.3 package manifest work and T5.1 helper packaging can run in
   parallel after those prerequisites.
4. T1 image/manifest compatibility protects both new and older binaries from
   resolving the wrong asset release.
5. T5 helper packaging can run alongside T0 because it touches Linux package
   contents and process helper discovery.
6. T3 hook runtime hardening can run independently, with focused Rust tests.
7. T2 UI policy settings can run independently, but requires visual
   verification before sign-off.
8. T8 policy integration E2E decides and proves the shipped Policy V2 scope
   before docs are finalized.
9. T4 docs follow the code fixes so docs describe the exact shipped surface.
10. T6 telemetry/session tooling follows the runtime work so audit views match
   the policy behavior that actually ships.
11. T9 release metadata/changelog runs after implementation decisions are final
    and owns the exact `1.1.xxx` version.
12. T10 focused verification runs after each fixed track has targeted tests.
13. T11 local release candidate gate runs only after T10 is green, then builds
    and installs the package locally with Elie + Codex sign-off.
14. T12 CI green release landing runs only after T11 is signed off.
15. T7 captures active reviewer findings and should be folded into the relevant
   track before closing the sprint.

## Cross-Track Verification Gate

- [ ] `git diff --check`
- [ ] `cargo test -p capsem-core asset_manager -- --nocapture`
- [ ] `cargo test -p capsem-core policy_hook -- --nocapture`
- [ ] `cargo test -p capsem-core policy_hook_spec -- --nocapture`
- [ ] `cargo test -p capsem-core mcp_frame -- --nocapture`
- [ ] `cargo test -p capsem-service policy_hook -- --nocapture`
- [ ] `cargo test -p capsem-service cleanup -- --nocapture`
- [ ] `cargo test -p capsem-logger`
- [ ] `cargo test -p capsem-gateway all_non_root_paths_require_auth -- --nocapture`
- [ ] `uv run pytest tests/test_docker.py::TestGenerateChecksums tests/test_gen_manifest.py tests/capsem-build-chain/test_manifest_regen.py -q`
- [ ] `uv run pytest tests/capsem-build-chain/test_create_hash_assets.py tests/capsem-install/test_asset_download.py tests/capsem-install/test_installed_layout.py -q`
- [ ] `uv run pytest tests/test_repack_deb.py tests/capsem-install/test_installed_layout.py -q`
- [ ] `cd frontend && pnpm run check && pnpm run build`
- [ ] `cargo test -p capsem-app`
- [ ] Visual verification: `just ui`, Settings -> Policy add/edit rename/delete/import/generated rule flow.
- [ ] T8 policy integration proof: shipped hook path E2E, or hidden/deferred
  hook controls with tests and docs.
- [ ] T6 timeline proof: layers `exec,mcp,net,fs,model,dns,hook,audit,snapshot`
  work against a fixture or real session DB.
- [ ] T9 metadata proof: version files, changelog, latest release, release
  page, and exact `1.1.xxx` version agree.
- [ ] T10 focused verification proof: all targeted track checks pass.
- [ ] T11 local release proof: preflight, `just test`, `just install`,
  installed CLI smoke, JS UI, desktop launch, and release hygiene pass.
- [ ] T12 CI release proof: tag `v1.1.xxx`, CI green, live assets verified,
  downloaded package install proof recorded.
- [ ] `just test-install`
- [ ] Final gate: `env UV_CACHE_DIR=/private/tmp/capsem-uv-cache just test`
