# Sprint: Policy V2 (MCP + HTTP + DNS + Models + Hooks)

## Why

The framed MITM MCP path now has a real enforcement boundary, but the
policy interface is not product-quality yet. Internally we still have
`audit_only` naming and a `Warn` decision that does not map to useful
user behavior. The richer request/response matchers exist in runtime
tests, but they are not exposed through `user.toml`, `corp.toml`, the
settings builder, or the settings API/UI. That means argument and return
value policy works as code, not as supported configuration.

This sprint turns agent-boundary policy into a first-class typed product
feature across MCP, HTTP, DNS, and model traffic: `allow`, `ask`,
`block`, and `rewrite`, with no raw string decision plumbing inside
Rust, complete TOML/settings support, an exportable hook API contract,
adversarial parsing tests, VM E2E proof, and session telemetry that tells
the truth.

## Current State

- `McpUserConfig` supports `[mcp] global_policy`,
  `default_tool_permission`, `server_enabled`, and
  `[mcp.tool_permissions]`.
- `ToolDecision` currently has `Allow`, `Warn`, and `Block`.
  `Warn` is effectively `allow` with a warning reason and should not
  remain as a user-facing decision.
- The framed MITM MCP decision provider evaluates runtime
  `McpDecisionRule`s for:
  - exact tool name
  - exact resource URI
  - argument name
  - argument value
  - return value
  - deny-over-allow precedence
- Those richer rules live in `McpPolicy.audit_rules` and tests; there is
  no TOML/settings schema that lets users or corp config express them.
- `policy_mode = "audit_only"` is now misleading because deny decisions
  are enforced.
- Existing tests prove runtime behavior for argument-name/value rules,
  but not config parsing, settings generation, reload, or VM E2E from
  TOML to enforced deny/ask behavior.
- HTTP policy exists today as domain/read-write rules plus the older
  `HttpPolicy` method/path engine, but it does not expose a product
  policy interface for method, URL path, query, header matching, request/response
  header stripping, `ask`, or `rewrite`.
- DNS is already parsed and recorded: `crates/capsem-core/src/net/dns`
  handles guest DNS requests, DNS parser tests cover malformed/adversarial
  inputs, and `dns_events` records qname, qtype, rcode, decision, matched
  rule, source protocol, resolver time, and trace id. DNS also already has
  an internal redirect primitive (`DnsRedirect` / `decision=redirected`),
  but no strict TOML/settings policy interface that exposes it as typed
  `rewrite`.
- Model traffic is already parsed and recorded through
  `crates/capsem-core/src/net/ai_traffic`,
  `crates/capsem-core/src/net/interpreters`, and `model_calls`,
  `tool_calls`, and `tool_responses`. We capture provider/model,
  request/response previews, system prompt preview, usage, model-emitted
  tool calls, and tool responses, but there is no policy interface for model
  request, model response, model tool-call, or model tool-response
  decisions.
- Remote hook/plugin integration is not yet a contract. If Capsem forwards
  policy hooks to a third-party HTTPS service and expects a decision back,
  the wire format must be exportable as a versioned spec so plugin authors
  can build receiving servers without reverse-engineering Rust structs.

## Known Verification Bugs To Suck Up

Parallel T5 recon found likely bugs that should be absorbed into this
sprint rather than hand-waved:

- `capsem-mcp-builtin` reads `CAPSEM_DOMAIN_ALLOW` and
  `CAPSEM_DOMAIN_BLOCK`, but `capsem-process` currently appears to pass
  only `CAPSEM_SESSION_DIR` and `CAPSEM_SESSION_DB`. If confirmed,
  builtin HTTP tools can bypass the configured domain policy and live
  reload does not update the already-running builtin subprocess.
- Builtin HTTP `net_events.port` is hardcoded to `443`, which makes
  `http://` telemetry incorrect.
- Builtin HTTP may return before emitting `net_events` on DNS/connect/body
  and binary response errors. The product invariant should be that
  attempted HTTP egress is logged, not only successful egress.
- Some older session/network tests contain vacuous assertions or stale
  DNS assumptions (`dnsmasq`). T0 should convert those into real tests or
  remove the false proof.

