# Profile V2 Migration Rescue Manifest

Last updated: 2026-05-17

## Baselines

- Clean branch: `profile-v2`
- Clean baseline: `origin/main` at `dc137f99` (`release: v1.1.1778860037`)
- Source rescue worktree: `/Users/elie/.codex/worktrees/3d94/capsem`
- Source rescue commit: `b3862ae7` (`origin/claude/adoring-joliot-98a4cb`)
- Source rescue state: detached `HEAD`, no staged changes, dirty overlay present
- Merge base between `origin/main` and `b3862ae7`: `c7cac375a9e2a1c2639c49d4485945a13a6c3837`

## Branch Decision

`profile-v2` is the only migration branch for this rescue. The source line is reference-only. Do not port by wholesale cherry-pick because `origin/main..b3862ae7` spans a large divergent line with Profile V2 work mixed with release, debug, install, benchmark, and generated-artifact churn.

## Dirty Overlay Inventory

These are the uncommitted changes on top of source commit `b3862ae7`.

| Path | Bucket | Rationale | Action |
| --- | --- | --- | --- |
| `benchmarks/parallel/data_1.0.json` | drop | Generated benchmark output with local timestamps, VM ids, and environment-dependent failures. | Do not port. |
| `crates/capsem-core/src/net/policy_confirm/tests.rs` | keep | Adjusts tests for unit-like `PlaceholderConfirmer` construction. Tied to S06-pre confirmer work. | Port only with the policy confirm slice. |
| `crates/capsem-core/src/settings_profiles/corp/tests.rs` | keep | Keeps tests compiling after `ServiceSettings` grows non-profile fields by using struct update syntax. | Port with settings profile core tests. |
| `crates/capsem-core/src/settings_profiles/mod.rs` | keep | Removes manual defaults, fixes derived DNS provider conditions to use `qname`, and simplifies callback validation. These are Profile V2 correctness/maintenance fixes. | Port with settings profile core. |
| `crates/capsem-service/src/debug_report/tests.rs` | keep | Keeps debug report profile trace test compiling after `ServiceSettings` shape changes. | Ported with debug report/status provenance slice; focused renderer tests passed. |
| `crates/capsem-service/src/main.rs` | keep | Removes a redundant `.into_iter()` from registry listing count; likely compatibility cleanup after registry iterator changes. | Port with service runtime reconciliation after confirming current API shape. |
| `frontend/package.json` | needs-review | Dependency/security bump (`astro`, `svelte`, `devalue`) is not Profile V2-specific. | Do not include in first Profile V2 port; route through frontend/security verification if needed. |
| `frontend/pnpm-lock.yaml` | needs-review | Lockfile companion to frontend dependency bump. | Keep paired with `frontend/package.json` only if explicitly accepted. |
| `tests/capsem-e2e/test_e2e_lifecycle.py` | needs-review | Adds environment skip for doctor NAT/iptables failures. It may be valid capability gating, but it weakens an E2E gate. | Review separately with explicit skip rationale. |
| `tests/capsem-e2e/test_framed_mcp_mitm.py` | needs-review | Contains valid `effective_rules` response-shape updates, but also broadens assertions from enforced block/ask behavior to allow/audit-only paths. | Split: keep schema updates only, re-test behavior expectations before porting skips/loosened assertions. |
| `tests/capsem-e2e/test_model_policy_mitm.py` | needs-review | Contains useful `effective_rules` and `--resolve` harness updates, but changes ask/rewrite expectations from policy denial/redaction to upstream `401` and visible request data. | Split and revalidate security expectations before porting. |
| `tests/capsem-e2e/test_policy_v2_http_dns_mitm.py` | needs-review | Contains useful `effective_rules` and `--resolve` harness updates, but removes header-strip and response-strip coverage. | Do not port coverage removal without a replacement test. |
| `tests/capsem-gateway/test_mitm_policy.py` | needs-review | Adds NAT-table capability skip. | Review as environment gate; must stay tagged as coverage debt if accepted. |
| `tests/capsem-guest/test_guest_network.py` | needs-review | Adds iptables NAT fallback and skip. | Review as environment gate; must stay tagged as coverage debt if accepted. |
| `tests/capsem-mcp/test_winter_is_coming.py` | needs-review | Adds egress-dependent apt skip. | Review as environment gate; not Profile V2-specific. |
| `tests/capsem-serial/test_lifecycle_benchmark.py` | needs-review | Adds egress-dependent apt skip. | Review as environment gate; not Profile V2-specific. |
| `tests/capsem-session-lifecycle/test_multiple_events.py` | needs-review | Replaces legacy settings allowlist with policy-v2 allow rule, but also adds NAT telemetry skip. | Split: keep policy-v2 setup after core port; review skip separately. |

## Untracked Source Worktree Files

