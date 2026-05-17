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

## Change Buckets (Working)
- `keep`: intentional Profile V2 design/implementation and valid test updates
- `drop`: generated artifacts, accidental local outputs, dead-end workaround edits
- `review`: ambiguous test behavior changes (especially skip-based gating)

## Coverage Ledger
- Unit/contract:
  `settings_profiles` core passed 118 matching Rust tests; `policy_confirm` passed 10 matching Rust tests; `capsem-proto` poll tests passed 5 tests; debug report provenance passed 7 focused renderer tests; service vm-effective attachment tests passed 5 focused tests
- Functional:
  `/settings*` service handler and Python integration tests passed for typed settings payload; `/debug/report` handler path passed focused Rust coverage; `/setup/assets` exposes Profile V2 asset-location origins
- Adversarial:
  policy enforcement/redaction test weakenings are blocked as `needs-review`
- E2E/VM or integration:
  NAT/egress skips classified as `needs-review`; VM gates still release-held
- Telemetry/observability:
  debug report now surfaces resolver-trace summary; lifecycle/net telemetry setup changes require split review before port
- Performance:
  generated benchmark outputs classified `drop`
- Missing/deferred:
  committed delta file-level replay, product-code port, and full-gate rerun pending
