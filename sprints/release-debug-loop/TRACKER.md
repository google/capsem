# Release Startup Reliability Tracker

Last updated: 2026-05-14

## Active Sprint

S4 - Saved VM Asset Dependencies (implementation complete; final live gate remains meta-sprint)

S1, S2, and S3 are closed for their current scope. The live sudo-backed
`capsem uninstall -> just install -> capsem status` proof remains the final
meta-sprint gate, not a local-unit substitute.

## Immediate Gate

Target gate:

```bash
capsem uninstall
just install
capsem status
```

`capsem status --json` is now the release health oracle for local/package
verification. S3/S4/S6/S7 will deepen the service asset states, saved-VM asset
dependencies, UI consumption, and update path.

## Meta-Sprint Checklist

- [x] S0 - Startup Contract And Scope Control
- [x] S1 - Package Install And `capsem status` Health Gate
- [x] S2 - Verification Harness
- [x] S3 - Service Asset Supervisor And Consumer Audit
- [x] S4 - Saved VM Asset Dependencies
- [ ] S5 - `capsem-setup` Hardening
- [ ] S6 - UI Wizard/Dashboard Startup States
- [ ] S7 - Update/Uninstall/Purge Integration

## S0 Checklist

- [x] Capture `capsem uninstall` as runtime removal, not purge.
- [x] Capture `capsem purge` as destructive user-state removal.
- [x] Capture asset policy: disposable cache except saved-VM-referenced blobs.
- [x] Capture service-owned asset supervision with no installer reconcile RPC.
- [x] Capture package install and expanded `capsem status` as their own sprint.
- [x] Rewrite `MASTER.md` around meta-sprint status.
- [x] Rewrite `plan.md` around implementation slices and proof matrix.
- [x] Rewrite this tracker around the new gate.
- [x] Add `startup-info.md`.
- [x] Reconcile and revert the earlier setup-blocking patch because it conflicts with the desired service-owned asset model.
- [x] Remove premature changelog language for the superseded setup-blocking behavior.

## S1 Checklist

- [x] Identify existing CLI command/test patterns for expanding `capsem status`.
- [x] Add a failing contract test for doctor/status preflight semantics.
- [x] Refactor existing status health logic into `crates/capsem/src/status.rs` with typed `HealthIssue` variants, stable issue codes, severity, and structured issue reports.
- [x] Make `capsem doctor` call the status health preflight before VM provisioning.
- [x] Add `capsem status --json` with `capsem.status.v1` typed issue reports.
- [x] Add typed status blockers for missing/non-executable host helper binaries.
- [x] Extend host helper binary status checks to include the installed MCP helper binaries.
- [x] Add typed status blockers for stale `capsem-service` / `capsem-process` helper binary versions.
- [x] Add typed status blockers for stale `capsem-gateway` / `capsem-tray` helper binary versions.
- [x] Add typed status blockers for missing/stale/unreadable service units.
- [x] Add typed status blockers for asset manifest problems even when service is down.
- [x] Add strict setup-state health checks instead of using default-on-error setup loading.
- [x] Add a typed status blocker for missing macOS app bundle on real installed runtimes.
- [x] Update docs and changelog for the doctor/status preflight slice.
- [x] Run focused Rust tests for the status health slice.
- [x] Implement the expanded `capsem status --json` surface with top-level state and grouped check states.
- [x] Add package/local install policy tests for the gate.
- [x] Add first Python/install test for `capsem status --json` typed blockers.
- [x] Add Python/install dirty-state test for missing helper binary status code.
- [x] Add Python/install dirty-state test for missing MCP helper binary status code.
- [x] Add Python/install dirty-state test for stale helper binary version status code.
- [x] Fix install-test fixture freshness so local simulated installs build current host binaries and refresh helpers when `CAPSEM_BIN_SRC` changes.
- [x] Add Python/install dirty-state test for corrupt setup-state status code.
- [x] Add Python/install dirty-state tests for missing asset manifest and missing canonical rootfs status codes.
- [x] Add Python/install positive test that completed setup state does not emit setup blockers.
- [x] Refactor `capsem uninstall` from destructive home removal into runtime removal that preserves durable state.
- [x] Update `just install` hard-clean proof to require old runtime removal without requiring `~/.capsem` to disappear.
- [x] Add safe black-box install-harness coverage for runtime uninstall preserving durable state under isolated `CAPSEM_HOME`.
- [x] Make service/status reporting honor install-test isolation instead of reading the developer's real LaunchAgent/systemd unit.
- [x] Add full Python/install tests for the expanded install/status gate.

