# F06 - Credential Brokerage Foundation

## Goal

Release credentials into sessions through Profile V2 policy, audit, and UI
contracts.

## Scope

- Credential source discovery handoff from `credential-pipeline`.
- Service settings credential storage and profile release policy.
- Google account brokerage contract hooks consumed by
  [Google Integration Sprint](../google/MASTER.md).
- Generic token family model that can represent OAuth, ADC, service-account
  JSON, API keys, scoped project credentials, freshness, revocation, and
  audit without collapsing them into one unsafe secret type.
- Session materialization, denial, stale/missing/locked handling, and audit.
- CLI/UI/status/docs through frozen Profile V2 terms.

## Acceptance Criteria

- Credentials are released only through explicit profile/session policy.
- Google-specific source, scope, freshness, revocation, and audit behavior is
  specified in [Google Integration Sprint](../google/MASTER.md) and consumed
  here through the generic credential broker.
- Allowed, denied, missing, stale, locked, and audited releases are tested.
- No raw credential value leaks into logs, support bundles, or status/debug.
