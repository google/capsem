---
title: MCP Tools
description: Configure the SDK-backed Capsem MCP server and use its typed tools.
sidebar:
  order: 2
---

`@capsem/mcp` is a standalone Node.js stdio server for AI clients. It uses
`@capsem/sdk` for every operation and communicates only with the authenticated
Capsem HTTP gateway. It does not open the service UDS, inspect VM state, or
start Capsem services.

## Install and configure

Install the npm package separately from the native Capsem package:

```sh
npm install --global @capsem/mcp
```

The native installer does not install Node.js or download npm packages. Pass an
explicit gateway URL, bearer token, and transport timeout when registering the
server. The token is read from `CAPSEM_GATEWAY_TOKEN` or `--token-file`;
`--token` is refused because argv is visible to every local process. For
example:

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

Use your MCP client's secret or environment-variable support rather than
committing the gateway token. The token belongs to the host MCP process. Do not
copy it into a VM, container, tool argument, or webpage.

stdout carries MCP protocol messages only. Sanitized diagnostics use stderr.
Successful tools return structured content. Failures set `isError` and return a
machine-readable category and, for HTTP failures, a status code. Gateway bodies,
tokens, registry credentials, and low-level causes are omitted from MCP errors.

`--timeout-ms` is the HTTP transport deadline. `timeout_secs` on `capsem_exec`
and `capsem_run` is the guest command deadline. Set the HTTP timeout high enough
for the guest deadline. Cancelling a tool call cancels its local HTTP request;
it does not delete the VM and cannot undo a mutation the gateway already
accepted. Mutations are not retried automatically.

## VM lifecycle and files

All VM-scoped tools take the immutable `vm_id` returned by `capsem_create` or
`capsem_list`.

| Tool | Main parameters | Purpose |
| --- | --- | --- |
| `capsem_status` | — | Read gateway and service status. |
| `capsem_list` | — | List VMs and their typed lifecycle state. |
| `capsem_create` | `profile?`, `name?`, `cpus?`, `memory?`, `env?`, `network_ids?`, `image?`, `command?`, `registry?`, `attach?` | Create a detached workload; an omitted profile uses the catalog default. Memory is GiB; an image selects OCI execution. |
| `capsem_info` | `vm_id` | Read VM identity, resources, network, files, and telemetry. |
| `capsem_exec` | `vm_id`, `command`, `timeout_secs?` | Execute in an existing VM. |
| `capsem_run` | `command`, `profile?`, `cpus?`, `memory?`, `env?`, `timeout_secs?` | Execute once in a temporary VM. |
| `capsem_start` / `capsem_stop` | `vm_id` | Start or stop a VM. |
| `capsem_pause` / `capsem_resume` | `vm_id` | Pause or resume a VM. |
| `capsem_delete` | `vm_id` | Delete the VM and its owned state. |
| `capsem_fork` | `vm_id`, `name`, `description?` | Create a stopped fork. |
| `capsem_persist` | `vm_id`, `name` | Retain an ephemeral VM under a stable name. |
| `capsem_purge` | `all?` | Purge service-reported disposable state. |
| `capsem_list_files` | `vm_id`, `path?`, `depth?` | List workspace files. |
| `capsem_read_file` | `vm_id`, `path`, `encoding?`, `offset?`, `max_bytes?` | Read a bounded window of file content as UTF-8 or base64. |
| `capsem_write_file` | `vm_id`, `path`, `content`, `encoding?` | Write UTF-8 or base64 bytes. |
| `capsem_container_status` | `vm_id` | Read workload diagnostics; creation already waits for readiness. |
| `capsem_port_open` | `vm_id`, `guest_port`, `host_port?`, `authenticate?` | Open a workload port; target selection is automatic. |
| `capsem_port_list` / `capsem_port_close` | `vm_id`, `port_id?` | Inspect or close the workload's ports. |

File transfers use the gateway's existing file API and require a running VM's
security ledger. Cancelling a request never deletes a VM.

## Diagnostics and audit

| Tool | Main parameters | Purpose |
| --- | --- | --- |
| `capsem_vm_logs` | `vm_id`, `grep?`, `tail?`, `max_bytes?` | Read VM serial and process logs. |
| `capsem_host_logs` | `source?`, `grep?`, `tail?`, `max_bytes?` | Read an allowlisted host log. |
| `capsem_panics` | `since?`, `limit?` | Read structured host panics. |
| `capsem_triage` | `vm_id?`, `since?`, `limit?` | Correlate host and optional VM failures. |
| `capsem_timeline` | `vm_id`, `trace_id?`, `since?`, `limit?`, `layers?` | Read correlated session events. |
| `capsem_history` | `vm_id`, `search?`, `limit?`, `offset?` | Read command and audit history. |
| `capsem_stats` / `capsem_stats_detail` | `vm_id` | Read aggregate or typed detailed telemetry. |

These tools query the same logger-owned data used by the SDK and UI. They do not
open SQLite directly or maintain a second projection cache.

## Private networks

`capsem_network_create`, `capsem_network_list`, `capsem_network_inspect`, and
`capsem_network_delete` manage networks by immutable `network_id`.
`capsem_network_attach` and `capsem_network_detach` change membership with an
immutable `vm_id`. `capsem_network_logs` reads cursor-based audit events and
accepts VM, connection, event type, decision, and time filters.

## Profiles and guest MCP tools

| Tool | Purpose |
| --- | --- |
| `capsem_profiles` | List the typed profile catalog. |
| `capsem_mcp_info` | Read MCP configuration and readiness for a profile. |
| `capsem_mcp_servers` | List configured servers for a profile. |
| `capsem_mcp_default` | Read the profile's default MCP permission. |
| `capsem_mcp_tools` | List tools for one `server_id`. |
| `capsem_mcp_refresh` | Refresh discovery for one `server_id`. |
| `capsem_mcp_call` | Invoke one `tool_id` with native JSON arguments. |

Profile MCP calls still travel through the running VM's guest relay, policy
engine, aggregator, and logger. Listing tools does not create phantom call rows.
Allowed and denied calls retain trusted VM, trace, server, tool, decision, byte,
and security-rule correlation in the existing ledgers.

A guest agent also discovers `capsem__expose_port` from its VM-owned endpoint.
It must select the `container` or `vm` namespace explicitly and may request host
port zero for allocation. Trusted VM identity comes from the existing relay;
the schema accepts no VM ID, gateway token, or control socket. MCP admission and
tool-call logging wrap the request, then the VM owner's existing exposure policy
and network audit run before any listener can forward traffic.

`capsem_pause` and `capsem_status` are the canonical tool names. The npm server
does not expose the retired `capsem_suspend`, `capsem_version`, or duplicate
`capsem_service_logs` tools.
