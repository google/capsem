# Sprint: Suspend IPC Schema Guard

## Tasks

- [x] Add failing compatibility test
- [x] Add shared build-info JSON
- [x] Update host binary health checks
- [x] Add service startup guard for `capsem-process`
- [x] Update changelog
- [x] Run focused verification
- [ ] Commit

## Notes

- Discovery: live suspend returned HTTP 500, while `process.log` showed `schema hash mismatch (same version, incompatible enum layout)`.
- Discovery: installed binaries all reported the same package version, so version-only health checks missed the mixed protocol layout.

## Coverage Ledger

- Unit/contract: added same-version/schema-mismatch tests for `capsem status` and service startup process-binary validation; reran host binary version and startup parsing tests
- Functional: `cargo run -q -p capsem-process -- --build-info-json` and `cargo run -q -p capsem-gateway -- --build-info-json` emit schema-bearing build info
- Adversarial: same-version/schema-mismatch fixtures now fail health/startup validation
- E2E/VM: manual live suspend reproduced the failing handshake; post-fix VM smoke not yet run
- Telemetry: process log captured `schema hash mismatch`
- Performance: not applicable
- Missing/deferred: none yet
