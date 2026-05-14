# Policy And Settings Requirements

Last updated: 2026-05-13

## Why This Exists

This document captures live architecture feedback from the MCP/local-tools bug
investigation. It is not an implementation plan yet. It records requirements
that must survive context resets before we cut code.

## Core Requirement

`config/defaults.json` as a separate defaults/schema/UI metadata source has to
die unless we can prove it is strictly generated from the canonical model.

Current problem: it is another source of truth. It defines product settings,
defaults, group/UI metadata, env injection hints, file write targets, and partial
policy hints separately from the Rust runtime code that actually persists,
merges, validates, and enforces settings.

That split is not useful enough to justify the drift risk.

## Requirement 2: Scope Boundary -- Service Settings Vs VM Profiles

Settings and profiles are different scopes.

Service/app settings are service-scoped. They configure Capsem itself: app
behavior, service behavior, profile roots, credential storage for now, telemetry
export, remote policy plugin endpoints, and other host/service-wide integration
settings.

Profiles are VM/session-scoped. A profile describes the settings and policy
baseline a VM/session runs with. The effective profile settings must be attached
to a VM.

A VM may derive its initial settings from existing policy sources, profiles,
defaults, corp policy, or user preferences, but once resolved for a
VM they are part of that VM's configuration and lifecycle.

Required implications:

- The UI must be clear whether it is editing a global/default policy source or a
  concrete VM's effective settings.
- VM creation must snapshot or resolve the settings that VM will run with.
- Runtime enforcement must use the VM-attached effective settings, not a
  mutable global config that can drift without being represented in the VM.
- Status/debug reports must show the settings/policy actually attached to the
  VM, including provenance from inherited policy sources where relevant.
- Saved VMs created after the cutover must preserve their attached settings
  across profile/schema version changes.
- MCP server/tool settings, provider credentials, domain policy, guest files,
  and environment injection must all be understandable in VM scope.
- Telemetry export and remote policy plugin endpoints are service settings, not
  profile settings.
- For now, credentials may live in TOML. Moving credentials into Keychain or a
  brokered secret store is a separate credential brokerage sprint.

The product model is therefore:

```text
service settings
  -> service behavior, profile roots, telemetry, remote plugins

policy sources / profiles / defaults / corp / user
  -> resolve for VM
  -> VM-attached effective settings
  -> runtime policy, guest materialization, UI/status/debug truth
```

This does not eliminate shared policy sources or service settings. It means
shared policy sources are inputs. The VM-attached effective profile settings are
the VM/session operational truth.

## Requirement 3: Security Levels Are Profiles

Security levels are no longer hard-coded presets. They are profiles.

Each profile is its own file and is an independent source of truth. A profile
describes a reusable VM/session settings and policy baseline. VM settings derive
from a selected profile, and then resolve into VM-attached effective settings.

Sessions use profiles too. Starting a session means choosing a profile, whether
explicitly in the UI or implicitly through a default profile.

Required implications:

- Profiles must be first-class files, not embedded enum-like security levels.
- Profiles must be parsed and validated through the Rust typed TOML schema.
- Profiles must carry enough metadata for UI display without requiring a
  separate hand-written defaults/schema JSON file.
- Profiles must carry version metadata because saved VMs may depend on them.
- A VM/session may only derive from an allowed base profile.
- Profile derivation must be explicit and inspectable in status/debug output.
- The resolved VM settings must record profile provenance so we can explain why
  a setting has a particular value.

## Requirement 4: Profile Directories Are Policy Roots

Corp and user profile sets are represented by directories of profile files.

Adding or removing profiles is not a database mutation or code edit. It is
changing the configured profile directory contents or changing which directory
is used as a profile root.

Required implications:

- There must be a base profile directory.
- Corp can specify or replace the directory containing the base profile set.
- Corp can add profiles by adding files to the corp profile directory.
- Corp can remove profiles by removing files or by pointing to a profile
  directory that does not include them.
