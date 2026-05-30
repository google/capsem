# Sprint: Guest Lifecycle Cleanup

## Tasks
- [x] Disable in-VM shutdown dispatch in `capsem-sysutil`
- [x] Stop installing shutdown/halt/poweroff/reboot symlinks in `capsem-init`
- [x] Keep guest suspend lifecycle path
- [x] Ignore deprecated shutdown frames in `capsem-process`
- [x] Remove guest-shutdown integration expectations
- [x] Update docs, skills, and changelog
- [x] Run focused verification
- [x] Commit and push

## Coverage Ledger
- Unit/contract: `cargo test -p capsem-agent --bin capsem-sysutil`, `cargo test -p capsem-proto`, `cargo test -p capsem-process`
- Functional: `cargo check -p capsem-service`, Python lifecycle files compile
- Adversarial: fresh `target/debug/capsem-sysutil shutdown` and `argv[0]=shutdown` checks fail with the disabled message
- E2E/VM: `just exec "capsem-doctor -k lifecycle"` passed in isolated `CAPSEM_HOME`; added `TestGuestShutdownDisabled::test_capsem_sysutil_shutdown_does_not_stop_vm`
- Telemetry: not touched
- Performance: not touched
- Missing/deferred: full `just test` deferred for this small cleanup
