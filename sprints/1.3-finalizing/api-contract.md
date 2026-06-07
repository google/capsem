# 1.3 API Contract Draft

Status: draft for approval before code changes.

## Naming Discipline

Endpoint path words are part of the contract:

| Word | Meaning |
| --- | --- |
| `info` | Configuration/metadata/contract state. No counters. |
| `status` | Runtime/live state, counters, readiness, health, progress. |
| `list` | List child resources. |
| `latest` | DB-backed latest ledger rows. |
| `evaluate` | Run supplied security event fixture through the engine without mutating config. |
| `reload` | Re-read/apply profile-owned rule/config files and push to running VMs when applicable. |
| `edit` | Patch a config object. |

No magic bare `GET /resource/{id}` for 1.3 authoring APIs. Use
`/resource/{id}/info` or `/resource/{id}/status` so callers know whether they
are reading configuration or runtime state.

## Prime Contract

Capsem has one service, many profiles, and VMs execute profiles.

- **Profile owns behavior.** Assets, VM config, enforcement rules, detection
  rules, plugins, MCP servers/tools/resources/prompts, skills, credentials, and
  any other setting that changes what a VM can do or what Capsem observes or
  enforces.
- **Settings own UI preferences only.** Appearance, notifications, UI density,
  and local app preferences. If it changes VM behavior, it is not a setting.
- **Corp owns constraints and reporting.** Corp can lock profile behavior,
  require rules, configure reporting endpoints, and provide detection/enforcement
  inputs that apply over profiles.
- **Service owns runtime state.** Daemon health, installed asset cache status,
  running VM status, and DB-backed runtime ledger views.

Authoring endpoints are profile-addressed. Runtime/reporting endpoints may be
service-global because they report what happened; they do not define policy.
UDS and HTTP expose the same paths, DTOs, and errors.

## Shared Objects

### Serializable Security Event

All enforcement/detection evaluation endpoints accept the same public
serializable security event DTO that the runtime ledger stores.

Required properties:

- Stable event id.
- Profile id when known.
- VM id when known.
- Event type and family from the typed security event contract.
- Typed first-party event body for HTTP, DNS, MCP, model, file, process,
  credential, snapshot, or future explicitly supported families.
- Rule/plugin effects as first-class vectors, not reconstructed summaries.
- Detection events vector. Empty is valid. `detection_level = "none"` is the
  non-detection value.

The ledger DB is the forensic truth. Runtime `latest` endpoints return stored
ledger DTOs, not a projection rebuilt from the active rule set.

### Rule Object

Rules have one shape whether they come from profile enforcement TOML, profile
detection Sigma YAML, corp config, convenience profile sections, or imports.

Core fields:

| Field | Contract |
| --- | --- |
| `id` | Stable id used in logs/endpoints. |
| `name` | Required, lowercase, no spaces, max 64 chars. |
| `match` | CEL expression over the security event DTO. |
| `action` | Enum: `allow`, `ask`, `block`, `preprocess`, `postprocess`, `rewrite`. Default `allow`. |
| `priority` | Integer `[-1000, 1000]` or the sentinel string `default`. User-authored priority defaults to `10`; default catch-all rules use `default`. |
| `corp_locked` | Corp-owned lock marker. User profiles cannot set negative locked corp semantics. |
| `detection_level` | Enum: `none`, `informational`, `low`, `medium`, `high`, `critical`. Default `none`. |
| `plugin` | Optional plugin id. Required for plugin-backed preprocess/postprocess/rewrite behavior. |
| `reason` | Human/audit reason. Required for shipped defaults and corp rules. |
| `group` | Backend grouping hint for UI: `corp`, `profile`, `default`, `mcp`, `credential`, `imported_sigma`, etc. It does not change evaluation semantics. |
| `source` | Source descriptor: profile enforcement TOML, profile detection Sigma YAML, corp overlay, built-in default, or generated convenience rule. |

