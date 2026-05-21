---
title: Policy
description: Named policy rules for MCP, HTTP, DNS, model traffic, and Policy Hook Spec0 infrastructure.
sidebar:
  order: 25
---

Capsem policy uses named TOML tables under `policy.<type>.<rule_name>`.
The same rule objects are exposed through the settings API and UI.

## Rule Shape

```toml
[policy.http.block_openai_github]
on = "http.request"
if = 'request.host == "github.com" && request.path.matches("^/openai(/|$)")'
decision = "block"
priority = 10
reason = "Block OpenAI organization GitHub paths"
```

| Field | Required | Description |
|---|---:|---|
| `on` | yes | Callback where the rule runs. Must match the policy table type. |
| `if` | yes | CEL-compatible condition over the callback subject. |
| `decision` | yes | `allow`, `ask`, `block`, or `rewrite`. |
| `priority` | yes | Lower numbers run first. Ties are sorted by rule name. |
| `reason` | no | Short audit string written to telemetry where the runtime supports it. |
| `rewrite_target` | rewrite | Target field and regex selector. |
| `rewrite_value` | rewrite | Replacement string. Named captures use `${name}`. |
| `strip_request_headers` | rewrite | HTTP request headers to remove before upstream dispatch. |
| `strip_response_headers` | rewrite | HTTP response headers to remove before the guest sees them. |

Rule names may contain ASCII letters, digits, `_`, and `-`.

## Policy Types

| Type | Callbacks |
|---|---|
| `mcp` | `mcp.request`, `mcp.response` |
| `http` | `http.request`, `http.response` |
| `dns` | `dns.query`, `dns.response` |
| `model` | `model.request`, `model.response`, `model.tool_call`, `model.tool_response` |

The table type must match the callback. For example, `on = "mcp.request"` is
valid only in `[policy.mcp.<name>]`. Policy Hook Spec0 has a `hook.decision`
subject in the wire contract, but configured external hook dispatch is
infrastructure-only for this release: the settings API and UI reject new
`policy.hook.*` rules until an integration gate wires and verifies production
dispatch.

## Decisions

| Decision | Behavior |
|---|---|
| `allow` | Continue through the boundary. |
| `ask` | Fail closed unless an approval path exists for that boundary. It must not dispatch upstream while waiting for product approval support. |
| `block` | Stop at the boundary and return a policy error or denial response. |
| `rewrite` | Mutate only the configured target, then continue. Rewritten or stripped secret values must not be written to telemetry. |

`warn` is not a policy decision. Older MCP default permission settings may
still contain legacy `warn`; named policy rules use `ask`.

## Condition Language

The current parser accepts a strict CEL-compatible subset:

| Form | Example |
|---|---|
| conjunction | `request.host == "github.com" && request.method == "POST"` |
| equality | `tool.name == "deploy"` |
| inequality | `provider != "local"` |
| presence | `has(arguments.prod_token)` |
| string contains | `content.contains("AWS_SECRET_ACCESS_KEY")` |
| string prefix/suffix | `request.host.endsWith(".example.com")` |
| regex match | `request.path.matches("^/openai(/|$)")` |

String literals may use single or double quotes. Conditions are validated before
TOML or settings API writes persist.

## Subject Fields

| Callback | Fields |
|---|---|
| `mcp.request` | `method`, `request.id`, `server.name`, `tool.name`, `resource.uri`, `arguments.*` |
| `mcp.response` | `method`, `request.id`, `server.name`, `tool.name`, `response.text`, `response.content`, `response.is_error`, `arguments.*`, `response.*` |
| `http.request` | `request.scheme`, `request.host`, `request.method`, `request.path`, `request.query`, `request.url`, `request.headers.*` |
| `http.response` | all `http.request` fields plus `response.status`, `response.body`, `response.text`, `response.headers.*` |
| `dns.query` | `qname`, `qtype`, `protocol`, `process.name` |
| `dns.response` | all `dns.query` fields plus `rcode`, `answer.*` |
| `model.request` | `provider`, `model`, `system_prompt`, `request.body`, `messages_count`, `tools_count`, `request.headers.*`, `messages.*` |
| `model.response` | `provider`, `model`, `response.text`, `text`, `content`, `thinking_content`, `stop_reason`, `response.*` |
| `model.tool_call` | `provider`, `model`, `tool.name`, `tool.call_id`, `tool.arguments.*` |
| `model.tool_response` | `provider`, `model`, `tool.name`, `tool.call_id`, `content`, `response.content`, `is_error`, `tool.arguments.*`, `response.*` |

Policy Hook Spec0 defines a future `hook.decision` subject with `callback`,
`decision`, `rule.id`, `endpoint.id`, `request.*`, and `response.*` fields.
It is not editable or enforced through release settings in this build.

## Rewrite Examples

Header stripping is modeled as a `rewrite` rule:

```toml
[policy.http.strip_credentials]
on = "http.request"
if = 'request.host == "api.example.com"'
decision = "rewrite"
priority = 20
strip_request_headers = ["authorization", "x-api-key"]
```

Regex capture rewrites use `rewrite_target` and `rewrite_value`:

```toml
[policy.model.redact_secret_tool_output]
on = "model.tool_response"
if = 'tool.name == "read_file" && content.contains("AWS_SECRET_ACCESS_KEY")'
decision = "rewrite"
priority = 20
rewrite_target = 'content =~ "(?P<prefix>AWS_SECRET_ACCESS_KEY=)[^\s]+"'
rewrite_value = "${prefix}[redacted by capsem policy]"
```

## Precedence

Configuration is resolved as `corp.toml > user.toml > defaults`. Corp rules
override user rules with the same `policy.<type>.<rule_name>` key. Distinct
rules from both files are merged.

At runtime, rules are evaluated by callback. Lower `priority` runs first. If
two matching rules share the same priority, the rule name decides the order.
Use explicit priority spacing, such as `10`, `20`, and `100`, so future rules
can be inserted without renumbering.

## Settings API Shape

The frontend and service exchange named rules with the same keys:

Use the `enforcement` and `detection` APIs for new runtime rules. The old
`policy.http`, `policy.mcp`, and external hook contracts have been removed from
the transport path; Network/File/Process engines emit typed security events and
the Security Engine owns rule validation, CEL compilation, decisions, findings,
and ask/block/rewrite outcomes.

## Telemetry

Security decisions are proved in `session.db`. MCP, HTTP, DNS, model, file, and
process telemetry keep typed event rows and Security Engine findings/decisions
are attached through the canonical event/journal path as S08b lands. The
removed Policy Hook Spec0 table and `/policy-hook/spec` endpoint are no longer
part of the schema or service API.

See [Session Telemetry](/architecture/session-telemetry/#policy-decision-audit)
for SQL queries.

## External Plugins

Future plugin support must use the normalized Security Engine event contract:
plugins return explicit decisions and mutations over a typed event, and Rust
validates/applies those mutations to the real transport body. Do not integrate
new policy code through the removed Policy Hook Spec0 path.