- User profiles, when allowed, live in a user profile directory.
- Profile discovery must be deterministic and explainable.
- Name/id collisions between base, corp, and user profiles must have explicit
  precedence and clear errors.
- Missing, invalid, or deprecated profiles referenced by a saved VM must produce
  clear status errors, not silent fallback.

## Requirement 5: Corp Profile Governance

Corp policy can govern profile creation and inheritance.

Required implications:

- Corp can forbid users from creating new profiles.
- Corp can restrict VMs/sessions to use only approved base/corp profiles.
- Corp can decide whether user profiles are visible, selectable, or ignored.
- Corp can lock individual profile-derived settings or whole profile families.
- The UI must surface when profile choice or profile creation is blocked by
  corp policy.
- Runtime enforcement must use the VM's resolved effective settings, not the
  editable profile file directly.

## Requirement 6: Profile Identity And UX Metadata

Profiles need a first-class identity in the UI.

Required profile metadata:

- stable id;
- display name;
- short description;
- "best for" description;
- profile type, such as `code` or `co-work`;
- icon, preferably an SVG asset with a default fallback;
- optional appearance defaults copied at launch and configurable per profile;
- version metadata.

The first built-in profile should be "Everyday Work" or equivalent: best for
normal day-to-day work, with reasonable defaults and enough power for common
coding/co-working tasks.

"Mid security" and "High security" are not good profile names and should not
survive as product concepts. Security posture should be expressed through
profile settings and security capabilities, not a crude security-level label.

## Requirement 7: Profile Security Has Capabilities Before Raw Rules

The Profile > Security section should not start with raw rules. Raw rules are
still necessary, but they are the lower-level layer.

The top of Profile > Security should expose higher-level security capabilities
that reconcile existing hooks and future controls, such as:

- credential brokerage and credential release behavior;
- PII detection, blocking, redaction, and alerting;
- MCP retrieval/RAG controls, including which connectors can pull context;
- MCP tool call policy and local-tool availability;
- network/domain/HTTP policy posture;
- model request/response scanning;
- file/read/write boundaries;
- audit capture expectations.

Telemetry export configuration itself is not a profile capability. It is a
service setting because it configures how the Capsem service emits events.

Below that, raw rules appear as the detailed policy view/editor:

- user-editable profile rules;
- derived rules from profile settings shown in gray;
- inherited corp/base profile rules shown locked;
- provenance links back to the source profile setting or capability.

## Required Follow-Up Sprints

This redesign needs many focused sprints. Do not collapse these into one giant
UI patch.

### Sprint 0: Remove V1 Policy/Settings System

We do not carry backward compatibility for the current v1 policy/settings stack.
Rip it out and prove Capsem still works on the new path.

Required work:

- Remove `config/defaults.json` as an interpreted runtime/UI authority.
- Remove legacy settings registry paths that accept ad hoc setting ids.
- Remove legacy network/domain/http/MCP policy builders once their replacement
  is ready enough to keep Capsem booting.
- Remove stale frontend assumptions around `settings.*`, `[mcp]`, and current
  Policy V2-as-parallel-authority shapes.
- Remove old config-shape awareness completely. No compatibility layer, no
  migration layer, no special diagnostics for v1.
- Add tests proving startup, service health, profile loading, and basic session
  creation still work after v1 removal.

### Sprint 1: Settings Type Design With User Review

Before implementation, present the typed Rust/TOML settings shape and ask how it
should work.

Required work:

- Propose typed Rust structs for app settings.
- Propose the TOML file layout for app/global settings.
- Propose field names, nesting, defaults, validators, and error messages.
- Propose how secrets are referenced without raw TOML secret storage.
- Propose how settings expose UI descriptors, info boxes, and policy semantics.
- Stop for user review before coding the settings type system.

### Sprint 2: Settings Type System Implementation

Build the typed settings system after the design is approved.

Required work:

