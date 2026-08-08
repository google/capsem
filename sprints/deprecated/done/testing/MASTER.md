# Testing Master Sprint Tracker

25 sub-sprints covering the full next-gen daemon test infrastructure.

## Status

### Phase 1: Foundation (Done)

| Sprint | Name | Status | Tests | Depends On |
|--------|------|--------|-------|------------|
| T0 | [Infrastructure](T0-infrastructure.md) | Done | - | - |
| T1 | [Service Unit Tests](T1-service-unit-tests.md) | Done (15 Rust) | Rust | T0 |
| T2 | [Process Unit Tests](T2-process-unit-tests.md) | Done (18 Rust, IPC) | Rust | T0 |
| T3 | [CLI Tests](T3-cli-tests.md) | Done (22 Rust + 11 Python) | Rust + Python | T0 |
| T4 | [MCP Unit Tests](T4-mcp-unit-tests.md) | Done (12 Rust) | Rust | T0 |

### Phase 2: Integration (Done)

| Sprint | Name | Status | Tests | Depends On |
|--------|------|--------|-------|------------|
| T5 | [Service Integration](T5-service-integration.md) | Done (38 tests) | Python | T0 |
| T6 | [Session.db Telemetry](T6-session-db-tests.md) | Done (22 tests) | Python | T5 |
| T7 | [Snapshots](T7-snapshot-tests.md) | Done (9 tests) | Python | T5 |
| T8 | [VM Isolation](T8-isolation-tests.md) | Done (7 tests) | Python | T5 |
| T9 | [Config Obedience](T9-config-tests.md) | Done (8 tests) | Python | T5 |
| T10 | [Security](T10-security-tests.md) | Done (11 tests) | Python | T5 |
| T11 | [Bootstrap](T11-bootstrap-tests.md) | Done (21 tests) | Python | T0 |
| T12 | [Doctor Transition](T12-doctor-transition.md) | Done (recipes) | Just | T3, T5 |
| T13 | [Stress Tests](T13-stress-tests.md) | Done (3 tests) | Python | T5, T8 |

### Phase 3: Build Validation & E2E (Done)

| Sprint | Name | Status | Tests | Depends On |
|--------|------|--------|-------|------------|
| T14 | [Sign Fixtures](T14-sign-fixtures.md) | Done (fix) | Helper | T0 |
| T15 | [Build Chain E2E](T15-build-chain.md) | Done (8 tests) | Python | T14 |
| T16 | [Guest Validation](T16-guest-validation.md) | Done (14 tests) | Python | T14 |
| T17 | [Cleanup Verification](T17-cleanup.md) | Done (4 tests) | Python | T14 |
| T18 | [Codesign Strict](T18-codesign-strict.md) | Done (7 tests) | Python | T14 |
| T19 | [Serial Console](T19-serial-console.md) | Done (4 tests) | Python | T14 |
| T20 | [Session.db Lifecycle](T20-session-lifecycle.md) | Done (7 tests) | Python | T14 |
| T21 | [Config Runtime](T21-config-runtime.md) | Done (5 tests) | Python | T14 |
| T22 | [Recipe Smoke](T22-recipe-tests.md) | Done (4 tests) | Python | T14 |
| T23 | [Recovery](T23-recovery.md) | Done (4 tests) | Python | T14 |
| T24 | [ROOTFS Artifacts](T24-rootfs-artifacts.md) | Done (7 tests) | Python | T0 |
| T25 | [Session.db Exhaustive](T25-session-db-tables.md) | Done (20 tests) | Python | T14 |

## Just Recipes

```
just test                  # Fast unit tests (Rust + Python, no VM)
just test-vm               # VM tests: build chain, guest, cleanup, serial, session lifecycle,
                           #   codesign, config runtime, recovery
just test-service          # Backend: HTTP API, IPC, config, hot-reload
just test-build-chain      # Build chain E2E (cargo build -> codesign -> pack -> boot)
just test-guest            # Guest validation (network, services, filesystem, env)
just test-cleanup          # VM cleanup verification (process, socket, session dir)
just test-codesign         # Codesigning strict (FAIL not skip)
just test-serial           # Serial console + boot timing
just test-session-lifecycle # Session.db lifecycle
just test-config-runtime   # Config runtime (CPU, RAM, blocked domains)
just test-recipes          # Just recipe smoke tests
just test-recovery         # Recovery and crash-resilience
just test-rootfs           # Rootfs artifact validation (no VM)
just test-session-exhaustive # Exhaustive per-table session.db
just test-all              # All tests combined
```

## Companion

[implementation-tasks.md](implementation-tasks.md) -- what the coding team must build for tests to pass.
