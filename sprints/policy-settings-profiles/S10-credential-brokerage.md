# S10 - Credential Brokerage

## Goal

Implement the promised credential release capability.

## Tasks

- Define service-settings credential storage and profile release policy.
- Add service broker APIs.
- Add audit events for credential release decisions.
- Test allowed, denied, missing, stale, locked, and audited releases.
- Evaluate Keychain as stretch work if the TOML-first cutover is stable.

## Coverage Ledger

- Unit/contract: release policy evaluation.
- Functional: broker API tests.
- Adversarial: missing/stale credentials, denied releases, profile lockout.
- E2E/VM: session credential materialization proof.
- Telemetry: audit events prove release/denial.
- Performance: not primary.
