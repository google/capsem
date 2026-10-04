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

A session's policy is merged from three sources:

| Source | File | May set |
|---|---|---|
| Built-in defaults | `crates/capsem-config/src/default_provider_rules.toml` (compiled in) | `default.*` rules, `ai.*` providers, plugin modes |
| User | `~/.capsem/settings.toml` | `default`, `profiles.rules`, `rule_files`, `ai`, `plugins`, `mcp` |
| Corp | `corp.toml` | `corp.rules`, `corp_rule_files`, `refresh_policy`, `network`, and everything the user may set |

Corp wins: a corp rule replaces the user's rule of the same id, corp plugin
modes and MCP servers are laid over the user's, and `corp_locked` rules cannot
be overridden. `settings.toml` that sets a corp-only key is refused. The
service writes the merged result into each session's `vm/active_policy.toml`;
capsem-process (`--active-policy`) and capsem-mcp-builtin
(`CAPSEM_ACTIVE_POLICY`) read only that file. Every VM enforces the same
policy; there is no per-VM policy selection.

`profiles.rules` is the name of the user rule table, kept for compatibility of
rule ids; it does not refer to a VM profile.

```toml
[profiles.rules.skill_loaded]
name = "skill_loaded"
action = "allow"
detection_level = "informational"
reason = "Skill markdown was loaded"
match = 'file.read.path.matches("(^|.*/)skills/.+\\.md$") && file.read.ext == "md"'
```

Referenced files let users and corp policy share the same rule packs.
`rule_files` belongs in `settings.toml`, `corp_rule_files` in the corp config:

```toml
# settings.toml
[rule_files]
enforcement = "rules/enforcement.toml"
sigma = "rules/detection.yaml"

# corp.toml
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

Provider-scoped rules live under `ai.<provider>.rules`. They compile into the
same runtime rule rail.

```toml
[ai.openai.rules.http_api]
name = "openai_api_requests"
action = "allow"
priority = 10
reason = "Allow OpenAI API requests."
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

Built-in, settings, and corp config track plugin policy and plugin-specific config. The plugin
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
| `/settings/info` | `GET` | Return the resolved settings tree, including the user's rules and corp locks, with validation issues. |
| `/plugins/list` | `GET` | Return every plugin's effective config and its source (built in, settings, corp) plus registry-owned version, name, description, info, stages, schemas, benchmark spec, and capabilities. |
| `/plugins/{plugin_id}/info` | `GET` | Inspect one plugin's effective config, registry descriptor, and runtime activity across sessions. |
| `/plugins/{plugin_id}/edit` | `PATCH` | Set one plugin's mode or detection level in `settings.toml`. Refused when corp decides it. |
| `/mcp/default/edit` | `PATCH` | Set the default MCP tool permission in `settings.toml`. |
| `/mcp/servers/{server_id}/tools/{tool_id}/edit` | `PATCH` | Allow, ask, or block one MCP tool in `settings.toml`. |
| `/vms/{vm_id}/enforcement/latest` | `GET` | Return stored `security_rule_events` rows for one VM. |
| `/vms/{vm_id}/enforcement/status` | `GET` | Return counters regenerated from stored security rule rows for one VM. |
| `/vms/{vm_id}/detection/latest` | `GET` | Return stored detection-bearing security rule rows for one VM. |
| `/vms/{vm_id}/detection/status` | `GET` | Return detection counters regenerated from stored security rule rows for one VM. |
| `/vms/{vm_id}/info` | `GET` | Return VM configuration/runtime info. |
| `/vms/{vm_id}/status` | `GET` | Return hot-path VM liveness/readiness counters from memory. No DB reads. |

Every `edit` route validates the whole next `settings.toml` (referenced rule
files merged, rules compiled) before writing it, records the change in the host
ledger table `policy_mutation_events`, and pushes the new active policy to every
running VM. Each VM must acknowledge the exact policy digest before the route
returns. Existing connections keep their decision; new events see the new
rules.

There are no rule list, evaluate, or reload routes. User rules are edited in
`settings.toml` (inline or through `rule_files`); corporate policy arrives from
corp config, referenced enforcement TOML, or referenced Sigma YAML, then
compiles through the same rule rail. There are no `/plugins/{id}/man` or global
provider-control endpoints. Plugin copy belongs in docs pages such as
`/security/plugins/credential-broker/`.

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
| User rules (`profiles.rules`, `ai.*`) | `10` | Must be `>= 10`; range ceiling is `1000`. |

Rules sort by `priority`, then by full rule id. Corporate rules therefore run
before user rules, and default catch-alls run last.

