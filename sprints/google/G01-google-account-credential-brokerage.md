# G01 - Google Account And Credential Brokerage

## Goal

Make Google credential release coherent across Gmail, Drive, gcloud, Firebase,
Jet Ski, Gemini, and Google AI.

## Scope

- OAuth accounts and scopes.
- gcloud Application Default Credentials.
- Service-account JSON.
- Firebase project credentials.
- Gemini / Google AI API keys and provider credentials.
- Freshness, revocation, locked/missing/stale behavior, audit, and redaction.

## Acceptance Criteria

- Google-backed capabilities use explicit profile/session policy.
- Credential families are not collapsed into one unsafe secret type.
- Allowed, denied, missing, stale, revoked, locked, and audited paths are
  testable.
