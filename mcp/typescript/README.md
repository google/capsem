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
CAPSEM_GATEWAY_TOKEN="$(cat ~/.capsem/run/gateway.token)" \
  capsem-mcp --gateway-url http://127.0.0.1:19222 --timeout-ms 30000
```

The bearer token is read from `CAPSEM_GATEWAY_TOKEN`, or from the file named by
`--token-file <path>`. It is never accepted as a command-line argument, where
any local process could read it from the process list; `--token` is refused.

An MCP client configuration passes the token through its environment block,
using the client's secret expansion support:

```json
{
  "mcpServers": {
    "capsem": {
      "command": "capsem-mcp",
      "args": ["--gateway-url", "http://127.0.0.1:19222", "--timeout-ms", "30000"],
      "env": {"CAPSEM_GATEWAY_TOKEN": "${CAPSEM_GATEWAY_TOKEN}"}
    }
  }
}
```

stdout is reserved for MCP protocol messages. Startup diagnostics name the
offending option, never its value, and are written to stderr.

The server exposes typed tools for VM and OCI-container creation, lifecycle
actions, container status, port lifecycle, command execution, file
listing and byte-preserving transfers, logs, history, timeline, statistics,
snapshots, panics, and triage. VM tools take the immutable `vm_id` returned by
`capsem_list` or `capsem_create`. File content can be passed as UTF-8 or base64.

Network tools create, list, inspect and retire private networks, attach or detach
VMs by immutable ID, and read cursor-based audit events with VM, connection,
event, decision, and time filters.

Profile tools list the typed profile catalog, inspect MCP readiness and default
permissions, discover or refresh one server, and invoke a tool through gateway
policy enforcement. Successful calls provide `structuredContent`; failures set
MCP `isError` and return a machine-readable error kind without HTTP bodies or
low-level causes.

The bearer token belongs only to this host process. Do not pass it to VM or
guest MCP tools, container environment variables, or workload content. The
server has no service-socket, database, VM-runtime, or virtualization access.

Guest agents discover a separate `capsem__expose_port` tool through their
existing framed relay. It requires an explicit `container` or `vm` target and
accepts only guest/host port values. The per-VM owner supplies trusted identity
and applies the existing MCP and exposure policy/audit rails; the guest receives
no gateway credential. This scoped tool never passes through the generic MCP
aggregator subprocess.

`--timeout-ms` bounds the SDK's HTTP request. Tool parameters named
`timeout_secs` bound guest execution. Cancellation closes the local request but
does not delete a VM or reverse a mutation already accepted by the gateway;
mutations are never retried automatically.

`capsem_pause` and `capsem_status` are the canonical names. Host logs use one
`capsem_host_logs` tool with an allowlisted `source`; the package does not expose
the legacy `suspend`, `version`, or duplicate service-log tools.