The first matching `allow`, `ask`, or `block` rule decides the boundary. This is
first-match-wins, not most-restrictive-wins, and the tie-break is the rule id:

```toml
# Both match evil.test at priority 10. "aaa_..." sorts first, so it decides,
# and the block never applies.
[profiles.rules.aaa_allow_all_http]
name = "aaa_allow_all_http"
action = "allow"
priority = 10
match = 'has(http.host)'

[profiles.rules.zzz_block_evil]
name = "zzz_block_evil"
action = "block"
priority = 10
match = 'http.host == "evil.test"'
```

Give the stricter rule the stronger (lower) priority rather than relying on
where its name lands in the alphabet. Two rules that can match the same event
and disagree on the action should never share a priority.

A plugin verdict is the one thing that overrides the selected rule, and only
upward: a plugin in `ask` or `block` mode raises an allowing rule, and no plugin
mode can lower a rule that blocks.

### Deny by default

Negation cannot express deny-by-default. Every CEL atom is false when the field
it reads is missing, so `http.host != "allowed.test"` blocks a host it can see
and goes quiet on an event carrying no host. Absent-is-false is what keeps a
`file.*` rule from firing on an HTTP event, so it is not going away.

Express default-deny in the priority ladder instead -- a `block` catch-all at
weak priority, with `allow` exceptions at stronger priority:

```toml
[profiles.rules.allow_known_host]
name = "allow_known_host"
action = "allow"
priority = 10
match = 'http.host == "allowed.test"'

[profiles.rules.deny_the_rest]
name = "deny_the_rest"
action = "block"
priority = 900
match = 'has(http.valid)'
```

This still denies when the field the exception reads is absent.

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
`process`, `ip`, `tcp`, `udp`, `container`, or `network`.

Every field a rule can read is listed below. The compiler rejects anything else,
including a misspelled leaf (`file.wrte.path`) and a bare root (`has(http)`),
because both would compile into a rule that silently never matches. Each root
also carries a `valid` field that is true whenever the event carries that family
at all -- `has(http.valid)`, not `has(http)`.

| Root | Fields |
|---|---|
| `http` | `http.valid`, `http.host`, `http.method`, `http.path`, `http.query`, `http.status`, `http.body` |
| `dns` | `dns.valid`, `dns.qname`, `dns.qtype` |
| `container` | `container.valid`, `container.image`, `container.registry`, `container.digest` |
| `network` | `network.valid`, `network.id`, `network.name`, `network.mode`, `network.action`, `network.target`, `network.side`, `network.protocol`, `network.publication.id`, `network.source.vm_id`, `network.source.vm_name`, `network.source.generation`, `network.source.ip`, `network.source.port`, `network.destination.vm_id`, `network.destination.vm_name`, `network.destination.generation`, `network.destination.ip`, `network.destination.port` |
| `mcp` | `mcp.valid`, `mcp.method`, `mcp.server.valid`, `mcp.server.name`, `mcp.tool_call.valid`, `mcp.tool_call.name`, `mcp.tool_list.valid`, `mcp.tool_list`, `mcp.request.valid`, `mcp.request.id`, `mcp.request.method`, `mcp.request.arguments`, `mcp.response.valid`, `mcp.response.content`, `mcp.event.valid` |
| `model` | `model.valid`, `model.provider`, `model.name`, `model.request.valid`, `model.request.body`, `model.request.tool_calls`, `model.response.valid`, `model.response.body`, `model.tool_call.valid` |
| `file` | `file.valid`, `file.content`, `file.kind` |
| `file.import` | `file.import.valid`, `file.import.path`, `file.import.name`, `file.import.ext`, `file.import.mime_type`, `file.import.content` |
| `file.export` | `file.export.valid`, `file.export.path`, `file.export.name`, `file.export.ext`, `file.export.mime_type`, `file.export.content` |
| `file.read` | `file.read.valid`, `file.read.path`, `file.read.name`, `file.read.ext`, `file.read.mime_type`, `file.read.content` |
| `file.create` | `file.create.valid`, `file.create.path`, `file.create.name`, `file.create.ext`, `file.create.mime_type`, `file.create.content` |
| `file.write` | `file.write.valid`, `file.write.path`, `file.write.name`, `file.write.ext`, `file.write.mime_type`, `file.write.content` |
| `file.delete` | `file.delete.valid`, `file.delete.path`, `file.delete.name`, `file.delete.ext`, `file.delete.mime_type`, `file.delete.content` |
| `process` | `process.valid`, `process.name`, `process.command`, `process.exec.valid`, `process.exec.id`, `process.exec.path`, `process.exec.exit_code`, `process.exec.stdout`, `process.exec.stderr`, `process.audit.valid` |
| `ip` | `ip.valid`, `ip.value`, `ip.version` |
| `tcp` | `tcp.valid`, `tcp.port` |
| `udp` | `udp.valid`, `udp.port` |

