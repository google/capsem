# T5: Service, Process, and Helper Packaging

## Objective

Make installed service/process layouts match runtime assumptions on every
platform. Process helper binaries, config reload, guest rootfs validation,
cleanup behavior, routes, and spawned-process environments must be explicit and
testable.

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

## Task List

### T5.1 Linux Helper Binaries

- [ ] Add `capsem-mcp-aggregator` and `capsem-mcp-builtin` to Linux release
  build.
- [ ] Add both helpers to `scripts/repack-deb.sh`.
- [ ] Add both helpers to `scripts/deb-postinst.sh` symlink loops.
- [ ] Add both helpers to `scripts/simulate-install.sh`.
- [ ] Update install-test expected binary lists to require the eight-binary
  contract.
- [ ] Add deb contents tests that fail on the old six-binary package contract.

### T5.2 Policy Spec Artifact and Routes

- [ ] Add/keep `config/policy-hook-openapi.json` in the explicit staged file
  list for the release commit.
- [ ] Add a clean-checkout test or CI preflight that proves the checked-in spec
  artifact exists before compiling tests.
- [ ] Add route/auth coverage for `/policy-hook/spec` through service or
  gateway route matrix.

### T5.3 Cleanup and Environment Isolation

- [ ] Move expected-exit session directory removal into `spawn_blocking` or the
  existing cleanup path.
- [ ] Add `env_clear()` plus an explicit safe env allowlist for the aggregator,
  or document and test the intended inherited env surface.
- [ ] Add `env_clear()` plus configured `def.env` for external stdio MCP
  children, or document and test intentional inheritance.
- [ ] Add tests that external stdio MCP servers do not receive Capsem test
  override/config path leakage.

### T5.4 Rootfs Validation Coverage

- [ ] Add `capsem-dns-proxy` to release rootfs validation.
- [ ] Add `capsem-sysutil` to release rootfs validation.
- [ ] Keep rootfs validation in sync with `guest/artifacts/capsem-init` and the
  canonical guest binary list.
- [ ] Keep this task aligned with T1 if validation moves to a hard build-assets
  gate.

### T5.5 Runtime Config Reload Semantics

- [ ] Surface reload failures to the frontend as saved-but-not-applied to N
  running sessions.
- [ ] Keep actionable error state and retry affordance in settings store/UI.
- [ ] Refresh builtin MCP/domain policy state when `ReloadConfig` updates live
  policy.
- [ ] Make `McpRefreshTools` use the builtin-aware builder with regenerated
  builtin env, or move builtin HTTP enforcement to live shared policy rather
  than startup env.
- [ ] Add service/process tests proving running sessions receive updated
  Policy V2/MCP/domain state after settings save.

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

- [ ] `cargo test -p capsem-core checked_in_artifact_matches_rust_export -- --nocapture`
- [ ] `cargo test -p capsem-gateway all_non_root_paths_require_auth -- --nocapture`
- [ ] `cargo test -p capsem-service cleanup -- --nocapture`
- [ ] `just cross-compile`
- [ ] `uv run pytest tests/test_repack_deb.py tests/capsem-install/test_installed_layout.py -q`
- [ ] `dpkg-deb --contents target/release/bundle/deb/*.deb | rg 'capsem-mcp-(aggregator|builtin)'`
- [ ] `pkgutil --expand-full packages/Capsem-*.pkg /tmp/capsem-pkg && find /tmp/capsem-pkg -type f | rg 'capsem-mcp-(aggregator|builtin)'`
- [ ] `uv run pytest tests/test_docker.py tests/capsem-rootfs-artifacts/ -q`
- [ ] Add targeted reload test for running session policy/domain refresh.

## Exit Criteria

- [ ] Linux and macOS package layouts have parity for runtime helper binaries.
- [ ] Install tests fail if helper binaries or manifest files are missing.
- [ ] Helper processes do not inherit broad parent env by default.
- [ ] `/policy-hook/spec` is covered at handler plus route/auth levels.
- [ ] Running sessions do not silently keep stale policy after settings save.
