# Profile V2 Migration Audit

Last updated: 2026-05-17

## Audit Standard

This audit compares each sprint request in `sprints/policy-settings-profiles/`
against the merged code on branch `profile-v2`. A sprint is only considered
landed when the requested product surface exists in code and has direct proof in
tests. Green smoke is not enough if the implementation still depends on V1
settings authority.

Status labels:

- `LANDED`: requested surface exists and has focused proof.
- `PARTIAL`: important slices landed, but required scope or proof is missing.
- `GAP`: no meaningful implementation found in the merged code.
- `BLOCKER`: must be fixed before this branch can be called clean Profile V2.

## Executive Summary

The rescue branch preserved and replayed a large amount of S01-S06 backend work:
typed service/profile settings, VM-effective attachments, Policy V2 runtime
conversion, confirmation hooks, rewrite support, derived rule ownership, and
focused VM/MITM parity are present.

The branch is not yet clean Profile V2, but the most dangerous process-runtime
V1 bridge has now been removed. `capsem-process` no longer reads global
`user.toml`/`MergedPolicies` for runtime authority; attached Profile V2
VM-effective settings now drive guest boot files/env, coarse network/DNS
policy, domain fast paths, MCP defaults, and exact Policy V2 evaluation.
Several public surfaces requested after S06 are still missing: dedicated
profiles/rules/skills UDS APIs, mirrored gateway APIs, CLI profile/rules/confirm
commands, credential brokerage, real pending-ask UX, OpenTelemetry metrics
architecture, and updated docs.

## Sprint Findings

### S00 - Meta Sprint Setup

Status: `LANDED`

Evidence:
- Sprint corpus exists under `sprints/policy-settings-profiles/`.
- Rescue control docs exist under `sprints/profile-v2-migration-rescue/`.

Open proof:
- Keep this audit synchronized with `MASTER.md` and `tracker.md`.

### S01 - Remove V1 Settings/Policy

Status: `BLOCKER`

Landed:
- `/settings` returns `mode = "settings_profiles_v2"` with typed
  `settings_profiles` and `effective_rules`.
- Service VM startup writes `vm-effective-settings.toml` and
  `vm-effective-trace.json`.
- `capsem-process` converts attached VM-effective rules into runtime network,
  domain, MCP, and Policy V2 state.

Resolved in this audit pass:
- `crates/capsem-process/src/mcp_runtime.rs` no longer reads global V1
  `user.toml` or `MergedPolicies::from_disk()`.
- `RuntimePolicyState.guest_config`, `network_policy`, and `domain_policy` now
  derive from attached Profile V2 VM-effective state/default fallback instead
  of V1 `MergedPolicies`.
- Simple Profile V2 `dns.request`/`http.request` domain rules now populate the
  coarse `NetworkPolicy` used by DNS full-block enforcement; conditional
  path/body/header rules remain in the exact Policy V2 engine.
- `scripts/integration_test.py` now installs a temporary Profile V2 smoke
  profile/service selection instead of depending on removed
  `CAPSEM_USER_CONFIG`/`CAPSEM_CORP_CONFIG` runtime policy plumbing.
- Focused tests now assert that global legacy `user.toml` is ignored, V2 guest
  boot env/files are produced without V1 settings, and the process runtime
  source contains no V1 policy bridge tokens.

Remaining blocking gaps:
- Setup/install paths still write and test `user.toml` as durable setup state.
- Docs and generated frontend fixtures still reference `config/defaults.json`,
  `user.toml`, standalone `[mcp]`, and old web allow/block settings.

TDD guardrails added:
- Runtime ignores global `user.toml` when VM-effective settings are present.
- Guest boot env/files are built from Profile V2 runtime state without
  `MergedPolicies`.
- Simple Profile V2 DNS/HTTP domain block rules are visible to the coarse DNS
  full-block `NetworkPolicy`, while path-scoped HTTP blocks do not become
  broad DNS blocks.
- `capsem-process` has no runtime dependency on `MergedPolicies::from_disk`.

### S02 - Service Settings Design

Status: `LANDED`

Evidence:
- `capsem_core::settings_profiles::ServiceSettings` contains app, profile
  roots, assets, credentials, telemetry, remote policy, and corp directives.
- Validation covers endpoints, credential values, asset roots, and profile
  roots.

Open proof:
- Later sprints still need to consume all fields through public APIs, UI, and
  docs.

