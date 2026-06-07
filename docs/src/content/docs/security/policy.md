---
title: Policy
description: Security-event rules for enforcement, detection, ask, and plugin actions.
sidebar:
  order: 25
---

Capsem policy is a single rule rail over the normalized `SecurityEvent`.
Network, MCP, model, file, process, credential, and snapshot parsers add typed
fields to that event. Rules match those fields with CEL, then the same match is
used for enforcement, detection, plugin execution, and forensic logging.

There is no separate HTTP rule engine, MCP decision provider, or callback
string list. If a rule does not match a first-party `SecurityEvent` field, it
does not compile.

## Where Rules Live

Rules can be written directly in `user.toml` or `corp.toml`:

```toml
[profiles.rules.skill_loaded]
name = "skill_loaded"
action = "allow"
detection_level = "informational"
reason = "Skill markdown was loaded"
match = 'file.read.path.matches("(^|.*/)skills/.+\\.md$") && file.read.ext == "md"'
```

Rules can also live in referenced files so profiles and corp policy can share
the same rule packs:

```toml
[rule_files]
enforcement = "profiles/base/enforcement.toml"
sigma = "profiles/base/detection.yaml"
```

Paths are resolved relative to the settings file that declares them. Corporate
config also accepts the reserved output integration:

```toml
[corp_rule_files]
sigma_output_endpoint = "https://security.example.invalid/capsem/sigma"
```

`sigma_output_endpoint` is parsed today and reserved for the SIEM export path.
The export sender is not wired yet.

## Rule Tables

Top-level rules use either `corp.rules` or `profiles.rules`.

```toml
[corp.rules.block_openai]
name = "openai_api_block"
action = "block"
detection_level = "high"
corp_locked = true
reason = "OpenAI API access is disabled by corporate policy"
match = 'http.host.matches("(^|.*\\.)(openai\\.com|chatgpt\\.com|oaistatic\\.com|oaiusercontent\\.com)$")'

[profiles.rules.scan_import]
name = "file_import_vt_scan"
plugin = "virus_total"
action = "postprocess"
match = 'file.import.path.matches(".*")'
```

Provider-scoped rules are only convenience authoring for default provider
packs. They compile into the same `profiles.rules.*` runtime list.

```toml
[ai.ollama]
name = "Ollama"
protocol = "ollama"
url = "http://127.0.0.1:11434"
files = []

[ai.ollama.rules.http_native_api]
name = "ollama_native_http_observed"
action = "allow"
detection_level = "informational"
match = 'http.path.matches("^/api/(chat|generate|embeddings|embed|tags|show|pull|push|create|copy|delete|ps|version)")'
```

The table key is the stable `rule_id` suffix. The `name` field is the stable
telemetry name. Both are intentionally required and validated.

## Rule Fields

| Field | Required | Default | Description |
|---|---:|---|---|
| `name` | yes | none | Stable lowercase rule name, max 64 chars. Use `a-z`, `0-9`, `_`, or `-`. |
| `action` | yes | none | One of `allow`, `ask`, `block`, `preprocess`, or `postprocess`. |
| `match` | yes | none | CEL expression over first-party `SecurityEvent` roots. |
| `detection_level` | no | none | Sigma-style severity: `informational`, `low`, `medium`, `high`, or `critical`. `info` is accepted as shorthand and canonicalizes to `informational`. |
| `priority` | no | source default | Lower values sort first. Explicit values must be from `-1000` to `1000`. |
| `corp_locked` | no | `false` | Treat the rule as corporate policy. Corp namespace rules are locked even without this field. |
| `reason` | no | none | Audit string stored with matched rule rows. |
| `plugin` | required for plugin actions | none | Plugin id for `preprocess` and `postprocess`. |
| plugin config | no | none | Extra TOML fields are passed to the plugin. Old fields `on`, `if`, `decision`, `actions`, and `level` are rejected. |

## Actions

| Action | Meaning |
|---|---|
| `allow` | Allow the event boundary to continue. It can still emit a detection when `detection_level` is set. |
| `ask` | Pause materialization until an approval or denial is recorded. |
| `block` | Deny the event boundary and log the matched rule. |
| `preprocess` | Run a plugin before enforcement evaluation. Requires `plugin`. |
| `postprocess` | Run a plugin after the first evaluation and before final materialization. Requires `plugin`. |

