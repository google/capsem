# Sprint: profile-v2-migration-rescue

## Where This Sprint Lives
- Repository: `/Users/elie/.codex/worktrees/824d/capsem`
- Migration branch: `profile-v2`
- Clean baseline: `origin/main` at `dc137f99`
- Source rescue repository: `/Users/elie/.codex/worktrees/3d94/capsem`
- Source rescue git state: detached `HEAD` at `origin/claude/adoring-joliot-98a4cb`
- Source rescue commit pin: `b3862ae7`
- Concurrency: none (single operator, single sprint)

## Tasks
- [x] Create migration sprint scaffolding (`plan.md`, `tracker.md`, `MASTER.md`)
- [x] Create branch `profile-v2` from `origin/main`
- [x] Preserve Profile V2 sprint corpus and adjacent triage docs on clean branch
- [x] Capture dirty overlay inventory and classify `keep/drop/review`
- [x] Produce rescue manifest linking changed files to intent and action
- [x] Separate generated artifacts/noise from source changes
- [x] Define migration commit sequence (context/docs -> code -> tests)
- [x] Execute first reconciliation pass on highest-risk files (`settings_profiles` core)
- [x] Re-run targeted verification gate for `settings_profiles` core
- [x] Port `policy_confirm` confirmation contract and targeted tests
- [x] Port `/settings*` service endpoint surface to typed settings-profiles payload
- [x] Port debug-report settings/profile provenance without regressing main's rich debug schema
- [x] Port service runtime Profile V2 asset locations, VM defaults, and vm-effective attachments
- [x] Port capsem-process runtime consumption of vm-effective settings
- [x] Port MCP Policy V2 `ask` confirmation resolution in the framed MITM path
- [x] Port HTTP Policy V2 `ask` confirmation resolution in the MITM hook path
- [x] Port model Policy V2 `ask` confirmation resolution in MITM model request/response paths
- [x] Port model Policy V2 `model.request` rewrite support and redacted upstream dispatch
- [x] Port Profile V2 corp-config install path and verify non-VM gateway parity
- [ ] Publish migration TL;DR and residual risk list

