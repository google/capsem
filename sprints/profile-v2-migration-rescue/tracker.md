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

## Change Buckets (Working)
- `keep`: intentional Profile V2 design/implementation and valid test updates
- `drop`: generated artifacts, accidental local outputs, dead-end workaround edits
- `review`: ambiguous test behavior changes (especially skip-based gating)

## Coverage Ledger
- Unit/contract:
  `settings_profiles` core passed 118 matching Rust tests; `policy_confirm` passed 10 matching Rust tests; `capsem-proto` poll tests passed 5 tests; debug report tests pending later slices
- Functional:
  `/settings*` service handler and Python integration tests passed for typed settings payload
- Adversarial:
  policy enforcement/redaction test weakenings are blocked as `needs-review`
- E2E/VM or integration:
  NAT/egress skips classified as `needs-review`; VM gates still release-held
- Telemetry/observability:
  lifecycle/net telemetry setup changes require split review before port
- Performance:
  generated benchmark outputs classified `drop`
- Missing/deferred:
  committed delta file-level replay, product-code port, and full-gate rerun pending
