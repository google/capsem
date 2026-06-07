# Sprint T3: CLI Tests

## Goal

Add Rust unit tests and Python integration tests for the `capsem` CLI binary. Also identifies missing CLI commands that need implementation before integration tests can run.

## Files

**Modify:**
- `crates/capsem/src/main.rs` -- add `#[cfg(test)] mod tests`

**Create:**
- `tests/capsem-cli/conftest.py` -- shared fixtures (service launcher, CLI runner)
- `tests/capsem-cli/test_start.py`
- `tests/capsem-cli/test_list.py`
- `tests/capsem-cli/test_exec.py`
- `tests/capsem-cli/test_info.py`
- `tests/capsem-cli/test_stop.py`
- `tests/capsem-cli/test_delete.py`
- `tests/capsem-cli/test_status.py`
- `tests/capsem-cli/test_shell.py`
- `tests/capsem-cli/test_errors.py`

## Missing Commands (implementation required before Python integration tests)

These commands need to be implemented or stubbed in `crates/capsem/src/main.rs`:

- [ ] `capsem exec <name> <command>` -- execute a command in a running VM
- [ ] `capsem delete <name>` -- delete a stopped or running VM
- [ ] `capsem info <name>` -- show detailed VM information
- [ ] `capsem stop <name>` -- stop a running VM (stub: return "not implemented" initially)
- [ ] `capsem status` -- show service daemon status (stub)
- [ ] `capsem doctor` -- check host environment prerequisites

## Tasks

### Rust unit tests (`main.rs`)

- [ ] Clap parsing: `start` subcommand with all flags (--name, --ram, --cpus, --timeout)
- [ ] Clap parsing: `list` subcommand with optional --json flag
- [ ] Clap parsing: `shell` subcommand with name argument
- [ ] Clap parsing: `exec` subcommand with name and command arguments
- [ ] `--uds-path` override changes the socket path used by UdsClient
- [ ] UdsClient request/response serde: verify JSON encoding matches API types
- [ ] RAM string conversion: "2G" -> bytes, "512M" -> bytes, invalid -> error
- [ ] Error messages: connection refused -> "is the service running?"
- [ ] Error messages: VM not found -> includes the name that was looked up
- [ ] List formatting: empty list prints header only, populated list aligns columns

### Python integration tests (require running service + VM)

- [ ] `test_start.py`: start VM with default args, start with custom name, start with resource flags, start duplicate name fails
- [ ] `test_list.py`: list empty, list after start shows VM, list --json produces valid JSON
- [ ] `test_exec.py`: exec echo returns stdout, exec false returns nonzero exit, exec with stderr, exec on nonexistent VM fails
- [ ] `test_info.py`: info shows name/ram/cpus/state, info nonexistent fails
- [ ] `test_stop.py`: stop running VM succeeds, stop already-stopped fails, stop nonexistent fails
- [ ] `test_delete.py`: delete running VM succeeds, delete nonexistent fails, delete twice fails
- [ ] `test_status.py`: status when service running shows "ok", status when service stopped shows error
- [ ] `test_shell.py`: shell connects and Ctrl+] disconnects cleanly
- [ ] `test_errors.py`: no args prints usage, unknown subcommand prints error, service not running prints helpful message

## Verification

- `cargo test -p capsem` passes all Rust unit tests
- `pytest tests/capsem-cli/ -m "not integration"` passes (unit-level Python tests, if any)
- `pytest tests/capsem-cli/ -m integration` passes with a running service
- All missing commands are at least stubbed before integration tests run

## Depends On

- **T0-infrastructure** (test directories, markers, recipes)
- **T1-service-unit-tests** (service must be stable before CLI integration tests)
