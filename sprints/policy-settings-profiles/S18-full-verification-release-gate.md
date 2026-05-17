# S18 - Full Verification And Release Gate

## Goal

Prove the redesign is releaseable.

## Tasks

- Run backend tests for settings, profiles, assembly, APIs, CLI, enforcement.
- Run frontend tests for settings, profiles, rules, security capabilities.
- Run E2E profile create/fork/delete/select/launch.
- Prove MCP, skills, AI providers, credential brokerage, PII, and canonical
  rules enforce through VM-effective settings.
- Prove fresh install still works after v1 removal.

## Coverage Ledger

- Unit/contract: complete.
- Functional: complete.
- Adversarial: complete.
- E2E/VM: complete.
- Telemetry: complete.
- Performance: complete or explicitly waived with rationale.
