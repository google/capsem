---
title: Policy
description: Named policy rules for MCP, HTTP, DNS, model traffic, and policy hooks.
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
| `hook` | `hook.decision` |

The table type must match the callback. For example, `on = "mcp.request"` is
valid only in `[policy.mcp.<name>]`.

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
| `hook.decision` | `callback`, `decision`, `rule.id`, `endpoint.id`, `request.*`, `response.*` |

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

```json
{
  "policy.http.block_openai_github": {
    "on": "http.request",
    "if": "request.host == \"github.com\"",
    "decision": "block",
    "priority": 10
  }
}
```

Sending `null` for a policy key deletes that user rule. The service validates
the whole batch before writing `user.toml`, so a malformed policy rule rejects
the entire settings save.

## Telemetry

Policy decisions are proved in `session.db`. MCP writes
`mcp_calls.policy_action`, `policy_rule`, and `policy_reason`. HTTP and DNS
write `matched_rule` plus `policy_action`, `policy_rule`, and `policy_reason`
on `net_events` and `dns_events` where the runtime path supports typed policy
metadata. Model request, response, tool-call, and tool-response policy write
the same fields on `net_events` for the enforced boundary. External Policy
Hook callouts additionally write `policy_hook_events` rows with endpoint id,
Spec0 version/hash, callback, decision id, latency, error/fallback state, and
trace/session ids.

See [Session Telemetry](/architecture/session-telemetry/#policy-decision-audit)
for SQL queries.

## Policy Hook Spec0

The hook contract is exported as OpenAPI 3.1 from the Rust wire types and is
checked in at `config/policy-hook-openapi.json`. A running service also exposes
it at:

```bash
curl --unix-socket "$CAPSEM_RUN_DIR/service.sock" \
  http://localhost/policy-hook/spec
```

Hook endpoint config is strict: unknown fields are rejected, HTTPS is required
outside localhost, remote endpoints require bearer auth, request/response body
size is capped, and transport/schema errors fail closed to the configured
fallback decision.
