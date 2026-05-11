# T5: Service, Process, and Helper Packaging

## Objective

Make installed service/process layouts match runtime assumptions on every
platform. Process helper binaries, config reload, guest rootfs validation,
cleanup behavior, routes, and spawned-process environments must be explicit and
testable.

## Status

Implementation complete as of 2026-05-10. Focused package-script,
route/auth, env-isolation, cleanup, rootfs-contract, and reload/refresh tests
pass. Live generated package payload inspection and running-VM E2E remain in
T10/T11 and T8/T10 respectively.

## Owned Files

- `.github/workflows/release.yaml`
- `scripts/build-pkg.sh`
- `scripts/pkg-scripts/postinstall`
- `scripts/repack-deb.sh`
- `scripts/deb-postinst.sh`
- `scripts/simulate-install.sh`
- `tests/test_repack_deb.py`
- `tests/capsem-install/*`
- `crates/capsem-process/src/main.rs`
- `crates/capsem-process/src/ipc.rs`
- `crates/capsem-core/src/mcp/server_manager.rs`
- `crates/capsem-service/src/main.rs`
- `crates/capsem-mcp-builtin/src/main.rs`
- `guest/artifacts/capsem-init`
- `tests/capsem-rootfs-artifacts/*`
- `config/policy-hook-openapi.json`

## Findings

- [P0] Linux release build, repack, postinst, and tests omit
  `capsem-mcp-aggregator` and `capsem-mcp-builtin`, although
  `capsem-process` expects both beside itself at runtime.
- [P1] Linux install tests encode the same stale six-binary package contract.
- [P1] `config/policy-hook-openapi.json` is referenced by `include_str!` tests
  but must be explicitly tracked/staged for clean checkout CI.
- [P2] Service cleanup performs `std::fs::remove_dir_all` directly inside a
  Tokio child-wait task.
- [P1] The process spawns the aggregator without `env_clear()`, and external
  stdio MCP servers inherit the aggregator environment unless explicitly
  cleared.
- [P2] Release rootfs validation omits `capsem-dns-proxy` and `capsem-sysutil`.
- [P3] `/policy-hook/spec` has handler tests but not route/auth matrix tests.
- [P1] Settings save can swallow `/reload-config` failures, so the frontend can
  report saved while running VMs keep stale policy.
- [P1] Built-in MCP policy/domain state is built from startup env and is not
  refreshed end-to-end on reload.
- [P1] `McpRefreshTools` uses plain `build_server_list`, not the builtin-aware
  builder, risking dropped builtin tools or missing session/env wiring.

## Swarm Transfer Tracker