## Design Direction

### Shared Decision Model

Replace the user-facing policy vocabulary with:

- `allow`: dispatch the request and return the response.
- `ask`: do not dispatch automatically. Return a structured
  approval-required JSON-RPC error until an approval UI/protocol exists.
  Record telemetry as an ask decision, including the request preview and
  matched rule. The important invariant is that `ask` must not silently
  behave as `allow`.
- `block`: deny before dispatch for request rules, or replace the
  response for response rules. Record terminal telemetry and do not leak
  blocked response content.
- `rewrite`: mutate only the matched field, byte/string fragment, or
  protocol answer using a validated `rewrite_target` plus
  `rewrite_value`. Runtime code sees an enum variant carrying typed
  rewrite data, not loose decision strings. `rewrite_target` is not a URL
  path or JSON path field; it is a compiled target expression, usually a
  regex over a normalized field, that can expose captures. `rewrite_value`
  is a replacement template that can use those captures and later broker
  references. Telemetry records the rule id, target metadata, and that a
  rewrite happened, but not sensitive rewrite payloads.

Internally use Rust enums end-to-end. TOML values are necessarily text,
but they must deserialize directly into enums with strict validation;
unknown decisions or unknown rule fields are config errors, not fallback
strings.

Rule config uses named TOML tables:

```toml
[policy.<type>.<rule_name>]
on = "<callback>"
if = "<CEL condition>"
decision = "allow|ask|block|rewrite"
priority = 100
```

The table path is the rule identity. `on` names the callback where the
rule evaluates, for example `mcp.request`, `http.request`, `dns.query`,
`model.tool_response`, or `hook.decision`. `if` is one CEL expression;
compound conditions use normal CEL conjunctions such as `&&`, not a list
of mini-matchers. `decision` is the typed result. `priority` is an
explicit integer sort key; lower numbers run first, and ties are broken
deterministically by policy type and rule name. Rewrite decisions add flat
rewrite fields such as `rewrite_target` and `rewrite_value`.

`rewrite` is the bridge to the future hook/credential broker system: a
hook can return "replace this MCP argument / HTTP header / DNS answer
with this brokered value" as a typed transform, with capture-aware target
matching. This sprint should build the policy shape and safe enforcement
semantics, not a full credential broker UI.

Per-protocol semantics:

- MCP `rewrite` applies to matched argument values or response fields,
  then dispatches or returns the rewritten value. Tool name/resource URI
  rewrites are out of scope until a concrete routing use case exists.
- HTTP `rewrite` applies to normalized request/response data before
  upstream dispatch or before returning to the caller: URL components,
  query values, header values, or body fragments selected by
  `rewrite_target`. Header strip allow/block lists compile to the same
  policy IR as named rules. Blocklist wins over allowlist.
- DNS `rewrite` synthesizes a DNS answer from the string payload instead
  of forwarding upstream. The first implementation should validate A/AAAA
  IP literals and reject invalid payloads at config load time. DNS `ask`
  fails closed without upstream resolution and records approval-required
  telemetry because DNS clients cannot perform an interactive approval
  handshake.
- Model request `rewrite` applies to matched request fields before the
  upstream model call, e.g. message content, system prompt, tool response
  content, or tool schemas. Model request `block`/`ask` does not call the
  upstream provider.
- Model response `rewrite` applies to matched response text or structured
  output before it reaches the guest/client. Model response `block`
  replaces the provider response with a safe policy result and must not
  leak the blocked content in telemetry.
- Model tool-call `rewrite` applies to provider-emitted tool-call
  arguments before the guest/client sees them. Model tool-call `block` or
  `ask` prevents unsafe tool calls from reaching the agent loop.
- Model tool-response `rewrite` applies to tool result content that is
  being sent back to the provider in a later model request. Blocking a
  model tool response prevents the provider from receiving sensitive local
  tool output.

### Hook API And Spec0

Policy V2 must define one normalized hook request/response contract shared
by the local engine and future third-party HTTPS plugins. The first
exported contract is **Policy Hook Spec0**, documented in
`sprints/mcp-policy-v2/hook-spec0.md` and generated as OpenAPI 3.1 from
the same Rust wire types used at runtime.

