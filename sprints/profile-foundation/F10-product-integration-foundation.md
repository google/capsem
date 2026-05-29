# F10 - Product Integration Foundation

## Goal

Bring OpenAPI-to-MCP, Local LLM, and deeper Google integration under Profile
V2 governance.

## Scope

- OpenAPI validation, review, selected operation activation, provenance, and
  generated MCP tool visibility.
- Local LLM provider configuration, selection, diagnostics, and enforcement.
- Google integration that consumes F06 credential brokerage, projects canonical
  model/tool evidence, and gives Gmail, Drive, gcloud, Firebase, Jet Ski,
  Gemini, Google AI, and other Google-backed tools coherent profile ownership.
- Integration-specific diagnostics for OAuth consent/scope mismatch, ADC
  lookup, service-account files, Firebase project selection, Gmail/Drive API
  enablement, Gemini provider configuration, and missing/expired credentials.
- Credential brokerage integration where authenticated APIs are used.
- Security, detection, audit, metrics, and UI treatment for both integrations.

## Acceptance Criteria

- Generated tools and local model providers are profile-owned.
- Gmail, Drive, gcloud, Firebase, Jet Ski, Gemini, and Google AI capabilities
  are profile-owned and do not require duplicate account setup per surface.
- Google-backed calls produce canonical Security Events, tool/model evidence,
  metrics, and audit records with redacted credential provenance.
- No integration bypasses MCP aggregation, enforcement, audit, diagnostics, or
  status.
- UI and CLI expose review/provenance before activation.
