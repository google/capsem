# F06 - Credential Brokerage Foundation

## Goal

Release credentials into sessions through Profile V2 policy, audit, and UI
contracts.

## Scope

- Credential source discovery handoff from `credential-pipeline`.
- Service settings credential storage and profile release policy.
- Google account brokerage as a first-class credential family.
- Deep Google integration handoff for Gmail, Drive, gcloud, Firebase, Jet Ski,
  Gemini, Antigravity, Google AI provider settings, and any Google-backed
  connector so one approved Google account can satisfy the relevant profile
  capabilities.
- Token family mapping for Google OAuth, gcloud ADC, service-account JSON,
  Firebase project credentials, Gmail/Drive scopes, and Gemini API/provider
  credentials without collapsing them into one unsafe secret type.
- Session materialization, denial, stale/missing/locked handling, and audit.
- CLI/UI/status/docs through frozen Profile V2 terms.

## Acceptance Criteria

- Credentials are released only through explicit profile/session policy.
- Google-backed capabilities share a coherent account/release model instead of
  requiring disconnected per-feature setup.
- Gmail, Drive, gcloud, Firebase, Jet Ski, Gemini, and Google AI credentials
  have explicit source, scope, freshness, revocation, and audit behavior.
- Allowed, denied, missing, stale, locked, and audited releases are tested.
- No raw credential value leaks into logs, support bundles, or status/debug.