### S03 - Service Settings Implementation

Status: `PARTIAL`

Landed:
- `service.toml` load/write helpers exist.
- Service startup resolves asset locations from typed service settings.
- `/setup/corp-config` installs corp profile TOML through
  `settings_profiles::install_corp_profile_toml`.
- Tests cover typed settings/debug/asset location behavior.

Gaps:
- Runtime setup/install still has V1 `user.toml` setup behavior.
- Credential storage exists as typed TOML, but credential release brokerage is
  not implemented until S10.

### S04 - Profile Design

Status: `LANDED`

Evidence:
- `Profile`, profile roots, profile source/provenance, canonical
  `security.rules.<type>.<name>`, validation, derived/catch-all priorities, and
  rule ownership metadata exist in `settings_profiles`.

Open proof:
- Public API/UI rendering of all design fields is incomplete in later sprints.

### S05 - Profile Implementation

Status: `LANDED`

Evidence:
- Profile discovery, create, update, delete, inheritance, parent validation,
  user operation gates, and rule mutation gates exist in
  `settings_profiles`.
- Focused `settings_profiles` tests cover the core implementation.

Open proof:
- Service/CLI/gateway/UI CRUD surfaces are incomplete and tracked under
  S07-S09/S16.

### S06 - Assembly and VM-Effective Settings

Status: `PARTIAL`

Landed:
- Service resolves selected/default profile to VM-effective settings.
- VM-effective TOML and trace are attached to sessions.
- `capsem-process` loads attached VM-effective rules and falls back to default
  profile when attachments are missing/corrupt.
- Focused VM/MITM parity tests passed for framed MCP, HTTP/DNS, and model
  Policy V2.

Gaps:
- Guest boot config now comes from Profile V2 runtime state, but the
  `GuestConfig` type still lives in the old `net::policy_config` namespace and
  should be moved before the final V1 removal claim.
- Full release gate `just test` remains pending.

### S06-pre - Network Contract and Confirm

Status: `PARTIAL`

Landed:
- DNS/HTTP/MCP/model ask callsites route through the shared `Confirmer` trait
  with placeholder confirmation.
- Focused unit tests cover denial mapping and redacted snapshots.

Gaps:
- `policy_confirm_events` durable telemetry table and capsem-doctor ask probe
  are not landed.
- Real pending ask queue and operator resolution are S15 and not implemented.

### S06a - Model Request Rewrite Support

Status: `LANDED`

Evidence:
- Model request rewrite support exists in the MITM model path.
- Tests cover request.data condition support, rewrite dispatch, redaction, and
  fail-closed cases.

### S06b - Legacy Allowlist Migration and Rule Ownership

Status: `PARTIAL`

Landed:
- Provider toggle derived rules, nested provider/connector rules, catch-all
  rules, priority windows, and owner metadata exist in `settings_profiles`.
- Mutation gate rejects direct edits of managed rules.

Gaps:
- The explicit non-migration of V1 default allow/block lists is undermined by
  the current `capsem-process` legacy bridge.
- UI/status/debug ownership rendering is incomplete.

### S07 - UDS Service API

Status: `PARTIAL`

Landed:
- Existing service routes include `/settings`, `/settings/presets`,
  `/settings/lint`, `/settings/validate-key`, and MCP routes
  `/mcp/servers`, `/mcp/tools`, `/mcp/policy`, refresh/approve/call.

Gaps:
- No dedicated profile list/get/create/fork/update/delete/resolve route group.
- No dedicated skills list/add/delete route group.
- No dedicated Rules API (`GET /rules`, `POST /rules`,
  `POST /rules/evaluate`, etc.).
- No `GET /confirm/pending` listing surface.
- No typed metrics IPC contract from S07 foundation.

### S08 - HTTP Gateway API

Status: `GAP`

Gaps:
- Gateway does not mirror the S07 Rules API.
- Gateway does not expose confirm pending/resolve/SSE surfaces.
- Gateway settings/profile CRUD parity beyond existing proxy/status behavior is
  not implemented.

### S09 - CLI Integration

Status: `PARTIAL`

Landed:
- `capsem mcp` commands exist for servers, tools, policy, refresh, and tool
  call.

Gaps:
- No `capsem profile ...` command family.
- No `capsem rules ...` command family.
- No `capsem skills ...` command family.
- No `capsem confirm ...` command family.

