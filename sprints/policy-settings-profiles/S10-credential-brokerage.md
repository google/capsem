# S10 - Credential Brokerage

## Goal

Implement the promised credential release capability.

This is a standalone post-bedrock split. Credential brokerage must not mutate
the Profile V2 engine terms. It consumes signed profiles, service settings,
Security Engine decisions, resolved-event logging, CLI/UI route conventions, and
status/debug contracts from the bedrock release.

## Tasks

- Define service-settings credential storage and profile release policy.
- Add service broker APIs.
- Add audit events for credential release decisions.
- Test allowed, denied, missing, stale, locked, and audited releases.
- Evaluate Keychain as stretch work if the TOML-first cutover is stable.
- Add CLI/UI/status/docs only through the frozen bedrock endpoint and event
  vocabulary.

## Coverage Ledger

- Unit/contract: release policy evaluation.
- Functional: broker API tests.
- Adversarial: missing/stale credentials, denied releases, profile lockout.
- E2E/VM: session credential materialization proof.
- Telemetry: audit events prove release/denial.
- Performance: not primary.