Required artifact:

- `config/policy-hook-openapi.json` or `docs/api/policy-hook-openapi.yaml`
  generated from Rust types, not hand-maintained.
- Golden/conformance tests proving the generated OpenAPI contains every
  callback, decision response, rewrite target/value, and error shape.

Spec0 endpoint shape:

- `POST /v1/policy/decision`: single decision request.
- `POST /v1/policy/batch-decision`: optional batching for hot paths.
- `GET /v1/policy/spec`: returns the exact OpenAPI document or a content
  hash/version so Capsem can audit compatibility.
- `GET /v1/health`: hook liveness.

The hook request uses a discriminated `on` callback union:

- `mcp.request`, `mcp.response`
- `http.request`, `http.response`
- `dns.query`, `dns.response`
- `model.request`, `model.response`
- `model.tool_call`, `model.tool_response`

The response returns `decision = allow|ask|block|rewrite`, a stable
reason/rule id, optional `rewrite_target`/`rewrite_value`, redaction
directives, cache TTL, and an audit-safe `decision_id`. Rewrite payloads
may be literals in local config, but hook-returned credentials should use
broker references or opaque values with strict no-log semantics.

Remote HTTPS hooks are powerful enough to become a credential broker, so
the plan must include security constraints from the start:

- HTTPS only except explicit localhost/dev mode.
- Configured endpoint allowlist, timeout, body-size cap, and retry budget.
- Bearer token or mTLS authentication.
- Fail-closed for configured enforcement hooks unless a rule explicitly
  says local fallback is acceptable.
- No hook request/response payload may be written to logs/session DB
  unless the field is explicitly marked audit-safe by the hook spec.
- Local corp `block` precedence cannot be weakened by a remote hook.

### TOML Shape

Policy V2 does not use `[[*.rules]]` arrays, action buckets, or nested
`match`/`then` sections. The canonical shape is
`policy.<type>.<rule_name>`, with flat fields:

```toml
[policy.http.block_openai_github]
on = "http.request"
if = 'request.host == "github.com" && request.path.matches("^/openai(/|$)")'
decision = "block"
priority = 10
reason = "Do not let this session fetch OpenAI-owned GitHub code"
```

The concrete strawman for T0/T1 should be expensive enough to prove the
shape is real: block traffic for `github.com/openai/...` or
`github.com/openclaw/...`, not only a domain-only deny. URL path is a
field inside the CEL subject (`request.path`); it is not a top-level TOML
policy key.

MCP rules use the same shape:

```toml
[policy.mcp.block_prod_token]
on = "mcp.request"
if = 'method == "tools/call" && tool.name == "deploy" && has(arguments.prod_token)'
decision = "block"
priority = 10
reason = "Do not send production tokens through MCP tools"

[policy.mcp.ask_prod_query]
on = "mcp.request"
if = 'method == "tools/call" && arguments.environment == "prod"'
decision = "ask"
priority = 20
reason = "Human approval required for production queries"

[policy.mcp.redact_secret_return]
on = "mcp.response"
if = 'tool.name == "read_file" && response.text.contains("AWS_SECRET_ACCESS_KEY")'
decision = "rewrite"
priority = 30
rewrite_target = 'response.text =~ "(?P<prefix>AWS_SECRET_ACCESS_KEY=)[^\\s]+"'
rewrite_value = "${prefix}[redacted by capsem policy]"
reason = "Do not return secret material to the guest agent"
```

HTTP request/response rules use CEL for matching and
`rewrite_target`/`rewrite_value` for capture-aware mutation:

```toml
[policy.http.rewrite_openai_github_to_openclaw]
on = "http.request"
if = 'request.host == "github.com" && request.path.matches("^/openai/(?P<repo>[^/?#]+)")'
decision = "rewrite"
priority = 20
rewrite_target = 'request.url =~ "^https://github\.com/openai/(?P<repo>[^/?#]+)(?P<rest>.*)$"'
rewrite_value = "https://github.com/openclaw/${repo}${rest}"
reason = "Route the strawman repository namespace through the allowed mirror"

[policy.http.strip_secret_request_headers]
on = "http.request"
if = 'request.host.endsWith(".example.com")'
decision = "rewrite"
priority = 30
strip_request_headers = ["authorization", "cookie", "x-api-key"]
reason = "Do not forward local credentials to example.com"
```