- Implement Serde/TOML parsing for app settings.
- Implement semantic validation with explicit, tested errors.
- Implement defaulting without `config/defaults.json`.
- Implement generated/Rust-owned UI descriptors for settings controls.
- Implement typed save/load tests for valid, missing, malformed, unknown, and
  semantically invalid configs.

### Sprint 3: Profile Type Design With User Review

Before implementation, present the typed profile shape and ask how it should
work.

Required work:

- Propose the profile TOML file format.
- Propose profile identity metadata: id, name, description, best-for, type, SVG
  icon/default icon, appearance defaults, and version.
- Propose profile sections: General, Appearance, AI Providers, MCP & Connectors,
  Skills, VM, Security.
- Propose security capability fields and raw rule layout.
- Propose profile error behavior for invalid files, duplicate ids, missing icons,
  forbidden user profiles, and locked corp fields.
- Stop for user review before coding the profile type system.

### Sprint 4: Profile Type System Implementation

Build profiles as first-class typed files.

Required work:

- Implement profile discovery across base, corp, and user profile directories.
- Implement profile TOML parsing and validation.
- Implement deterministic precedence and collision errors.
- Implement profile CRUD primitives: create, fork/clone, update, delete.
- Add heavy test coverage for malformed TOML, bad ids, duplicate ids, invalid
  inheritance, forbidden user profiles, missing required fields, invalid icons,
  bad rules, bad connector references, and bad VM settings.

### Sprint 5: Profile Assembly, Corp Override, And VM Effective Settings

Build the resolution layer that assembles app settings, base/corp/user profiles,
corp governance, and VM-specific choices.

Required work:

- Define and implement profile assembly precedence.
- Prove corp can add, remove, replace, lock, and forbid profiles.
- Prove corp can override individual fields and capability families.
- Assemble generated/derived rules from profile settings and security
  capabilities.
- Materialize VM-attached effective settings.
- Add provenance for every effective setting/rule.
- Add tests for corp override, user override, locked fields, inherited rules,
  derived rules, VM-effective snapshots, and saved VM/fork behavior.

### Sprint 6: UDS Service API

Expose typed settings and profiles over the service UDS API first.

Required work:

- Add UDS endpoints for app settings.
- Add UDS endpoints for profile list/get/create/fork/update/delete.
- Add UDS endpoints for profile resolution and VM-effective settings.
- Add UDS endpoints for MCP list/add/delete and skills list/add/delete in the
  new typed model.
- Fully test CRUD, fork, delete, invalid payloads, corp locks, forbidden user
  actions, and concurrent updates.
- Add E2E service tests that create/fork/delete profiles through the UDS path.

### Sprint 7: HTTP Gateway API

Wire the same profile/settings service through the HTTP gateway after UDS is
solid.

Required work:

- Add HTTP endpoints backed by the UDS API.
- Preserve the same typed validation errors and provenance payloads.
- Add HTTP E2E tests for app settings, profile CRUD, fork/delete, profile
  resolution, MCP list/add/delete, and skills list/add/delete.
- Add tests that a session created through the HTTP path uses the selected
  profile and enforces the expected VM-effective settings.

### Sprint 8: CLI Integration

Wire the CLI after backend APIs are proven.

Required work:

- Add `capsem profile list/create/fork/update/delete/show/resolve` or equivalent.
- Add `capsem mcp list/add/delete/show` against the new typed model.
- Add `capsem skills list/add/delete/show` against the new typed model.
- Keep command shapes consistent across profile, MCP, and skills.
- Add parser tests, service integration tests, error tests, and E2E smoke tests.

### Sprint 9: Credential Brokerage

For now, credentials may live in TOML. Credential brokerage is still a promised
security capability and needs its own sprint.

Required work:

- Define how credentials move from service settings into VM/session use.
- Define credential release policy in profiles.
- Decide whether and how Keychain participates after the TOML-first cutover.
- Add service-side broker APIs and audit events.
- Add tests for allowed release, denied release, missing credentials, stale
  credentials, profile lockout, and audit capture.

