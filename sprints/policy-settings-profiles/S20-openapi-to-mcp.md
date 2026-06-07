# S20 - OpenAPI To MCP

Status: S24 child sprint

## Product Goal

Allow Capsem users and admins to turn an OpenAPI-described HTTP service into a
profile-owned MCP tool surface that can be reviewed, governed, tested, and used
from a VM/session without hand-writing an MCP server.

The feature exists to make existing internal APIs usable by agents while keeping
the resulting tools inside the Profile V2 security model.

## Problem

Many teams already have HTTP services with OpenAPI specs. Today, using those
services from agent workflows requires custom MCP server code, manual tool
schema translation, or direct HTTP access that is harder to review as an agent
tool surface.

The missing product capability is a first-class import path from OpenAPI into
Capsem-managed MCP tools, with profile ownership, reviewability, auth handling,
diagnostics, and UI visibility.

## Users

- Admins who publish approved API tools for a profile.
- Developers who want to use an internal HTTP API from an agent VM.
- Security reviewers who need to inspect which operations are exposed.
- End users choosing profile-approved tools in the UI.

## Core Scenarios

1. An admin imports an OpenAPI document and produces a reviewed MCP tool set
   attached to a profile.
2. A developer previews generated tools before enabling them for a VM/session.
3. A profile owner narrows exposed operations to a safe subset.
4. A security reviewer can see the source OpenAPI document, generated tool
   names, operation ids, auth requirements, and risk metadata.
5. A VM/session can use the generated tools through the same MCP aggregation,
   enforcement, detection, audit, and diagnostic paths as manually configured MCP
   servers.

## Required Product Capabilities

- Accept OpenAPI 3.x documents from trusted local or managed sources.
- Validate that the document is well-formed before creating any tool surface.
- Generate MCP tool definitions from selected operations.
- Preserve source provenance for each generated tool:
  - source document identity,
  - operation id,
  - HTTP method and path,
  - schema version/hash,
  - auth requirement,
  - owning profile.
- Allow users/admins to select which operations become tools.
- Provide a review step before generated tools become active.
- Represent generated tools as profile-owned MCP capability, not loose global
  runtime state.
- Surface generated tools in UI wherever profile MCP tools are reviewed.
- Feed generated tool calls into the same security event, audit, timeline,
  and diagnostics story as other MCP activity.
- Support profile-level governance such as editability, ownership locks,
  enforcement/detection attachment, credential requirements, and future usage
  budgets.

## Auth And Secrets Requirements

- Generated tools must not embed secrets into generated schemas or profile
  payloads.
- The import/review flow must show which operations require auth.
- Credential release must use the existing profile/service credential model
  once S10 defines the brokerage path.
- Missing credentials must be visible as a diagnostic state, not a mysterious
  tool failure.

## UI Requirements

The UI must eventually let a user/admin:

- import or review an OpenAPI source,
- inspect generated tools before activation,
- include/exclude operations,
- see auth requirements,
- see validation errors,
- see which profile owns the generated tools,
- see whether generated tools are usable by the current VM/session.

The UI wording should focus on the source API, generated tools, ownership, and
health. It should not expose implementation internals as the primary user
model.

## Admin And CLI Product Requirements

This sprint requires admin and developer-facing entry points, but the exact
public command/API shape is decided by the owning CLI/API sprint. The product
requirement is that OpenAPI import, validation, preview, activation, and
diagnostics are available through Capsem's established profile/VM workflow,
not as a separate unscoped global tool registry.

## Non-Goals

- No automatic exposure of every operation without review.
- No generated tools outside profile ownership.
- No permanent secret material written into generated tool definitions.
- No promise to support every OpenAPI extension in the first version.
- No direct bypass around MCP aggregation, enforcement, audit, or diagnostics.

## Dependencies

- S08b Security Event Engine and resolved event contract.
- S09/S16 public surfaces for profile/VM scoped workflows.
- S10 credential brokerage for authenticated APIs.
- S11 diagnostics/provenance for explainability.
- S14/S17 UI components for reviewing security capability surfaces.

## Acceptance Criteria

- A valid OpenAPI document can be validated and previewed.
- A selected subset of operations can become profile-owned generated MCP tools.
- Invalid or unsupported OpenAPI features fail with clear diagnostics.
- Generated tools preserve source provenance and owning profile identity.
- Generated tool calls are visible in the MCP aggregation path.
- Generated tool calls produce security/audit/timeline evidence equivalent to
  other MCP tools.
- Missing auth, unsupported schema shapes, and unreachable upstream services
  are diagnosable before a user relies on the tool.
- UI/product surfaces can distinguish generated OpenAPI-backed tools from
  manually configured MCP tools without treating them as second-class.

## Coverage Ledger

- Unit/contract: OpenAPI validation, operation selection, generated tool schema
  shape, provenance metadata.
- Functional: import/preview/activate flow through the approved profile-scoped
  surface.
- Adversarial: malformed specs, unsupported auth, schema collisions, duplicate
  operation names, unsafe defaults, secret leakage.
- E2E/VM: generated tool usable from a VM/session through the normal MCP path.
- Telemetry/audit: generated tool call appears in resolved event/timeline
  evidence with source provenance.
- UI: generated tools are reviewable and distinguishable in profile/security
  capability views.
