# Sprint: 1.3 VM Restore State

## Tasks
- [x] Reproduce: installed `capsem list` shows two stopped VMs; `resume` fails on profile payload hash mismatch.
- [x] Root cause: list/info do not surface profile payload drift even though resume correctly rejects it.
- [x] Contract patch: inactive persistent VMs expose typed `Incompatible` state/reason.
- [x] UI/CLI action gating: frontend disables start when `can_resume=false`; CLI displays incompatible reason.
- [x] Tests: service drift list/info, strict lifecycle serde, gateway status tests, CLI client tests, frontend check.
- [ ] Installed verification deferred by explicit instruction: do not kill/reinstall/touch installed runtime.
- [ ] Commit/push.

## Coverage Ledger
- Unit/contract: service list/info drift tests pass; lifecycle serde rejects unknown/missing states.
- Functional: source CLI/gateway/frontend checks pass; installed CLI check deferred by instruction not to touch runtime.
- Adversarial: existing direct resume drift rejection remains; new list/info report `Incompatible` before action.
- E2E/VM: not run in this slice by instruction not to touch installed runtime.
- Telemetry/observability: not applicable; this is state contract presentation.
- Performance: not applicable.
