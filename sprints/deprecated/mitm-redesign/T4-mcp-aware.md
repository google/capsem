# T4: mcp-protocol-aware-mitm

**Status:** Not Started
**Depends on:** T1
**Blocks:** T5

## Goal

Make the MITM proxy MCP-protocol-aware. Detect JSON-RPC over HTTP (content-type + envelope sniffing), classify method names, emit `mcp_calls` rows from the host MITM for HTTP-transport MCP servers (e.g., remote streamable-HTTP MCP). Populate the dormant `tool_calls.mcp_call_id` FK so model→tool→MCP correlation is end-to-end across model_calls and mcp_calls.

## Deliverables

- `crates/capsem-core/src/net/parsers/jsonrpc_parser.rs` — bytes → `JsonRpcMessage`; chunk-boundary safe; bounded buffer.
- `crates/capsem-core/src/net/parsers/jsonrpc_parser/{tests.rs, fixtures/*.rmp}` — happy / malformed / batched / notification / oversized; replay corpora from real Smithery / Anthropic remote MCP.
- `crates/capsem-core/src/net/interpreters/mcp_interpreter.rs` — L2 `JsonRpcMessage` → L3 `McpCall` (classify methods: `tools/list`, `tools/call`, `prompts/list`, `prompts/get`, `resources/list`, `resources/read`, `initialize`, ...).
- `crates/capsem-core/src/net/interpreters/mcp_interpreter/{tests.rs, fixtures/*.rmp}` — extensive method classification tests.
- `crates/capsem-logger/` — `mcp_calls` writer reachable from the host MITM (currently only the in-guest gateway writes here); `tool_calls.mcp_call_id` populated from a correlation step in the interpreter.
- `mitm.mcp_methods_total{method}` counter wired.

## Acceptance

- A guest talking to a streamable-HTTP MCP server through the MITM produces `mcp_calls` rows with `trace_id` and method classification.
- For a session with both LLM API calls (model_calls) and MCP tool calls (mcp_calls), the corresponding `tool_calls.mcp_call_id` FK is populated.
- `mitm.mcp_methods_total{method="tools/call"}` increments per `tools/call`.
- Chunk-boundary fuzz harness for `jsonrpc_parser` survives 60s.
- ≥40 unit tests in `jsonrpc_parser/tests.rs` and `mcp_interpreter/tests.rs`.
- `inspect-session` joins `model_calls` ↔ `tool_calls` ↔ `mcp_calls` on `trace_id` for a known LLM-tool-MCP chain.
- `mitm-load` baseline regression check passes.

## Commit shape

Three expected commits:
1. `feat(mitm): JSON-RPC parser hook + MCP interpreter` — parser + interpreter + tests + fixtures.
2. `feat(mitm): host-side mcp_calls emission + trace_id` — wire the interpreter to the writer.
3. `feat(mitm): correlate tool_calls.mcp_call_id` — finally populate the dormant FK.
