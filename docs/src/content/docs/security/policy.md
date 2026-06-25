---
title: Policy
description: Security-event rules for enforcement, detection, ask, and plugin runtime policy.
sidebar:
  order: 25
---

Capsem policy is a single rule rail over the normalized `SecurityEvent`.
Network, MCP, model, file, and process parsers add typed fields to that event.
Rules match those fields with CEL, then the same match is used for enforcement,
detection, and forensic logging. Plugins are configured separately; each plugin
owns its own filtering/scope, display metadata, status, stats, and stage-specific
mutation. Plugin stages are still one contract: `SecurityEvent` in,
`SecurityEvent` out.

There is no separate HTTP rule engine, MCP decision provider, or callback
string list. If a rule does not match a first-party `SecurityEvent` field, it
does not compile.

## Where Rules Live

Rules live in enforcement TOML files referenced by a profile or corp config.
Profile and corp files own the pointer; rule files own the rule bodies.

```toml
[profiles.rules.skill_loaded]
name = "skill_loaded"
action = "allow"
detection_level = "informational"
reason = "Skill markdown was loaded"
match = 'file.read.path.matches("(^|.*/)skills/.+\\.md$") && file.read.ext == "md"'
```

Referenced files let profiles and corp policy share the same rule packs:

```toml
[rule_files]
enforcement = "profiles/code/enforcement.toml"
sigma = "profiles/code/detection.yaml"

[corp_rule_files]
enforcement = "corp/enforcement.toml"
sigma = "corp/detection.yaml"
```

Paths are resolved relative to the config file that declares them. Corporate
config also accepts a reserved `sigma_output_endpoint` integration for SIEM
export. The export sender is not wired yet.

## Rule Tables

Top-level rules use either `corp.rules` or `profiles.rules`.

```toml
[corp.rules.block_evil_example]
name = "block_evil_example"
action = "block"
detection_level = "high"
reason = "Example corp rule"
match = 'http.host.matches("(^|.*\\.)evil\\.example$")'
```

Provider-scoped rules are valid only as a single control rule for that provider.
They compile into the same runtime rule rail.

```toml
[ai.openai.rule]
name = "openai_api_requests"
action = "allow"
priority = 10
reason = "Allow OpenAI API requests for this profile."
match = 'http.host.matches("(^|.*\\.)openai\\.com$")'
```

The table key is the stable `rule_id` suffix. The `name` field is the stable
telemetry name. Both are intentionally required and validated.

## Rule Fields

| Field | Required | Default | Description |
|---|---:|---|---|
| `name` | yes | none | Stable lowercase rule name, max 64 chars. Use `a-z`, `0-9`, `_`, or `-`. |
| `action` | yes | none | One of `allow`, `ask`, `block`, `preprocess`, `rewrite`, or `postprocess`. |
| `match` | yes | none | CEL expression over first-party `SecurityEvent` roots. |
| `detection_level` | no | none | Sigma-style severity: `informational`, `low`, `medium`, `high`, or `critical`. `info` is accepted as shorthand and canonicalizes to `informational`. |
| `priority` | no | source default | Lower values sort first. Explicit values must be from `-1000` to `1000`. |
| `reason` | no | none | Audit string stored with matched rule rows. |

## Actions

| Action | Meaning |
|---|---|
| `allow` | Allow the event boundary to continue. It can still emit a detection when `detection_level` is set. |
| `ask` | Pause materialization until an approval or denial is recorded. |
| `block` | Deny the event boundary and log the matched rule. |
| `preprocess` | Mutate/enrich before enforcement decision. |
| `rewrite` | Mutate the event or materialized boundary. Aliases `redact`, `mutate`, and `neutralize` canonicalize to `rewrite`. |
| `postprocess` | Mutate/enrich after enforcement decision but before durable ledger materialization. |

Detection is not an action. A rule reports a detection by setting
`detection_level`, and can still allow, ask, or block.

## Plugins

If behavior can be expressed as a CEL/Sigma rule, it is a rule. Plugins exist
for work rules cannot do by themselves: mutation, materialization, external
scanning, credential substitution, protocol rewrites, or other audited side
effects. Plugins own their own filtering/scope; CEL rules do not invoke
plugins.

Profile/corp config tracks plugin policy and plugin-specific config. The plugin
registry/runtime owns `version`, `name`, `description`, `info`, execution
stages, status schemas, stats schemas, benchmark specs, and capability metadata
for UI reflection. The UI reads those fields from the plugin object; it does
not rename plugins or invent descriptions.

Plugin descriptors expose typed `stages`: `preprocess`, `postprocess`, and
`logging`. Operators can see whether a plugin can mutate before CEL
enforcement, mutate after CEL enforcement, or produce the final ledger-safe
event output. Plugin descriptors also expose a benchmark spec so
`capsem-bench` can measure plugin overhead with the same fixtures every time.
Every plugin also exposes in-memory performance counters: invocation count,
match/skip count, mutation count, allow/ask/block/rewrite count, error count,
total latency, p50/p95/p99 latency, max latency, and per-stage latency.

```toml
[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"
```

