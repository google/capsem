# F10 - Product Integration Foundation

## Goal

Bring OpenAPI-to-MCP, Local LLM, and deeper Google/Gemini integration under
Profile V2 governance.

## Scope

- OpenAPI validation, review, selected operation activation, provenance, and
  generated MCP tool visibility.
- Local LLM provider configuration, selection, diagnostics, and enforcement.
- Google/Gemini provider integration that consumes F06 credential brokerage,
  projects canonical model/tool evidence, and gives Drive/Gemini/Google-backed
  tools coherent profile ownership.
- Credential brokerage integration where authenticated APIs are used.
- Security, detection, audit, metrics, and UI treatment for both integrations.

## Acceptance Criteria

- Generated tools and local model providers are profile-owned.
- Google/Gemini/Drive-backed capabilities are profile-owned and do not require
  duplicate account setup per surface.
- No integration bypasses MCP aggregation, enforcement, audit, diagnostics, or
  status.
- UI and CLI expose review/provenance before activation.
