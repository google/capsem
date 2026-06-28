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

The service is the only global runtime object. Profiles own behavior and
configuration. Sessions execute profiles.

## Verb Discipline

Route suffixes are part of the contract:

| Suffix | Meaning |
|---|---|
| `info` | Static or slow-changing configuration, descriptors, file origins, schema metadata, and debug facts. |
| `status` | Runtime readiness, counters, progress, and liveness. Status routes must avoid hot-path DB reads unless explicitly documented. |
| `list` | Inventory of child objects. |
| `latest` | Recent ledger rows, including event ids needed for forensic lookup. |
| `evaluate` | Dry-run a supplied event or rule payload through the production evaluator. |
| `edit` | Mutate an existing settings/profile/plugin/rule object through its typed contract. |
| `reload` | Re-read persisted profile, corp, rule, detection, or catalog material. |
| `ensure` | Materialize or download missing profile assets. |
| `create`, `delete`, `clone`, `fork`, `save`, `start`, `resume`, `pause`, `stop`, `restart` | Command routes with explicit side effects. |

Unknown routes must return 404 at the gateway or service boundary. No generic
path forwarding is allowed.

## Service-Global Routes

These routes describe the daemon, service-wide runtime summaries, or global
catalog entry points. They are not profile behavior.

| Method | Route | Contract |
|---|---|---|
| `GET` | `/version` | Installed service version. |
| `GET` | `/update/status` | Binary, VM asset, profile, and image update availability from the installed manifest and release-channel cache. |
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
| `GET` | `/profiles/list` | Profile catalog visible to this service. |
| `GET` | `/profiles/status` | Profile readiness and asset status summary. |
| `POST` | `/profiles/reload` | Reload the profile catalog. |
| `GET` | `/settings/info` | UI/application settings, not VM behavior. |
| `PATCH` | `/settings/edit` | Edit UI/application settings. |
| `GET` | `/corp/info` | Corporate constraints and reporting config. |
| `PUT` | `/corp/edit` | Replace corporate constraints where local policy permits. |
| `POST` | `/corp/validate` | Validate corporate config without applying it. |
| `POST` | `/corp/reload` | Reload corporate config. |

## Profile Routes

Profile routes are scoped by `profile_id`. Rules, detection, plugins, MCP,
skills, assets, and profile metadata all belong here.

| Method | Route | Contract |
|---|---|---|
| `GET` | `/profiles/{profile_id}/info` | Profile descriptor, icon, description, VM defaults, and file origins. |
| `GET` | `/profiles/{profile_id}/obom` | Base-image OBOM evidence for this profile. |
| `POST` | `/profiles/{profile_id}/validate` | Validate the profile and pinned files. |
| `POST` | `/profiles/{profile_id}/reload` | Reload one profile. |
| `GET` | `/profiles/{profile_id}/assets/info` | Profile asset declaration and origins. |
| `GET` | `/profiles/{profile_id}/assets/status` | Per-asset readiness, hash, and missing/download state. |
| `POST` | `/profiles/{profile_id}/assets/ensure` | Download or materialize missing profile assets. |

### Enforcement and Detection

| Method | Route | Contract |
|---|---|---|
| `POST` | `/profiles/{profile_id}/enforcement/evaluate` | Evaluate a supplied `SecurityEvent` against profile enforcement rules. |
| `GET` | `/profiles/{profile_id}/enforcement/info` | Enforcement file origins and compile status. |
| `GET` | `/profiles/{profile_id}/enforcement/rules/list` | Compiled enforcement rules with source/default/priority/action metadata. |
| `PUT` | `/profiles/{profile_id}/enforcement/rules/{rule_id}/edit` | Add or replace one profile enforcement rule. |
| `DELETE` | `/profiles/{profile_id}/enforcement/rules/{rule_id}/delete` | Delete one mutable profile enforcement rule. |
| `POST` | `/profiles/{profile_id}/enforcement/reload` | Reload enforcement rules for the profile. |
| `POST` | `/profiles/{profile_id}/detection/evaluate` | Evaluate a supplied event against profile detection rules. |
| `GET` | `/profiles/{profile_id}/detection/info` | Detection file origins and compile status. |
| `GET` | `/profiles/{profile_id}/detection/rules/list` | Compiled detection rules, including Sigma-derived rules. |
| `PUT` | `/profiles/{profile_id}/detection/rules/{rule_id}/edit` | Add or replace one profile detection rule. |
| `DELETE` | `/profiles/{profile_id}/detection/rules/{rule_id}/delete` | Delete one mutable profile detection rule. |
| `POST` | `/profiles/{profile_id}/detection/reload` | Reload detection rules for the profile. |