Detection is not an action. A rule reports a detection by setting
`detection_level`, and can still allow, ask, block, preprocess, or postprocess.

## Runtime Endpoints

Capsem exposes policy runtime state through explicit service/gateway routes.
Unknown gateway paths are not forwarded.

| Endpoint | Method | Contract |
|---|---|---|
| `/enforcements/evaluate` | `POST` | Test a supplied `SecurityEvent` fixture and rule TOML through the same `SecurityEventEngine` used at runtime. The response uses `SerializableSecurityEvent`, with every first-party root present and absent roots encoded as `null`. |
| `/enforcements/rules/{rule_id}` | `POST` | Add or replace one user profile rule. The rule body is the native rule object; Capsem compiles it with `SecurityRuleProfile` before writing `user.toml`. |
| `/enforcements/rules/{rule_id}` | `DELETE` | Remove one user profile rule from `user.toml`. Corporate rules are not mutable through this endpoint. |
| `/enforcements/reload` | `POST` | Broadcast config reload to running VMs. |
| `/enforcements/{id}/latest` | `GET` | Return stored `security_rule_events` rows for one VM. |
| `/enforcements/{id}/info` | `GET` | Return counters regenerated from stored security rule rows for one VM. |
| `/detections/{id}/latest` | `GET` | Alias over the same stored rule ledger rows, scoped for detection consumers. |
| `/detections/{id}/info` | `GET` | Alias over the same stored rule counters, scoped for detection consumers. |
| `/plugins` | `GET` | Return global built-in plugin policy and defaults. |
| `/plugins/global/{plugin_id}` | `GET`/`POST` | Inspect or update global plugin mode and detection level. |
| `/plugins/{id}` | `GET` | Return per-VM effective plugin policy after default and global overrides. |
| `/plugins/{id}/{plugin_id}` | `GET`/`POST` | Inspect or update one VM-specific plugin override. |

Rule add/update is profile-user scoped by design. Corporate policy arrives from
corp config, referenced enforcement TOML, or referenced Sigma YAML, then compiles
through the same rule rail.

## Priority Defaults

| Source | Implicit priority | Explicit priority rule |
|---|---:|---|
| Corporate rules | `-10` | Must be `<= -10`; range floor is `-1000`. |
| Built-in defaults | `0` | Must be exactly `0`. |
| User/profile rules | `10` | Must be `>= 10`; range ceiling is `1000`. |

Rules sort by `priority`, then by full rule id. Corporate rules therefore run
before defaults, and user rules run after defaults unless an admin explicitly
chooses a later value.

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
| simple PII helper | `model.request.body.contains_pii()` |

Missing roots evaluate as non-matches. That means a cross-root rule can safely
match HTTP or model events without callback fan-out:

```toml
[profiles.rules.openai_boundary]
name = "openai_boundary"
action = "allow"
detection_level = "informational"
match = 'http.host == "api.openai.com" || model.provider == "openai"'
```

## First-Party Fields

Rules must use one of these roots: `http`, `dns`, `mcp`, `model`, `file`,
`process`, `credential`, or `snapshot`.

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
| `credential` | `provider`, `reference`, `ref` |
| `snapshot` | `action` |

Do not use old callback-local roots such as `request.host` or
`tool.name`. The rule compiler rejects them because they are not
`SecurityEvent` fields.

## Parser-Tested Examples

The rule fixture used by Rust tests lives at
`sprints/security-event-rule-spine/fixtures/enforcement.toml`. It includes:

```toml
[ai.openai.rules.http_api]
name = "openai_http_api_observed"
action = "allow"
detection_level = "informational"
match = 'http.host.matches("(^|.*\\.)(openai\\.com|chatgpt\\.com|oaistatic\\.com|oaiusercontent\\.com)$")'

[ai.openai.rules.api_key_broker]
name = "openai_api_key_broker"
plugin = "credential_broker"
action = "postprocess"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
credential = "api_key"
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