All rule actions are enums in Rust. No stringly verbs in runtime code.

Default rules are normal rules. There is no `/defaults` endpoint and no special
default engine. `priority = "default"` only means "last catch-all tier".

### Plugin Object

Plugin metadata is backend-owned. Full plugin documentation lives on the docs
site under `/plugins/...`; it is not an API endpoint.

| Field | Contract |
| --- | --- |
| `id` | Stable plugin id. |
| `name` | Backend-owned display name. |
| `description` | Backend-owned short description. |
| `mode` | Enum: `allow`, `ask`, `block`, `rewrite`, `disabled`. |
| `detection_level` | Same enum as rules; default `informational` when enabled unless plugin says otherwise. |
| `required_by_rules` | Rule ids that reference this plugin. |
| `scope` | `profile` or `corp`. |

Invariant: every real enabled profile plugin must be referenced by at least one
effective rule. `dummy_*` debug plugins are exempt and only exist for tests.

## Profile Authoring Plane

### Profiles

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/profiles/list` | List profiles with summary metadata. |
| `POST` | `/profiles/create` | Create a profile. |
| `GET` | `/profiles/{profile_id}/info` | Read the full profile contract. |
| `PATCH` | `/profiles/{profile_id}/edit` | Update profile metadata and profile-owned fields. |
| `DELETE` | `/profiles/{profile_id}/delete` | Delete a profile if no VM/session depends on it. |
| `POST` | `/profiles/{profile_id}/clone` | Clone a profile under a new id/name. |
| `POST` | `/profiles/{profile_id}/validate` | Validate profile plus corp overlay without applying it. |
| `POST` | `/profiles/{profile_id}/reload` | Re-read/apply the profile contract and push to running VMs using it where applicable. |

Profile-owned VM defaults, including CPU, memory, disk sizing, selected assets,
network mechanics, capture limits, MCP, skills, credentials, detection, and
enforcement, are part of `/profiles/{profile_id}/info` and
`/profiles/{profile_id}/edit`. Do not add vague profile subresources such as
`/vm/network/edit`; if a field is profile behavior, it belongs in the profile
contract.

### Profile Assets

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/profiles/{profile_id}/assets/info` | Read asset references selected by the profile. |
| `PATCH` | `/profiles/{profile_id}/assets/edit` | Change asset references selected by the profile. |
| `GET` | `/profiles/{profile_id}/assets/status` | Runtime/cache status for assets required by this profile. |
| `POST` | `/profiles/{profile_id}/assets/ensure` | Download/build/install missing assets required by this profile. |

Service-wide asset cache status can exist separately, but profile asset
selection is profile-owned.

### Enforcement

No separate `rule-files` API. Enforcement owns its rules and source file.
`rules/list` tells the UI every rule and where it came from. `reload` is the
operation that validates/reloads changed enforcement config.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/profiles/{profile_id}/enforcement/info` | Read enforcement config, source file refs, default groups, and reload state. |
| `GET` | `/profiles/{profile_id}/enforcement/rules/list` | List effective enforcement rules for this profile, including `source` and `group`. |
| `PUT` | `/profiles/{profile_id}/enforcement/rules/{rule_id}/edit` | Add or replace a profile-owned enforcement rule. |
| `DELETE` | `/profiles/{profile_id}/enforcement/rules/{rule_id}/delete` | Delete a profile-owned enforcement rule. |
| `POST` | `/profiles/{profile_id}/enforcement/evaluate` | Evaluate a supplied security event fixture against this profile. |
| `POST` | `/profiles/{profile_id}/enforcement/reload` | Validate/reload enforcement config file and push to running VMs using this profile. |

### Detection

No separate `rule-files` API. Detection owns its Sigma/source files.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/profiles/{profile_id}/detection/info` | Read detection config, Sigma/source refs, output mode, and reload state. |
| `GET` | `/profiles/{profile_id}/detection/rules/list` | List effective detection rules for this profile, including `source` and `group`. |
| `PUT` | `/profiles/{profile_id}/detection/rules/{rule_id}/edit` | Add or replace a profile-owned detection rule. |
| `DELETE` | `/profiles/{profile_id}/detection/rules/{rule_id}/delete` | Delete a profile-owned detection rule. |
| `POST` | `/profiles/{profile_id}/detection/evaluate` | Evaluate a supplied security event fixture against this profile. |
| `POST` | `/profiles/{profile_id}/detection/reload` | Validate/reload detection Sigma/source file and push to running VMs using this profile. |