For an HTTP request, `ip.value` is the address the proxy connects to, not one
read off the name: the host resolves the requested name before the rules run
and then dials only the addresses it resolved, so a second DNS answer cannot
move the connection after the check. When a name resolves to several
addresses and any of them is loopback, private, link-local or otherwise
non-public, `ip.value` is that one, so a rule on the address cannot be
sidestepped by choosing a name. An IPv4-mapped IPv6 answer is reported as its
IPv4 address. Hosts routed by `network.upstream_overrides` are administrator
routing: they are dialed as configured and carry `ip.value` only when the
requested host is itself an IP literal.

Credential broker state is plugin/runtime evidence, exposed through plugin
status and BLAKE3 references on real events. It is not a CEL root. Neither is
`security`: decision state is the engine's output, not an input a rule reads.

The `network` contract describes owner-supplied routing facts. Modes are
`expose`, `http_preview`, and `private`; preview actions are `preview_request`
and `preview_upgrade`. Sides are `source` and `destination`, identifying the endpoint
whose policy is evaluated. Ports and boot generations use decimal strings in
CEL, like the existing `tcp.port` field. Boot generations also serialize as
decimal strings in audit JSON to preserve their full 64-bit identity. Network names and IDs exist for private
routes; a publication ID exists for expose routes. Host socket endpoints have no
VM identity. Connection and synthetic probe authorization require complete
facts and an explicit allow rule. Missing facts are errors. Counters, close
reasons, connection IDs, and decision state are audit data and cannot be read
by rules. Expose records also include the actual loopback listener address.

Opening an exposure is itself a `network.lifecycle` event, evaluated on the VM
owner against the VM's current rules and plugins before its listener accepts
anything, and recorded in the session ledger first: if that row cannot be
written the exposure is refused. Its facts are `network.mode == "expose"`,
`network.action` (`published` on request, `restored` when a saved exposure
reopens after the owner starts again, `revoked` when it is closed),
`network.target` (`container` or `vm`), `network.publication.id`, the loopback
listener as `network.source.ip`/`network.source.port`, and the guest endpoint as
`network.destination.*`. It carries no `network.side` or `network.protocol`, so
connection rules never match it, and with no matching rule it is allowed. A
block or ask refuses it, since an exposure change has no one to approve it; a
saved exposure the rules now refuse is forgotten rather than reopened. Revoking
always closes the listener and is recorded afterwards. To keep an exposure
from existing, match `network.action != "revoked"`:

```toml
[profiles.rules.no_published_ssh]
name = "no_published_ssh"
action = "block"
match = 'network.action != "revoked" && network.destination.port == "22"'
```

Lifecycle events for named networks also carry `network.action`.

Published TCP ports evaluate the destination VM's current rules and plugins
before requesting any guest connection. The built-in defaults carry a visible
expose allow rule (`default.expose`); a more specific deny or ask prevents setup. An unavailable
audit writer, evaluation error, or expired guest control lease also refuses
setup. Existing connections retain their decision until closed; a rule edit is
pushed to the running VM before the edit route returns and applies to new
connections. A control disconnect immediately revokes live sockets, and queued
requests from that lease cannot cross a replacement control stream.

The primary transport ledger records requests, setup results, and close reports
with one connection ID. Matched rules use that same event identity and the
existing security ledger. These are buffered audit records, not a per-connection
disk sync; counters describe transport bytes, not decoded application payloads.
Private routing remains subsequent work.

Do not use old callback-local roots such as `request.host` or
`tool.name`. The rule compiler rejects them because they are not
`SecurityEvent` fields.

## Parser-Tested Examples

The rule fixture used by Rust tests lives at
`tests/fixtures/config/security-rule-profile/enforcement.toml`. It includes:

```toml
[ai.openai.rules.http_api]
name = "openai_http_api_observed"
action = "allow"
detection_level = "informational"
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
`tests/fixtures/config/security-rule-profile/detection.yaml`, and is checked by
both the Rust importer and the Python Sigma parser compatibility gate.

## Ledger

Every matched rule writes a forensic row to `security_rule_events` with the
primary event id, rule id, rule name, action, detection level, priority,
plugin id, reason, rule snapshot, and matched event payload. Ask rules also
write append-only rows to `security_ask_events`.

Runtime endpoints expose the same DB-facing structures; they should not invent
fields that cannot be regenerated from `session.db`.