Header names are normalized and validated when config loads. Header
stripping is represented in the same policy IR as other rewrites; stripped
header values must not be written to telemetry.

DNS rules expose the existing redirect behavior as typed `rewrite`:

```toml
[policy.dns.pin_internal_api]
on = "dns.query"
if = 'qname == "api.internal.example" && qtype == "A"'
decision = "rewrite"
priority = 10
rewrite_target = 'answer.A'
rewrite_value = "10.20.30.40"
reason = "Route test API traffic to the local broker"

[policy.dns.block_openai_dns]
on = "dns.query"
if = 'qname == "api.openai.com"'
decision = "block"
priority = 20
reason = "Do not resolve direct OpenAI egress"
```

Model rules match parsed model traffic, not raw provider-specific JSON
strings:

```toml
[policy.model.block_prod_system_prompt]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o" && system_prompt.contains("PROD_SECRET")'
decision = "block"
priority = 10
reason = "Do not send production system prompts to hosted models"

[policy.model.redact_secret_tool_output]
on = "model.tool_response"
if = 'tool.name == "read_file" && content.contains("AWS_SECRET_ACCESS_KEY")'
decision = "rewrite"
priority = 10
rewrite_target = 'content =~ "(?P<prefix>AWS_SECRET_ACCESS_KEY=)[^\\s]+"'
rewrite_value = "${prefix}[redacted by capsem policy]"
reason = "Do not send local secret-bearing tool output back to the model"

[policy.model.ask_browser_tool_call]
on = "model.tool_call"
if = 'tool.name == "browser_open"'
decision = "ask"
priority = 10
reason = "Browser automation requests require approval"
```

Simple UI/settings affordances are still allowed, but they are not a
second engine. Domain allow lists, domain block lists, read/write HTTP
toggles, per-tool MCP permissions, and header strip controls must compile
into this named policy IR with stable generated rule names. The settings
tests must prove both the human-friendly input and the generated
`policy.<type>.<rule_name>` rules produce the same enforcement result.

### Settings System

The policy model must round-trip through all settings layers:

- `user.toml`
- `corp.toml`
- preset application
- config loader validation
- service settings API
- settings export/schema generation
- hot reload into running `capsem-process`
- MCP, HTTP, DNS, model, and hook policy sections
- OpenAPI/spec export for policy hooks
- UI/settings-generated allow/block lists compiled into named policy
  rules, not interpreted by a parallel policy engine

Corp config continues to override user config. For rule conflicts,
stable precedence should be:

1. Corp `block`
2. Corp `ask`
3. User `block`
4. User `ask`
5. Explicit allow
6. Default decision

Within the same precedence level, first matching deny/ask rule should be
deterministic by config order, with `block` taking precedence over `ask`
and `ask` over `allow`.

### Telemetry

Rename or replace misleading policy mode/decision strings:

- `policy_mode`: use `local_enforce` or `local_policy_v2`, not
  `audit_only`.
- `policy_decision`: one of `allow`, `ask`, `block`, or `rewrite`.
- `rewrite` must be recorded as a decision without storing
  sensitive payload values. Store the rule id, reason, target field, and
  whether the payload was literal or broker-provided.
- `decision`: keep user-facing terminal result compatible, but add or
  map ask clearly, e.g. `approval_required`.
- Preserve `policy_rule`, `policy_reason`, request hash, and previews.
- For response-block rules, do not store leaked response preview.
- DNS currently records `allowed`, `denied`, `redirected`, and `error`.
  Policy V2 should expose `rewrite` while either migrating
  `redirected -> rewrite` or preserving a compatibility mapping that is
  tested and documented.
- `model_calls`, `tool_calls`, and `tool_responses` must record matched
  policy decision/rule/reason and redact blocked or rewritten content. If
  this requires schema changes, they must be migrated and covered by
  reader/writer tests.
