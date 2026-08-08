# Plan: 1.3 Main Cleanup

## Why

The current `main` snapshot preserved the 1.3 work, but the first verification pass found contract drift:

- `CHANGELOG.md` says HTTP, DNS, MCP, model, file, process, credential, and snapshot enforcement are unified on the security-event rule engine.
- Runtime code still contains old Policy V2 / `NetworkPolicy` / MCP decision-provider enforcement rails.
- Setup wizard references remain in defaults/docs even though setup authority was removed.
- EROFS build defaults still conflict: approved release default is `lz4hc` level `12`, while `guest/config/build.toml` and docs still say zstd in places.
- Benchmark history needs to be preserved on the docs site. We tested zstd on macOS and Linux and found it was not worth it for this speed-first release; release prep must record that decision with numbers instead of letting it become tribal memory.

This sprint makes `main` clean enough for 1.3 release prep.

## Key Decisions

- Treat current `main` as truth; do not merge old branches.
- Burn old runtime security paths rather than preserving compatibility shims.
- Keep the native security rule authoring surface: `[corp.rules.*]`, `[profiles.rules.*]`, provider convenience `[ai.<provider>.rules.*]`, and `rule_files`.
- Keep detection vectors on `SecurityEvent`: rules and plugins can append multiple `SecurityDetectionEvent` entries.
- Keep PySigma as a facade/import gate over the same native rules.
- Use `lz4hc` level `12` as the EROFS default. Zstd may remain as a supported option, not the default.
- Release process skill and docs benchmark pages must require fresh benchmark artifacts before tagging.

## Implementation Slices

### T0: Changelog And Sprint Truth

- Write sprint artifacts.
- Audit `CHANGELOG.md` claims against code.
- Mark overclaims as blockers or adjust wording only after code reality is known.

Files:

- `sprints/1-3-main-cleanup/*`
- `CHANGELOG.md`

### T1: EROFS, Setup, And Defaults Cleanup

- Change default guest/scaffold/docs examples from zstd to `lz4hc` level `12`.
- Keep zstd tests for optional support where appropriate.
- Remove setup wizard references from defaults, docs, and settings UI text.
- Confirm install flow waits for service/gateway and asset state remains first-class.
- Add plugin policy examples to default user/corp template surfaces.
- Expose plugin policy in the UI with typed select controls for plugin `mode`
  and `detection_level`.

Likely files:

- `guest/config/build.toml`
- `src/capsem/builder/scaffold.py`
- `config/defaults.toml`
- `config/defaults.json`
- `config/user.toml.default`
- `frontend/src/**`
- `docs/src/content/docs/**`
- `skills/release-process/SKILL.md`
- `benchmarks/**`
- `tests/test_config.py`
- `tests/test_validate.py`
- `tests/test_docker.py`

### T2: Single Security Engine Runtime Rail

- Remove old runtime Policy V2 HTTP hook enforcement as a separate evaluator.
- Remove DNS `NetworkPolicy::is_fully_blocked`/Policy V2 decision rail from enforcement and route DNS boundary through `SecurityEvent` + `SecurityRuleSet::evaluate`.
- Remove `LocalMcpDecisionProvider` legacy decisions and evaluate framed MCP request/response boundaries via security engine only.
- Replace model `policy_v2_model::*evaluate*` runtime calls with security-event evaluation.
- Keep protocol parsing in network/file/process engines, but decisions/logging must use the unified security engine.
- Delete stale callback-demux authoring tests or rewrite them to native rule tests.

Likely files:

- `crates/capsem-core/src/security_engine/mod.rs`
- `crates/capsem-core/src/net/mitm_proxy/policy_v2_http_hook.rs`
- `crates/capsem-core/src/net/mitm_proxy/policy_v2_model.rs`
- `crates/capsem-core/src/net/mitm_proxy/mcp_frame.rs`
- `crates/capsem-core/src/net/dns/server.rs`
- `crates/capsem-core/src/net/policy_config/**`
- `crates/capsem-service/src/main.rs`
- `tests/capsem-e2e/**`

### T3: Docs And Release Text

- Rewrite docs to describe the implemented architecture, not the intended one.
- Remove old setup pages or convert them to install/service/assets docs.
- Confirm plugin man pages match the code.
- Confirm the UI exposes plugin policy using enum/select controls for mode and
  detection level.
- Ensure changelog only claims features backed by tests.
- Update the release-process skill so every release run includes benchmark artifact generation and docs benchmark updates.
- Update `docs/src/content/docs/benchmarks/results.md` with current 1.3 benchmark numbers and notes explaining why `lz4hc` level `12` won over zstd on macOS and Linux.

### T4: Verification

- Focused tests after each slice.
- Smoke and full test gates before release handoff.
- If Linux-only KVM/filesystem tests fail on macOS, record exact failure and hand to Linux team Monday.

## Done Means

- `rg "capsem-setup|setup wizard|/setup/"` shows only historical release notes or test names that explicitly prove removal.
- `rg "PolicyV2HttpHook|LocalMcpDecisionProvider|legacy_decision|policy_v2_model::evaluate|NetworkPolicy::is_fully_blocked"` has no runtime enforcement hits.
- EROFS defaults are `lz4hc` level `12`; zstd remains optional only.
- Benchmark docs include the current 1.3 numbers and the zstd rejection note.
- `SecurityEvent` still carries multiple detections and tests prove it.
- Plugin policy appears in default templates/docs and endpoint tests pass.
- Changelog matches implementation.
- Smoke/tests run, with any Linux-only debt explicitly named.

## Proof Matrix

| Slice | Unit/Contract | Functional | Adversarial | E2E/VM | Telemetry/DB | Performance |
| --- | --- | --- | --- | --- | --- | --- |
| T1 setup/assets/defaults | config parser tests, asset status tests | service `/assets/*`, install tests | corrupt setup state remains dead | install smoke | asset status JSON | n/a |
| T2 single engine | security_engine tests, CEL tests | HTTP/DNS/MCP/model evaluate through service/core | deny/ask/rewrite fail closed | focused e2e where feasible | `security_rule_events` rows share event id | security-action bench |
| T3 docs/changelog | link/build docs checks | n/a | stale-term grep | n/a | n/a | n/a |
| T4 gates | cargo/pytest/frontend | `just smoke` | full suite failures triaged | VM smoke | inspect DB where touched | fresh benchmark artifacts + docs results |

## Known Initial Findings

- `SecurityEvent.detections` is implemented as a vector and has tests for rule + plugin detections.
- Plugin endpoints and PySigma fixture tests pass.
- Runtime single-evaluator invariant is not yet true.
- Setup removal is functionally implemented in routes/tests, but stale docs/defaults remain.
- EROFS default is split between `just` lz4hc-12 and config/docs zstd.
- Existing `sprints/kernel-7-erofs-zstd/benchmark-ledger.md` records lz4hc-12 as the local speed winner; the final docs page still needs the release-ready benchmark summary.
