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
CAPSEM_AGENT_CAPABILITY="$capability" \
  capsem-mcp --gateway-url http://127.0.0.1:19222 --access-id "$access_id" --timeout-ms 30000
```

An operator issues finite scoped access with `POST /agent-access`. Supply its
access ID and the managed capability provided at issuance. Only
`CAPSEM_AGENT_CAPABILITY` carries the secret; the URL and nonsecret access ID
are explicit arguments. Token files, `CAPSEM_GATEWAY_TOKEN`, operator runtime
files, and credential arguments are not supported. Expiry, revocation, or
service restart requires fresh operator-issued access; there is no operator
fallback.

An MCP client configuration passes the capability through its environment block,
using the client's secret expansion support:

```json
{
  "mcpServers": {
    "capsem": {
      "command": "capsem-mcp",
      "args": ["--gateway-url", "http://127.0.0.1:19222", "--access-id", "<operator-issued-access-id>", "--timeout-ms", "30000"],
      "env": {"CAPSEM_AGENT_CAPABILITY": "${CAPSEM_AGENT_CAPABILITY}"}
    }
  }
}
```

stdout is reserved for MCP protocol messages. Startup diagnostics name the
offending option, never its value, and are written to stderr.

The server exposes typed tools for VM and OCI-container creation, lifecycle
actions, container status, port lifecycle, command execution, file
listing and byte-preserving transfers, logs, history, timeline, statistics,
panics, and triage. VM tools take the immutable `vm_id` returned by
`capsem_list` or `capsem_create`. File content can be passed as UTF-8 or base64.

`capsem_image_list` reads the service-admitted catalog with compatible pins and
reported cache state; `unknown` does not prove readiness. Optional `refresh`
requests a fresh catalog read. `capsem_image_pull` prefetches a catalog name or
registry-qualified reference and returns its resolved pin without creating a
VM. These tools accept no registry credentials. Image admission and private
access stay with the service's configured policy.

Network tools create, list, inspect and retire private networks, attach or detach
VMs by immutable ID, and read cursor-based audit events with VM, connection,
event, decision, and time filters.

MCP tools cover the servers every VM runs (settings.toml `[mcp]` with corp's
laid over it): they count and list them, read the default permission, discover
or refresh one server, and invoke a tool through gateway policy enforcement. Successful calls provide `structuredContent`; failures set
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

For development, build the SDK and this package, then run `pnpm prewarm:packed`
before `pnpm run test:mcp`. Prewarming installs
the actual local tarballs in a temporary runtime-only consumer to cache their
dependencies. The acceptance test installs offline with scripts disabled,
checks archive/source/installed payloads and drives the packed executable over
stdio. Both temporary consumers are removed. Set `NPM_CONFIG_CACHE` to select
an isolated npm cache. The gate declares prewarming outside its network sandbox.