## S2 Checklist

- [x] Identify the first harness slice: failed `capsem status --json` gates must leave structured evidence, not just a nonzero exit.
- [x] Add `scripts/capture-install-status.py` as a safe evidence collector around the installed `capsem status --json` command.
- [x] Capture raw status stdout/stderr, parsed typed status JSON when valid, command return codes/timing, `capsem version`, environment hints, and a shallow `CAPSEM_HOME` tree snapshot.
- [x] Capture optional `capsem debug` output without letting debug failure override the status gate exit code.
- [x] Capture runtime breadcrumbs (`service.pid`, `service.sock`, `gateway.pid`, `gateway.port`, redacted `gateway.token`) without leaking gateway credentials.
- [x] Capture an explicit install-layout index for expected helper binaries, asset manifests, setup state, service unit, and the macOS app bundle path.
- [x] Capture saved-VM registry and persistent-session summaries without leaking saved VM environment variable values.
- [x] Add fake-binary tests proving the harness preserves typed failing status reports.
- [x] Add fake-binary tests proving malformed status output is kept raw with a parse error in metadata.
- [x] Add a missing-`capsem` partial package test proving the capture bundle is still written with command return code 127.
- [x] Add status-timeout coverage proving hangs return 124 and still write metadata.
- [x] Wire the capture harness into `just install` after gateway health and before guest DNS/HTTPS.
- [x] Add an installed-layout dirty fixture proving the capture bundle preserves `host_binary_missing` for a partial install.
- [x] Add an installed-layout dirty fixture proving the capture bundle preserves `service_not_running` and run-state breadcrumbs for a dead service.
- [x] Add a stale-service-unit evidence fixture pairing `service_unit_stale_path` status with captured unit contents.
- [x] Add dirty-state fixtures for missing app/tray bundles and partial package failure.
- [x] Add clean/reinstall/over-existing black-box install gate coverage.
- [x] Defer update-over-existing black-box gate coverage to S7 update semantics.
- [x] Add saved-VM fixture coverage for referenced asset evidence capture; preservation enforcement lives in S4.
- [x] Capture service/gateway health check states in failed-gate metadata; UI consumption lives in S6.

## S3 Checklist

- [x] Add a typed service asset state model: `checking`, `updating`, `ready`, `error`.
- [x] Add progress, retry, missing-artifact, version, and arch detail to the model.
- [x] Start service-owned asset supervision immediately at service startup.
- [x] Re-check assets on a timer without requiring installer/setup RPC calls.
- [x] Download missing current-version assets in the background.
- [x] Report retryable release-source/download failures as asset `error`, not unknown.
- [x] Expose the same asset state through service `/list` and `/setup/assets`.
- [x] Preserve the full asset state through gateway `/status`.
- [x] Make CLI status consume service asset state when the service is reachable.
- [x] Update frontend-facing types so `updating` is not collapsed to missing/unknown.
- [x] Add focused service/gateway/frontend contract tests for the first slice.
- [x] Update changelog and S3 coverage ledger for the first slice.
- [x] Audit tray/app consumers against the expanded gateway status shape.
- [x] Teach the tray menu to surface asset `updating` and disable New Session while assets are not ready.
- [x] Add a service integration or install-harness proof that missing release assets move from `updating` to `ready` after background download.
- [x] Add adversarial service proof for unreachable release source through the real background loop.

## B4 Checklist

- [x] Reproduce that the built-in `local` MCP server ignores enabled overrides.
- [x] Make the runtime built-in server honor `mcp.servers.local.enabled`.
- [x] Make settings save accept `mcp.servers.<name>.enabled` and persist it to `[mcp.server_enabled]`.
- [x] Keep disabled MCP server nodes visible in the settings tree.
- [x] Remove disabled generated stdio MCP servers from agent configs while preserving unrelated user servers.
- [x] Add frontend interaction coverage for disabling and re-enabling the local MCP server.

## S4 Checklist

