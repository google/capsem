---
title: MCP reference
description: Which Capsem MCP interfaces and tools are available and what authority do they have?
---

This reference describes Capsem's two Model Context Protocol (MCP) interfaces:
the host server for managing VMs and the guest relay for tools used inside a VM.
It covers their tools, connection options, results, and different credentials
and permissions.

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
The catalogue below is generated from the host server's `tools/list` response.
Expand a tool's input schema for required arguments, types, defaults, and limits.
Output schemas and annotations appear when declared by the server. See
[Results, errors, and cancellation](#results-errors-and-cancellation) to interpret
returned results.

<!-- MCP-TOOLS:START -->

<!-- Generated from @capsem/mcp tools/list. Do not edit this block. -->

### `capsem_container_status`

Read the current pull, staging, startup, running, exit, or failure state of a VM container.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_create`

Create a detached VM and return its immutable ID.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "name": {
      "type": "string",
      "minLength": 1
    },
    "cpus": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    },
    "memory": {
      "description": "Guest memory in GiB",
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    },
    "env": {
      "type": "object",
      "propertyNames": {
        "type": "string"
      },
      "additionalProperties": {
        "type": "string"
      }
    },
    "network_ids": {
      "type": "array",
      "items": {
        "type": "string",
        "minLength": 1
      }
    },
    "image": {
      "type": "string",
      "minLength": 1
    },
    "command": {
      "type": "array",
      "items": {
        "type": "string"
      }
    },
    "registry": {
      "type": "object",
      "properties": {
        "username": {
          "type": "string"
        },
        "password": {
          "type": "string"
        },
        "ca_pem": {
          "type": "string"
        }
      }
    }
  },
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_delete`

Delete a VM and destroy its owned state.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_exec`

Run a shell command in an existing VM and return stdout, stderr, and exit code.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    },
    "command": {
      "type": "string",
      "minLength": 1
    },
    "timeout_secs": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    }
  },
  "required": [
    "vm_id",
    "command"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_fork`

Fork a VM into a new stopped VM.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    },
    "name": {
      "type": "string",
      "minLength": 1
    },
    "description": {
      "type": "string"
    }
  },
  "required": [
    "vm_id",
    "name"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_history`

Read paginated command and audit history for a VM.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    },
    "limit": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    },
    "offset": {
      "type": "integer",
      "minimum": 0,
      "maximum": 9007199254740991
    },
    "search": {
      "type": "string"
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_host_logs`

Read an allowlisted host log through the authenticated gateway.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "source": {
      "default": "service",
      "type": "string",
      "enum": [
        "service",
        "mcp",
        "gateway",
        "tray",
        "app"
      ]
    },
    "grep": {
      "type": "string"
    },
    "tail": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    },
    "max_bytes": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    }
  },
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_image_list`

Read the service-admitted image catalog, compatible immutable pins and reported cache state. Unknown cache state does not prove readiness.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "refresh": {
      "type": "boolean"
    }
  },
  "$schema": "http://json-schema.org/draft-07/schema#",
  "additionalProperties": false
}
```

</details>

### `capsem_image_pull`

Prefetch an image through service admission and return its resolved pin. Does not create a VM; private access remains service-owned.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "image": {
      "type": "string",
      "minLength": 1
    }
  },
  "required": [
    "image"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#",
  "additionalProperties": false
}
```

</details>

### `capsem_info`

Read lifecycle, resource, storage, network, and telemetry details for a VM.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_list`

List VMs with identity, lifecycle, resources, and telemetry.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {}
}
```

</details>

### `capsem_list_files`

List files in a VM using the gateway file API. Paths are the guest's: /root/x in a VM, /workspace/x in its container, or x relative to the workspace; other absolute paths are refused. An empty path lists the workspace root.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    },
    "path": {
      "default": "",
      "type": "string"
    },
    "depth": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_mcp_call`

Call one discovered MCP tool through gateway policy enforcement.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "server_id": {
      "type": "string",
      "minLength": 1
    },
    "tool_id": {
      "type": "string",
      "minLength": 1
    },
    "arguments": {
      "default": {},
      "allOf": [
        {
          "$ref": "#/definitions/__schema0"
        }
      ]
    }
  },
  "required": [
    "server_id",
    "tool_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#",
  "definitions": {
    "__schema0": {
      "anyOf": [
        {
          "type": "string"
        },
        {
          "type": "number"
        },
        {
          "type": "boolean"
        },
        {
          "type": "null"
        },
        {
          "type": "array",
          "items": {
            "$ref": "#/definitions/__schema0"
          }
        },
        {
          "type": "object",
          "propertyNames": {
            "type": "string"
          },
          "additionalProperties": {
            "$ref": "#/definitions/__schema0"
          }
        }
      ]
    }
  }
}
```

</details>

### `capsem_mcp_default`

Read the default MCP tool permission.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {}
}
```

</details>

### `capsem_mcp_info`

Count the MCP servers every VM runs and whether the built-in local server is enabled.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {}
}
```

</details>

### `capsem_mcp_refresh`

Request fresh tool discovery for one configured MCP server.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "server_id": {
      "type": "string",
      "minLength": 1
    }
  },
  "required": [
    "server_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_mcp_servers`

List the configured MCP servers every VM runs and their discovery status.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {}
}
```

</details>

### `capsem_mcp_tools`

List discovered tools for one configured MCP server.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "server_id": {
      "type": "string",
      "minLength": 1
    }
  },
  "required": [
    "server_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_network_attach`

