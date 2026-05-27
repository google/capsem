# F06 - Credential Brokerage Foundation

## Goal

Release credentials into sessions through Profile V2 policy, audit, and UI
contracts.

## Scope

- Credential source discovery handoff from `credential-pipeline`.
- Service settings credential storage and profile release policy.
- Google account brokerage as a first-class credential family.
- Session materialization, denial, stale/missing/locked handling, and audit.
- CLI/UI/status/docs through frozen Profile V2 terms.

## Acceptance Criteria

- Credentials are released only through explicit profile/session policy.
- Allowed, denied, missing, stale, locked, and audited releases are tested.
- No raw credential value leaks into logs, support bundles, or status/debug.
