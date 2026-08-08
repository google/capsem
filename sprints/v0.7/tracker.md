# Capsem v0.7 tracker

## Current state

- [x] Preserve the GUI feasibility work and transport findings in the design.
- [x] Establish the v0.7 architecture, delivery tranches, security model, and
  hermetic test plan.
- [x] Reconcile credential material refs with session-bound grants.
- [x] Make terminal history retention a measured configurable budget rather
  than a fixed 10,000-row promise.
- [x] Move the design package out of ignored `tmp/` state and deprecate legacy
  sprint directories without deleting their evidence.
- [ ] Approve the remaining open decisions in design section 19.
- [ ] Begin T0 typed contract implementation and generation.

## Contract rule

Profiles and SessionSpec declare credential grant requests. Raw material enters
through the credential-binding endpoint. The engine privately stores the
content-addressed material ref and issues a session-bound grant. Public APIs
expose neither identifier.

## Verification ledger

- Contract syntax: JSON Schema and OpenAPI snapshots parse successfully.
- Structural: `sprints/` contains only `v0.7` and `deprecated` at top level.
- Functional/E2E/VM/security/performance: specified in `design.md`; deferred to
  their implementation tranches. This documentation commit claims no runtime
  implementation proof.