Sigma is a facade/import-export format for detection authoring. Internally it
round-trips through the same rule object when possible. Python Sigma parser
compatibility is a gate for Sigma YAML files.

### Plugins

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/profiles/{profile_id}/plugins/info` | Read plugin configuration summary and validation state for this profile. |
| `GET` | `/profiles/{profile_id}/plugins/list` | List effective plugin config and metadata for the profile. |
| `GET` | `/profiles/{profile_id}/plugins/{plugin_id}/info` | Read one plugin config/metadata object. |
| `PATCH` | `/profiles/{profile_id}/plugins/{plugin_id}/edit` | Enable/disable the plugin and update its mode plus detection logging level. |

Plugins do not define a second policy engine. A plugin can mutate the event,
append detection events, and set/strengthen a decision according to the plugin
contract. A block decision is absolute.

### MCP

There is no global tool list. Tools, resources, and prompts live under an MCP
server.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/profiles/{profile_id}/mcp/info` | Read MCP config summary for this profile. |
| `GET` | `/profiles/{profile_id}/mcp/servers/list` | List MCP servers configured by the profile. |
| `PUT` | `/profiles/{profile_id}/mcp/servers/{server_id}/edit` | Add or replace an MCP server in the profile. |
| `DELETE` | `/profiles/{profile_id}/mcp/servers/{server_id}/delete` | Remove an MCP server from the profile. |
| `GET` | `/profiles/{profile_id}/mcp/servers/{server_id}/status` | Runtime discovery/connection status for one MCP server. |
| `GET` | `/profiles/{profile_id}/mcp/servers/{server_id}/tools/list` | List tools for one MCP server. |
| `PATCH` | `/profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/edit` | Edit per-tool profile config. |
| `GET` | `/profiles/{profile_id}/mcp/servers/{server_id}/resources/list` | List resources for one MCP server. |
| `PATCH` | `/profiles/{profile_id}/mcp/servers/{server_id}/resources/{resource_id}/edit` | Edit per-resource profile config. |
| `GET` | `/profiles/{profile_id}/mcp/servers/{server_id}/prompts/list` | List prompts for one MCP server. |
| `PATCH` | `/profiles/{profile_id}/mcp/servers/{server_id}/prompts/{prompt_id}/edit` | Edit per-prompt profile config. |
| `POST` | `/profiles/{profile_id}/mcp/servers/{server_id}/refresh` | Refresh discovery for one MCP server. |

MCP allow/ask/block is expressed as rules over MCP security event fields. There
is no MCP decision provider.

### Skills

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/profiles/{profile_id}/skills/info` | Read skill config summary for this profile. |
| `GET` | `/profiles/{profile_id}/skills/list` | List skills attached to the profile. |
| `POST` | `/profiles/{profile_id}/skills/add` | Add a skill to the profile. |
| `PUT` | `/profiles/{profile_id}/skills/{skill_id}/edit` | Attach or update a skill in the profile. |
| `DELETE` | `/profiles/{profile_id}/skills/{skill_id}/delete` | Remove a skill from the profile. |

Skill file reads are first-party file events. Rules can detect skill loads by
matching file events.

### Credentials

There is no provider API in 1.3. Provider behavior is detected through network,
model, file, and credential events, then governed by rules. The profile-owned
authoring object is credential/broker configuration and saved credential
references.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/profiles/{profile_id}/credentials/info` | Read credential broker config summary for this profile. |
| `GET` | `/profiles/{profile_id}/credentials/status` | Runtime counters for broker captures, substitutions, failures, and per-credential use counts from OTel/ledger counters. |
| `GET` | `/profiles/{profile_id}/credentials/list` | List brokered credential references and BLAKE3 hashes, not secret values. |
| `GET` | `/profiles/{profile_id}/credentials/{credential_id}/info` | Read one brokered credential reference and BLAKE3 hash metadata. |
| `DELETE` | `/profiles/{profile_id}/credentials/{credential_id}/delete` | Remove one brokered credential reference. |
| `POST` | `/profiles/{profile_id}/credentials/reload` | Re-read credential broker config for this profile. |

