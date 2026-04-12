# Sprint T4: MCP Unit Tests

## Goal

Add Rust `#[cfg(test)]` unit tests to `capsem-mcp`, covering parameter serialization, environment variable handling, error mapping, schema constants, and tool router registration.

## Files

**Modify:**
- `crates/capsem-mcp/src/main.rs` -- add `#[cfg(test)] mod tests`

## Tasks

### CreateParams serde

- [ ] `CreateParams` serializes field names in camelCase (e.g., `sandbox_name` -> `sandboxName`)
- [ ] `CreateParams` deserializes from camelCase JSON
- [ ] Roundtrip: serialize then deserialize produces identical struct
- [ ] Missing optional fields deserialize to `None`/defaults
- [ ] Extra unknown fields are rejected

### All param types roundtrip

- [ ] `ExecParams` serde roundtrip (sandbox_name, command, timeout)
- [ ] `WriteFileParams` serde roundtrip (sandbox_name, path, content)
- [ ] `ReadFileParams` serde roundtrip (sandbox_name, path)
- [ ] `DeleteParams` serde roundtrip (sandbox_name)
- [ ] `ListParams` serde roundtrip (empty struct or optional filters)
- [ ] `InspectParams` serde roundtrip (sandbox_name, query)
- [ ] `InfoParams` serde roundtrip (sandbox_name)
- [ ] `LogsParams` serde roundtrip (sandbox_name, optional lines)
- [ ] `ShellParams` serde roundtrip (sandbox_name)

### Environment variable handling

- [ ] `CAPSEM_RUN_DIR` override: when set, UDS path uses the override directory
- [ ] `CAPSEM_RUN_DIR` unset: falls back to `HOME`-based default path
- [ ] `HOME` fallback: constructs correct socket path from home directory
- [ ] Both unset: returns clear error (not a panic)

### Error mapping

- [ ] Connection refused maps to user-friendly "service not running" message
- [ ] VM not found maps to MCP error with correct error code
- [ ] Timeout maps to MCP error with correct error code
- [ ] Internal errors include enough context for debugging

### Schema and router

- [ ] `inspect_schema` constant contains valid JSON schema
- [ ] `inspect_schema` includes required fields (sandbox_name, query)
- [ ] Tool router registers exactly 9 tools
- [ ] Each registered tool name matches expected string (create, exec, write_file, read_file, delete, list, info, inspect, logs)
- [ ] Duplicate tool registration is rejected or idempotent

## Verification

- `cargo test -p capsem-mcp` passes all new tests
- No tests require a running service or VM
- Tests complete in under 2 seconds total

## Depends On

- **T0-infrastructure** (coverage tooling)