## Notes
- Situation: large volume of Profile V2 work is mixed with triage edits and generated files.
- Guardrail: preserve design context before code cleanup.
- Guardrail: no concurrent sprint, no hidden side branches for this phase.
- Branch created from `origin/main` at `dc137f99`; source line remains reference-only.
- Dirty overlay manifest: `sprints/profile-v2-migration-rescue/rescue-manifest.md`.
- `origin/main..b3862ae7` is too mixed for wholesale cherry-pick; replay by slice.
- Core `settings_profiles` module ported as the first product slice.
- Proof: `cargo test -p capsem-core settings_profiles` passed 118 matching tests.
- `policy_confirm` requires the source-line `RetryOpts: Clone` support change in `capsem-proto`; ported with the confirmation slice because `poll_until` consumes retry options.
- Proof: `cargo test -p capsem-core policy_confirm` passed 10 matching tests.
- Proof: `cargo test -p capsem-proto poll` passed 5 matching tests.
- Service `/settings*` now returns `settings_profiles_v2`, profile presets, and effective rules. The handler keeps temporary validation through the old policy-config grammar where compatible, but bridges `model.request` + `request.data` until the S06a runtime slice lands.
- Proof: `cargo test -p capsem-service settings` passed 7 focused tests.
- Proof: `cargo test -p capsem-service handle_` passed 24 handler tests.
- Proof: `uv run pytest tests/capsem-service/test_svc_settings.py -q` passed 10 tests after building `capsem-process`, `capsem-service`, `capsem-gateway`, and `capsem-tray`.
- Debug report provenance port was additive: kept main's status/setup/host/assets/log JSON report intact and added a redacted `[settings_profiles]` text section plus resolved asset-location origins.
- Proof: `cargo test -p capsem-service debug_report --lib` passed 7 focused renderer tests.
- Proof: `cargo test -p capsem-service handle_debug_report_returns_pasteable_text` passed the service handler path.
- Proof: `cargo fmt --check` passed.
- Service runtime now loads `service.toml` at startup for asset-location resolution, exposes those origins on `/setup/assets`, resolves omitted VM RAM/CPU from the default Profile V2 VM settings, and writes coherent `vm-effective-settings.toml` + `vm-effective-trace.json` into provisioned/resumed/forked session directories.
- Runtime port preserved main's asset-supervisor and saved-VM base-asset dependency behavior instead of replaying source-line deletions.
- Proof: `cargo test -p capsem-service vm_effective` passed 5 focused attachment tests.
- Proof: `cargo test -p capsem-service startup_` passed 5 startup/manifest tests.
- Proof: `cargo test -p capsem-service handle_asset_status_exposes_service_asset_locations` passed.
- Proof: `cargo test -p capsem-service settings` passed 15 focused settings/runtime tests.
- capsem-process now builds runtime network defaults, MCP defaults/tool decisions, and Policy V2 rules from the session-attached `vm-effective-settings.toml`, with a default-profile fallback when attachments are missing/corrupt.
- Proof: `cargo test -p capsem-process mcp_runtime` passed 7 focused conversion tests.
- Proof: `cargo test -p capsem-process` passed 97 tests.
- Framed MCP Policy V2 `ask` decisions now resolve through the shared confirmer/backoff contract before request dispatch and before response surfacing. Placeholder confirmation preserves current allow-by-default behavior; focused mock-confirmer tests cover deny mapping, response asks, canonical rule ids, and redacted snapshots.
- Proof: `cargo test -p capsem-core mcp_frame` passed 52 matching tests.
- Proof: `cargo test -p capsem-core policy_confirm` passed 10 matching tests after the MCP integration.
- Proof: `cargo test -p capsem-process` passed 97 tests after the MCP endpoint constructor change.
- HTTP Policy V2 `ask` decisions now resolve through the shared confirmer/backoff contract in the MITM head hook for both `http.request` and `http.response`. Placeholder confirmation keeps current allow-by-default runtime behavior, while mock-confirmer tests lock deny mapping and no-header snapshot exposure.
- Proof: `cargo test -p capsem-core policy_v2_http_hook` passed 9 focused hook tests.
- Proof: `cargo test -p capsem-core policy_v2_http_ask_placeholder_confirmer_allows_upstream_dispatch` passed the full MITM fixture path.
- Proof: `cargo test -p capsem-core policy_v2_http` passed 14 focused HTTP Policy V2 tests.
- Model Policy V2 `ask` decisions now resolve through the shared confirmer/backoff contract before model request dispatch, model response surfacing, tool-call delivery, and tool-response forwarding. Placeholder confirmation preserves current allow-by-default runtime behavior; mock-confirmer tests cover deny mapping, canonical rule ids, and redacted metadata-only snapshots that omit request bodies, response text, tool arguments, and tool-response content.
- Proof: `cargo test -p capsem-core policy_v2_model` passed 28 focused model Policy V2 tests.
- Proof: `cargo test -p capsem-core policy_confirm` passed 10 confirmation-contract tests after model integration.
- Proof: `cargo test -p capsem-process` passed 97 tests after the MITM proxy constructor change.
- Proof: `cargo fmt --check` and `git diff --check` passed.
- Model Policy V2 `model.request` rewrite now rewrites outbound request bodies before upstream dispatch. `request.data` is accepted in validated model-request conditions and rewrite targets, while the current `request.body` spelling remains a runtime compatibility alias. Fail-closed coverage rejects unsupported rewrite targets, non-matching rewrite regexes, and non-UTF-8 request bodies.
- Proof: `cargo test -p capsem-core policy_v2_model` passed 32 focused model Policy V2 tests, including the full MITM rewrite fixture that verifies redacted upstream dispatch and redacted telemetry.
- Proof: `cargo test -p capsem-core policy_v2_accepts_documented_cel_condition_shapes` passed the documented condition allowlist test for `request.data`.
- Proof: `cargo fmt --check` passed after the rewrite slice.
- `/setup/corp-config` now installs Profile V2 corp profile TOML for inline and URL payloads through `settings_profiles::install_corp_profile_toml`, so the typed `/settings` response remains readable after corp profile installation.
- Gateway Rust status/proxy behavior was retained from main rather than replaying source-line deletions of richer asset-health fields. Non-VM gateway parity is verified through Rust unit tests plus Python status/proxy gateway tests.
- Proof: `cargo test -p capsem-gateway` passed 156 tests.
- Proof: `cargo test -p capsem-service settings` passed 15 focused service settings/debug/vm-effective tests.
- Proof: `uv run pytest tests/capsem-service/test_svc_setup.py tests/capsem-service/test_svc_settings.py tests/capsem-service/test_svc_mcp_api.py::TestMcpPolicy::test_policy_returns_merged_shape -q` passed 19 tests.
- Proof: `uv run pytest tests/capsem-gateway/test_gw_status.py tests/capsem-gateway/test_gw_status_advanced.py tests/capsem-gateway/test_gw_proxy.py -q` passed 19 tests.
- Remaining VM-dependent proof: `uv run pytest tests/capsem-service/test_svc_setup.py tests/capsem-service/test_svc_mcp_api.py tests/capsem-service/test_svc_settings.py -q` reached 23 passing tests but `TestMcpCall.test_call_unknown_tool_with_running_vm_rejected` timed out waiting for exec-ready; keep under VM gate debt, not a policy-runtime regression.

