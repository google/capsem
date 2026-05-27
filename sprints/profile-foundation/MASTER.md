# Profile Foundation Sprint

Last updated: 2026-05-27

## Mission

Finish the Profile V2 foundations so future work builds on stable product,
security, telemetry, and extension contracts instead of reopening architecture.

This is the active post-ship Profile V2 meta sprint. The old
`policy-settings-profiles` S-numbered board is historical evidence and detailed
source material. Foundation sub-sprints use F-numbering for the execution order
we trust now.

We are done with foundations when:

- installed Profile V2 behavior is proved from package to VM startup;
- Security Events and Resolved Security Events are the canonical runtime and
  session journal for every shipped event family;
- engine boundaries, policy packs, detection, ask/confirm, credentials,
  metrics, reporting, timeline/workbench, plugins, local providers,
  OpenAPI-to-MCP, and quotas have named contracts and proof;
- docs, status, debug, and UI can explain the same truth without fallback
  wording or hidden "later" buckets.

## Code Reality Check

Code checked on 2026-05-27 from branch `profile-v2` in
`/Users/elie/.codex/worktrees/824d/capsem`.

Focused command:

```bash
cargo test -p capsem-security-engine -p capsem-network-engine -p capsem-file-engine -p capsem-process-engine -p capsem-logger --lib
```

Result: passed.

What this proves:

- `capsem-security-engine`: 41 tests passed. Security event schema,
  resolved-event schema, CEL enforcement/detection, ask default-deny,
  throttle action roundtrip, plugin transform validation, runtime rule
  registry, canonical AI evidence, host-vs-VM accounting, and emitter sink
  behavior compile and pass focused unit coverage.
- `capsem-logger`: 114 tests passed. The canonical `security_events`,
  `security_event_steps`, `detection_findings`, finding tags, event links, and
  live in-memory VM metrics snapshot paths compile and pass focused unit
  coverage.
- `capsem-network-engine`: 241 tests passed. DNS/HTTP/MCP/model evidence,
  provider parsing, SSE, model tool evidence, host attribution, and runtime
  security projections compile and pass focused unit coverage.
- `capsem-file-engine`: 4 tests passed. File security event construction,
  classification, and same-millisecond event id separation pass.
- `capsem-process-engine`: 5 tests passed. Process security event construction,
  command classification, CEL blocking, and missing-confirm default-deny pass.

What this does not prove yet:

- installed package behavior;
- full service/process/gateway runtime dispatch across every event family;
- real VM end-to-end session journal parity;
- production ask/confirm UI;
- credential brokerage into sessions;
- remote/WASM plugin execution;
- OTel/export/reporting packaging;
- workbench timeline UX;
- quota enforcement.

## Renaming Crosswalk

| Old Board | Foundation Name | New Owner |
| --- | --- | --- |
| S18, `release-hit-list.md` | Installed product baseline | F00, F01 |
| S08b, S08 side evidence | Security event system | F02, F03 |
| S08c, S08d | Rule packs, detection, benchmarks | F04 |
| S09, S16 | Product surface polish | F01, F05 |
| S10, `credential-pipeline` | Credential brokerage and Google account integration | F06 |
| S11, S12, S19b | Dashboard, status, metrics, OpenTelemetry, reporting, remote alert logging | F07 |
| S14, S15, S17 | Rules, ask, capabilities UX | F05 |
| S16a | Timeline and workbench | F08 |
| S13, S23 plugins | Security plugin system, remote decisions, observer alerts | F09 |
| S20, S21, Google/Gemini providers | Product integrations | F10 |
| S22 | Quotas, budgets, rate limits | F11 |
| S19, S19a | Docs, site, release story | F12 |
| S24 | Meta sprint glue | This board |

## Execution Order

The order is intentional: prove what shipped, lock the event ledger, wire every
runtime into that ledger, then layer product and extension systems on top.

