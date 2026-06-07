# Sprint T12: Doctor Transition to Service Daemon

## Goal

Transition the capsem-doctor diagnostic suite from standalone VM execution to running through the service daemon stack. The `capsem doctor` CLI subcommand should start a VM via the service, execute diagnostics, collect results, and tear down -- all through the daemon.

## Files

Modify existing files:
```
crates/capsem/src/main.rs              # Add `doctor` subcommand
crates/capsem-service/src/api.rs       # Ensure exec endpoint supports doctor flow
justfile                               # Update recipes
tests/capsem-bootstrap/test_doctor.py  # Update for service-based doctor
tests/integration_test.py              # Update for service stack
tests/injection_test.py                # Update for service stack
tests/doctor_session_test.py           # Update for service stack
```

## Tasks

### CLI Subcommand
- [ ] Add `capsem doctor` subcommand that connects to the service daemon
- [ ] Doctor flow: start VM -> exec diagnostics -> collect results -> delete VM
- [ ] Display pass/fail summary to terminal with exit code

### Just Recipes
- [ ] Update `just smoke-test` to use capsem CLI + capsem-service (not standalone VM)
- [ ] Add `just run-service` recipe to start the daemon in foreground
- [ ] Add `just run` recipe: start service + boot VM + open shell
- [ ] Add `just ui` recipe: start service + `cargo tauri dev`

### Test Migration
- [ ] Update `doctor_session_test.py` to run doctor through the service
- [ ] Update `integration_test.py` to use service-based VM lifecycle
- [ ] Update `injection_test.py` to use service-based exec

### Verification Checks
- [ ] Verify session.db schema is identical between old and new paths
- [ ] Verify `doctor_session_test.py` passes end-to-end through the service
- [ ] Verify `just smoke-test` passes with the new stack

## Verification

```bash
just run-service &
capsem doctor
just smoke-test
```

Doctor runs through the service daemon, produces the same session.db output, and all migrated tests pass.

## Depends On

- **T3** (CLI exec): `capsem exec` must work before `capsem doctor` can run diagnostics
- **T5** (Service integration): The service daemon must manage VM lifecycle
