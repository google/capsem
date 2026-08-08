# VM Restore State Fix

## Why
Installed 1.3 correctly fails closed when a persistent VM was created under an older profile payload hash. The UI/CLI list contract still renders those entries as ordinary `Stopped` VMs, so users see restore/start actions that cannot succeed.

## Root Cause
`PersistentVmEntry` stores `profile_revision`, `profile_payload_hash`, and asset pins. `resume_sandbox` validates them before boot and rejects drift. `handle_list`/`handle_info` only map registry flags to `Stopped`, `Suspended`, or `Defunct`; they do not compute profile/payload/asset compatibility for inactive persistent entries.

## Tasks
- [ ] Add compatibility validation to list/info for persistent entries without mutating the registry.
- [ ] Expose a clear status/reason/actionability contract for incompatible VMs.
- [ ] Gate CLI/UI restore/start actions on that contract.
- [ ] Add tests for payload-hash drift in list/info and UI action state.
- [ ] Verify installed behavior with the two existing stale VMs.

## Done
`capsem list` and the UI no longer present stale profile-pinned VMs as restorable. `resume` still fails closed if called directly. Fresh VM creation remains unaffected.