| Source | Priority | Owner task | Required transfer point | Required proof |
|---|---:|---|---|---|
| FD05 service-process | P0 | T5.1 | MCP helper install/discovery can fail silently; Linux scripts/tests omit `capsem-mcp-aggregator` and `capsem-mcp-builtin`. | Installed layout and package contents tests require both helpers and fail old six-binary layouts. |
| FD05 service-process | P1 | T5.5 | Settings save/apply semantics are not release-safe: persisted settings can return success without applied running-session state. | Service/process tests return persisted/applied/failed-session details and frontend can surface them through T8. |
| FD05 service-process | P1 | T5.5 | Running builtin MCP HTTP/domain policy is startup-only and not refreshed for live sessions. | Running session refresh proof preserves builtin tools and updated domain policy. |
| FD05 service-process | P1 | T5.5 | `McpRefreshTools` drops builtin wiring and masks errors. | Refresh uses builtin-aware builder and service propagates `McpRefreshResult` failures. |
| FD05 service-process | P1 | T5.3 | Helper child env is not explicitly isolated. | Aggregator/external stdio env-leak tests prove only allowed/configured/trace env is visible. |
| FD05 service-process | P1 | T5.3 | Cleanup is partly nondeterministic and fire-and-forget. | Cleanup tests prove delete/stop/purge wait for deterministic cleanup or expose pending state. |
| FD05 service-process | P2 | T5.2 | `/policy-hook/spec` lacks route/auth coverage. | Service/gateway route matrix covers `/policy-hook/spec`. |
| FD06 cli-updater-install | P0 | T5.1 | Linux packages omit MCP helper binaries. | `.deb` contents test requires all eight binaries. |
| FD07 mcp-policy-boundary | P0 | T5.1 | Linux release packaging omits MCP helper binaries and runtime falls back to empty stub. | `.deb` and installed layout proof include both helpers. |
| FD07 mcp-policy-boundary | P1 | T5.3 | External stdio MCP servers inherit parent env. | Fixture server reports env; sentinel secrets/config paths are absent. |
| FD07 mcp-policy-boundary | P1 | T5.5 | Builtin HTTP tools check domain policy only before redirects. | Redirect from allowed host to blocked host never fetches blocked final host and telemetry records denial. |
| FD07 mcp-policy-boundary | P1 | T5.5 | `McpRefreshTools` drops builtin MCP server from live sessions. | Running VM refresh keeps `local__echo`/`local__http_headers` and honors updated domain policy. |
| FD07 mcp-policy-boundary | P2 | T5.3 | Trace correlation for child processes must survive env isolation. | Explicit trace env is passed to builtin/external MCP children and trace continuity is tested with T6. |
| FD07 mcp-policy-boundary | P3 | T5.2 | Gateway auth proof is thin for `/policy-hook/spec` and fallback routes. | Gateway integration matrix shows 401 without token and proxy only with token. |
| FD09 guest-image-builder | P1 | T5.4 | Rootfs validation must cover `capsem-sysutil`, `capsem-dns-proxy`, and required guest artifacts. | Validation derives from canonical guest binary list and blocks release. |
| FD10 ci-packaging | P0 | T5.1 | Linux `.deb` omits runtime MCP helpers. | CI/repack/install tests require helpers in every published package. |
| FD13 ci-release-landing-1-1 | P0 | T5.1 | Linux CI builds stale companion-binary package contract. | Linux job builds/validates helper binaries before publish. |
| FD13 ci-release-landing-1-1 | P1 | T5.4 | Local/CI rootfs validation must share source-of-truth guest binary list. | `scripts/preflight.sh` and release workflow catch missing `capsem-dns-proxy`/`capsem-sysutil`. |

## Task List

### T5.1 Linux Helper Binaries

- [x] Add `capsem-mcp-aggregator` and `capsem-mcp-builtin` to Linux release
  build.
- [x] Add both helpers to `scripts/repack-deb.sh`.
- [x] Add both helpers to `scripts/deb-postinst.sh` symlink loops.
- [x] Add both helpers to `scripts/simulate-install.sh`.
- [x] Update install-test expected binary lists to require the eight-binary
  contract.
- [x] Add deb contents tests that fail on the old six-binary package contract.

### T5.2 Policy Spec Artifact and Routes

- [x] Add/keep `config/policy-hook-openapi.json` in the explicit staged file
  list for the release commit.
- [x] Add a clean-checkout test or CI preflight that proves the checked-in spec
  artifact exists before compiling tests.
- [x] Add route/auth coverage for `/policy-hook/spec` through service or
  gateway route matrix.

### T5.3 Cleanup and Environment Isolation

- [x] Move expected-exit session directory removal into `spawn_blocking` or the
  existing cleanup path.
- [x] Add `env_clear()` plus an explicit safe env allowlist for the aggregator,
  or document and test the intended inherited env surface.
- [x] Add `env_clear()` plus configured `def.env` for external stdio MCP
  children, or document and test intentional inheritance.
- [x] Add tests that external stdio MCP servers do not receive Capsem test
  override/config path leakage.

### T5.4 Rootfs Validation Coverage

- [x] Add `capsem-dns-proxy` to release rootfs validation.
- [x] Add `capsem-sysutil` to release rootfs validation.
- [x] Keep rootfs validation in sync with `guest/artifacts/capsem-init` and the
  canonical guest binary list.
- [x] Keep this task aligned with T1 if validation moves to a hard build-assets
  gate.

