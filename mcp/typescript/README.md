# Capsem MCP

An MCP stdio server backed by `@capsem/sdk` and authenticated gateway HTTP.
It does not discover or start local services and never reads Capsem UDS or VM
runtime state directly.

```sh
npm install --global @capsem/mcp
```

Node.js is an explicit prerequisite; native Capsem installation does not install
Node or download this package.

```sh
capsem-mcp --gateway-url http://127.0.0.1:19222 --token "$CAPSEM_TOKEN" --timeout-ms 30000
```

stdout is reserved for MCP protocol messages. Startup diagnostics are sanitized
and written to stderr.

The server exposes typed tools for VM listing and creation, lifecycle actions,
command execution, file listing and byte-preserving transfers, logs, history,
timeline, statistics, snapshots, panics, and triage. VM tools take the immutable
`vm_id` returned by `capsem_list` or `capsem_create`. File content can be passed
as UTF-8 or base64.

Network tools create, list, inspect and retire private networks, attach or detach
VMs by immutable ID, and read cursor-based audit events with VM, connection,
event, decision, and time filters.

Profile tools list the typed profile catalog, inspect MCP readiness and default
permissions, discover or refresh one server, and invoke a tool through gateway
policy enforcement. Successful calls provide `structuredContent`; failures set
MCP `isError` and return a machine-readable error kind without HTTP bodies or
low-level causes.

`capsem_pause` and `capsem_status` are the canonical names. Host logs use one
`capsem_host_logs` tool with an allowlisted `source`; the package does not expose
the legacy `suspend`, `version`, or duplicate service-log tools.
