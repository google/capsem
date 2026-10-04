---
title: Service API
description: Route contract and verb discipline for the Capsem service and gateway.
sidebar:
  order: 4
---

Capsem clients talk to `capsem-service` through one explicit HTTP route table.
The desktop UI, TUI, CLI, tray, and gateway must reflect these routes; they must
not invent fallback paths, compatibility aliases, or display-only contract
names.

The service is the only global runtime object. Policy comes from the built-in
defaults, `~/.capsem/settings.toml`, and the corp config; every session boots
the same runtime image set and enforces that merged policy.

The API contract is version 3.0.0. Request bodies refuse unknown fields with
a 400 rather than ignoring them.

## Verb Discipline

Route suffixes are part of the contract:

| Suffix | Meaning |
|---|---|
| `info` | Static or slow-changing configuration, descriptors, file origins, schema metadata, and debug facts. |
| `status` | Runtime readiness, counters, progress, and liveness. Status routes must avoid hot-path DB reads unless explicitly documented. |
| `list` | Inventory of child objects. |
| `latest` | Recent ledger rows, including event ids needed for forensic lookup. |
| `edit` | Mutate an existing settings, plugin, or MCP permission object through its typed contract. |
| `reload` | Re-read persisted corp or credential-store material. |
| `ensure` | Download missing runtime assets. |
| `create`, `delete`, `clone`, `fork`, `save`, `start`, `resume`, `pause`, `stop`, `restart` | Command routes with explicit side effects. |

Unknown routes must return 404 at the gateway or service boundary. No generic
path forwarding is allowed.

## Service-Global Routes

These routes describe the daemon and service-wide runtime summaries.

| Method | Route | Contract |
|---|---|---|
| `GET` | `/status` | Service status, including `assets` (runtime asset readiness). |
| `GET` | `/version` | Installed service version. |
| `GET` | `/update/status` | Binary, VM asset, and image update availability from the installed manifest and release-channel cache. |
| `GET` | `/stats` | Service-wide runtime counters. |
| `GET` | `/service-logs` | Service log tail for diagnostics. |
| `GET` | `/triage` | Structured support bundle summary. |
| `GET` | `/panics` | Recent panic/crash evidence. |
| `GET` | `/host-logs/{name}` | Named host-side log stream. |
| `POST` | `/purge` | Delete defunct service/session state that is no longer recoverable. |
| `POST` | `/run` | Compatibility command for creating/running a session through the service path. |
| `GET` | `/security/latest` | Service-wide recent security ledger rows. |
| `GET` | `/security/status` | Service-wide security counters. |
| `GET` | `/enforcement/latest` | Service-wide recent enforcement ledger rows. |
| `GET` | `/enforcement/status` | Service-wide enforcement counters. |
| `GET` | `/detection/latest` | Service-wide recent detection ledger rows. |
| `GET` | `/detection/status` | Service-wide detection counters. |
| `GET` | `/settings/info` | The unified `settings.toml` tree with corp locks and validation issues. |
| `PATCH` | `/settings/edit` | Batch-edit `app.*`/`appearance.*` preferences in `settings.toml` and return the refreshed tree; corp-owned ids are refused. |
| `GET` | `/corp/info` | Corporate constraints and reporting config. |
| `PUT` | `/corp/edit` | Replace corporate constraints where local policy permits. |
| `POST` | `/corp/validate` | Validate corporate config without applying it. |
| `POST` | `/corp/reload` | Reload corporate config. |

## Assets

Every VM boots one runtime image set: the kernel, initrd, and rootfs of the
installed manifest's release for this host's architecture. A persistent VM
keeps the asset pins it was created with.

| Method | Route | Contract |
|---|---|---|
| `GET` | `/assets/status` | Readiness of the runtime image set: per-asset presence, expected hash and size, manifest provenance, and reconciliation progress. |
| `POST` | `/assets/ensure` | Start a reconciliation that downloads missing assets; answers with the same status and whether this call started it. |

## Policy

The policy a VM enforces is the built-in defaults, the user's
`~/.capsem/settings.toml`, and the corp config, merged per session into
`vm/active_policy.toml`. Corp wins. The `edit` routes below write
`settings.toml`, record the change in the host ledger table
`policy_mutation_events`, and put it in force in every running VM -- each VM
acknowledges the exact policy digest -- before they answer. An edit to a value
corp already decides is refused.

### Plugins

Runtime plugin activity for a running session also appears under session stats
and security ledger routes.