- Hook request/response audit rows must include hook endpoint id, spec
  version/hash, decision id, latency, timeout/error status, and whether
  local fallback was used. They must not include secret rewrite payloads.

### Approval Boundary

This sprint should not build a full approval UI. It should make `ask`
semantically correct and safe: no dispatch without approval. If a future
approval channel is added, it can consume the structured JSON-RPC error
or policy event as the handshake.

## Task Breakdown

### T0: Red Tests And Existing Behavior Audit

- Add failing tests showing that TOML cannot express named
  `policy.<type>.<rule_name>` rules with `on`, CEL `if`, `decision`, and
  `priority` plus rewrite target/value fields today.
- Add failing tests showing that the strawman
  `github.com/openai/...` or `github.com/openclaw/...` HTTP policy cannot
  be expressed and enforced today.
- Add failing HTTP tests for method/URL path/query/header CEL rules and
  request/response header stripping.
- Add failing DNS tests for configured `block`, `ask`, and
  `rewrite_target`/`rewrite_value` policies from TOML/settings into
  `DnsRedirect`.
- Add failing model-policy tests for model request, model response, model
  tool-call, and model tool-response matchers.
- Add failing hook Spec0 tests proving OpenAPI export exists, is generated
  from the Rust wire types, and contains every callback/decision/rewrite
  shape.
- Add failing settings/API tests for `allow|ask|block|rewrite`.
- Add failing VM E2E from `user.toml` to framed MCP enforcement for an
  argument-name policy.
- Record the current runtime-only coverage honestly in the tracker.

### T1: Typed Policy Model

- Introduce a shared typed policy decision:
  `PolicyDecision { Allow, Ask, Block, Rewrite { target, value } }`, plus
  protocol-specific validators/targets.
- Map MCP runtime policy onto the shared enum.
- Map HTTP network policy onto the shared enum without regressing the
  existing read/write defaults.
- Map DNS `Redirected` semantics onto `rewrite` while preserving
  compatible telemetry where needed.
- Map model traffic policy onto the shared enum and define policy
  subjects for model request, model response, model tool-call, and model
  tool-response.
- Define hook request/response Rust wire types that are the single source
  for OpenAPI export and remote plugin compatibility.
- Remove or migrate user-facing `Warn`.
- Replace `audit_only` naming with an enforcement-oriented mode.
- Keep backwards compatibility for existing `default_tool_permission`
  and `tool_permissions` only if the migration is explicit and tested.

### T2: Strict TOML And Settings Loader

- Add strict serde/settings types for:
  - named `policy.<type>.<rule_name>` maps
  - callback enum from `on`
  - CEL expression string from `if`
  - decision enum
  - priority ordering with deterministic tie-breaks
  - rewrite target regex/selector and replacement template
  - HTTP header allow/strip lists
  - DNS qname/qtype rewrite rules
  - model request/response/tool-call/tool-response matchers
  - hook endpoint config and fail-closed/fallback behavior
- Reject unknown decisions, unknown fields, invalid CEL, invalid callback
  names, invalid rewrite regexes, and invalid replacement captures.
- Merge user/corp policy deterministically.
- Compile UI/settings domain allow lists, domain block lists, per-tool
  permissions, and header controls into the same policy rule IR.
- Update schema/defaults generation.
- Generate Policy Hook Spec0 OpenAPI and add golden/conformance tests.

### T3: MITM Enforcement Semantics

- Wire `allow|ask|block` through `LocalMcpDecisionProvider`.
- Ensure `ask` does not dispatch.
- Ensure request `block` does not dispatch.
- Ensure response `block` redacts/replaces without leaking content.
- Ensure MCP request/response `rewrite` mutates only the matched target
  and redacts rewrite payload telemetry.
- Ensure HTTP `block` does not dial upstream, HTTP `ask` fails closed,
  HTTP header strip lists run before telemetry/upstream dispatch, and
  HTTP `rewrite` mutates only the data selected by `rewrite_target`.
- Ensure DNS `block` does not call upstream, DNS `ask` fails closed, and
  DNS `rewrite` synthesizes the answer without upstream resolution.
