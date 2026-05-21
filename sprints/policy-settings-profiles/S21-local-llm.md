# S21 - Local LLM

Status: proposed standalone sprint

## Product Goal

Let a profile/VM use a local model service as an approved AI provider while
keeping local model traffic inside Capsem's profile, enforcement, detection,
diagnostic, audit, and UI model.

The product promise is not merely "Capsem can reach localhost." The promise is
that local LLM use is governed and explainable the same way remote AI provider
use is governed and explainable.

## Problem

Users increasingly run local models for privacy, cost, offline work,
development speed, and resilience. Without first-class Local LLM support,
teams either bypass Capsem's model-governance path or treat local inference as
generic HTTP traffic with weak model identity, weak diagnostics, and poor UI
explanation.

The missing capability is a profile-owned local AI provider path that users can
select, operators can diagnose, and security enforcement can reason about.

## Users

- Developers running local models during agent work.
- Teams that prefer local inference for sensitive projects.
- Admins defining which profiles may use local models.
- Operators debugging model availability and enforcement behavior.
- UI users choosing between approved provider options for a VM/session.

## Core Scenarios

1. A profile permits local model usage for a VM/session.
2. A user chooses an approved local model path through the normal profile/VM
   workflow.
3. Capsem can show whether the local model service is available and compatible.
4. Model requests and responses remain visible to the same enforcement, detection,
   audit, and timeline paths as remote model traffic.
5. A local model outage, unsupported response shape, or missing configuration is
   reported clearly in status/diagnostics/UI.

## Required Product Capabilities

- Represent local model providers as profile-owned AI capability.
- Preserve VM/session ownership for local model use.
- Detect and report local model service health.
- Capture model identity where available:
  - provider/runtime name,
  - model name,
  - endpoint family or compatibility mode,
  - availability state,
  - owning profile/VM.
- Route local model requests through the same model-governance event path as
  other AI providers.
- Support enforcement and detection over local model activity using the normalized
  model event vocabulary.
- Ensure local model use appears in timeline/audit evidence.
- Show local model readiness and failure reasons in UI/status/debug surfaces.
- Keep future usage budgets/rate limits applicable to local model activity.

## UI Requirements

The UI must eventually show:

- whether local LLM is allowed for the current profile/VM,
- whether the local model service is reachable,
- which model is selected or discovered,
- whether the selected model is compatible with the requested workflow,
- recent local model errors,
- enforcement/detection outcomes for local model calls.

The UI should treat local LLM as a first-class provider option, not as a hidden
network endpoint.

## Admin And CLI Product Requirements

This sprint requires profile/VM-scoped ways to configure, select, validate, and
diagnose local model use. The exact external command/API shape belongs to the
owning CLI/API sprint. The product requirement is that Local LLM stays inside
profiles and VM-effective settings.

## Non-Goals

- No requirement to ship a bundled model in the first version.
- No requirement to manage GPU drivers or model downloads in this sprint.
- No generic model marketplace.
- No bypass around model enforcement, detection, audit, or status.
- No guarantee that every local inference server dialect is supported in the
  first version.

## Dependencies

- S08b normalized model/security event contract.
- S09/S16 profile/VM public surfaces.
- S11 status/debug/provenance.
- S12 metrics for model/provider/token/cost counters where available.
- Future service diagnostics sprint for AI provider health.

## Acceptance Criteria

- A profile can allow local model usage as an AI provider capability.
- A VM/session can resolve local model configuration from VM-effective settings.
- Capsem can report local model health and compatibility state.
- Local model requests are represented as model activity, not only generic HTTP.
- Local model activity participates in enforcement/detection/audit/timeline flows.
- Local model failures produce actionable diagnostic output.
- UI-facing state can distinguish disabled, configured, available, unavailable,
  and unsupported local model states.
- The sprint leaves room for future usage budgets/rate limits without changing
  the product model.

## Coverage Ledger

- Unit/contract: profile parsing/resolution for local model capability,
  normalized event fields, health state model.
- Functional: local provider selected through approved profile/VM path.
- Adversarial: unreachable service, malformed response, unsupported dialect,
  missing model identity, enforcement denial.
- E2E/VM: VM/session model activity flows through the local provider path.
- Telemetry/audit: local model activity appears in resolved event/timeline
  evidence.
- UI/status: local model readiness and failure reasons are visible.
