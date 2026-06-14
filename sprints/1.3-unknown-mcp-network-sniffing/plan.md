# Sprint: 1.3 Unknown MCP Network Sniffing

## Goal

Close the gap where VM-installed remote MCP servers are visible as HTTP/DNS
traffic but not as first-party MCP activity unless they use Capsem's framed MCP
rail.

## Contract

- MCP JSON-RPC seen over normal HTTP must still emit normal HTTP telemetry.
- Bounded JSON-RPC MCP request previews must also emit MCP activity:
  `tools/call` as `mcp.tool_call`, `tools/list` as `mcp.tool_list`, and other
  MCP JSON-RPC methods as `mcp.event`.
- Emission must use the canonical security DB writer path:
  `security_engine::emit_security_write(WriteOp::McpCall(...))`.
- MCP rules must be evaluated before forwarding. A blocking rule over
  `mcp.*` must block the HTTP request.
- The implementation must not add another decision engine or bypass the
  SecurityEvent/CEL rail.

## Files

- `crates/capsem-core/src/net/mitm_proxy/mod.rs`
- `crates/capsem-core/tests/mitm_integration.rs`
- `CHANGELOG.md`
- this sprint tracker

## Tests

- RED/GREEN integration test: remote MCP-over-HTTP `tools/call` forwards
  original body, writes one HTTP event, and writes one MCP call.
- Unit tests for bounded MCP preview classification.
- Blocking rule test if feasible within the same MITM path.

## Done

- Focused tests pass.
- Tracker and changelog updated.
- Commit and push before manual AGY testing resumes.