### Sprint 10: Status, Debug, And Provenance

Status/debug is critical and must not be incidental.

Required work:

- Add status output for service settings, selected profiles, VM-effective
  settings, profile provenance, derived rules, and locks.
- Add debug report sections for app/service settings, profile roots, profile
  resolution, VM-effective settings, MCP/tools/skills, and policy assembly.
- Add "why" explanations for effective values and generated rules.
- Add tests proving status/debug output matches the active service settings and
  VM-attached effective profile.

### Sprint 11: Observability Plugin

Observability is service-based, not profile/VM-based. It should be a plugin that
exports events to an OpenTelemetry endpoint configured in service settings.

Required work:

- Define service settings for observability plugin enablement and endpoint.
- Wire OpenTelemetry export from service/process event streams.
- Decide event categories, sampling, redaction, batching, retry, and failure
  behavior.
- Add tests for disabled plugin, bad endpoint, retry/failure behavior,
  redaction, and export payload shape.

### Sprint 12: Remote Policy Plugin

Remote policy is a separate service plugin. It forwards events/context to a
configured endpoint and receives policy decisions or policy updates.

Required work:

- Define service settings for remote policy endpoint, auth, timeout, and failure
  behavior.
- Define which events and context are forwarded.
- Define fail-open/fail-closed behavior per decision surface.
- Wire into the policy decision path without making profile TOML depend on the
  remote endpoint.
- Add tests for allow/block/ask decisions, endpoint failure, timeout, auth
  failure, redaction, and audit output.

### Sprint 13: Rules UI Component System

The current rules UI is not acceptable. Raw CEL text input is not a product UI
for normal users and is not good enough for advanced users either.

Required work:

- Design a reusable rule-builder component for the settings/profile toolset.
- Cover the full verb/action set we need, not just today's partial UI.
- Provide autocomplete for fields, operators, functions, constants, connector
  names, MCP tools, provider names, domains, and profile-scoped objects.
- Make raw expression editing an advanced escape hatch, not the default path.
- Support provenance display for generated/derived rules.
- Support locked/read-only rule display for corp/base profile rules.
- Support validation errors inline before save.
- Support rule previews/explanations: "this blocks OpenAI POST requests" rather
  than only showing a CEL expression.

### Sprint 14: Settings UI Redesign

Redo the Settings UI using the new reusable component toolkit.

Required work:

- Build reusable setting controls for toggles, selects, text inputs, secret
  references, file references, connector credentials, chips/lists, and nested
  groups.
- Build reusable info boxes/help panels that can appear across settings and
  profile security capabilities.
- Make source/provenance visible where relevant: app default, user value, corp
  lock, profile-derived value, VM-effective value.
- Make disabled/locked states honest and visually clear.
- Ensure every control maps to typed TOML-backed API fields, not ad hoc string
  keys.

### Sprint 15: Profile UI

Profiles become first-class in the UI.

Required work:

- Add profile list/select/create/fork/delete flows.
- Show profile identity: SVG/default icon, name, description, "best for", type,
  and version.
- Add profile subsections: General, Appearance, AI Providers, MCP & Connectors,
  Skills, VM, Security.
- Keep app settings and selected-profile settings visually separate.
- Make session creation use a selected/default profile.
- Make profile inheritance/provenance inspectable.

### Sprint 16: Security Capabilities UI

Build Profile > Security around capabilities first, raw rules second.

Required work:

- Capability controls must use the same reusable setting component system:
  toggles, selects, info boxes, lock states, derived-rule previews.
- Cover credential brokerage, PII detection/blocking/redaction, MCP retrieval/RAG
  controls, MCP tool policy, local tool policy, network/domain/HTTP posture,
  model request/response scanning, file boundaries, and audit posture.
- Show generated rules in gray below capability controls.
- Show raw editable profile rules in a separate Rules area.
- Make rule provenance explicit and clickable back to the capability/setting.

