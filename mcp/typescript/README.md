# Capsem MCP

An MCP stdio server backed by `@capsem/sdk` and authenticated gateway HTTP.
It does not discover or start local services and never reads Capsem UDS or VM
runtime state directly.

```sh
capsem-mcp --gateway-url http://127.0.0.1:19222 --token "$CAPSEM_TOKEN" --timeout-ms 30000
```

stdout is reserved for MCP protocol messages. Startup diagnostics are sanitized
and written to stderr.
