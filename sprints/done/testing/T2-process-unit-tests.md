# Sprint T2: Process Unit Tests

## Goal

Add Rust `#[cfg(test)]` unit tests to `capsem-process`, covering CLI arg parsing, IPC message routing, job correlation, terminal I/O, and FD cloning. These are fast, in-process tests with no VM dependencies.

## Files

**Modify:**
- `crates/capsem-process/src/main.rs` -- add `#[cfg(test)] mod tests`

## Tasks

### CLI arg parsing

- [ ] All required args present: parses successfully with correct values
- [ ] Missing required arg: returns descriptive error for each required arg
- [ ] Optional args use correct defaults when omitted
- [ ] Invalid value types (e.g., non-numeric RAM) produce clear error

### IPC routing

- [ ] `Ping` message routes to `Pong` response
- [ ] `Exec` message creates job_store entry and returns `ExecResult`
- [ ] `WriteFile` message routes correctly and returns acknowledgment
- [ ] `ReadFile` message routes correctly and returns file content
- [ ] `Shutdown` message triggers clean exit path
- [ ] Unknown/malformed message returns error without crashing

### Job correlation

- [ ] Concurrent jobs get unique correlation IDs
- [ ] Looking up a missing job ID returns appropriate error
- [ ] Dropped sender (client disconnect) does not panic or leak resources
- [ ] Job completion removes entry from job store

### Terminal I/O

- [ ] Output broadcast reaches all subscribed listeners
- [ ] Input forwarding delivers bytes to correct PTY
- [ ] Resize event propagates new dimensions
- [ ] Unsubscribe stops delivery without affecting other listeners

### FD cloning

- [ ] Cloned FDs are independent (closing one does not affect the other)
- [ ] Operations on cloned FDs do not block each other
- [ ] Invalid FD clone returns error

## Verification

- `cargo test -p capsem-process` passes all new tests
- No tests require a running VM, socket, or service
- Tests complete in under 5 seconds total

## Depends On

- **T0-infrastructure** (coverage tooling)