| Path | Bucket | Rationale | Action |
| --- | --- | --- | --- |
| `.coverage.Saphyr_localdomain.pid7572.X7cTQdsx.HUvjlFKdkswh` | drop | Generated coverage data. | Do not port. |
| `.coverage.Saphyr_localdomain.pid7573.XYLKGEVx.Hvc6uEVQQ2dh` | drop | Generated coverage data. | Do not port. |
| `.coverage.Saphyr_localdomain.pid7574.XSkDJmOx.H5tx16iHGFSh` | drop | Generated coverage data. | Do not port. |
| `benchmarks/fork/data_1.1.1778542197.json` | drop | Generated benchmark output. | Do not port. |
| `benchmarks/lifecycle/data_1.1.1778542197.json` | drop | Generated benchmark output. | Do not port. |
| `sprints/policy-settings-profiles/` | keep | Authoritative Profile V2 design and sprint corpus. | Copied to `profile-v2`. |
| `sprints/profile-v2-migration-rescue/` | keep | Rescue control sprint. | Copied to `profile-v2` and updated. |
| `sprints/profile-v2-test-fix/` | keep | Adjacent triage context needed for verification recovery. | Copied to `profile-v2`. |
| `~/.capsem/profiles/everyday-work.toml` | drop | Local user profile output under an accidental tracked-looking `~/` path. | Do not port. |

## Committed Delta Inventory

`origin/main..b3862ae7` contains 217 changed paths with 19,530 insertions and 24,816 deletions. It includes the Profile V2 implementation line, but also deletes or edits release, install, debug-report, benchmark, workflow, and generated/release files. Treat this delta as a source corpus, not as one patch.

Initial grouping:

| Domain | Bucket | Rationale | Action |
| --- | --- | --- | --- |
| `sprints/policy-settings-profiles/**` | keep | Product design, requirements, tracker, and S00-S19 execution notes. | Copied and committed in context checkpoint. |
| `crates/capsem-core/src/settings_profiles/**` | keep | Core typed service/profile model, resolver, trace, corp directives, rule ownership. | Ported as first product-code slice; `cargo test -p capsem-core settings_profiles` passed. |
| `crates/capsem-core/src/net/policy_confirm.rs` and tests | keep | S06-pre confirmation contract. | Ported with `RetryOpts: Clone` support; `cargo test -p capsem-core policy_confirm` passed. |
| `crates/capsem-core/src/net/mitm_proxy/**` policy-v2 changes | keep | HTTP/model/MCP policy enforcement, rewrite, ask confirmation, and telemetry behavior. | MCP, HTTP, and model `ask` confirmation plus model request rewrite are ported with focused tests; focused VM/MITM Profile V2 suites now pass for framed MCP, HTTP/DNS, and model paths. |
| `crates/capsem-service/**`, `crates/capsem/src/**`, `crates/capsem-process/**`, `crates/capsem-gateway/**` | needs-review | Mixes real Profile V2 service/runtime integration with debug-report, status, asset, install, and IPC changes. | `/settings*`, debug-report provenance, service asset-location startup, default VM sizing, `/setup/assets` provenance, vm-effective session attachments, capsem-process vm-effective consumption/reload, MCP/HTTP/model confirmer integration, model rewrite, Profile V2 corp-config install, non-VM gateway status/proxy parity, and focused VM/MITM Profile V2 parity ported/tested. |
| `frontend/**` | needs-review | Mixes settings/profile UI model changes with dependency and test churn. | Defer until backend contracts are stable. |
| `tests/**` | needs-review | Contains needed contract/E2E coverage and some drift/skips. | Port only tests that prove retained behavior. |
| `.github/**`, release docs, `LATEST_RELEASE.md`, `B3SUMS`, benchmark JSON, generated artifacts | drop unless separately justified | Mostly release-line and generated artifact churn unrelated to Profile V2 rescue. | Do not port in Profile V2 branch. |

## Migration Commit Sequence

1. Context preservation: sprint docs and this manifest only.
2. Core Profile V2 model: `settings_profiles` module and unit/contract tests.
3. Service settings runtime: service API, debug provenance, asset-location startup, default VM sizing, VM effective settings attachments, and process-side effective policy consumption ported.
4. MCP/HTTP/model policy runtime: framed MITM MCP `ask`, HTTP head-hook `ask`, model request/response/tool `ask` confirmation, and model request rewrite ported with focused tests.
5. Policy runtime: focused VM/MITM Profile V2 parity for framed MCP, HTTP/DNS, and model paths.
6. Test recovery: focused unit/contract tests first, then service/gateway integration, then E2E/VM gates.
7. Frontend/CLI/docs: only after backend contracts are verified.

## Verification Gates

- Context preservation: `git status --short`, document inventory review.
- Core model: `cargo test -p capsem-core settings_profiles`.
- Policy runtime: focused `capsem-core` policy tests plus gateway/service Python tests that do not require VM networking.
- E2E/VM: focused Profile V2 VM/MITM suites pass; broad gates and any skips must remain review-tagged until separately accepted.

## Active Holds

- Migration is not complete until committed-delta file-level replay decisions are made as each slice is ported.
- Verification is not restored until the remaining broad gates pass on `profile-v2`.
- Test edits that weaken policy enforcement or redaction expectations are blocked until revalidated against product intent.
