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
- [x] Recover focused VM/MITM Profile V2 parity for HTTP/DNS, model, and framed MCP paths
- [x] Restore long-term `just smoke` ordering and Profile V2 VM compatibility gate
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
- VM/MITM parity recovery replaced legacy V1 `security.web.*`/AI allowlist setup in focused e2e tests with Profile V2 effective rules. The process reload path now refreshes running sessions from `vm-effective-settings.toml`, so live MCP/HTTP/DNS/model policy updates use the same Profile V2 source of truth as newly provisioned sessions.
- Profile V2-to-legacy bridge fixes: conditional MCP tool rules and conditional HTTP host rules no longer collapse into broad per-tool/domain allow/block lists; only pure `tool.name == ...`, `request.host == ...`, and `qname == ...` rules feed the builtin fast-path lists.
- Builtin MCP HTTP tools now receive `CAPSEM_DOMAIN_DEFAULT`, preserving `network_egress = ask/block` as default-deny even when allow/block lists are empty.
- Default user profile discovery now resolves under `CAPSEM_HOME`/`HOME` instead of reading a literal `./~/.capsem/profiles` directory, preventing accidental local profile artifacts from contaminating tests or runtime defaults.
- Proof: `cargo check -p capsem-core -p capsem-service -p capsem-process -p capsem-mcp-builtin` passed.
- Proof: `cargo test -p capsem-core settings_profiles --lib` passed 118 focused tests.
- Proof: `cargo test -p capsem-core domain_policy --lib` passed 57 matching tests.
- Proof: `cargo test -p capsem-core mcp_frame --lib` passed 52 matching tests.
- Proof: `cargo test -p capsem-process mcp_runtime` passed 7 focused runtime conversion tests.
- Proof: `uv run python -m py_compile tests/capsem-e2e/test_framed_mcp_mitm.py tests/helpers/service.py` passed.
- Proof: `uv run pytest tests/capsem-e2e/test_framed_mcp_mitm.py -q` passed 15 VM tests.
- Proof: `uv run pytest tests/capsem-e2e/test_policy_v2_http_dns_mitm.py -q` passed 2 VM tests.
- Proof: `uv run pytest tests/capsem-e2e/test_model_policy_mitm.py -q` passed 4 VM tests.
- Smoke restoration used long-term ordering instead of skip-based fixes: `just smoke`/`just test`/`build-ui` now build `frontend/dist` before Rust workspace clippy/test/build phases that compile `capsem-app` and run Tauri `generate_context!`.
- Test-isolated smoke services now bind the gateway to an ephemeral port, and `capsem status`/doctor skip persistent service-unit checks when `CAPSEM_HOME`/`CAPSEM_RUN_DIR` isolation means no installed service unit is required.
- capsem-process now preserves the legacy guest boot contract from an existing isolated `user.toml` while still loading Profile V2 `vm-effective-settings.toml` for rules/MCP defaults. This keeps AI CLI config files, CA env vars, legacy network/domain defaults, and Profile V2 policy attachments coherent during the migration window.
- Gateway MITM telemetry coverage now installs an explicit Profile V2 DNS/HTTP deny fixture instead of depending on the ambient default egress profile. Winter fork smoke keeps the documented compact-image budget (<100 MB actual allocated blocks) rather than a brittle package-cache threshold.
- Proof: `cargo fmt --check` passed.
- Proof: `cargo test -p capsem --bin capsem status::` passed 29 focused status tests.
- Proof: `cargo test -p capsem --bin capsem service_install::` passed 18 focused service-install tests.
- Proof: `cargo test -p capsem-process --bin capsem-process mcp_runtime` passed 8 focused runtime compatibility tests.
- Proof: `cargo clippy -p capsem-core --tests -- -D warnings` passed.
- Proof: `cargo clippy -p capsem-service --all-targets -- -D warnings` passed.
- Proof: `uv run pytest tests/capsem-gateway/test_mitm_policy.py::test_mitm_policy_telemetry -q` passed.
- Proof: `uv run pytest tests/capsem-mcp/test_winter_is_coming.py::test_winter_is_coming -q` passed.
- Proof: `rm -rf frontend/dist && just smoke` passed in 229s, including doctor (`307 passed, 4 skipped, 1 deselected`), injection (`5 passed`), integration diagnostics (`94 passed, 2 skipped`) and telemetry audit (`40 passed, 3 warnings`), Python gateway/MCP/service/CLI groups (`91 passed`, `62 passed, 50 skipped, 20 deselected`, `140 passed, 5 skipped`), state transitions (`12 passed`), and resume/suspend durability (`7 passed`).

## Change Buckets (Working)
- `keep`: intentional Profile V2 design/implementation and valid test updates
- `drop`: generated artifacts, accidental local outputs, dead-end workaround edits
- `review`: ambiguous test behavior changes (especially skip-based gating)

## Coverage Ledger
- Unit/contract:
  `settings_profiles` core passed 118 matching Rust tests; `policy_confirm` passed 10 matching Rust tests; `capsem-proto` poll tests passed 5 tests; debug report provenance passed 7 focused renderer tests; service vm-effective attachment tests passed 5 focused tests; framed MCP Policy V2 confirmation passed 52 focused `mcp_frame` tests; HTTP Policy V2 confirmation passed 9 hook tests and 14 focused HTTP Policy V2 tests; model Policy V2 confirmation/rewrite passed 32 focused tests; policy condition allowlist accepts documented `request.data`; capsem-process runtime conversion passed 7 focused tests and 97 full package tests; domain policy/default-env behavior passed 57 matching core tests; capsem-gateway passed 156 Rust tests
- Functional:
  `/settings*` service handler and Python integration tests passed for typed settings payload; `/setup/corp-config` installs Profile V2 corp profile TOML and leaves `/settings` typed/readable; `/debug/report` handler path passed focused Rust coverage; `/setup/assets` exposes Profile V2 asset-location origins; capsem-process consumes attached effective policy state and reloads running sessions from it; framed MCP request/response `ask` decisions route through confirmer resolution before dispatch/response handling; HTTP request/response `ask` decisions route through confirmer resolution before upstream dispatch/guest response surfacing; model request, model response, tool-call, and tool-response `ask` decisions route through confirmer resolution before upstream or guest delivery; model request rewrite forwards redacted bytes upstream before telemetry records the request preview; gateway status/proxy non-VM Python tests passed
- Adversarial:
  policy enforcement/redaction test weakenings are blocked as `needs-review`; MCP confirmation snapshots are covered for argument-value redaction in focused unit tests; HTTP confirmation snapshots are covered for no request-header exposure in focused unit tests; model confirmation snapshots are covered for request-body, response-text, tool-argument, and tool-response redaction; model request rewrite fails closed for unsupported targets, no regex match, and non-UTF-8 bodies
- E2E/VM or integration:
  Focused VM/MITM suites passed for framed MCP (15), HTTP/DNS Policy V2 (2), and model Policy V2 (4). Full `just smoke` passed after ordering/runtime rescue. NAT/egress skips classified as `needs-review`; full `just test` remains pending for final release confidence.
- Telemetry/observability:
  debug report now surfaces resolver-trace summary; `just smoke` telemetry audit passed (`40 passed`, `3 warnings` for missing live Gemini key); lifecycle/net telemetry setup changes require split review before port
- Performance:
  generated benchmark outputs classified `drop`
- Missing/deferred:
  Full `just test` and ambiguous environment skips remain unaccepted until separately reviewed.
