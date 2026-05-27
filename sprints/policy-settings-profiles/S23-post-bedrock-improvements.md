# S23 - Post-Bedrock Improvements

## Status

Proposed later sprint. This is not rescue work and it is not a license to keep
reshaping the Profile V2 bedrock after release.

## Goal

Add product capabilities after the engine/profile/API/UI contract stands.

The bedrock release freezes:

- Network Engine, File Engine, Process Engine, Security Engine, and Resolved
  Event Emitter boundaries;
- canonical `SecurityEvent` / `ResolvedSecurityEvent` identity and journal
  semantics;
- canonical policy roots for CEL and detection lowering;
- profile-owned enforcement and detection packs;
- runtime `/enforcement/*` and `/detection/*` route families;
- profile catalog/revision/pin semantics;
- CLI and UI contract shapes for operating the engine.

S23 work must extend those terms through documented extension points. It must
not rename the roots, rebuild the journal model, split profile semantics again,
or introduce a second policy/event authority.

## Improvement Lanes

- Remote enforcement and observer plugins from S13, including signed plugin
  bundles and deterministic `SecurityEvent -> SecurityEvent` transforms.
- Richer workbench and timeline experiences from S16a/S17 after the single
  structured timeline endpoint exists.
- Marketing/site polish from S19a after S08d/S18 produce release artifacts.
- OpenAPI-to-MCP and Local LLM product sprints from S20/S21.
- Reporting setup from S19b after S12 fields are stable.
- Deeper OpenTelemetry/dashboard polish after the bedrock status/debug truth is
  correct. Historical `analytics-dashboard` and `better_stats` ideas must be
  reintroduced through S16/S16a/S12/S19b, not by reviving the retired boards.
- VM resource recommendation polish: detect host CPU/RAM, estimate realistic
  active VM capacity at roughly 80% of system RAM, warn when selected defaults
  or active sessions exceed the machine envelope, and keep the warning based on
  active/running VMs rather than suspended or stopped VMs.
- Credential discovery hardening: finish the planned `credential-pipeline`
  spec-driven detector, explicitly scan legacy `~/.capsem/user.toml` and older
  provider setting paths during cutover, and surface source-by-source scan
  results so "not found" is distinguishable from "scan failed" in the UI.

Credential brokerage remains its own split sprint in S10. Rate limits, budgets,
and quotas remain their own split sprint in S22.

S18 release verification and S19 documentation are not improvement lanes. They
are table stakes for shipping the bedrock release.

## Plugin Improvement Contract

Future plugin work may expose a TypeScript/WASM authoring model, but it must
compile down to explicit event transforms:

```text
SecurityEvent -> SecurityEvent
```

Plugins may add labels, findings, decisions, and declarative mutations. They
must not mutate immutable event identity, subject payload, context, or trace
snapshot as authority. If a plugin wants to change real request/response/model/
MCP content, it returns declarative mutations and Rust validates/applies them.

The invariant for signed/replayable plugin bundles is:

```text
same plugin hash + same input event hash = same output event hash
```

`HookOutcome` remains internal transport machinery. Plugins consume and return
Security Events; the bedrock runtime maps final decisions/mutations to transport
or file/process actions.

## Acceptance Criteria

- Every improvement names which frozen bedrock contract it uses.
- No improvement introduces an alternate profile/settings/rule/event source of
  truth.
- New CLI, UI, HTTP, UDS, telemetry, and docs surfaces compose with the S08b/
  S09/S16/S19/S18 bedrock release instead of bypassing it.
- Any proposed contract mutation is treated as a release-blocking bedrock bug,
  not as ordinary improvement scope.
