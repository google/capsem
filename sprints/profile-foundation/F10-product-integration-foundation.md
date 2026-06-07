# F10 - Product Integration Foundation

## Goal

Bring OpenAPI-to-MCP and Local LLM under Profile V2 governance while consuming
Google-specific integration decisions from
[Google Integration Sprint](../google/MASTER.md).

## Scope

- OpenAPI validation, review, selected operation activation, provenance, and
  generated MCP tool visibility.
- Local LLM provider configuration, selection, diagnostics, and enforcement.
- Google-specific surfaces are split to [Google Integration Sprint](../google/MASTER.md).
  F10 keeps the common integration contracts that Google must consume:
  profile ownership, canonical evidence, diagnostics, audit, metrics, and UI
  review/provenance.
- Credential brokerage integration where authenticated APIs are used.
- Security, detection, audit, metrics, and UI treatment for both integrations.

## Acceptance Criteria

- Generated tools and local model providers are profile-owned.
- Google-backed calls satisfy the same profile ownership, Security Event,
  evidence, metrics, audit, and redaction contracts through the Google sprint.
- No integration bypasses MCP aggregation, enforcement, audit, diagnostics, or
  status.
- UI and CLI expose review/provenance before activation.
