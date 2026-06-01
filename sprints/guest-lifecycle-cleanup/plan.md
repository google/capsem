# Guest Lifecycle Cleanup Plan

## Goal
Remove the in-VM shutdown path now that VM lifecycle is host-owned through the service, CLI, and TUI. Keep guest suspend because it is still useful for persistent VM checkpointing.

## Decisions
- Do not remove protocol variants yet. `GuestToHost::ShutdownRequest` and `ProcessToService::ShutdownRequested` stay as deprecated compatibility frames so older guests or processes cannot break decoding.
- Stop installing `/sbin/shutdown`, `/sbin/halt`, `/sbin/poweroff`, and `/sbin/reboot` in the guest overlay.
- Make direct `/run/capsem-sysutil shutdown|halt|poweroff` calls fail with a clear error instead of sending a lifecycle frame.
- Ignore any old shutdown lifecycle frame at the host boundary.

## Files
- `crates/capsem-agent/src/bin/capsem_sysutil.rs`
- `guest/artifacts/capsem-init`
- `guest/artifacts/diagnostics/test_lifecycle.py`
- `tests/capsem-lifecycle/test_vm_lifecycle.py`
- `crates/capsem-process/src/vsock.rs`
- `crates/capsem-proto/src/lib.rs`
- `crates/capsem-proto/src/ipc.rs`
- `crates/capsem-service/src/main.rs`
- `docs/`, `skills/`, and `CHANGELOG.md`

## Done
- In-VM shutdown commands are not installed by `capsem-init`.
- Direct sysutil shutdown commands fail.
- Host ignores deprecated guest shutdown frames.
- Guest suspend remains wired.
- Focused Rust and Python checks pass.
