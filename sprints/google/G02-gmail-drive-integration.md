# G02 - Gmail And Drive Integration

## Goal

Make Gmail and Drive profile-owned tools/connectors with clear scopes,
diagnostics, evidence, and redaction.

## Scope

- Gmail read/send/summarize/search capability boundaries.
- Drive list/read/write/share capability boundaries.
- OAuth consent and scope diagnostics.
- Security Events, tool evidence, audit, metrics, graph nodes, and dashboard
  state for Gmail/Drive actions.

## Acceptance Criteria

- Gmail and Drive cannot run outside approved profile capabilities.
- Tool calls produce canonical evidence and redacted provenance.
- UI/status can explain missing scopes, expired credentials, disabled APIs, and
  policy denials.