### S10 - Credential Brokerage

Status: `GAP`

Landed:
- Typed service TOML credential storage exists.
- Legacy-style guest credential file injection exists through the old boot
  config path.

Gaps:
- No service credential broker API.
- No release policy enforcement.
- No release/denial audit events.
- No VM materialization proof through the Profile V2 broker path.

### S11 - Status, Debug, Provenance

Status: `PARTIAL`

Landed:
- Debug report includes `[settings_profiles]`, service settings, profile roots,
  selected/effective profile, VM settings, MCP/skills counts, rule counts, and
  resolver trace summary.
- Credential values are redacted.

Gaps:
- `capsem status` does not yet expose the full Profile V2 provenance story.
- Generated-rule ownership details (`owner_setting_path`,
  `owner_setting_label`, editable/managed state) are not fully rendered in
  status/debug.
- Active VM-effective state proof remains incomplete outside smoke.

### S12 - OpenTelemetry Metrics Architecture

Status: `GAP`

Gaps:
- `VmMetricsSnapshot` and `ServiceToProcess::GetMetricsSnapshot` are not
  implemented.
- Service still has only a placeholder comment for live metrics.
- `/metrics/json` and Prometheus/OTel surfaces are not implemented.
- No accumulator seeding from session DB in `capsem-process`.

### S13 - Remote Policy Plugin

Status: `GAP`

Landed:
- `ConfirmerKind::RemotePlugin` exists as a type-level enum variant.

Gaps:
- No `RemotePluginConfirmer`.
- No remote policy endpoint dispatch, auth, timeout, failure mapping, or audit
  output.
- No service setting cutover to select the remote plugin confirmer.

### S14 - Rules UI Components

Status: `PARTIAL`

Landed:
- `PolicyRulesSection.svelte` provides an existing named-rule editor surface
  for settings.

Gaps:
- It is not the requested shared rule editor/renderer architecture.
- Per-type DNS/HTTP/Model/MCP visual blocks with provenance and managed-by
  labels are incomplete.
- Autocomplete and full rewrite validation UI are incomplete.
- Locked/managed rule direct-edit prevention is not fully surfaced.

### S15 - Confirm UX

Status: `GAP`

Landed:
- Placeholder confirmer exists in core policy confirmation plumbing.

Gaps:
- No pending ask queue.
- No service confirmer that enqueues and awaits operator resolution.
- No confirm UDS/gateway routes or stream endpoint.
- No CLI confirm commands.
- No UI bell/drawer/detail prompter.
- No auto-rule derivation module.
- No `policy_confirm_events` telemetry integration.

### S16 - Profile UI

Status: `GAP`

Gaps:
- No first-class profile selector/create/fork/delete UI was found.
- Settings UI still works primarily from dynamic settings sections and policy
  rules, not a profile-centered experience.
- Session launch does not expose selected profile UI proof.

### S17 - Security Capabilities UI

Status: `PARTIAL`

Landed:
- Existing Settings UI has Security, MCP, and Policy sections.

Gaps:
- No capability-first Profile > Security UI covering credential brokerage,
  PII, MCP retrieval/RAG, local tools, network/domain/HTTP, model scanning,
  file boundaries, and audit posture.
- Managed/generated rules are not shown with complete source-setting guidance.

### S18 - Full Verification and Release Gate

Status: `GAP`

Gaps:
- `just smoke` passed after the Profile V2 runtime/DNS integration rescue, but
  `just test` has not been re-run on this branch after the rescue.
- Fresh-install proof after V1 removal is impossible until V1 setup/install
  paths are replaced.
- E2E profile create/fork/delete/select/launch, API/CLI/UI enforcement, and
  credential/PII/skills proofs are not complete.

### S19 - Documentation and Site

Status: `BLOCKER`

Gaps:
- Docs still describe `user.toml`, `corp.toml`, `config/defaults.json`,
  standalone `[mcp]`, old settings authority, and old network policy flows.
- New settings/profile/policy architecture pages are not present.
- Docs site release gate cannot pass until S07-S18 public surfaces stabilize.

## Active Cleanup Plan

1. Remove or quarantine V1 runtime authority from setup/install/docs and old
   policy-config type surfaces.
2. Re-run broad gates beyond smoke, especially `just test`.
3. Continue S07-S19 sprint implementation from the documented gaps, one sprint
   at a time.
