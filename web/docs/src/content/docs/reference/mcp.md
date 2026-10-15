---
title: MCP reference
description: Which Capsem MCP interfaces and tools are available and what authority do they have?
---

Capsem has two MCP interfaces: a host server for managing VMs, and a guest relay
for tools used inside a VM. They have different credentials and permissions.

## Interfaces

| Interface | Transport | Authority |
| --- | --- | --- |
| `@capsem/mcp` / `capsem-mcp` | Host stdio MCP → TypeScript SDK → authenticated gateway HTTP. | API operations allowed to its host caller. No direct VM-owner, database, or virtualization access. |
| Guest `capsem-mcp-server` | Stdio → framed vsock → VM-assigned MCP capability. | VM-scoped tool policy. Receives no gateway bearer credential. |
| Native aggregator | Granted RPC and brokered HTTP. | Dispatch to configured external HTTP MCP servers and first-party builtins. Confined before readiness. |

Configured external servers currently use HTTP. A host stdio program in a
third-party MCP client configuration is not automatically an admitted native
server executable.

## Host server installation and options

Requires Node.js 20 or later, installed Capsem, and gateway access.

```sh
npm install --global @capsem/mcp
```

| Input | Contract |
| --- | --- |
| `--gateway-url URL` | Explicit HTTP(S) gateway endpoint. |
| `CAPSEM_GATEWAY_TOKEN` | Host process bearer credential. |
| `--token-file PATH` | Alternative file containing the bearer token. |
| `--timeout-ms N` | SDK HTTP request deadline; default 30000. |
| stdout / stderr | MCP protocol only / startup diagnostics. |

The server does not start Capsem or discover a local gateway. Tokens are not
accepted as command-line values. Keep the gateway credential in the host
client's secret configuration, never in a VM environment or tool argument.

## Host tools

VM tools use `vm_id`, the immutable ID returned by create/list.
Use the MCP client's `tools/list` response for the exact JSON input schema
and descriptions of every argument.

| Purpose | Tools |
| --- | --- |
| Service and inventory | `capsem_status`, `capsem_list`, `capsem_info`. |
| Creation and lifecycle | `capsem_create`, `capsem_run`, `capsem_start`, `capsem_stop`, `capsem_pause`, `capsem_resume`, `capsem_delete`, `capsem_fork`, `capsem_persist`, `capsem_purge`. |
| Image catalog and prefetch | `capsem_image_list`, `capsem_image_pull`. |
| Files | `capsem_list_files`, `capsem_read_file`, `capsem_write_file`. |
| Container and ports | `capsem_container_status`, `capsem_port_open`, `capsem_port_list`, `capsem_port_close`. |
| Activity and diagnostics | `capsem_vm_logs`, `capsem_host_logs`, `capsem_panics`, `capsem_triage`, `capsem_timeline`, `capsem_history`, `capsem_stats`, `capsem_stats_detail`. |
| Networks | `capsem_network_create`, `capsem_network_list`, `capsem_network_inspect`, `capsem_network_delete`, `capsem_network_attach`, `capsem_network_detach`, `capsem_network_logs`. |
| Configured MCP servers | `capsem_mcp_info`, `capsem_mcp_servers`, `capsem_mcp_default`, `capsem_mcp_tools`, `capsem_mcp_refresh`, `capsem_mcp_call`. |

`capsem_create` accepts name, CPU count, memory in GiB, environment, network
IDs, image, command, and optional registry access. Names retain VMs; omitted
names create temporary VMs. `capsem_run` executes a finite command in a
service-managed VM.

File tools preserve UTF-8/base64 bytes. Guest paths are workspace-relative or
within the VM/container workspace; a single read returns a bounded window.
Host log sources are allowlisted: service, mcp, gateway, tray, app.

Port creation accepts `guest_port`, optional `host_port` (0 selects one),
and optional `authenticate`. The SDK selects the VM or container namespace.
Authenticated publication returns browser bootstrap material to the caller;
do not put it into ordinary logs.

## Guest tools

Configured tool names are namespaced as `SERVER__TOOL`. Permission checks use
the server identity and tool identity, not a tool name alone.

The guest-local `capsem__expose_port` tool requires an explicit `vm` or
`container` target and guest/host port values. The VM owner supplies identity,
policy admission, and audit records. This tool does not pass through the
generic external-server aggregator.

See [Enforce security policy](/guides/enterprise/enforce-security-policy/) to
restrict a server or tool, and [Security architecture](/architecture/security/)
for the native worker sandboxes.

## Results, errors, and cancellation

Successful host calls include `structuredContent`. Failures set MCP
`isError` and return a machine-readable error kind without HTTP response
bodies or low-level causes.

`timeout_secs` bounds guest execution; `--timeout-ms` bounds HTTP.
Cancellation closes the request, but does not reverse an accepted mutation.
Mutations are not automatically retried. Canonical host tool names are
`capsem_pause` and `capsem_status`; legacy suspend/version duplicates are absent.