- [x] Add saved-VM base asset identity metadata for asset version, arch, kernel hash, initrd hash, rootfs hash, and guest ABI.
- [x] Keep legacy persistent-registry entries loadable when they lack base asset identity.
- [x] Preserve saved-VM-referenced hash-named assets during startup cleanup.
- [x] Load the persistent registry before startup asset cleanup so saved-VM references are known.
- [x] Make forks inherit base asset identity from running and stopped persistent source VMs.
- [x] Make saved-VM resume and clone use pinned base assets instead of silently resolving the current asset set.
- [x] Fail saved-VM launch/resume before session cloning when pinned base assets are missing.
- [x] Expose missing saved-VM dependencies separately from current-version asset readiness through service `/list`.
- [x] Preserve saved-VM dependency state through gateway `/status`, tray status, frontend types, and `capsem status --json`.
- [x] Add typed `saved_vm_asset_missing` status blockers for the install/update health oracle.
- [x] Add focused cleanup, registry, service, gateway, tray, frontend, and CLI status tests.

## Evidence Log

- 2026-05-13: Original hitlist contained B1 assets/setup, B2 provider onboarding/settings, and B3 VM list/session UI symptoms.
- 2026-05-13: A narrow proof showed setup could previously mark itself complete after failed asset download.
- 2026-05-13: User rejected the narrow setup-blocking fix because download must run in the background and setup/config work must proceed with fan-out.
- 2026-05-13: User confirmed update should use `capsem uninstall`; package uninstall should preserve user data.
- 2026-05-13: User clarified assets are cache unless required by saved VMs, and the service should supervise assets itself on start/periodic/update triggers without a special installer call.
- 2026-05-13: User set the release gate as `capsem uninstall -> just install -> health/check everything`.
- 2026-05-13: Meta-sprint created with dedicated package install/expanded `capsem status`, verification, service assets, saved VM assets, setup, UI, and integration sprints.
- 2026-05-13: Reverted the earlier setup-blocking experiment and removed its premature changelog entry.
- 2026-05-13: Reopened S0 after user rejected a separate check command. The health gate belongs in `capsem status`.
- 2026-05-13: User clarified doctor must call the status health check. Added `crates/capsem/src/status.rs`, moved status health logic there, and wired doctor preflight through it.
- 2026-05-13: User rejected stringly typed health messages. Reworked the status gate to return typed `HealthIssue` enum variants with stable `HealthIssueCode`, `HealthSeverity`, and `HealthIssueReport`, then render strings only at the CLI/error boundary.
- 2026-05-13: Added `capsem status --json` and pure `StatusReport` construction so install/UI/test consumers can read `schema`, `ok`, service fields, and typed issue reports without parsing CLI prose.
- 2026-05-13: Added typed status blockers for missing and non-executable helper binaries before service/gateway checks.
- 2026-05-13: Added typed status blockers for missing/stale/unreadable service units, asset manifest checks that run before service liveness, and strict setup-state checks for missing/corrupt/incomplete setup state.
- 2026-05-13: Hardened service-unit path checks to accept raw, systemd-escaped, and LaunchAgent XML-escaped paths.
- 2026-05-13: Verified the current doctor/status slice with `cargo test -p capsem status::tests -- --nocapture` (19 tests), `cargo test -p capsem parse_status -- --nocapture`, `cargo test -p capsem parse_doctor -- --nocapture`, and `cargo check -p capsem`.
- 2026-05-13: Added black-box install harness coverage for `capsem status --json` typed blockers. The first run failed because the installed test binary had not been rebuilt with `--json`; after `cargo build -p capsem`, `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_typed_install_blockers -q` passed.
- 2026-05-13: Added black-box dirty-state coverage for missing helper binaries. `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_typed_install_blockers tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_helper_binary -q` passed.
- 2026-05-13: Added black-box dirty-state coverage for corrupt setup-state. The three status JSON install-harness tests passed together with `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_typed_install_blockers tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_helper_binary tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_corrupt_setup_state -q`.
- 2026-05-13: Added black-box positive setup-state coverage. The four status JSON install-harness tests passed together with `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_typed_install_blockers tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_helper_binary tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_corrupt_setup_state tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_accepts_completed_setup_state -q`.
- 2026-05-13: Added black-box dirty-state coverage for missing asset manifest and missing canonical rootfs. `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_asset_manifest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_rootfs_asset -q` passed.
- 2026-05-13: Re-ran the full status JSON install-harness slice. `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_typed_install_blockers tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_helper_binary tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_corrupt_setup_state tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_asset_manifest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_rootfs_asset tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_accepts_completed_setup_state -q` passed.
- 2026-05-13: Added a failing black-box proof that removing `capsem-mcp-builtin` was invisible to status. Extended `CapsemPaths` and `check_host_binaries` to include `capsem`, `capsem-mcp`, `capsem-mcp-aggregator`, and `capsem-mcp-builtin`; after `cargo build -p capsem`, `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_mcp_helper_binary -q` passed.
- 2026-05-13: Verified the MCP-helper path slice with `cargo test -p capsem discover_paths -- --nocapture`, `cargo test -p capsem status::tests -- --nocapture`, `cargo check -p capsem`, and `rustfmt --edition 2021 --check crates/capsem/src/status.rs crates/capsem/src/status/tests.rs crates/capsem/src/paths.rs`.
- 2026-05-13: Added a failing black-box proof that a stale executable `capsem-process` helper was invisible to status. Added `host_binary_version_mismatch` issue reports for `capsem-service` and `capsem-process`; after `cargo build -p capsem -p capsem-service -p capsem-process`, `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_stale_process_helper_binary -q` passed.
- 2026-05-13: Re-ran the full status JSON install-harness slice including stale helper version coverage. `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_typed_install_blockers tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_helper_binary tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_mcp_helper_binary tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_stale_process_helper_binary tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_corrupt_setup_state tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_asset_manifest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_missing_rootfs_asset tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_accepts_completed_setup_state -q` passed.
- 2026-05-13: Reproduced B4 with failing core tests: the runtime built-in `local` MCP server ignored `mcp.servers.local.enabled=false`, settings save rejected `mcp.servers.local.enabled` as unknown, and agent config injection still wrote disabled stdio MCP servers.
- 2026-05-13: Fixed B4 by applying corp-over-user enabled overrides to the built-in runtime server, accepting `mcp.servers.<name>.enabled` in settings save, keeping disabled MCP server nodes visible, and removing disabled generated stdio MCP servers during agent config injection while preserving unrelated user servers.
- 2026-05-13: Verified B4 with `cargo test -p capsem-core --lib build_server_list_builtin_local -- --nocapture`, `cargo test -p capsem-core --lib batch_update_mcp_local_enabled_writes_override_and_keeps_node_visible -- --nocapture`, `cargo test -p capsem-core --lib disabled_mcp_servers_are_not_injected_into_agent_configs -- --nocapture`, `cargo test -p capsem-core --lib mcp_servers_in_tree -- --nocapture`, `rustfmt --edition 2021 --check crates/capsem-core/src/mcp/mod.rs crates/capsem-core/src/mcp/tests.rs crates/capsem-core/src/net/policy_config/builder.rs crates/capsem-core/src/net/policy_config/tree.rs crates/capsem-core/src/net/policy_config/loader.rs crates/capsem-core/src/net/policy_config/tests.rs`, `cargo check -p capsem-core`, and `cargo check -p capsem`.
- 2026-05-13: Added frontend interaction coverage for B4. `npx vitest run src/lib/__tests__/mcp-section.test.ts` passed from `frontend/`.
- 2026-05-13: Found an S1 contract mismatch: `capsem uninstall` still removed the entire Capsem home and its known-binary list missed `capsem-mcp-aggregator` and `capsem-mcp-builtin`. Refactored uninstall to remove service/runtime wiring, binaries, stale run files, temp sessions, and helper processes while preserving config, setup state, assets, logs, session/audit data, persistent VM state, and `persistent_registry.json`.
- 2026-05-13: Updated `just install` so `assert_clean_uninstall` checks that the runtime bin dir and transient run-state are gone while allowing preserved `run/persistent` and `run/persistent_registry.json`.
- 2026-05-13: Made `capsem uninstall` respect install-test isolation: when `CAPSEM_HOME`, `CAPSEM_RUN_DIR`, or `CAPSEM_ASSETS_DIR` is set, it skips real LaunchAgent/systemd mutation and only removes the isolated runtime tree. This unlocked safe black-box uninstall coverage outside `live_system`.
- 2026-05-13: Made service-status reporting respect the same install-test isolation boundary: with isolation env vars set, service status reports the isolated socket state and does not read the developer's real LaunchAgent/systemd unit. Verified with `cargo test -p capsem service_status_ignores_platform_unit_in_isolation_env -- --nocapture`.
- 2026-05-13: Verified the uninstall-policy slice with `cargo test -p capsem uninstall -- --nocapture`, `cargo build -p capsem`, `cargo check -p capsem`, `rustfmt --edition 2021 --check crates/capsem/src/uninstall.rs`, `uv run pytest tests/capsem-install/test_uninstall.py::TestUninstall::test_runtime_uninstall_preserves_durable_state -q`, and `uv run pytest tests/test_release_workflow_policy.py::test_local_install_removes_old_runtime_before_installing_package -q`.
- 2026-05-13: `cargo fmt --check` is clean for `status.rs`, `status/tests.rs`, and `paths.rs`, but the repo still has unrelated formatting diffs in `crates/capsem/src/completions.rs`, `crates/capsem/src/support_bundle.rs`, `crates/capsem-service/src/debug_report/tests.rs`, `crates/capsem-service/src/main.rs`, and `crates/capsem-service/src/tests.rs`.
- 2026-05-13: S2 exposed that the harness could capture `/Applications/Capsem.app` but status could not fail on it. Added typed `app_bundle_missing` status coverage for real installed macOS runtimes while skipping dev and install-test isolation paths. Verified with `cargo test -p capsem status::tests -- --nocapture`, `cargo check -p capsem`, and `rustfmt --edition 2021 --check crates/capsem/src/status.rs crates/capsem/src/status/tests.rs`.
- 2026-05-13: Fixed a real install-test harness freshness bug: the simulated install fixture now builds the default local host binaries once per pytest process, compares installed binary contents against `CAPSEM_BIN_SRC`, and reruns `simulate-install.sh` when any helper differs instead of accepting stale existing files. Verified with `uv run pytest tests/capsem-install/test_fixture_refresh.py -q` and `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_capture_records_partial_install_missing_helper -q`.
- 2026-05-13: Extended typed helper-version status checks to `capsem-gateway` and `capsem-tray`. Moved gateway/tray clap parsing ahead of runtime initialization and added `version` metadata so `--version` is side-effect-free. Verified with `target/debug/capsem-gateway --version`, `target/debug/capsem-tray --version`, `cargo test -p capsem host_binary_version_check_reports_stale -- --nocapture`, and `cargo test -p capsem-gateway args_have_sensible_defaults -- --nocapture`.
- 2026-05-13: Started S2 by adding `scripts/capture-install-status.py`, which runs `capsem status --json` and writes a deterministic evidence bundle with status stdout/stderr, parsed status JSON when available, metadata, version output, and a shallow `CAPSEM_HOME` tree. Verified fake-binary failure and invalid-JSON paths with `uv run pytest tests/test_install_status_capture.py -q`.
- 2026-05-13: Extended the status capture bundle with optional `capsem debug` stdout/stderr/JSON capture. Debug command failures are recorded in metadata but the harness still returns the `capsem status --json` exit code. Covered by `uv run pytest tests/test_install_status_capture.py -q`.
- 2026-05-13: Extended the status capture bundle with `run-state.json` for service/gateway pid, socket, and port breadcrumbs while explicitly redacting `gateway.token`. Covered by `uv run pytest tests/test_install_status_capture.py -q`.
- 2026-05-13: Extended the status capture bundle with `install-layout.json`, a focused index of expected helper binaries, asset manifest/signature files, setup state, platform service unit, and the macOS app bundle path. Covered by `uv run pytest tests/test_install_status_capture.py -q`.
- 2026-05-13: Added stale-service-unit evidence coverage: the harness now proves a typed `service_unit_stale_path` status issue is paired with the captured LaunchAgent/systemd unit contents in `install-layout.json`. Covered by `uv run pytest tests/test_install_status_capture.py -q`.
- 2026-05-13: Added `saved-vm-state.json` capture with persistent registry summaries, persistent-session tree evidence, malformed-registry parse errors, and saved-VM env-key-only redaction. Covered by `uv run pytest tests/test_install_status_capture.py -q`.
- 2026-05-13: Added missing-`capsem` capture coverage so partial package failures still produce a bundle with `version` and `status` return code 127. Covered by `uv run pytest tests/test_install_status_capture.py -q`.
- 2026-05-13: Added capture timeout coverage so a hung `capsem status --json` returns 124, marks the status command as timed out in metadata, and preserves stderr. Covered by `uv run pytest tests/test_install_status_capture.py -q`.
- 2026-05-13: Wired status evidence into the real local install gate: `just install` now runs `python3 scripts/capture-install-status.py --capsem-bin "$HOME/.capsem/bin/capsem" --label just-install` after gateway health and before guest DNS/HTTPS. Verified with `uv run pytest tests/test_release_workflow_policy.py::test_local_install_verifies_fresh_install_and_guest_network -q`.
- 2026-05-13: Added the first installed-layout dirty S2 fixture: removing `capsem-service` and running the capture harness now records a `host_binary_missing` issue in `capture.meta.json` and `status.json`. Verified with `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_capture_records_partial_install_missing_helper -q`.
- 2026-05-13: Added a dead-service installed-layout capture fixture: with completed setup state but no daemon, the bundle records `service_not_running`, captures debug command metadata, and preserves run-state evidence for stale/missing service/gateway files. Verified with `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_capture_records_dead_service -q`.
- 2026-05-13: Closed the first app/tray dirty-state capture gap. The harness can now pair a typed `app_bundle_missing` issue with missing app-bundle evidence without mutating `/Applications`, and installed-layout capture proves a missing `capsem-tray` helper is preserved as `host_binary_missing` plus `install-layout.json` evidence. Verified with `uv run pytest tests/test_install_status_capture.py::test_capture_install_status_pairs_app_bundle_issue_with_bundle_state tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_capture_records_missing_tray_helper -q`.
- 2026-05-13: Added black-box reinstall gate coverage for the simulated install path: runtime uninstall followed by reinstall restores all helpers, and reinstalling over a corrupted `capsem-gateway` replaces the helper and clears runtime-layout status blockers. Verified with `uv run pytest tests/capsem-install/test_reinstall.py::TestReinstall::test_reinstall_after_runtime_uninstall_restores_status_layout tests/capsem-install/test_reinstall.py::TestReinstall::test_reinstall_over_existing_replaces_corrupt_helper -q`.
- 2026-05-13: Closed S1/S2 status oracle shape by adding top-level `state` and grouped `checks` to `capsem status --json`; the capture harness now preserves those grouped checks in metadata. Verified with `cargo test -p capsem status::tests -- --nocapture`, the full status JSON install-harness slice, and the S2 capture/reinstall slice.
- 2026-05-13: Closed S2 saved-VM evidence capture by preserving saved-VM asset-reference fields when present, including file-state evidence for referenced asset paths while keeping env values redacted. Actual saved-VM asset preservation enforcement remains S4.
- 2026-05-13: Started S3 and added the first service-owned asset supervisor slice. The service now owns an asset state machine (`checking`, `updating`, `ready`, `error`) with missing-artifact, progress, retry, arch, and version detail; starts the supervisor at daemon startup; re-checks on a timer; downloads missing current-version assets in the background; exposes the same state through service `/list`, legacy `/setup/assets`, gateway `/status`, `capsem status --json`, and frontend runtime types.
- 2026-05-13: Extended the tray consumer audit: the tray now deserializes gateway asset health, shows an asset status row while assets are not ready, and disables New Session instead of treating `updating` as ready. The native Tauri app has no separate status parser; it consumes the frontend gateway model already covered by the frontend runtime-state test.
- 2026-05-13: Closed S3 for the service asset supervisor scope with local release-fixture tests proving the spawned background loop downloads missing current-version assets to `ready`, and a failed release source becomes retryable asset `error` through the real supervisor path.
- 2026-05-14: Implemented S4 saved-VM asset dependencies. Persistent VM entries can now carry base asset identity, forks inherit it, cleanup preserves referenced hash-named blobs, saved-VM resume/clone resolves pinned assets instead of current assets, and missing saved-VM rootfs/kernel/initrd files surface separately from current-version asset readiness.
- 2026-05-14: Wired S4 through status consumers. Service `/list` adds saved-VM dependency issues to asset health, gateway `/status` and frontend/tray types preserve them, the tray shows saved-VM asset gaps without blocking new sessions, and `capsem status --json` emits typed `saved_vm_asset_missing` blockers.
- 2026-05-14: Verified S4 with focused tests: `cargo test -p capsem-core cleanup_preserves_saved_vm --lib`, `cargo test -p capsem-service --bin capsem-service saved_vm`, `cargo test -p capsem-service --bin capsem-service handle_fork_`, `cargo test -p capsem-service registry --lib`, `cargo test -p capsem status::tests -- --nocapture`, `cargo test -p capsem-gateway --bin capsem-gateway fetch_status_preserves_service_asset_state` (escalated for temporary UDS binding), `cargo test -p capsem-tray --bin capsem-tray spec_shows_saved_vm_asset_gap_without_blocking_new_session`, `npx vitest run src/lib/__tests__/session-runtime-truth.test.ts`, and `cargo check -p capsem-core -p capsem-service -p capsem -p capsem-gateway -p capsem-tray`.

