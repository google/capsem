# Sprint: 1.3 Unknown MCP Network Sniffing

## Tasks

- [x] T0 failing integration test
- [x] T1 bounded MCP JSON-RPC classifier
- [x] T2 MITM emission through security writer
- [x] T3 MCP rule block path
- [x] Changelog
- [x] Verification
- [x] Commit and push

## Notes

- This fixes the asymmetry where unknown model-shaped HTTP is promoted into
  `model.*`, but unknown remote MCP-over-HTTP stayed only `http.*`.
- No new DB path. Use `emit_security_write(WriteOp::McpCall(...))`.
- Observed remote MCP server identity is derived from `host:port/path` as
  `observed:<host>:<port><path>` until the profile declares it.
- Blocking is proven against the real CEL path with
  `mcp.tool_call.name == "search_web"`.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-core mcp_http --lib -- --nocapture`
  proves MCP sniffing is JSON/content-length bounded and only accepts MCP
  JSON-RPC method shapes.
- Functional: `cargo test -p capsem-core --test mitm_integration
  mitm_proxy_plain_http_unknown_mcp_shape_emits_mcp_call -- --nocapture`
  proves remote MCP-over-HTTP still emits HTTP telemetry and now emits one
  first-party `McpCall`.
- Adversarial/enforcement: `cargo test -p capsem-core --test mitm_integration
  mitm_proxy_plain_http_unknown_mcp_shape_can_be_blocked_by_mcp_rule --
  --nocapture` proves an `mcp.*` CEL rule blocks before upstream and writes a
  denied `McpCall`.
- Observability: promoted requests log `mcp_method`, `mcp_server`, `mcp_tool`,
  host, path, and bounded body bytes.
- Missing/deferred: profile-declared correlation is still a future enhancement;
  this slice records observed identity without pretending it is declared.

## Final Gate

- `cargo fmt --check`
- `cargo test -p capsem-core mcp_http --lib -- --nocapture`
- `cargo test -p capsem-core provider_detection -- --nocapture`
- `cargo test -p capsem-core unknown_model_body_sniffing --lib -- --nocapture`
- `cargo test -p capsem-core --test mitm_integration mitm_proxy_plain_http_unknown_mcp_shape -- --nocapture`
- `cargo test -p capsem-core --test mitm_integration mitm_proxy_plain_http_unknown_openai_shape_emits_model_call -- --nocapture`
- `git diff --check`
