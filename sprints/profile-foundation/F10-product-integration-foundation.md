# F10 - Product Integration Foundation

## Goal

Bring OpenAPI-to-MCP and Local LLM under Profile V2 governance.

## Scope

- OpenAPI validation, review, selected operation activation, provenance, and
  generated MCP tool visibility.
- Local LLM provider configuration, selection, diagnostics, and enforcement.
- Credential brokerage integration where authenticated APIs are used.
- Security, detection, audit, metrics, and UI treatment for both integrations.

## Acceptance Criteria

- Generated tools and local model providers are profile-owned.
- No integration bypasses MCP aggregation, enforcement, audit, diagnostics, or
  status.
- UI and CLI expose review/provenance before activation.