| Method | Route | Contract |
|---|---|---|
| `GET` | `/plugins/list` | Every catalogued plugin with its effective mode and whether settings or corp set it. |
| `GET` | `/plugins/{plugin_id}/info` | One plugin's descriptor, effective config, and runtime activity across sessions. |
| `PATCH` | `/plugins/{plugin_id}/edit` | Set one plugin's mode or detection level. |
| `GET` | `/plugins/credential_broker/credentials/info` | Credential broker store, grants, and brokered activity, without raw secrets. |
| `POST` | `/plugins/credential_broker/credentials/reload` | Retry the durable credential store and re-read session counters. |

### MCP

Every VM runs the same MCP servers: `settings.toml` `[mcp]` with corp's laid
over it.

| Method | Route | Contract |
|---|---|---|
| `GET` | `/mcp/info` | Server count and whether the built-in `local` server is enabled. |
| `GET` | `/mcp/servers/list` | Configured servers and their discovery status. |
| `GET` | `/mcp/default/info` | Default MCP tool permission and who set it. |
| `PATCH` | `/mcp/default/edit` | Set the default MCP tool permission. |
| `GET` | `/mcp/servers/{server_id}/tools/list` | One server's discovered tools with effective permissions. |
| `POST` | `/mcp/servers/{server_id}/refresh` | Rediscover one server's tools in every running VM. |
| `PATCH` | `/mcp/servers/{server_id}/tools/{tool_id}/edit` | Allow, ask, or block one tool. |
| `POST` | `/mcp/servers/{server_id}/tools/{tool_id}/call` | Call one tool through a running VM's aggregator, under that VM's policy. |

There are no rule list, evaluate, or reload routes; rules are edited in
`settings.toml` (or corp) and read back through `/settings/info`.

## Session Routes

Session routes are runtime operations for one existing session id. User-facing
UI can call these sessions; internal debug output may still mention VM where it
describes virtualization state.

| Method | Route | Contract |
|---|---|---|
| `POST` | `/vms/create` | Create a new session from the runtime image set (default 4 CPUs, 12 GiB RAM, 64 GiB scratch disk). |
| `GET` | `/vms/list` | List sessions. |
| `GET` | `/vms/{id}/info` | Session config/runtime info, including resources, resume eligibility, process, and storage diagnostics. |
| `GET` | `/vms/{id}/status` | In-memory session liveness, readiness, state, and counters. |
| `POST` | `/vms/{id}/stop` | Stop a running session. |
| `POST` | `/vms/{id}/pause` | Pause or suspend a running session. |
| `POST` | `/vms/{id}/start` | Start a stopped session. |
| `POST` | `/vms/{id}/resume` | Resume a paused or stopped session through the service path. |
| `DELETE` | `/vms/{id}/delete` | Delete a session. |
| `POST` | `/vms/{id}/save` | Persist session state. |
| `GET` | `/vms/{id}/save/status` | Save progress/status. |
| `POST` | `/vms/{id}/fork` | Fork a session. |
| `GET` | `/vms/{id}/fork/status` | Fork progress/status. |
| `GET` | `/vms/{id}/logs` | Session log stream. |
| `POST` | `/vms/{id}/exec` | Execute a command; each output names `utf8` or `base64` encoding. |
| `GET` | `/vms/{id}/files/list` | List files through the service file browser route. |
| `GET` | `/vms/{id}/files/content` | Download exact file bytes through the audited service route. |
| `POST` | `/vms/{id}/files/content` | Upload exact file bytes through the audited service route. |
| `GET` | `/vms/{id}/timeline` | Session timeline. |
| `GET` | `/vms/{id}/history` | Session history. |
| `GET` | `/vms/{id}/history/processes` | Process history. |
| `GET` | `/vms/{id}/history/counts` | History counters. |
| `GET` | `/vms/{id}/history/transcript` | Terminal transcript history. |
| `GET` | `/vms/{id}/security/latest` | Recent security ledger rows for this session. |
| `GET` | `/vms/{id}/security/status` | Security counters for this session. |
| `GET` | `/vms/{id}/enforcement/latest` | Recent enforcement ledger rows for this session. |
| `GET` | `/vms/{id}/enforcement/status` | Enforcement counters for this session. |
| `GET` | `/vms/{id}/detection/latest` | Recent detection ledger rows for this session. |
| `GET` | `/vms/{id}/detection/status` | Detection counters for this session. |

## UI/TUI Rules

- Plugins, MCP, and assets pages use the global `/plugins`, `/mcp`, and
  `/assets` routes; there is no per-VM scope.
- Session actions are state-dependent. Incompatible or defunct sessions must
  not offer start/resume/pause actions. A persistent VM created from a VM
  profile before profiles were removed is Incompatible and can only be
  deleted.
- Raw JSON is a debug view. Normal panels should render the typed fields once.