## Runtime vs Ledger Materialization

Capsem deliberately has two materialization paths:

| Path | Purpose | Credential handling |
|---|---|---|
| Runtime/upstream | Preserve protocol behavior for allowed traffic. | May resolve broker refs back to real credential bytes when the upstream protocol requires them. |
| Ledger/log/route/UI | Persist and display forensic truth. | Must contain only broker refs, hashes, bounded previews, typed detections, and plugin execution evidence. |

The credential broker owns capture, storage, and runtime injection. The
`log_sanitizer` logging plugin owns the final ledger materialization. Network
formatters, DB readers, frontend transforms, route adapters, and test harnesses
must not add their own credential parsing, ref creation, or redaction.

## Runtime Endpoints

Capsem exposes policy runtime state through explicit service/gateway routes.
Unknown gateway paths are not forwarded. The HTTP gateway is an explicit
allowlist: unknown paths, retired paths, typo paths, and compatibility aliases
return 404 without contacting the UDS service.

| Endpoint | Method | Contract |
|---|---|---|
| `/profiles/{profile_id}/enforcement/evaluate` | `POST` | Test a supplied `SecurityEvent` fixture and rule TOML through the same `SecurityEventEngine` used at runtime. The response uses `SerializableSecurityEvent`, with every first-party root present and absent roots encoded as `null`. |
| `/profiles/{profile_id}/enforcement/rules/list` | `GET` | Return compiled profile rule truth, including source, default-rule, priority, action, detection level, and lock metadata. |
| `/profiles/{profile_id}/enforcement/rules/{rule_id}/edit` | `PUT` | Add or replace one profile enforcement rule. The rule body is the native rule object; Capsem compiles it with `SecurityRuleProfile` before writing profile-owned config. |
| `/profiles/{profile_id}/enforcement/rules/{rule_id}/delete` | `DELETE` | Remove one profile enforcement rule. Corporate rules are not mutable through this endpoint. |
| `/profiles/{profile_id}/enforcement/reload` | `POST` | Reload that profile's enforcement rules. |
| `/profiles/{profile_id}/detection/evaluate` | `POST` | Test a supplied `SecurityEvent` fixture against the profile detection rules. |
| `/profiles/{profile_id}/detection/info` | `GET` | Return detection file/config info for the profile. |
| `/profiles/{profile_id}/detection/rules/list` | `GET` | Return compiled profile detection rule truth. |
| `/profiles/{profile_id}/detection/rules/{rule_id}/edit` | `PUT` | Add or replace one profile detection rule. |
| `/profiles/{profile_id}/detection/rules/{rule_id}/delete` | `DELETE` | Remove one profile detection rule. |
| `/profiles/{profile_id}/detection/reload` | `POST` | Reload that profile's detection rules. |
| `/profiles/{profile_id}/plugins/list` | `GET` | Return profile plugin config plus registry-owned version, name, description, info, stages, schemas, benchmark spec, and capabilities. No runtime counters. |
| `/profiles/{profile_id}/plugins/info` | `GET` | Return plugin subsystem info for the profile. |
| `/profiles/{profile_id}/plugins/{plugin_id}/info` | `GET` | Inspect one profile plugin config object plus registry-owned version, name, description, info, stages, schemas, benchmark spec, and capabilities. |
| `/profiles/{profile_id}/plugins/{plugin_id}/edit` | `PATCH` | Update one profile plugin config object where policy allows it. |
| `/vms/{vm_id}/enforcement/latest` | `GET` | Return stored `security_rule_events` rows for one VM. |
| `/vms/{vm_id}/enforcement/status` | `GET` | Return counters regenerated from stored security rule rows for one VM. |
| `/vms/{vm_id}/detection/latest` | `GET` | Return stored detection-bearing security rule rows for one VM. |
| `/vms/{vm_id}/detection/status` | `GET` | Return detection counters regenerated from stored security rule rows for one VM. |
| `/vms/{vm_id}/info` | `GET` | Return VM configuration/runtime info, including active profile/plugin descriptors. |
| `/vms/{vm_id}/status` | `GET` | Return hot-path VM liveness/readiness counters from memory. No DB reads. |

There are no `/plugins/{id}/man` or global provider-control endpoints. Plugin
copy belongs in docs pages such as `/security/plugins/credential-broker/`; UI
state comes from profile plugin configuration and VM info/status.

Rule add/update is profile-scoped by design. Corporate policy arrives from
corp config, referenced enforcement TOML, or referenced Sigma YAML, then compiles
through the same rule rail.

Security engine status must expose CEL/rule performance counters too: compile
latency, evaluation count, matched-rule count, no-match count, error count,
p50/p95/p99/max evaluation latency, latency by event family/type, per-rule hot
counters, plugin stage time, logging enqueue time, and total boundary time.
These counters are in-memory debug/benchmark truth and must not require a
`session.db` read on VM status hot paths.

## Priority Defaults

| Source | Implicit priority | Explicit priority rule |
|---|---:|---|
| Corporate rules | `-10` | Must be `<= -10`; range floor is `-1000`. |
| Built-in defaults | `default` (`1001`) | Must use the named sentinel `default`. |
| User/profile rules | `10` | Must be `>= 10`; range ceiling is `1000`. |