### Plugins

Plugins expose profile config and registry-owned descriptors. Runtime plugin
activity for a running session appears under session stats and security ledger
routes.

| Method | Route | Contract |
|---|---|---|
| `GET` | `/profiles/{profile_id}/plugins/info` | Plugin subsystem info for the profile. |
| `GET` | `/profiles/{profile_id}/plugins/list` | Profile plugin config plus registry metadata. |
| `GET` | `/profiles/{profile_id}/plugins/{plugin_id}/info` | One plugin descriptor, config, capabilities, stages, and status schema. |
| `PATCH` | `/profiles/{profile_id}/plugins/{plugin_id}/edit` | Enable, disable, or edit one plugin config object. |
| `GET` | `/profiles/{profile_id}/plugins/credential_broker/credentials/info` | Credential broker inventory summary without raw secrets. |

### MCP

MCP is profile-owned. There is no global MCP tool list.

| Method | Route | Contract |
|---|---|---|
| `GET` | `/profiles/{profile_id}/mcp/info` | Profile MCP subsystem info. |
| `GET` | `/profiles/{profile_id}/mcp/default/info` | Default MCP policy for this profile. |
| `PATCH` | `/profiles/{profile_id}/mcp/default/edit` | Edit the profile default MCP action. |
| `GET` | `/profiles/{profile_id}/mcp/servers/list` | MCP servers declared or discovered for this profile. |
| `PUT` | `/profiles/{profile_id}/mcp/servers/{server_id}/edit` | Add or replace one profile MCP server. |
| `DELETE` | `/profiles/{profile_id}/mcp/servers/{server_id}/delete` | Delete one profile MCP server. |
| `POST` | `/profiles/{profile_id}/mcp/servers/{server_id}/refresh` | Refresh one server's tool/resource inventory. |
| `GET` | `/profiles/{profile_id}/mcp/servers/{server_id}/tools/list` | Tools for one MCP server. |
| `PATCH` | `/profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/edit` | Edit one tool's action for this profile. |
| `POST` | `/profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/call` | Call one MCP tool through the audited service path. |

### Skills

Skills are profile-owned. The current routes reserve the profile-scoped control
surface; implementation must keep skill metadata and mutation behind the
profile contract.

| Method | Route | Contract |
|---|---|---|
| `GET` | `/profiles/{profile_id}/skills/info` | Profile skill subsystem info. |
| `GET` | `/profiles/{profile_id}/skills/list` | Skills enabled or available for the profile. |
| `POST` | `/profiles/{profile_id}/skills/add` | Add a skill to the profile. |
| `PATCH` | `/profiles/{profile_id}/skills/{skill_id}/edit` | Edit one profile skill. |
| `DELETE` | `/profiles/{profile_id}/skills/{skill_id}/delete` | Delete one profile skill. |

## Session Routes

Session routes are runtime operations for one existing session id. User-facing
UI can call these sessions; internal debug output may still mention VM where it
describes virtualization state.

| Method | Route | Contract |
|---|---|---|
| `POST` | `/vms/create` | Create a new session from a profile. |
| `GET` | `/vms/list` | List sessions. |
| `GET` | `/vms/{id}/info` | Session config/runtime info, including profile, process, and storage diagnostics. |
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
| `POST` | `/vms/{id}/exec` | Execute a command through the audited control path. |
| `POST` | `/vms/{id}/files/write` | Write a file through the audited control path. |
| `POST` | `/vms/{id}/files/read` | Read a file through the audited control path. |
| `GET` | `/vms/{id}/files/list` | List files through the service file browser route. |
| `GET` | `/vms/{id}/files/content` | Download file content through the service route. |
| `POST` | `/vms/{id}/files/content` | Upload file content through the service route. |
| `GET` | `/vms/{id}/snapshots/status` | Snapshot subsystem readiness for the session. |
| `GET` | `/vms/{id}/snapshots/list` | Snapshot entries exposed by the snapshot subsystem, not security activity. |
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

- The UI/TUI must use profile routes for profile behavior and settings routes
  only for UI/application preferences.
- Profile cards render name, description, icon, readiness, and asset checklist
  from profile route data.
- Enforcement, detection, plugins, MCP, assets, and skills pages are scoped by
  profile id.
- Session actions are state-dependent. Incompatible or defunct sessions must
  not offer start/resume/pause actions.
- Raw JSON is a debug view. Normal panels should render the typed fields once.