Credential capture/substitution is implemented by profile rules plus the
credential broker plugin. Secret values do not appear in API responses.

## Corp Plane

Corp config is not a profile. It constrains profiles and owns reporting.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/corp/info` | Read corp overlay summary. |
| `PUT` | `/corp/edit` | Install or replace corp overlay, if permitted. |
| `POST` | `/corp/validate` | Validate corp overlay without installing. |
| `POST` | `/corp/reload` | Re-read/apply corp overlay, including reporting and remote enforcement endpoint config. |

Corp endpoint fields:

- OpenTelemetry debug/reporting endpoint.
- Sigma/SIEM detection output endpoint. FIXME: implement sink.
- Remote enforcement endpoint. FIXME: implement remote sync.

Corp can provide enforcement TOML and detection Sigma YAML inputs that apply over
profiles. Corp priority may use negative priorities and locks. User profiles may
not create corp-locked negative-priority rules.

## UI Settings Plane

Settings are UI/app preferences only.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/settings/info` | Read UI/app settings only. |
| `PATCH` | `/settings/edit` | Update UI/app settings only. |

Examples: theme, notifications, UI density, local app preferences. No MCP,
credential, plugin, enforcement, detection, asset, or VM-behavior config belongs
here.

## VM Runtime Plane

VM runtime endpoints operate on running or stored VM/session records. Creating a
VM must name a profile.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/vms/list` | List VM/session records. |
| `POST` | `/vms/create` | Create/start a VM from `profile_id`. |
| `GET` | `/vms/{vm_id}/info` | Read VM config identity, including assigned profile id. |
| `GET` | `/vms/{vm_id}/status` | Read live VM runtime status. |
| `PATCH` | `/vms/{vm_id}/edit` | Edit VM-specific mutable config such as CPU, memory, disk sizing, or persistence metadata where technically supported. The assigned profile is immutable. |
| `DELETE` | `/vms/{vm_id}/delete` | Stop/delete VM. |
| `POST` | `/vms/{vm_id}/start` | Start VM using its assigned profile. |
| `POST` | `/vms/{vm_id}/resume` | Resume a stopped/suspended VM using its assigned immutable profile. |
| `POST` | `/vms/{vm_id}/pause` | Pause/suspend a running VM when supported. |
| `POST` | `/vms/{vm_id}/stop` | Stop VM. |
| `POST` | `/vms/{vm_id}/restart` | Restart VM using its assigned profile. |
| `POST` | `/vms/{vm_id}/save` | Persist this VM/session record and its current VM-specific config. |
| `GET` | `/vms/{vm_id}/save/status` | Runtime status/progress for the most recent save operation. |
| `POST` | `/vms/{vm_id}/fork` | Fork this VM into a reusable image/profile target. |
| `GET` | `/vms/{vm_id}/fork/status` | Runtime status/progress for the most recent fork operation. |
| `POST` | `/vms/{vm_id}/reload-profile` | Apply the current profile config to this VM when supported. |
| `POST` | `/vms/{vm_id}/exec` | Execute a command in the VM. |
| `GET` | `/vms/{vm_id}/logs` | Read VM serial/process logs. |
| `POST` | `/vms/{vm_id}/inspect` | Run an explicit diagnostic query against the VM session ledger. |
| `GET` | `/vms/{vm_id}/timeline` | Read the VM timeline projection. |
| `GET` | `/vms/{vm_id}/history` | Read command/history ledger rows. |
| `GET` | `/vms/{vm_id}/history/processes` | Read process-grouped history rows. |
| `GET` | `/vms/{vm_id}/history/counts` | Read history counters. |
| `GET` | `/vms/{vm_id}/history/transcript` | Read the base64 transcript projection. |
| `POST` | `/vms/{vm_id}/files/read` | Read a guest file through the structured file I/O body. |
| `POST` | `/vms/{vm_id}/files/write` | Write a guest file through the structured file I/O body. |
| `GET` | `/vms/{vm_id}/files/list` | List guest/workspace files. |
| `GET` | `/vms/{vm_id}/files/content` | Download guest/workspace file bytes. |
| `POST` | `/vms/{vm_id}/files/content` | Upload guest/workspace file bytes. |

VM records store the immutable profile id they execute plus any explicit
VM-specific resource overrides. Runtime events carry profile id and VM id when
known. Changing profile means creating/forking a new VM, not editing an existing
one.

## Service Runtime / Reporting Plane

These endpoints are global because they report service state or DB-backed
runtime facts. They do not mutate profile behavior.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/health/status` | Daemon health. |
| `GET` | `/status` | Daemon status, VM summary, and install readiness. |
| `GET` | `/assets/status` | Service-wide asset cache/install status. |
| `POST` | `/assets/ensure` | Ensure service cache has required shared assets. |
| `GET` | `/security/latest` | Latest security ledger rows across the service. |
| `GET` | `/security/status` | Security ledger counters/stats across the service. |
| `GET` | `/detection/latest` | Latest detection ledger rows across the service. |
| `GET` | `/detection/status` | Detection counters/stats across the service. |
| `GET` | `/enforcement/latest` | Latest enforcement ledger rows across the service. |
| `GET` | `/enforcement/status` | Enforcement counters/stats across the service. |
| `GET` | `/vms/{vm_id}/security/latest` | Latest security ledger rows for one VM. |
| `GET` | `/vms/{vm_id}/detection/latest` | Latest detection ledger rows for one VM. |
| `GET` | `/vms/{vm_id}/enforcement/latest` | Latest enforcement ledger rows for one VM. |
| `GET` | `/profiles/{profile_id}/security/latest` | Latest security ledger rows for VMs running one profile. |
| `GET` | `/profiles/{profile_id}/detection/latest` | Latest detection ledger rows for VMs running one profile. |
| `GET` | `/profiles/{profile_id}/enforcement/latest` | Latest enforcement ledger rows for VMs running one profile. |

