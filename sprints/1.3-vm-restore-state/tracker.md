# Sprint: 1.3 VM Restore State

## Tasks
- [x] Reproduce: installed `capsem list` shows two stopped VMs; `resume` fails on profile payload hash mismatch.
- [x] Root cause: list/info do not surface profile payload drift even though resume correctly rejects it.
- [x] Contract patch: inactive persistent VMs expose typed `Incompatible` state/reason.
- [x] UI/CLI action gating: frontend disables start when `can_resume=false`; CLI displays incompatible reason.
- [x] Tests: service drift list/info, strict lifecycle serde, gateway status tests, CLI client tests, frontend check.
- [ ] Debug note only: verify the `capsem` binary and TUI both reflect every VM
  lifecycle state and never offer resume/start for `Defunct` or
  `Incompatible` VMs.
- [ ] Debug note only: ensure `capsem purge` and the TUI purge action delete
  defunct VM rows/directories while preserving valid stopped/suspended VMs.
- [ ] Debug note only: add TDD coverage before implementation: TUI resume
  shortcut/enter disabled for non-resumable states; purge removes defunct
  persistent VMs; purge does not remove healthy resumable VMs.
- [ ] Installed verification deferred by explicit instruction: do not kill/reinstall/touch installed runtime.
- [ ] Commit/push.

## Pending Debug Loop Notes

- User paused implementation and asked to take notes only. Do not patch code
  until explicitly resumed.
- Contract: the state enum is the source of truth. UI/TUI/CLI must display the
  state and reason returned by the service, not infer a resumable action from a
  loose status string.
- Contract: a VM is resumable only when the service says `can_resume=true`.
  `Stopped` without `can_resume=true` is not enough.
- Contract: `Defunct` means the VM is not recoverable through resume. The
  command/UI should make that visible and purge should remove it.
- Contract: purge must not be a dangerous broad cleanup. Default purge should
  delete defunct/broken VM state and stale failed runtime debris; valid
  stopped/suspended VMs remain unless an explicit destructive option exists and
  is tested.

## Coverage Ledger
- Unit/contract: service list/info drift tests pass; lifecycle serde rejects unknown/missing states.
- Functional: source CLI/gateway/frontend checks pass; installed CLI check deferred by instruction not to touch runtime.
- Adversarial: existing direct resume drift rejection remains; new list/info report `Incompatible` before action.
- E2E/VM: not run in this slice by instruction not to touch installed runtime.
- Telemetry/observability: not applicable; this is state contract presentation.
- Performance: not applicable.