| # | Foundation Sprint | Status | Purpose | Old Sources |
| --- | --- | --- | --- | --- |
| 0 | [F00 - Code Reality And Baseline](F00-code-reality-and-baseline.md) | Active | Keep code checks, installed state, branch/worktree, and known gaps current before implementation starts. | S18, S24 |
| 1 | [F01 - Installed Profile Product Proof](F01-installed-profile-product-proof.md) | Not Started | Prove package install, app startup, Settings Profiles, dashboard cards, CLI run/shell, repeated install coherence, and profile provisioning truth. | `release-hit-list.md`, S16, S18 |
| 2 | [F02 - Security Event Contract Closure](F02-security-event-contract-closure.md) | Not Started | Freeze SecurityEvent/ResolvedSecurityEvent schemas, plugin transform records, quota dimensions, pack identity, redaction, and compatibility fixtures. | S08b, S08 side evidence, S23 |
| 3 | [F03 - Runtime Engine And Journal Wiring](F03-runtime-engine-and-journal-wiring.md) | Not Started | Wire network/file/process/model/MCP/profile/conversation/snapshot events through the Security Engine and canonical session journal. | S08b, S11 |
| 4 | [F04 - Policy Packs Detection And Benchmarks](F04-policy-packs-detection-and-benchmarks.md) | Not Started | Close CEL enforcement, detection packs, backtest/hunt, corpus parity, and benchmark/release artifact proof. | S08a, S08c, S08d |
| 5 | [F05 - Rules Confirm And Capability UX](F05-rules-confirm-capability-ux.md) | Not Started | Build rule editors, ask/confirm resolver UX, CLI parity, capability controls, and detection finding/backtest views. | S14, S15, S17 |
| 6 | [F06 - Credential Brokerage Foundation](F06-credential-brokerage-foundation.md) | Not Started | Broker credentials from service/profile settings into sessions with audit, policy, UI/status, source discovery handoff, and Google account integration for Drive/Gemini/Google-backed providers. | S10, `credential-pipeline` |
| 7 | [F07 - Metrics Status And Reporting Foundation](F07-metrics-status-reporting-foundation.md) | Not Started | Finish dashboard improvements, live metrics, OTel/export surfaces, status/debug truth, remote alert logging, dashboard counters, and reporting setup. | S11, S12, S19b |
| 8 | [F08 - Timeline And Workbench Foundation](F08-timeline-workbench-foundation.md) | Not Started | Define conversation/timeline engine and everyday-work review UI over canonical resolved events. | S16a |
| 9 | [F09 - Plugin System Foundation](F09-plugin-system-foundation.md) | Not Started | Implement deterministic signed security plugins, remote/WASM enforcement decisions, observer extensions, and remote alert event contracts. | S13, S23 |
| 10 | [F10 - Product Integration Foundation](F10-product-integration-foundation.md) | Not Started | Bring OpenAPI-to-MCP, Local LLM, and deeper Google/Gemini provider integration under profile-owned security, diagnostics, audit, and UI contracts. | S20, S21, Google/Gemini |
| 11 | [F11 - Quotas Budgets And Rate Limits](F11-quotas-budgets-rate-limits.md) | Not Started | Implement quota dimensions into enforceable HTTP/MCP/model/token/cost/request limits with UI/status/docs. | S22 |
| 12 | [F12 - Docs Site And Foundation Release](F12-docs-site-foundation-release.md) | Not Started | Align docs/site/marketing/release notes with proved foundation capabilities and final gates. | S19, S19a, release process |

## Current Trust Position

Trusted as implemented enough to build on:

- typed Profile V2 service/profile surfaces and profile-backed VM settings;
- SecurityEvent and ResolvedSecurityEvent core Rust types;
- canonical event families for DNS, HTTP, MCP, model, file, process,
  credential, VM, profile, conversation, and snapshot;
- CEL enforcement/detection runtime primitives;
- logger schema and writer path for canonical security event tables;
- focused unit coverage across the security/event foundation crates.

Not trusted until Foundation proves it:

- installed product flow after package install;
- full service/process/gateway runtime use of the canonical journal;
- security plugin execution beyond transform validation primitives;
- remote decision and remote alert logging paths;
- production ask/confirm;
- credential release into sessions;
- Google deep integration beyond first-slice account brokerage and Gemini proof;
- live metrics/export/reporting completeness;
- timeline/workbench product workflow;
- quotas/budgets enforcement.

## Global Acceptance Gates

- `git diff --check`
- focused crate tests for any modified foundation crate
- frontend checks for UI sub-sprints
- installed package proof for F01 and final F12
- real VM proof for runtime, credentials, metrics, timeline, plugin, and quota
  paths that cross the VM boundary
- `just smoke` or an explicitly documented narrower release replay only at the
  final Foundation gate