`status` responses contain counters and latency stats derived from the ledger
and OpenTelemetry/debug counters. `latest` responses return the full stored
event DTOs for auditability.

## Error Contract

All HTTP and UDS endpoints return the same structured error body:

| Field | Purpose |
| --- | --- |
| `code` | Stable machine code. |
| `message` | Human-readable summary. |
| `details` | Optional structured detail. |
| `profile_id` | Present when profile-scoped. |
| `vm_id` | Present when VM-scoped. |
| `request_id` | Gateway/service trace id. |

Gateway logs must be structured and include route, method, request id,
profile id, VM id when present, status code, and duration.

## Burn Or Reshape List

These are not final 1.3 contracts:

| Old/global shape | Final direction |
| --- | --- |
| `/enforcements/list` | `/profiles/{profile_id}/enforcement/rules/list` for authoring; `/enforcement/latest|status` for runtime ledger. |
| `/enforcements/rules/{rule_id}` | `/profiles/{profile_id}/enforcement/rules/{rule_id}/edit|delete`. |
| `/enforcements/evaluate` | `/profiles/{profile_id}/enforcement/evaluate`. |
| `/enforcements/reload` | `/profiles/{profile_id}/enforcement/reload` or `/vms/{vm_id}/reload-profile`. |
| `/profiles/{profile_id}/vm/info` | Fold into `/profiles/{profile_id}/info`. |
| `/profiles/{profile_id}/vm/resources/edit` | Fold profile defaults into `/profiles/{profile_id}/edit`; use `/vms/{vm_id}/edit` for a specific VM. |
| `/profiles/{profile_id}/vm/network/edit` | Burn. Too vague; profile network mechanics belong in profile info/edit, and security decisions belong in rules. |
| `/plugins` | `/profiles/{profile_id}/plugins/list` for config; optional runtime diagnostic must be ledger/status only. |
| `/plugins/global/{plugin_id}` | Burn. Plugins are profile/corp config, not global behavior config. |
| `/plugins/{plugin_id}/man` | Burn. Plugin docs live on the docs site under `/plugins/...`. |
| `/corp/endpoints/info` | Fold into `/corp/info` and `/corp/edit`. |
| `/mcp/tools` | Burn. MCP tools live under `/profiles/{profile_id}/mcp/servers/{server_id}/tools/list`. |
| `/mcp/policy` | Burn. MCP decisions are profile rules. |
| `/provision`, `/list`, `/info/{id}`, `/stop/{id}` | Burn. Use `/vms/create`, `/vms/list`, `/vms/{vm_id}/info`, and `/vms/{vm_id}/stop`. |
| `/suspend/{id}`, `/delete/{id}`, `/resume/{id}`, `/persist/{id}`, `/fork/{id}` | Burn. Use `/vms/{vm_id}/pause`, `/vms/{vm_id}/delete`, `/vms/{vm_id}/resume`, `/vms/{vm_id}/save`, and `/vms/{vm_id}/fork`. |
| `/exec/{id}`, `/logs/{id}`, `/inspect/{id}`, `/timeline/{id}` | Burn. Use `/vms/{vm_id}/exec`, `/vms/{vm_id}/logs`, `/vms/{vm_id}/inspect`, and `/vms/{vm_id}/timeline`. |
| `/read_file/{id}`, `/write_file/{id}`, `/files/{id}`, `/files/{id}/content`, `/history/{id}` | Burn. Use `/vms/{vm_id}/files/read`, `/vms/{vm_id}/files/write`, `/vms/{vm_id}/files/list`, `/vms/{vm_id}/files/content`, and `/vms/{vm_id}/history`. |
| `/providers` | Burn. Provider is not a profile API object in 1.3. |
| MCP permission mutation in settings | Move to profile MCP config plus profile rules. |
| Provider/model config in settings | Burn/reshape as profile credentials plus rules. |
| Asset selection in settings | Move to profile assets. |
| VM behavior in settings | Move to profile VM config. |
| Any domain/default/MCP decision provider endpoint | Burn. Single CEL/security-rule rail only. |

Temporary migration routes may exist only as internal cleanup debt and must not
be documented as product API.

## UI Contract

The UI reflects backend contract fields:

- Rule names from `rule.name`.
- Rule descriptions from `rule.reason`.
- Rule grouping from `rule.group`.
- Rule source from `rule.source`.
- Plugin name/description from plugin metadata and docs links.
- Detection levels from the enum.
- Actions from the enum.
- Enforcement/detection source refs from `/profiles/{profile_id}/enforcement/info`
  and `/profiles/{profile_id}/detection/info`.

The UI does not invent names, descriptions, rule paths, plugin meaning, or file
locations.

## Open Decisions

- Exact on-disk profile schema and whether the TOML namespace is
  `[profile.*]` or current `[profiles.*]`.
- Exact default group ids and how a user tweak of a default is represented:
  replace profile-owned default rule vs add profile override.
- Whether 1.3 includes raw enforcement/detection source editing or only
  preview/validation/reload.
- Exact representation of credential references in API responses.
