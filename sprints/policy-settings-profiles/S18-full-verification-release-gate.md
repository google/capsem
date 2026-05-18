# S18 - Full Verification And Release Gate

## Goal

Prove the redesign is releaseable.

## Tasks

- Run backend tests for settings, profiles, assembly, APIs, CLI, enforcement.
- Run frontend tests for settings, profiles, rules, security capabilities.
- Run E2E profile create/fork/delete/select/launch.
- Run manifest/profile-catalog install/update/remove/revoke tests.
- Run profile-backed VM create with missing assets to prove first-use download,
  signature/hash verification, VM pinning, and successful boot.
- Run resume-after-profile-update tests to prove existing VMs keep their pinned
  profile revision and asset hashes.
- Prove MCP, skills, AI providers, credential brokerage, PII, and canonical
  rules enforce through VM-effective settings.
- Prove fresh install still works after v1 removal.
- Prove asset cleanup preserves files referenced by installed active/deprecated
  profile revisions and existing VM pins, and removes unreferenced
  removed/revoked profile assets.

## Coverage Ledger

- Unit/contract: complete.
- Functional: complete.
- Adversarial: complete.
- E2E/VM: complete.
- Telemetry: complete.
- Performance: complete or explicitly waived with rationale.