- Ensure model request `block`/`ask` does not call upstream, model request
  `rewrite` mutates only the matched target, and model telemetry redacts
  rewritten payloads.
- Ensure model response, model tool-call, and model tool-response
  block/rewrite paths do not leak original sensitive content.
- Add local hook invocation using the same request/response structs as
  remote hooks, then add remote HTTPS hook forwarding behind strict
  config, timeout, auth, and fail-closed rules.
- Ensure response `ask` is either rejected as unsupported or mapped to a
  safe approval-required response, with tests documenting the choice.

### T4: Settings API/UI Surface

- Update service settings response types to expose typed MCP policy.
- Update any frontend/settings schema consumers.
- Ensure reload applies new MCP policy without restarting the VM.
- Add service/settings response types for model and hook policy.
- Add Policy Hook Spec0 export endpoint or CLI/export command.
- Add settings-builder tests proving UI allow/block/header controls
  compile into equivalent named policy rules.
- Add docs/examples for common policies and hook server authors.

### T4b: Docs, Just, Session Reference, And UI Product Surface

This is not polish. Policy V2 changes the user contract, so the docs site,
developer recipes, session database reference, and settings UI must move
together with the implementation.

- Audit stale docs after the framed MITM MCP cutover and Policy V2 work:
  `docs/src/content/docs/architecture/mcp-gateway.md`,
  `docs/src/content/docs/architecture/mcp-aggregator.md`,
  `docs/src/content/docs/architecture/mitm-proxy.md`,
  `docs/src/content/docs/architecture/session-telemetry.md`,
  `docs/src/content/docs/security/network-isolation.md`,
  `docs/src/content/docs/usage/mcp-tools.md`,
  `docs/src/content/docs/architecture/settings.md`,
  `docs/src/content/docs/architecture/settings-schema.md`, and
  `docs/src/content/docs/development/just-recipes.md`.
- Remove or clearly historicalize stale MCP gateway language that implies
  guest MCP primarily uses the old `vsock:5003` path. The current product
  path is `/run/capsem-mcp-server` through framed MITM MCP on `vsock:5002`.
- Add a Policy V2 reference page for the canonical
  `policy.<type>.<rule_name>` TOML shape, callbacks, CEL subject fields,
  decisions, priority ordering, rewrite semantics, header stripping, and
  user/corp precedence.
- Add a session telemetry reference section that describes how
  `session.db` proves policy decisions across `mcp_calls`, `net_events`,
  `dns_events`, `model_calls`, `tool_calls`, `tool_responses`, and future
  hook audit rows, including example `just query-session` SQL.
- Update `development/just-recipes.md` with the verification workflow for
  this sprint: focused Rust suites, `just smoke`, `just inspect-session`,
  `just query-session`, frontend checks, docs build, and when `just test`
  is the gate.
- Update the settings UI so existing allow/block/domain/header affordances
  generate, display, edit, delete, and save named Policy V2 rules rather
  than old one-off settings where the policy model has moved.
- Add frontend tests for generated policy-rule objects and import/export
  behavior. Add docs build verification with `cd docs && pnpm run build`.

### T5: VM E2E And Telemetry Proof

- VM E2E: tool argument-name `block` from `user.toml`.
- VM E2E: tool argument-value `ask` from `user.toml`.
- VM E2E: response return-value `block` from `user.toml`.
- VM E2E: configured external MCP tool inspected and blocked/asked at
  the framed MITM boundary.
- VM E2E: HTTP method/URL path/query/header block and request-header strip.
- VM E2E: DNS block and DNS rewrite from `user.toml`.
- VM E2E: model request block/ask/rewrite from `user.toml`.
- VM E2E: model response and model tool-call/tool-response block/rewrite
  with no-leak assertions.
- VM E2E: local hook and HTTPS hook server returning allow/block/rewrite
  decisions against the exported OpenAPI contract.
- VM E2E: builtin HTTP through framed MCP writes both `mcp_calls` and
  `net_events` with correct policy decision, domain, method, port, status,
  bytes, process name, and trace correlation.
