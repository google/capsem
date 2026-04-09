# Sprint: Install Lifecycle (S14)

`just install` must deploy all binaries (including gateway and tray), restart the service so it picks up new code, and leave the system in a testable state. Currently the install script only copies 4 of 6 binaries and does not restart anything.

Depends on: core changes landing on next-gen (suspend/resume, gateway, tray).
Crate changes: none (justfile + scripts + tests only).

## Current State

- `simulate-install.sh` copies: capsem, capsem-service, capsem-process, capsem-mcp
- Missing: **capsem-gateway**, **capsem-tray**
- `just install` does not stop/restart the running service
- `host_binaries` in justfile includes gateway but not tray
- Service binary in `~/.capsem/bin/` is from April 3 -- no companion spawning code
- Tray was started manually, not by the service

## Sub-sprints

### SS0: Clean System Baseline

Status: TODO

Gate: nothing else in this sprint starts until SS0 is 100% green. The system must be fully clean -- no warnings, no skipped tests, no known failures. Fix everything, don't defer.

- [ ] `cargo build --workspace` succeeds with zero warnings (fix all warnings, not suppress)
- [ ] `cargo clippy --workspace` passes clean (zero warnings, zero errors)
- [ ] `cargo test --workspace` passes -- all unit tests across all crates green
- [ ] `cargo test -p capsem-tray` passes (47 tests)
- [ ] `cargo test -p capsem-gateway` passes
- [ ] `cargo test -p capsem-service` passes
- [ ] `just smoke` passes on next-gen with all current changes
- [ ] All new code has tests (no untested code paths shipped)
- [ ] No `#[allow(dead_code)]` on new code -- if it's dead, delete it
- [ ] No `todo!()`, `unimplemented!()`, or `// TODO` in shipped code paths
- [ ] No stale/orphaned processes from previous runs (clean PID files)

### SS1: Add Missing Binaries to Install

Status: TODO

- [ ] Add `capsem-gateway` and `capsem-tray` to `simulate-install.sh` binary copy loop
- [ ] Add `capsem-tray` to `host_binaries` and `host_crates` in justfile
- [ ] Verify: `just install` copies 6 binaries to `~/.capsem/bin/`
- [ ] Codesign loop already globs `capsem*` so gateway/tray get signed automatically

### SS2: Service Restart on Install

Status: TODO

- [ ] `just install` must stop the running service before copying binaries
- [ ] After copy + codesign, restart the service
- [ ] Service restart should use the same mechanism as `_ensure-service` recipe
- [ ] If service was running via launchctl/systemd, use `capsem service restart` or reload unit
- [ ] If service was running as a foreground process, kill + re-spawn
- [ ] Gateway and tray come up automatically via new service companion spawning
- [ ] Verify: after `just install`, `ps aux | grep capsem` shows service + gateway + tray

### SS3: Graceful Stop Before Install

Status: TODO

- [ ] Send graceful shutdown to running service (SIGTERM to service PID from `service.pid`)
- [ ] Wait up to 10s for service to exit (it kills gateway + tray via kill_on_drop)
- [ ] If service doesn't exit, SIGKILL
- [ ] Clean up stale PID files (`service.pid`, `gateway.pid`, `gateway.port`, `gateway.token`)
- [ ] Verify: no orphaned gateway/tray processes after install

### SS4: Smoke Test Post-Install

Status: TODO

- [ ] After restart, verify gateway is reachable: `curl http://127.0.0.1:$(cat gateway.port)/status`
- [ ] Verify tray process is running (macOS only)
- [ ] Verify service responds on UDS
- [ ] Add post-install health check to `just install` recipe (fail loudly if service doesn't come up)

### SS5: Tray End-to-End Verification

Status: TODO (manual, after SS1-SS4)

- [ ] Tray icon appears in macOS menu bar
- [ ] Click menu -- shows Permanent/Temporary sections (or empty + global actions if no VMs)
- [ ] Icon is black template (idle, adapts to light/dark)
- [ ] Provision a temp VM -- icon turns purple
- [ ] Stop VM -- icon returns to idle
- [ ] Kill gateway process -- icon turns red, menu shows "Service unavailable"
- [ ] Gateway restarts (service respawns it or manual) -- tray recovers via token hot-reload
- [ ] "Quit" exits tray cleanly
- [ ] Mark tray sprint acceptance criteria complete

## Acceptance Criteria

- [ ] `just install` deploys 6 binaries: capsem, capsem-service, capsem-process, capsem-mcp, capsem-gateway, capsem-tray
- [ ] Running service is gracefully stopped before binary replacement
- [ ] Service auto-restarts after install with new binaries
- [ ] Gateway and tray spawn automatically from new service
- [ ] Post-install health check passes (service + gateway reachable)
- [ ] `just install` is idempotent (safe to run multiple times)

## Reference

- Install script: `scripts/simulate-install.sh`
- Service companion spawn: `crates/capsem-service/src/main.rs` (spawn_companions)
- Tray sprint: `sprints/tray/tracker.md`
- Native installer sprint: `sprints/native-installer/tracker.md`
- Just recipes: `justfile` (install, _ensure-service, smoke)
