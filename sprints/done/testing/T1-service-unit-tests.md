# Sprint T1: Service Unit Tests

## Goal

Add Rust `#[cfg(test)]` unit tests to `capsem-service`, covering API type serialization, state management, asset resolution, and daemon lifecycle. These are fast, in-process tests with no VM or network dependencies.

## Files

**Modify:**
- `crates/capsem-service/src/main.rs` -- add `#[cfg(test)] mod tests`
- `crates/capsem-service/src/api.rs` -- add `#[cfg(test)] mod tests`

## Tasks

### API type serde roundtrips (`api.rs`)

- [ ] `ProvisionRequest` serializes and deserializes with all fields
- [ ] `ProvisionResponse` serializes and deserializes with all fields
- [ ] `ListResponse` serializes and deserializes (empty list, populated list)
- [ ] `ExecRequest` serializes and deserializes with all fields
- [ ] `ExecResponse` serializes and deserializes with all fields
- [ ] `ReadFile` request/response serde roundtrip
- [ ] `WriteFile` request/response serde roundtrip
- [ ] `Inspect` request/response serde roundtrip
- [ ] `SandboxInfo` serializes and deserializes with all fields
- [ ] Verify unknown fields are rejected (deny_unknown_fields where applicable)

### State management (`main.rs`)

- [ ] Instance map: insert, lookup, remove (CRUD)
- [ ] Job counter atomicity: concurrent fetch_add returns unique IDs
- [ ] Duplicate name rejection: provisioning with an existing name returns error
- [ ] Auto-ID format: generated IDs match expected pattern (prefix + random)
- [ ] Stale socket discovery: detect and clean up leftover UDS files

### Asset resolution (`main.rs`)

- [ ] Version path resolves correctly for current arch
- [ ] Fallback to previous version when current missing
- [ ] Arch fallback works (aarch64 -> arm64 alias, if applicable)
- [ ] Missing assets return a clear error with expected path

### Daemon lifecycle (`main.rs`)

- [ ] Binding to an already-in-use socket path fails with clear error
- [ ] Stale socket (no listener) is removed and rebound successfully
- [ ] Delete cascade: deleting a VM cleans up its process, socket, and state entry
- [ ] Delete nonexistent VM returns 404-equivalent error

## Verification

- `cargo test -p capsem-service` passes all new tests
- No tests require a running VM, network, or filesystem side effects
- `cargo test -p capsem-service -- --nocapture` shows no warnings

## Depends On

- **T0-infrastructure** (coverage tooling, marker setup)