## Change Buckets (Working)
- `keep`: intentional Profile V2 design/implementation and valid test updates
- `drop`: generated artifacts, accidental local outputs, dead-end workaround edits
- `review`: ambiguous test behavior changes (especially skip-based gating)

## Coverage Ledger
- Unit/contract:
  `settings_profiles` core passed 118 matching Rust tests; `policy_confirm` passed 10 matching Rust tests; `capsem-proto` poll tests passed 5 tests; debug report provenance passed 7 focused renderer tests; service vm-effective attachment tests passed 5 focused tests; framed MCP Policy V2 confirmation passed 52 focused `mcp_frame` tests; HTTP Policy V2 confirmation passed 9 hook tests and 14 focused HTTP Policy V2 tests; model Policy V2 confirmation/rewrite passed 32 focused tests; policy condition allowlist accepts documented `request.data`; capsem-process runtime conversion passed 7 focused tests and 97 full package tests; capsem-gateway passed 156 Rust tests
- Functional:
  `/settings*` service handler and Python integration tests passed for typed settings payload; `/setup/corp-config` installs Profile V2 corp profile TOML and leaves `/settings` typed/readable; `/debug/report` handler path passed focused Rust coverage; `/setup/assets` exposes Profile V2 asset-location origins; capsem-process consumes attached effective policy state; framed MCP request/response `ask` decisions route through confirmer resolution before dispatch/response handling; HTTP request/response `ask` decisions route through confirmer resolution before upstream dispatch/guest response surfacing; model request, model response, tool-call, and tool-response `ask` decisions route through confirmer resolution before upstream or guest delivery; model request rewrite forwards redacted bytes upstream before telemetry records the request preview; gateway status/proxy non-VM Python tests passed
- Adversarial:
  policy enforcement/redaction test weakenings are blocked as `needs-review`; MCP confirmation snapshots are covered for argument-value redaction in focused unit tests; HTTP confirmation snapshots are covered for no request-header exposure in focused unit tests; model confirmation snapshots are covered for request-body, response-text, tool-argument, and tool-response redaction; model request rewrite fails closed for unsupported targets, no regex match, and non-UTF-8 bodies
- E2E/VM or integration:
  NAT/egress skips classified as `needs-review`; VM gates still release-held; one MCP service VM-call test currently times out waiting for exec-ready and remains VM-gate debt
- Telemetry/observability:
  debug report now surfaces resolver-trace summary; lifecycle/net telemetry setup changes require split review before port
- Performance:
  generated benchmark outputs classified `drop`
- Missing/deferred:
  gateway VM/MITM policy telemetry replay, E2E/VM gates, and full-gate rerun pending