## Coverage Ledger

- Unit/contract: `status::tests::doctor_preflight_fails_when_status_has_issues`, `status::tests::doctor_preflight_accepts_clean_status`, `status::tests::status_gate_fails_without_doctor_wording`, `status::tests::health_issue_is_typed_before_rendering`, `status::tests::health_issue_has_stable_machine_identity`, `status::tests::health_issue_report_is_machine_readable`, `status::tests::status_report_contains_service_and_typed_issues`, `status::tests::status_report_groups_issue_codes_by_install_surface`, `status::tests::status_report_preserves_service_asset_updating_state`, `status::tests::status_report_blocks_on_saved_vm_asset_dependencies`, host-binary readiness/version tests including gateway/tray, service-unit tests, setup-state tests, app-bundle tests, asset-manifest tests, signed-manifest rejection tests for status/doctor asset loading, install-fixture freshness tests, uninstall runtime-preservation policy tests, B4 MCP enabled-override/settings-injection tests, S3 asset-supervisor state/progress/error tests, and S4 registry/cleanup/base-asset identity tests; planned for purge policy.
- Functional: parser coverage for `capsem status`, `capsem status --json`, and `capsem doctor`; black-box install harness coverage for `capsem status --json` typed blockers and grouped check states, missing service helper binaries, missing MCP helper binaries, stale process helper version, corrupt setup-state, missing asset manifest, missing canonical rootfs, completed setup-state, and runtime uninstall preserving durable state; S3 has service `/list`, gateway `/status`, CLI status JSON, tray menu, and frontend runtime-type pass-through coverage for asset states; S4 adds service `/list`, gateway `/status`, tray menu, frontend type, and CLI status coverage for saved-VM dependency gaps; planned for setup reruns and provider settings fallback.
- Adversarial: missing binaries, missing tray helper, corrupt setup state, missing manifest, missing rootfs, missing app-bundle evidence, runtime-uninstall preservation, captured partial-install evidence, dead-service evidence, stale service-unit evidence, malformed persistent registry evidence, S3 retryable asset-supervisor error-state coverage, and S4 saved-VM missing-rootfs launch refusal now have focused coverage; planned for bad permissions and unreadable assets.
- E2E/install: simulated reinstall-after-uninstall and reinstall-over-corrupt-helper gates now have black-box coverage; planned for live clean install final gate and true update-over-existing in S7. S4 has local cleanup/startup wiring proof, but the live update-over-existing proof that old saved-VM assets survive package replacement remains part of S7/meta-gate.
- UI/product: B4 has focused frontend interaction coverage for local MCP disable/re-enable; planned for wizard/dashboard/tray/app startup states and retry flows.
- Telemetry/observability: failed-gate evidence bundle exists for `capsem status --json`, grouped status checks, optional `capsem debug`, redacted run-state breadcrumbs, install-layout evidence, app/tray evidence, and saved-VM state plus saved-VM asset-reference fields; it has fake-binary and installed-layout dirty coverage and is wired into `just install`; UI rendering proof lives in S6.
- Performance: not a release blocker unless service asset supervisor introduces startup regressions; measure startup latency if status polling/download supervision becomes heavy.
- Missing/deferred: live sudo-backed `capsem uninstall -> just install -> capsem status` remains the final meta-sprint gate. Deeper wizard/dashboard rendering lives in S6, and true update-over-existing plus destructive whole-product purge live in S7.

## Superseded Work To Reconcile

Earlier in this checkout, a setup patch made asset download failure abort setup.
That patch proved the old chain could lie, but it was not the desired final
architecture. It has been reverted; the replacement behavior belongs to S1/S3/S5.