Attach a VM to a private network and return actual membership state.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "network_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable network ID"
    },
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID"
    }
  },
  "required": [
    "network_id",
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_network_create`

Create a private VM network and return its immutable ID and assigned subnet.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "name": {
      "type": "string",
      "minLength": 1
    }
  },
  "required": [
    "name"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_network_delete`

Retire a private network by immutable ID.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "network_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable network ID"
    }
  },
  "required": [
    "network_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_network_detach`

Detach a VM from a private network and return actual membership state.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "network_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable network ID"
    },
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID"
    }
  },
  "required": [
    "network_id",
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_network_inspect`

Inspect a private network by immutable ID.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "network_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable network ID"
    }
  },
  "required": [
    "network_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_network_list`

List private VM networks and their current members.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {}
}
```

</details>

### `capsem_network_logs`

Read a cursor-based private-network audit stream with optional correlation filters.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "network_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable network ID"
    },
    "cursor": {
      "type": "string"
    },
    "limit": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    },
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID"
    },
    "connection_id": {
      "type": "string",
      "minLength": 1
    },
    "event_type": {
      "type": "string",
      "minLength": 1
    },
    "decision": {
      "type": "string",
      "enum": [
        "allowed",
        "denied",
        "error",
        "redirected"
      ]
    },
    "since_unix_ms": {
      "type": "integer",
      "minimum": 0,
      "maximum": 9007199254740991
    },
    "until_unix_ms": {
      "type": "integer",
      "minimum": 0,
      "maximum": 9007199254740991
    }
  },
  "required": [
    "network_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_panics`

Read structured host panics, newest first.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "since": {
      "type": "string"
    },
    "limit": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    }
  },
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_pause`

Pause a running VM.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_persist`

Persist an ephemeral VM under a stable name.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    },
    "name": {
      "type": "string",
      "minLength": 1
    }
  },
  "required": [
    "vm_id",
    "name"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_port_close`

Close a live workload port.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    },
    "port_id": {
      "type": "string",
      "minLength": 1
    }
  },
  "required": [
    "vm_id",
    "port_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_port_list`

List the live ports owned by a workload.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_port_open`

Open a workload port; the SDK selects its VM or container namespace.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    },
    "guest_port": {
      "type": "integer",
      "minimum": 1,
      "maximum": 65535
    },
    "host_port": {
      "type": "integer",
      "minimum": 0,
      "maximum": 65535
    },
    "authenticate": {
      "type": "boolean"
    }
  },
  "required": [
    "vm_id",
    "guest_port"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_purge`

Purge VMs that the service reports as purgeable.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "all": {
      "type": "boolean"
    }
  },
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_read_file`

Read a bounded window of a VM file as UTF-8 text or base64 through the gateway file API. Paths are the guest's: /root/x in a VM, /workspace/x in its container, or x relative to the workspace; other absolute paths are refused.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    },
    "path": {
      "type": "string",
      "minLength": 1
    },
    "encoding": {
      "default": "utf8",
      "type": "string",
      "enum": [
        "utf8",
        "base64"
      ]
    },
    "offset": {
      "default": 0,
      "type": "integer",
      "minimum": 0,
      "maximum": 9007199254740991
    },
    "max_bytes": {
      "default": 262144,
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 262144
    }
  },
  "required": [
    "vm_id",
    "path"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_resume`

Resume a stopped or paused VM.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_run`

Run a command in a fresh service-managed VM and return stdout, stderr, and exit code.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "command": {
      "type": "string",
      "minLength": 1
    },
    "timeout_secs": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    },
    "cpus": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    },
    "memory": {
      "description": "Guest memory in GiB",
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    },
    "env": {
      "type": "object",
      "propertyNames": {
        "type": "string"
      },
      "additionalProperties": {
        "type": "string"
      }
    }
  },
  "required": [
    "command"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_start`

Start a stopped VM.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_stats`

Read aggregate telemetry statistics for a VM.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_stats_detail`

Read typed security, network, file, process, tool, and model events for a VM.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_status`

Read gateway, VM asset, update, and service status.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {}
}
```

</details>

### `capsem_stop`

Stop a VM.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_timeline`

Read the correlated exec, tool, network, file, and model timeline for a VM.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    },
    "trace_id": {
      "type": "string"
    },
    "since": {
      "type": "string"
    },
    "limit": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    },
    "layers": {
      "type": "array",
      "items": {
        "type": "string",
        "enum": [
          "exec",
          "tool",
          "net",
          "fs",
          "model"
        ]
      }
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_triage`

Correlate recent host failures, optionally with one VM's session.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    },
    "since": {
      "type": "string"
    },
    "limit": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    }
  },
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_vm_logs`

Read serial and process logs for a VM.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    },
    "grep": {
      "type": "string"
    },
    "tail": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    },
    "max_bytes": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "maximum": 9007199254740991
    }
  },
  "required": [
    "vm_id"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

### `capsem_write_file`

Write UTF-8 text or base64 bytes to a VM through the gateway file API. Paths are the guest's: /root/x in a VM, /workspace/x in its container, or x relative to the workspace; other absolute paths are refused. Returns the path the guest sees it at.

<details>
<summary>Input schema</summary>

```json
{
  "type": "object",
  "properties": {
    "vm_id": {
      "type": "string",
      "minLength": 1,
      "description": "Immutable VM ID returned by capsem_list or capsem_create"
    },
    "path": {
      "type": "string",
      "minLength": 1
    },
    "content": {
      "type": "string"
    },
    "encoding": {
      "default": "utf8",
      "type": "string",
      "enum": [
        "utf8",
        "base64"
      ]
    }
  },
  "required": [
    "vm_id",
    "path",
    "content"
  ],
  "$schema": "http://json-schema.org/draft-07/schema#"
}
```

</details>

<!-- MCP-TOOLS:END -->

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