Rules sort by `priority`, then by full rule id. Corporate rules therefore run
before user/profile rules, and default catch-alls run last.

## CEL Shape

The current CEL subset supports:

| Form | Example |
|---|---|
| `&&` and `||` | `http.host == "api.openai.com" || model.provider == "openai"` |
| equality and inequality | `process.exec.exit_code != "0"` |
| presence | `has(file.read.content)` |
| contains | `mcp.tool_call.name.contains("email")` |
| prefix/suffix | `file.read.name.endsWith(".md")` |
| regex | `dns.qname.matches("(^|.*\\.)openai\\.com$")` |
| regex | `file.read.path.matches("(^|.*/)skills/.+\\.md$")` |

Missing roots evaluate as non-matches. That means a cross-root rule can safely
match HTTP or model events without callback fan-out:

```toml
[profiles.rules.openai_http_boundary]
name = "openai_http_boundary"
action = "allow"
detection_level = "informational"
match = 'http.host.matches("(^|.*\\.)(openai\\.com|chatgpt\\.com|oaistatic\\.com|oaiusercontent\\.com)$")'
```

## First-Party Fields

Rules must use one of these roots: `http`, `dns`, `mcp`, `model`, `file`,
`process`, or `security`.

| Root | Current fields |
|---|---|
| `http` | `host`, `method`, `path`, `status`, `body` |
| `dns` | `qname`, `qtype` |
| `mcp` | `method`, `server.name`, `tool_call.name`, `tool_list` |
| `model` | `provider`, `name`, `request.body`, `response.body`, `request.tool_calls` |
| `file.import` | `path`, `name`, `ext`, `mime_type`, `content` |
| `file.export` | `path`, `name`, `ext`, `mime_type`, `content` |
| `file.read` | `path`, `name`, `ext`, `mime_type`, `content` |
| `file.create` | `path`, `name`, `ext`, `mime_type`, `content` |
| `file.write` | `path`, `name`, `ext`, `mime_type`, `content` |
| `file.delete` | `path`, `name`, `ext`, `mime_type`, `content` |
| `file` | `content` |
| `process` | `exec.id`, `exec.path`, `exec.exit_code`, `exec.stdout`, `exec.stderr`, `command` |
Credential broker state is plugin/runtime evidence, exposed through plugin
status and BLAKE3 references on real events. It is not a CEL root. Workspace
snapshots are MCP/tool/runtime activity unless and until we deliberately add a
first-party snapshot parser and rules contract.

Do not use old callback-local roots such as `request.host` or
`tool.name`. The rule compiler rejects them because they are not
`SecurityEvent` fields.

## Parser-Tested Examples

The rule fixture used by Rust tests lives at
`sprints/security-event-rule-spine/fixtures/enforcement.toml`. It includes:

```toml
[ai.openai.rule]
name = "openai_api_requests"
action = "allow"
priority = 10
reason = "Allow OpenAI API requests for this profile."
match = 'http.host.matches("(^|.*\\.)(openai\\.com|chatgpt\\.com|oaistatic\\.com|oaiusercontent\\.com)$")'

[profiles.rules.skill_loaded]
name = "skill_loaded"
action = "allow"
detection_level = "informational"
reason = "Skill markdown was loaded"
match = 'file.read.path.matches("(^|.*/)skills/.+\\.md$") && file.read.ext == "md"'
```

These examples are covered by
`cargo test -p capsem-core --lib security_rule_profile -- --nocapture`.

## Sigma Detection YAML

Security teams can write parser-compatible Sigma YAML under `rule_files.sigma`.
Capsem imports it into the same `SecurityRule` contract; it is not a second
detection engine.

```yaml
title: OpenAI Traffic To Unexpected Endpoint
id: 11111111-1111-4111-8111-111111111111
status: experimental
description: Detect OpenAI model traffic routed outside approved hosts.
author: capsem
date: 2026/06/05
logsource:
  product: capsem
  service: security_event
detection:
  selection_model:
    model.provider: openai
  filter_approved_endpoint:
    http.host: api.openai.com
  condition: selection_model and not filter_approved_endpoint
level: high
capsem:
  action: block
  reason: OpenAI traffic must use the approved endpoint.
```

Sigma import requires `logsource.product = capsem` and
`logsource.service = security_event`. Selection fields must be first-party
`SecurityEvent` roots. `level` maps to `detection_level`; `capsem.action`
defaults to `allow` when omitted.

The fixture used by tests lives at
`sprints/security-event-rule-spine/fixtures/detection.yaml`, and is checked by
both the Rust importer and the Python Sigma parser compatibility gate.

## Ledger

Every matched rule writes a forensic row to `security_rule_events` with the
primary event id, rule id, rule name, action, detection level, priority,
plugin id, reason, rule snapshot, and matched event payload. Ask rules also
write append-only rows to `security_ask_events`.

Runtime endpoints expose the same DB-facing structures; they should not invent
fields that cannot be regenerated from `session.db`.