Telemetry export and remote policy plugin endpoints stay in service settings.
Profile security may control what gets audited or blocked, but not where service
telemetry is exported.

### Sprint 17: Full Verification And Release Gate

End with a dedicated proof sprint.

Required work:

- Full backend tests for typed settings, profiles, assembly, APIs, CLI, and
  enforcement.
- Frontend tests for Settings, Profiles, Rules, and Security capabilities.
- E2E tests for creating, forking, deleting, selecting, and launching sessions
  with profiles.
- E2E tests proving MCP, skills, AI providers, credential brokerage, PII, and raw
  rules enforce through VM-effective settings.
- Release gate proving fresh install still works after v1 removal.

## Current Source-Of-Truth Problem

Today the system effectively has multiple authorities:

- `config/defaults.json` says which settings exist, how they appear in the UI,
  what defaults they have, and some policy-ish metadata.
- `~/.capsem/user.toml` and `/etc/capsem/corp.toml` persist user/corp choices.
- Rust builders decide what those choices actually mean for domain, HTTP, MCP,
  model, DNS, guest files, and environment injection.
- MCP has additional `[mcp]` config that bypasses the normal settings registry.
- Policy V2 has its own `[policy.*]` namespace.

The MCP local-tools bug is a symptom of this: the UI can write a setting-shaped
MCP key that does not map to the runtime MCP config path.

## Requirement: Rust Typed TOML Schema

The canonical configuration schema must be native Rust structs that deserialize
from TOML with Serde and validate with Rust validation code.

Everything else should be derived from it:

- UI groups and controls.
- Defaults.
- Syntax, required fields, and data types.
- Semantic validation.
- User/corp merge behavior.
- Guest file/env materialization.
- Network/domain/HTTP/MCP/model/DNS policy meaning.
- Debug/status/debrief output.

If a machine-readable schema is needed by the frontend, docs, or tests, it
should be generated from the Rust TOML schema/metadata, not maintained as a
parallel hand-written registry.

This explicitly rejects JSON Schema as the enforcement source. The best fit here
is Serde + the `toml` crate for parsing/types and a Rust validation layer
(`validator` crate or equivalent explicit validators) for semantic constraints.

## Requirement: Typed Settings Explain Policy Directly

A typed setting struct/field must carry or point to the rule semantics that make
it real.

Example: `ai.openai.allow` must not be just a UI toggle. Its canonical definition
must explain:

- the related domain scope (`ai.openai.domains`);
- the API key/env materialization (`OPENAI_API_KEY`);
- the network/domain/HTTP behavior when enabled or disabled;
- how corp overrides affect allowed and blocked domains;
- how status/debug output should explain a denial.

The current OpenAI definition in `config/defaults.json` hints at this with
`enabled_by`, `domains`, `env_vars`, and `meta.rules`, but those hints are only a
partial, parallel registry. The requirement is to make that relationship
native Rust-owned behavior and make it enforceable through typed TOML parsing
and validation.

## Requirement: MCP Must Join The Same Model

MCP cannot remain a side namespace that only partly overlaps with settings.

Local MCP tools must be represented by canonical settings such as:

- local MCP server enabled/disabled;
- default MCP tool behavior;
- per-tool policy overrides;
- tool visibility versus callability;
- corp locks.

Disabling local tools must update the same typed TOML-backed config that the UI,
service status, runtime server list, and MCP frame enforcement consume.

## Non-Requirement

We do not need another source of truth.

A schema artifact is acceptable only if it is generated output. It must not
become another hand-edited place where product behavior can drift.

## Open Architecture Question

Where should the Rust typed TOML schema live?

Candidate directions:

- Rust config structs plus explicit UI/policy descriptors in Rust.
- Rust config structs plus derive/generated frontend descriptors.
- A smaller TOML-first config schema with Rust owning all semantics and
  generated UI metadata.

This decision is still open. The hard requirement is no independent
`defaults.json` authority.