### T5.5 Runtime Config Reload Semantics

- [x] Surface reload failures to the frontend as saved-but-not-applied to N
  running sessions.
- [x] Keep actionable error state and retry affordance in settings store/UI.
- [x] Refresh builtin MCP/domain policy state when `ReloadConfig` updates live
  policy.
- [x] Make `McpRefreshTools` use the builtin-aware builder with regenerated
  builtin env, or move builtin HTTP enforcement to live shared policy rather
  than startup env.
- [x] Add service/process tests proving running sessions receive updated
  Policy V2/MCP/domain state after settings save.
  - Targeted proof covers structured reload/refresh failure propagation,
    builtin-aware refresh server construction, and regenerated domain/session
    env. Full running-VM E2E remains T8.4/T10.5.

## Proof Matrix

| Category | Required proof |
|---|---|
| Package | Linux `.deb` contains and symlinks all runtime helper binaries. |
| Functional | process discovers aggregator/builtin helpers from installed layout. |
| Security | external stdio MCP env is explicitly allowlisted. |
| Async safety | cleanup avoids blocking filesystem work on Tokio worker path. |
| Runtime | reload applies policy/domain changes to running sessions or reports failure. |
| Rootfs | release validation covers every guest binary required by init. |

## Verification

- [x] `cargo test -p capsem-core checked_in_artifact_matches_rust_export -- --nocapture`
- [x] `cargo test -p capsem-gateway all_non_root_paths_require_auth -- --nocapture`
- [x] `cargo test -p capsem-service cleanup -- --nocapture`
- [x] `cargo test -p capsem-service mcp_refresh_surfaces_process_failure -- --nocapture`
  (rerun with sandbox escalation because the fake process IPC server binds a
  temporary Unix socket).
- [x] `cargo test -p capsem-core stdio_child_base_env_allows_trace_and_execution_only -- --nocapture`
- [x] `cargo test -p capsem-process aggregator_parent_env_allows_execution_and_logging_only -- --nocapture`
- [x] `cargo test -p capsem-process mcp_runtime -- --nocapture`
- [x] `cargo test -p capsem-proto reload_config_result_roundtrip -- --nocapture`
- [x] `cargo test -p capsem-mcp-aggregator -- --nocapture`
- [x] `uv run pytest tests/test_package_scripts.py tests/test_repack_deb.py -q`
  (3 passed, 6 skipped).
- [x] `uv run pytest tests/test_repack_deb.py tests/capsem-install/test_installed_layout.py -q`
  (15 passed, 6 skipped).
- [x] `uv run pytest tests/capsem-install/test_installed_layout.py tests/capsem-install/test_smoke.py tests/capsem-install/test_reinstall.py -q`
  (17 passed, 3 skipped).
- [x] `uv run pytest tests/test_release_workflow_policy.py tests/capsem-rootfs-artifacts/ -q`
  (26 passed).
- [x] `uv run pytest tests/capsem-gateway/test_gw_auth.py tests/capsem-gateway/test_gw_proxy.py -q`
  (19 passed).
- [ ] `just cross-compile` (deferred to T10/T11 package build gate).
- [ ] `dpkg-deb --contents target/release/bundle/deb/*.deb | rg 'capsem-mcp-(aggregator|builtin)'`
  (requires generated package artifact; deferred to T10/T11).
- [ ] `pkgutil --expand-full packages/Capsem-*.pkg /tmp/capsem-pkg && find /tmp/capsem-pkg -type f | rg 'capsem-mcp-(aggregator|builtin)'`
  (requires generated package artifact; deferred to T10/T11).

## Exit Criteria

- [x] Linux and macOS package layouts have parity for runtime helper binaries.
- [x] Install tests fail if helper binaries or manifest files are missing.
- [x] Helper processes do not inherit broad parent env by default.
- [x] `/policy-hook/spec` is covered at handler plus route/auth levels.
- [x] Running sessions do not silently keep stale policy after settings save.
  - Targeted service/process proof is complete; full running-VM E2E remains
    tracked in T8/T10.