- Query `session.db` for MCP policy decisions, HTTP/net policy decisions,
  DNS decisions, model policy decisions, hook decisions, rule, reason,
  previews, process attribution, no-dispatch/no-upstream/no-leak
  invariants, and rewrite redaction.

### T6: Performance And Regression Gate

- Run focused Rust suites.
- Run full framed MCP E2E file.
- Run `capsem-doctor -k mcp`.
- Run scoped `mcp-load` and compare to T4 coverage-hardening baseline.
- Run focused HTTP/DNS policy perf checks, including DNS cache/rewrite
  hot paths if available.
- Run model parser/policy perf checks for streaming response and tool-call
  paths.
- Run hook latency benchmarks for local decisions, timeout path, and HTTPS
  hook round trip.
- Run `just smoke`; keep `just test` as final sprint gate.

## Coverage Matrix

| Category | Required Proof |
|---|---|
| Unit/contract | Enum serde, strict TOML parse failures, CEL parse/type checks, callback validation, priority ordering, rule matching, precedence, ask/block no-dispatch decisions, rewrite target regex/capture validation, HTTP header normalization, DNS qtype/IP validation, model field extraction, hook request/response serde, OpenAPI generation |
| Functional | Config loader and service settings API round-trip MCP/HTTP/DNS/model/hook typed policy, UI-generated allow/block/header controls compile into named rules, and hot reload applies them |
| Adversarial | Unknown decisions, missing rewrite target/value, unknown TOML fields, invalid CEL, invalid callbacks, invalid rewrite regex/captures, invalid query/header names, invalid DNS payloads, malformed model payloads, duplicate/conflicting rules, deny/ask precedence, blocked response redaction, stripped header no-leak, hook timeout/auth/schema mismatch |
| E2E/VM | Real VM with `/run/capsem-mcp-server`, HTTP requests, DNS lookups, model provider-shaped traffic, local/HTTPS hook server, `user.toml` policy, external stdio MCP server, and framed MITM enforcement |
| Telemetry | `mcp_calls`, `net_events`, `dns_events`, `model_calls`, `tool_calls`, `tool_responses`, and hook audit rows record decision/rule/reason/previews/process/trace; ask/block rows show terminal decisions; blocked/rewrite/stripped payloads do not leak |
| Performance | `mcp-load` no material regression versus the latest T4 framed baseline; HTTP/DNS/model/hook policy and rewrite paths stay within recorded budgets |
| Docs/UI/recipes | Docs site builds, stale MITM/MCP docs are corrected, Policy V2 reference examples match parser tests, `session.db` reference names the real tables/fields, just recipe docs match the justfile, and the settings UI saves the same named rules the TOML parser accepts |

## Done

- User-facing policy decisions are exactly `allow`, `ask`, `block`, and
  `rewrite` with a validated payload.
- Runtime code uses enums, not raw decision strings.
- TOML/settings/schema/API all support the same typed policy model for
  MCP, HTTP, DNS, model traffic, and hooks.
- TOML uses named `policy.<type>.<rule_name>` rules with `on`, CEL `if`,
  `decision`, `priority`, and `rewrite_target`/`rewrite_value`; simple UI
  controls compile into the same rules.
- MCP argument-name/value and return-value policies work from config,
  not only from test fixtures.
- HTTP method/URL path/query/header policies and header strip lists work from
  config.
- DNS block and rewrite policies work from config and populate
  `dns_events`.
- Model request, model response, model tool-call, and model tool-response
  policies work from config and populate model/tool telemetry.
- Policy Hook Spec0 is exported as OpenAPI 3.1 from runtime Rust types and
  can be used by a receiving hook server.
- Docs site, settings UI, just recipe docs, and session database reference
  all describe the same Policy V2 and framed MITM MCP product surface.
- VM E2E proves allow, ask, block, and rewrite on builtin/external MCP,
  HTTP, DNS, model, and hook boundaries.
- Telemetry proves the enforcement result and does not leak blocked,
  stripped, or rewritten sensitive content.
- Tracker and changelog are current, and the sprint closes only after
  the testing gate is run or explicit remaining debt is named.
