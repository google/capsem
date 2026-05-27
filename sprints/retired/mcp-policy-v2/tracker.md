# Sprint: Policy V2 (MCP + HTTP + DNS + Models + Hooks)

## Status

Planning started after the T4/T5 review exposed a product gap: runtime
MCP policy matchers exist and are tested, but the full policy model is
not exposed through TOML/settings. Current user-facing terminology also
has two misleading concepts: `Warn`, which behaves like allow, and
`audit_only`, which now enforces denies.

Scope expanded on 2026-05-08: Policy V2 must cover MCP, HTTP, DNS, model
traffic, and policy hooks as one typed boundary model. HTTP needs
method/URL path/query/header matching plus request/response header
stripping.
DNS is already parsed and logged through `dns_events`, and its internal
redirect behavior should become typed user-facing `rewrite`. Model
traffic is already parsed into `model_calls`, `tool_calls`, and
`tool_responses`, so it needs request/response/tool-call/tool-response
policy too. Hook forwarding needs an exportable OpenAPI Spec0 so
third-party HTTPS decision servers can be implemented safely.

Design correction on 2026-05-08: canonical TOML is
`policy.<type>.<rule_name>` with flat fields. `on` names the callback,
`if` is one CEL expression, `decision` is the typed outcome, and rewrite
rules use `rewrite_target` plus `rewrite_value`. Every rule has an
explicit integer `priority`; lower numbers run first, with deterministic
tie-breaks by type/name. Conditions are not lists; CEL conjunction handles
compound predicates. Rewrite targets must be powerful enough for
regex/capture-based replacements, and simple UI domain/tool/header
controls must compile into this same rule IR.

Product-surface correction on 2026-05-08: docs site, session database
reference, just recipe docs, and settings UI are part of the sprint. They
are not final cleanup because users learn and operate the policy model
through those surfaces.

## Honest Answer To The Trigger Question

- Do we have MCP policy? **Yes, partially.**
- Do we have runtime tests that an argument name/value can block a tool
  call? **Yes, in `mcp_frame` unit/framed-session tests using
  `McpPolicy.audit_rules`.**
- Do we have TOML/settings support that lets a user or corp policy
  configure those argument rules? **Yes, for named `policy.mcp.*`
  rules.** The settings API now saves and returns MCP rule objects, and
  VM E2E proves a saved argument-name block reaches the guest framed MCP
  path. Argument-value ask, return-value block, rewrite, and external-tool
  configured VM coverage are still pending.
- Do we have E2E proof that a configured TOML argument policy blocks a
  real guest MCP tool call and records telemetry? **Yes, for builtin
  `local__echo` argument-name block through `/settings` + `/reload-config`
  + real guest MCP, including `session.db` decision/rule/reason/process
  attribution and no-leak preview assertions.**
- Do we have an `ask` decision? **Yes, partially.** MCP/HTTP/DNS runtime
  paths fail closed without dispatch/upstream resolution; there is still
  no approval UI or persisted approval workflow.
- Do we have a typed `rewrite` decision for MCP/HTTP/DNS? **Yes,
  partially.** MCP request/response, HTTP request, and DNS query runtime
  paths enforce `rewrite`; HTTP response/body, VM E2E, and stricter
  per-surface rewrite validation are still pending.
- Do we have HTTP policy for method/URL path/query/header and header
  stripping from TOML/settings? **Yes, for the T5 slice.** VM E2E now
  proves configured `http.request` block rules match method, URL path,
  query, and request headers; request-header strip runs before upstream and
  telemetry; response-header strip runs before guest delivery and telemetry.
- Do we have DNS E2E/session proof for configured block/rewrite from
  TOML? **Yes.** VM E2E now proves configured `dns.query` block and
  rewrite rules through the guest resolver path and `dns_events`.
- Do we have model policy for model request, model response, model
  tool-call, or model tool-response? **Yes, for the implemented T5
  slice.** `model.request` allow/block/ask now runs before provider
  dispatch and records `net_events` policy fields; VM E2E proves configured
  request block plus ask/rewrite fail-closed no-leak behavior.
  `model.tool_response` block/rewrite now runs before provider dispatch,
  redacts telemetry, and rewrites OpenAI-shaped tool-result message
  content. `model.response` block/rewrite and provider-emitted
  `model.tool_call` block/ask/rewrite now run before guest delivery with
  host MITM/session DB proof, and deterministic VM E2E covers response and
  provider-emitted tool-call block/rewrite no-leak behavior through a local
  OpenAI-compatible fixture.
- Do we have a hook API contract for third-party HTTPS policy plugins?
  **Yes.** Policy Hook Spec0 is exported as checked-in OpenAPI generated
  from strict Rust wire types and served by `GET /policy-hook/spec`.
- Is `audit_only` accurate for the current framed path? **No; deny is
  enforced.**

## Tasks

- [ ] T0: Red tests and behavior audit
  - [ ] Add failing TOML parse/settings tests for `allow|ask|block|rewrite`
  - [x] Add failing TOML tests for named `policy.<type>.<rule_name>` rules with `on`, CEL `if`, `decision`, priority, and rewrite target/value fields
  - [ ] Add failing TOML rule tests for MCP argument-name, argument-value, return-value, and rewrite decisions expressed through CEL
  - [ ] Add failing TOML/settings tests for HTTP method/URL path/query/header CEL rules
  - [x] Add failing TOML/settings tests for the `github.com/openai/...` or `github.com/openclaw/...` strawman block/rewrite policy
  - [ ] Add failing TOML/settings tests for HTTP request/response header allow/strip lists
  - [x] Add failing TOML/settings tests for DNS block/ask/rewrite rules
  - [ ] Add failing TOML/settings tests for model request, model response, model tool-call, and model tool-response rules
  - [x] Add failing OpenAPI Spec0 export tests for every policy callback/decision/rewrite shape
  - [x] Add failing VM E2E for configured argument-name block
  - [x] Add failing VM E2E for HTTP header strip no-leak
  - [x] Add failing VM E2E for DNS rewrite plus `dns_events` proof
  - [x] Add failing VM E2E for model request/response/tool-call/tool-response no-leak policy
  - [ ] Add failing E2E for local and HTTPS hook server decisions using the exported Spec0 contract
  - [ ] Add failing tests for builtin HTTP policy env/live reload wiring
  - [x] Replace vacuous net/dns telemetry assertions with real row/value assertions
  - [x] Record all current runtime-only coverage and missing settings/E2E coverage
- [ ] T1: Typed decision model
  - [ ] Introduce shared `PolicyDecision { Allow, Ask, Block, Rewrite { target, value } }`
  - [ ] Map MCP decision handling onto the shared enum
  - [ ] Map HTTP decision handling onto the shared enum without regressing read/write defaults
  - [ ] Map DNS redirect/redirected semantics onto typed `rewrite`
  - [ ] Map model request/response/tool-call/tool-response policy onto the shared enum
  - [x] Define hook request/response Rust wire types as the source for remote plugin compatibility
  - [ ] Remove/migrate user-facing `Warn`
  - [ ] Replace `audit_only` mode naming
  - [ ] Preserve or explicitly migrate existing `default_tool_permission` / `tool_permissions`
- [ ] T2: TOML and settings loader
  - [x] Strict serde/settings model for named `policy.<type>.<rule_name>` maps
  - [x] Strict serde for callback enum from `on`
  - [x] Parse/type-check the documented CEL-compatible subset from `if`
  - [x] Strict serde for decision enum
  - [x] Strict serde and deterministic ordering for rule priority
  - [x] Strict serde for rewrite target regex/selector and replacement captures
  - [ ] Strict serde for HTTP header allow/strip lists
  - [ ] Strict serde for DNS qname/qtype/IP rewrite rules
  - [ ] Strict serde for model request/response/tool-call/tool-response matchers
  - [x] Strict serde for hook endpoint config, auth, fail-closed, timeout, and fallback settings
  - [x] User/corp merge precedence tests
  - [x] Unknown decision/unknown field/invalid CEL/invalid callback/invalid rewrite target errors
  - [ ] Schema/defaults/settings builder update
  - [ ] UI domain allow/block lists, per-tool controls, and header controls compile into equivalent named policy rules
  - [x] Generate Policy Hook Spec0 OpenAPI from runtime Rust wire types
- [ ] T3: MITM enforcement
  - [ ] `allow` dispatches
  - [x] MCP `ask` does not dispatch and returns a policy error
  - [x] MCP request `block` does not dispatch
  - [x] MCP response `block` replaces/redacts original result
  - [x] MCP request `rewrite` mutates only matched argument targets and redacts payload telemetry
  - [x] MCP response `rewrite` mutates matched response text and redacts payload telemetry
  - [x] HTTP `block` does not dial upstream
  - [x] HTTP `ask` fails closed without upstream dispatch
  - [x] HTTP request header strip runs before upstream dispatch and before telemetry capture
  - [x] HTTP response header strip runs before guest delivery and before telemetry capture
  - [x] HTTP request `rewrite` mutates only data selected by `rewrite_target`
  - [x] HTTP response header/status `rewrite` mutates only data selected by `rewrite_target`; unsupported body rewrite targets fail closed
  - [x] DNS `block` returns a denial response without upstream resolution
  - [x] DNS `ask` fails closed without upstream resolution
  - [x] DNS `rewrite` synthesizes the configured answer without upstream resolution
  - [x] Model request `block`/`ask` prevents upstream provider dispatch
  - [x] Model request `rewrite` rules fail closed without upstream dispatch or request-body telemetry leakage until targeted mutation is implemented
  - [ ] Model request `rewrite` mutates only matched request targets
  - [x] Model response `block`/`rewrite` prevents leaks before the guest/client sees content
  - [x] Model tool-call `block`/`ask` prevents unsafe provider-emitted tool calls from reaching the agent loop
  - [x] Model tool-response `block`/`rewrite` prevents sensitive local tool output from reaching the provider
  - [ ] Local hook dispatch uses the same request/response types as remote hooks
  - [x] HTTPS hook forwarding enforces auth, timeout, body cap, schema version, and fail-closed behavior
  - [ ] telemetry uses typed decision/rule/reason consistently
- [ ] T4: Settings API/UI interface
  - [x] Service settings API exposes typed MCP/HTTP/DNS/model/hook policy
  - [ ] Settings schema/export understands typed MCP/HTTP/DNS/model/hook rules
  - [ ] Settings builder translates existing UI allow/block/header affordances into named policy rules
  - [x] OpenAPI Spec0 export is available as a checked-in generated artifact and service export path
  - [ ] Reload applies policy to running process
  - [x] Docs/examples updated for policy authors and hook server authors
- [x] T4b: Docs, just recipes, session reference, and UI product surface
  - [x] Audit stale docs after framed MITM MCP cutover and the policy model:
    `architecture/mcp-gateway.md`, `architecture/mcp-aggregator.md`,
    `architecture/mitm-proxy.md`, `architecture/session-telemetry.md`,
    `security/network-isolation.md`, `usage/mcp-tools.md`,
    `architecture/settings.md`, `architecture/settings-schema.md`, and
    `development/just-recipes.md`
  - [x] Remove or historicalize stale guest MCP `vsock:5003` language;
    current product path is framed MITM MCP on `vsock:5002`
  - [x] Add policy docs reference page for named TOML rules, callbacks,
    CEL subjects, decisions, priority, rewrite, header stripping, and
    user/corp precedence
  - [x] Add session.db policy audit reference with table/field mapping and
    `just query-session` SQL for MCP, HTTP, DNS, model, and hooks
  - [x] Update development just-recipes docs for the policy verification
    workflow: focused Rust suites, frontend checks, docs build,
    `just smoke`, `just inspect-session`, `just query-session`, and
    final `just test`
  - [x] Update settings UI to display/generate/edit/delete named policy
    rules for allow/block/domain/header controls instead of hiding the
    real policy object model
  - [x] Add frontend import/export and generated-rule tests for the policy UI
  - [x] Full docs consistency pass: DNS proxy vs legacy dnsmasq, vsock
    port references, MCP tool count, session DB tables, policy telemetry,
    benchmark docs, and internal docs links
  - [x] Verify with `cd docs && pnpm run build`, frontend checks, and
    explicit UI visual verification after UI changes
- [ ] T5: VM E2E and telemetry proof
  - [x] `user.toml` argument-name `block` for builtin tool
  - [x] `user.toml` argument-value `ask` for builtin tool
  - [x] `user.toml` return-value `block`
  - [x] `user.toml` MCP argument or return `rewrite`
  - [x] Configured external MCP tool is inspected and policy-controlled at the MITM boundary
  - [x] `user.toml` HTTP method/URL path/query/header block
  - [x] `user.toml` HTTP request/response header strip no-leak
  - [x] Builtin HTTP through framed MCP writes both `mcp_calls` and `net_events`
  - [x] Builtin HTTP blocked by config records denial without upstream side effect
  - [x] `user.toml` DNS block
  - [x] `user.toml` DNS rewrite
  - [x] `user.toml` model request block
  - [x] `user.toml` model request ask/rewrite
  - [x] `user.toml` model response block/rewrite
    - [x] Host MITM functional/session proof for configured TOML-shaped model response block/rewrite
    - [x] VM E2E fixture proof for configured model response block/rewrite no-leak behavior
  - [x] `user.toml` model tool-call block/ask/rewrite
    - [x] Host MITM functional/session proof for configured TOML-shaped model tool-call block/ask/rewrite
    - [x] VM E2E fixture proof for configured model tool-call block/rewrite no-leak behavior
  - [x] `user.toml` model tool-response block/rewrite
  - [ ] Local hook decision path
  - [ ] HTTPS hook decision path against a test server generated or validated from Spec0
  - [x] `session.db` proves MCP decision/rule/reason/previews/process attribution
  - [x] `session.db` proves HTTP/net decision/rule/header-redaction behavior
  - [x] `session.db` proves DNS decision/rule/qname/rcode/rewrite behavior
  - [x] `session.db` proves model request block policy fields and redaction behavior
  - [x] `session.db` proves model request ask/rewrite and model tool-response block/rewrite decisions and redaction behavior
  - [x] `session.db` proves model response/tool-call policy decisions and redaction behavior on the host MITM fixture path
  - [x] Hook audit rows prove endpoint id, spec version/hash, decision id, latency, timeout/error status, and fallback status
  - [x] MCP no-dispatch, no-leak, fail-closed, and rewrite-redaction invariants asserted for configured ask/block/rewrite rules
  - [x] HTTP/DNS/model-request/model-tool-response no-dispatch, no-upstream, no-leak, fail-closed, and rewrite-redaction invariants asserted
  - [x] Model response/tool-call no-dispatch, no-upstream, no-leak, fail-closed, and rewrite-redaction invariants asserted
  - [ ] Hook no-dispatch, no-upstream, no-leak, fail-closed, and rewrite-redaction invariants asserted from configured policy
- [x] T6: Performance and final regression
  - [x] Focused Rust suites
  - [x] Full framed MCP E2E
  - [x] `capsem-doctor -k mcp`
  - [x] Scoped `mcp-load`
  - [x] Scoped HTTP/DNS policy/rewrite perf checks
  - [x] Scoped model parser/policy perf checks
  - [x] Hook wire decode/config perf checks
  - [x] `just smoke`
  - [x] `just test` or explicit named debt
- [x] T7: Full test and E2E release gate
  - [x] Every implemented T5 policy VM E2E passes from a clean debug build
  - [x] Every focused Rust policy suite passes with warnings as errors
  - [x] `capsem-doctor -k mcp` passes against the built guest path
  - [x] Full session DB policy audit queries prove MCP, HTTP, DNS, and
    model rows where implemented
  - [x] `just smoke` passes
  - [x] `just test` passes, or every remaining failure is named in the
    tracker with owner, reason, and follow-up
  - [x] Final E2E/perf numbers are recorded in the coverage ledger before
    the sprint is called complete
- [x] Changelog
- [ ] Commit series

## Coverage Ledger

- Unit/contract: current green slice covers named TOML maps, callback enum
  serde, decision enum serde including `warn` rejection, priority
  preservation and callback ordering, regex/capture-aware rewrite
  validation, callback/table consistency, rule-name validation, unknown
  policy-type rejection, HTTP header-strip name normalization/validation,
  strict CEL-compatible condition validation for documented conjunction,
  comparison, `has`, string-method, and regex `matches` expressions against
  per-callback subject fields, normalized-subject rule evaluation with
  priority/name ordering, MCP decision-provider integration for
  block/ask request rules, framed-MITM MCP no-dispatch behavior for a
  named Policy V2 block rule, session `mcp_calls` policy fields and
  argument-redacted request preview for that block, MCP response
  block/redaction with empty response preview, MCP response rewrite with
  redacted guest response and redacted telemetry
  preview, MCP request rewrite before aggregator dispatch with redacted
  request telemetry, adversarial request-rewrite failure no-dispatch/no-leak
  telemetry, Policy V2 HTTP request block/ask/rewrite enforcement in the
  MITM hook pipeline, request header stripping before upstream construction
  and telemetry, Policy V2 HTTP response header stripping and response
  header rewrite enforcement in the MITM hook pipeline, adversarial
  fail-closed behavior for unsupported response body rewrite targets,
  `net_events` policy mode/action/rule/reason fields, and user/corp policy
  merge precedence, model-request provider/model/header/body matching,
  truncated JSON fallback matching, and model-request ask/rewrite/invalid
  condition fail-closed handling. Still missing the full CEL language
  surface, HTTP response body chunk rewrite support, model response and
  model tool dispatch integration, DNS qtype/IP validation, richer model
  field extraction beyond request metadata, hook request/response serde,
  and OpenAPI generation.
- Bug found during T3 request-rewrite hardening: an unsupported
  `mcp.request` rewrite target could take a fail-closed error path while
  still serializing the original request arguments into `mcp_calls`.
  Fixed by logging a scrubbed request preview on rewrite-target errors and
  added `framed_session_rewrite_policy_v2_mcp_request_error_redacts_telemetry`.
- Bug found during T5 configured MCP E2E: a `policy.mcp.*` request block
  stopped dispatch but still serialized the original `arguments` object
  into `mcp_calls.request_preview`, including the blocked token value.
  Fixed by scrubbing arguments for Policy V2 pre-dispatch denials while
  preserving method/tool attribution, then proved it with
  `test_framed_guest_mcp_policy_v2_argument_block_from_settings_no_leak`.
- Bug found during T5 builtin HTTP E2E: `capsem-process` spawned
  `capsem-mcp-builtin` with session path/session DB environment but did
  not pass the merged domain allow/block policy through
  `CAPSEM_DOMAIN_ALLOW` and `CAPSEM_DOMAIN_BLOCK`. A configured blocked
  builtin HTTP call therefore attempted upstream resolution instead of
  failing at the policy boundary. Fixed by deriving builtin HTTP env from
  the merged domain policy at process startup and proved with real guest
  framed MCP calls plus `mcp_calls` and `net_events` assertions.
- Functional: current green HTTP slice covers `policy.http.*` request
  block/ask no-upstream behavior through the real TLS MITM path,
  request URL rewrite plus credential-header strip before telemetry/upstream
  construction, hook-level adversarial rejection of cross-host rewrites,
  plain-HTTP proxy-path response header strip/rewrite before guest delivery,
  response telemetry using the rewritten header hash instead of the original,
  and fail-closed response rewrite errors that do not leak upstream headers or
  bodies to the guest or `net_events`. Still missing VM E2E configured from
  `user.toml` and response body chunk rewrite support.
- Functional: current green DNS slice covers named `policy.dns.*`
  `dns.query` allow/block/ask/rewrite rules through the production DNS
  handler: allow forwards upstream while preserving audit fields, block
  and ask return synthetic NXDOMAIN without upstream resolution, rewrite
  synthesizes configured A/AAAA answers without upstream resolution,
  invalid rewrite answers and wrong-surface rewrite targets fail closed
  with SERVFAIL and no upstream dispatch, and live policy mutation is
  checked before cached answers.
- Functional: current green T5 HTTP/DNS VM slice covers configured
  `policy.http.*` and `policy.dns.*` rules saved through `/settings` and
  enforced in a real guest session: HTTP block matches method, path,
  query, and request header without upstream dispatch; request-header
  strip redacts before upstream and `net_events`; response-header strip
  redacts before guest delivery and telemetry; DNS block returns denial
  without upstream resolution; DNS rewrite synthesizes the configured
  answer without upstream resolution.
- Functional: current green model-request slice covers `policy.model.*`
  `model.request` allow/block/ask through the MITM request path:
  allow dispatches and records policy fields, block/ask stop before
  upstream dial, unsupported rewrite fails closed, invalid runtime
  conditions fail closed, truncated JSON can still match by fallback model
  extraction, and non-LLM provider paths do not accidentally run model
  rules.
- Functional: current green model tool-response slice covers
  `policy.model.*` `model.tool_response` block/rewrite before provider
  dispatch for OpenAI-shaped tool-result messages. Block returns a policy
  denial without leaking the local tool output; rewrite mutates matched
  tool-result content, repairs `Content-Length`, and records redacted
  `net_events`, `model_calls`, and `tool_responses` previews. Matching by
  `tool.call_id`, content, response content, and `is_error` is covered;
  matching by `tool.name` is not yet available because OpenAI trailing
  tool-result request messages carry `tool_call_id`, not the tool name.
- Functional: current green model response/tool-call slice covers
  `policy.model.*` `model.response` block/rewrite and `model.tool_call`
  block/ask/rewrite before guest delivery for OpenAI-shaped SSE streams.
  Block and ask replace the upstream response with a policy denial without
  leaking upstream text or tool-call arguments to the guest or
  `net_events`; response rewrite redacts text before `model_calls`
  `text_content`; tool-call rewrite redacts provider-emitted arguments
  before guest delivery and before `tool_calls.arguments` persistence.
- Bug found during model response/tool-call integration testing: the host
  MITM response policy path could be given an explicit OpenAI provider in
  enforcement tests, but the downstream SSE parser/interpreter hooks still
  gated only on `ConnMeta.domain`. Local OpenAI-compatible fixture traffic
  therefore enforced policy but missed `model_calls` text/tool-call
  extraction. Fixed by carrying `ConnMeta.ai_provider` through the chunk
  hook chain and using it consistently with domain-based provider
  detection.
- Verification gap found during model tool-call session DB testing:
  `recent_model_calls()` intentionally does not hydrate nested
  `tool_calls`; tests must query `tool_calls_for(model_call_id)` or SQL
  join the nested table. The new coverage asserts the actual
  `tool_calls` row instead of relying on a shallow reader.
- Bug found during model tool-response adversarial testing: a provider
  request can contain multiple trailing tool-result messages, and the first
  implementation could let an allow decision for one result short-circuit a
  later secret-bearing result. Fixed by evaluating each parsed tool result
  as its own callback event and combining outcomes so any matched ask/block
  denies before provider dispatch, while rewrites still mutate before
  dispatch.
- Functional: current green slice covers config loader preservation,
  `/settings` save/return/delete of object-valued policy rules, atomic
  rejection of invalid policy saves mixed with regular settings, service
  handler rejection of invalid policy conditions and callback/type
  mismatches without mutating the user config, direct MCP/model-policy rule
  save/return tests, and frontend settings-store/model types for staged
  policy-rule objects. Still missing reload, schema/defaults generation, and
  UI-to-policy compilation for MCP, HTTP, DNS, model, and hook policy.
- Functional: current green MCP VM slice covers configured
  `policy.mcp.*` request `block`, argument-value `ask`, request-argument
  `rewrite`, external stdio request `block`, and external stdio response
  `block` through `/settings`, `/reload-config`, the real
  `/run/capsem-mcp-server` relay, and MITM-owned `mcp_calls` telemetry.
  The new T5 tests passed on the first run, so this slice added missing
  black-box proof rather than finding another runtime bug.
- Adversarial: current green slice covers rejected `warn`, missing
  rewrite target/value, empty rewrite values, malformed/unquoted/
  unterminated regex targets, trailing regex garbage, unknown rewrite
  captures, rewrite fields on non-rewrite decisions, unknown rule fields,
  callback/table mismatch, unknown policy types, invalid rule names, invalid
  settings-save policy keys, invalid header-strip names, malformed CEL
  conjunctions and string literals, invalid CEL subject fields, unknown CEL
  methods, invalid `matches` regexes, bad `has(...)` arguments, unsupported
  literal types, and the bug where a missing field incorrectly satisfied a
  negative comparison. It also covers atomic
  rejection of invalid or corp-locked policy updates mixed with regular
  settings, runtime DNS fail-closed behavior for invalid rewrite IP values,
  wrong-surface DNS rewrite targets, unsupported HTTP response rewrite
  targets, response header stripping leak prevention, response rewrite
  fail-closed no-leak behavior, malformed/truncated model-request bodies,
  invalid model runtime conditions, non-LLM path bypass, configured MCP
  ask fail-closed no-leak behavior, MCP request rewrite redaction, external
  MCP request block no-dispatch behavior, external MCP response block
  no-response-leak behavior, configured HTTP no-upstream block and
  header-redaction behavior, configured DNS no-upstream block/rewrite
  behavior, configured model request/tool-response no-leak behavior, and
  multi-tool-result model tool-response bypass attempts where an allow rule
  for one result must not bless another secret result. Still missing invalid
  query/header names beyond strip lists, model response and provider-emitted
  model tool-call adversarial cases, hook timeout, hook auth failure, and
  hook schema mismatch.
- E2E/VM: current green slice boots a real VM, saves a `policy.model.*`
  rule through `/settings`, sends OpenAI-shaped HTTPS traffic from the
  guest through the MITM, blocks before upstream dispatch, and asserts
  `net_events` plus `model_calls` policy/no-leak rows in `session.db`.
  The MCP slice also boots a real VM, saves a `policy.mcp.*`
  argument-name block, argument-value ask, request rewrite, external stdio
  request block, and external stdio response block through `/settings`,
  reloads config, calls `local__echo` and `fast__ping` through
  `/run/capsem-mcp-server`, and asserts `mcp_calls`
  decision/rule/reason/process attribution plus redacted request/response
  previews in `session.db`. The HTTP/DNS slice saves configured
  `policy.http.*` and `policy.dns.*` rules, drives guest `curl` and
  `socket.getaddrinfo`, and asserts `net_events`/`dns_events` decision,
  rule, reason, path/query/header-redaction, qname/rcode/rewrite, and
  no-upstream fields. The model slice saves configured
  `policy.model.*` rules, drives guest OpenAI-shaped HTTPS requests, and
  asserts model-request ask/rewrite fail-closed plus model tool-response
  block/rewrite no-leak behavior in `net_events`, `model_calls`, and
  `tool_responses`. Host MITM fixture coverage proves model response and
  provider-emitted model tool-call no-leak behavior with session DB
  assertions, and deterministic VM fixture coverage now routes
  `https://api.openai.com` through MITM to a local OpenAI-compatible
  upstream to prove response block/rewrite and provider-emitted tool-call
  block/rewrite before guest delivery. Local/HTTPS hook E2E from
  `user.toml` or `corp.toml` is still missing.
- Telemetry: current green DNS slice covers `DnsHandlerResult` to
  `DnsEvent` propagation of Policy V2 mode/action/rule/reason and
  `dns_events` schema/writer persistence of those fields, including an
  index on `policy_rule` for audit queries. Current green HTTP response
  slice covers `net_events` response header telemetry after policy mutation
  and fail-closed response rewrite telemetry without upstream header/body
  leaks. Current VM HTTP/DNS slice covers configured `net_events` and
  `dns_events` policy mode/action/rule/reason, status/rcode, no-upstream
  byte/resolver timing, rewritten path/answer, and stripped header
  redaction behavior. Current VM model-request slice covers configured
  `net_events` policy mode/action/rule/reason/status/byte counts and
  `model_calls` no-leak request preview behavior for blocked, ask, and
  rewrite-fail-closed requests. Current VM model tool-response slice covers
  redacted `net_events.request_body_preview`, `model_calls.request_preview`,
  and `tool_responses.content_preview` after block/rewrite decisions.
  Current VM MCP slice covers configured `mcp_calls` policy
  mode/action/rule/reason, process attribution, request/response previews,
  denied decision, and no-leak behavior for blocked request, ask request,
  rewritten request, external blocked request, and external blocked response
  paths. Current VM model response/tool-call fixture covers `net_events`
  policy fields and no-leak previews for configured response and
  provider-emitted tool-call rules. Hook audit rows are covered by the hook
  runtime unit/contract suite; still missing VM `session.db` proof for
  configured local/HTTPS hook dispatch from user or corp policy.
- Performance: current T7 `mcp-load` run stayed above the T4 baseline:
  c1 1945.6 rps (p50 0.5ms, p95 0.8ms, p99 1.3ms), c10 8968.8 rps
  (p50 1.1ms, p95 1.5ms, p99 2.0ms), c50 9441.1 rps (p50 5.2ms,
  p95 6.1ms, p99 7.7ms), and c200 9173.1 rps (p50 21.5ms,
  p95 24.4ms, p99 28.3ms). Current `just smoke` integration throughput
  was 21,370,333 B/s in 0.467235s. Scoped `policy_v2` Criterion coverage
  now records HTTP request match 1.61-1.76 us, DNS query match 960-967 ns,
  model response match 1.32-1.37 us, model tool-call match 2.11-2.12 us,
  hook-decision match 1.51-1.52 us, and hook response decode 330-335 ns.
- Docs/UI/recipes: current green slice covers the Policy V2 reference page,
  stale framed-MITM MCP doc cleanup, session telemetry policy-audit SQL for
  `mcp_calls`, `net_events`, `dns_events`, model/tool rows, and future hook
  rows, just recipe verification docs, settings import/export of named
  `policy.<type>.<rule_name>` objects, settings UI editing/deleting/staging
  generated rules, and browser visual verification of the live Policy panel.
  Still missing VM E2E proof that generated UI rules loaded from
  `user.toml`/`corp.toml` enforce in a real session.
- Missing/deferred: approval UI is deliberately out of scope. `ask`
  should be safe by returning approval-required or DNS/HTTP fail-closed
  behavior without dispatch/upstream resolution. The credential broker is
  also out of scope; this sprint builds the typed `rewrite` decision shape,
  hook wire contract, and HTTPS forwarding guardrails that future broker
  hooks can return safely.

## Notes

- Avoid raw strings in runtime decision plumbing. TOML text should
  deserialize directly into enums and fail closed on unknown values.
- The rule table path is identity: `policy.<type>.<rule_name>`. Do not
  reintroduce `[[*.rules]]`, action buckets, or nested `match`/`then`
  sections.
- `if` is one CEL expression. Use CEL `&&` and helper functions for
  conjunction; do not make it a list.
- The first T2 implementation validates a strict CEL-compatible subset
  before settings are written: `&&`, `==`, `!=`, `has(field)`,
  `.matches()`, `.contains()`, `.endsWith()`, and `.startsWith()` over
  documented per-callback subject fields. Runtime subject evaluation and
  any broader CEL language support remain separate work.
- `priority` is required in the product examples and should be preserved
  through TOML, settings save, settings response, and generated UI rules.
- `rewrite_target` is a validated target expression, usually regex-based
  over a normalized field, and `rewrite_value` is a capture-aware
  replacement template. URL path matching belongs inside CEL as subject
  data, not as a top-level policy key.
- `Warn` should not survive as user-facing policy. If compatibility is
  required, migration must be explicit and tested.
- `audit_only` should not remain as a mode name for enforced decisions.
- This sprint is separate from the MITM transport cutover; it is policy
  productization and settings-system hardening.
- `rewrite` must redact payload values in telemetry by default. Future
  broker-returned values must never be written to `mcp_calls`,
  `net_events`, `dns_events`, process logs, or settings export output.
- DNS is already parsed and recorded. The required cleanup is exposing
  DNS block/ask/rewrite in the same typed TOML/settings model and
  proving the VM/session-db path.
- Parallel T5 recon found a likely builtin HTTP policy bug: the builtin
  server reads `CAPSEM_DOMAIN_ALLOW/BLOCK`, but process startup appears
  to pass only session path and DB path; live reload also appears not to
  update the already-running builtin subprocess. Treat this as a bug to
  prove with a failing test, not a design footnote.
- T5 closed the builtin HTTP startup-policy half of that bug: process
  startup now passes merged domain allow/block lists to the builtin server,
  and VM E2E proves configured denial records both MCP and net telemetry
  without upstream side effects. Live reload for an already-running
  builtin subprocess remains a separate follow-up unless the builtin is
  respawned or taught to receive policy updates.
- Parallel T5 recon also flagged hardcoded builtin HTTP telemetry port
  `443`, possible missing `net_events` on failed attempts, stale
  `dnsmasq` assumptions, and vacuous net-event assertions. T0 should turn
  those into concrete red tests.
- External MCP tool calls are inspected at the MCP boundary. Downstream
  host-side network performed inside external MCP server processes is a
  separate boundary and does not currently show up as guest MITM
  `net_events`; policy-v2 should document and test that distinction.
- Policy Hook Spec0 should default to OpenAPI 3.1 because third-party
  plugin authors can generate receiving servers from it. If implementation
  chooses another spec format, it must be equally server-generator
  friendly and checked into the repo as an exported compatibility
  artifact.
- Hook decisions must use the same normalized subject model as local
  policy. Remote hooks are an extension point, not a second policy
  language.
- Existing UI settings such as allow domains, block domains, per-tool
  permissions, and header stripping must compile into named rules in the
  same policy engine. Tests should assert the generated rules, not just
  the UI-facing input.
- Docs site updates should land before calling the policy surface usable.
  Stale docs for MCP gateway, MITM ports, session telemetry, settings
  schema, just recipes, or the UI can cause users to configure the wrong
  boundary even when the code is correct.
- Implementation found and fixed a spec/example bug in the strawman HTTP
  rewrite rule: TOML literal strings do not consume backslash escapes, so
  regex examples must use `github\.com`, not `github\\.com`, when the
  intent is to escape the dot in the regex.
- Adversarial rewrite hardening found and fixed parser bugs: TOML policy
  maps accepted callback/type mismatches, unknown `policy.<type>` tables
  were silently ignored, quoted regex rewrite targets accepted trailing
  garbage after the closing quote, and HTTP header strip names were not
  normalized or validated.
- T4b browser verification found and fixed two live settings UI crashes:
  generated Policy V2 rules now tolerate omitted metadata arrays from the
  service response and deduplicate canonical generated rule keys before
  Svelte keyed rows render.
- T4b verification completed on 2026-05-08 with
  `pnpm -C frontend test -- settings-model settings-export api settings-store`,
  `pnpm -C frontend run check`, `pnpm -C docs run build`, `just run-service`,
  and an in-app browser pass against `http://127.0.0.1:5173/`.
- DNS Policy V2 enforcement completed on 2026-05-08. Discovery: DNS was
  still using only legacy `NetworkPolicy` block/redirect rules and did not
  share the Policy V2 reload handle used by framed MCP/HTTP, so configured
  `policy.dns.*` rules could parse but never affect live DNS. Fixed by
  wiring the shared Policy V2 handle into `DnsHandler` and
  `capsem-process`.
- Bug found during DNS allow telemetry hardening: a matched `dns.query`
  allow rule forwarded upstream but initially would not have populated
  policy fields on the eventual allowed/error `dns_events` row. Fixed by
  carrying the matched allow decision through cache/upstream completion and
  added `policy_v2_dns_allow_forwards_upstream_and_records_policy_fields`.
- HTTP response Policy V2 enforcement completed on 2026-05-08. Discovery:
  `RawResponseHead` was dispatched as observe-only and
  `PolicyV2HttpHook` did not subscribe to it, so configured
  `policy.http.*` `http.response` rules parsed but could not strip, rewrite,
  block, or fail closed before guest delivery. Fixed by evaluating
  `http.response` rules against request + response subjects, honoring stop
  outcomes in `handle_request`, and proving guest/no-leak/`net_events`
  behavior with localhost proxy-path tests.
- Model request Policy V2 enforcement completed on 2026-05-08 for
  allow/block/ask. Discovery: `policy.model.*` rules parsed and could be
  evaluated in unit tests, but the MITM request path never consulted them
  before provider dispatch. Fixed by evaluating `model.request` rules after
  body capture and before upstream dial; block/ask deny without dispatch,
  allow carries policy fields through `net_events`, and request rewrite
  currently fails closed without dispatch or body-preview leakage.
- Model request E2E hardening on 2026-05-08 found a verification gap:
  the Python VM E2E helper uses `target/debug/*` binaries directly, so a
  stale `capsem-process` binary produced a false failure where the saved
  policy existed in `user.toml` but the guest request reached upstream and
  leaked the body preview. Rebuilt debug binaries, reran the VM E2E, and
  added settings-response assertions plus service settings tests for
  model-policy saves and callback/type mismatch rejection.
- Clippy hardening on 2026-05-08 found a production redundant closure in
  setup host-config detection and std `MutexGuard` usage held across
  async settings endpoint tests. Fixed both and reran clippy with
  warnings as errors.
- Verification also found a MITM integration test fixture bug: the fake
  upstream request reader capped total head+body reads at 16 KiB and the
  tests ignored upstream task panics. Fixed the helper to drain by
  `Content-Length` and unwrap all fake-upstream joins so fixture panics are
  real test failures.
- Full docs consistency pass on 2026-05-08 removed active-product drift:
  DNS docs now describe `capsem-dns-proxy` over vsock:5007 instead of the
  old dnsmasq sentinel path, service/hypervisor docs list audit and DNS
  vsock ports, MCP docs match the 26 host tools, session telemetry includes
  exec/audit events and typed policy fields, and benchmark docs match the
  latest local artifacts. Verified with `pnpm -C docs run build` and an
  internal absolute-link check across docs pages.
- MCP T5 E2E expansion on 2026-05-09 added real-VM coverage for
  argument-value ask, request rewrite, external stdio request block, and
  external stdio response block. The new E2E tests passed on the first
  run, so no product bug was found in this slice; the bug was missing
  black-box coverage. Verified with
  `cargo test -p capsem-core net::mitm_proxy::mcp_frame --lib -- --nocapture`
  and `uv run python -m pytest tests/capsem-e2e/test_framed_mcp_mitm.py -q -s`
  (14 passed).
- HTTP/DNS/model T5 expansion on 2026-05-09 added real-VM coverage for
  configured HTTP method/path/query/header block, HTTP request and response
  header strip no-leak behavior, configured DNS block/rewrite, model
  request ask/rewrite fail-closed no-leak behavior, builtin HTTP policy
  environment propagation, and model tool-response block/rewrite before
  provider dispatch. Adversarial model unit coverage found and fixed a
  multi-tool-result bypass where an allow decision for one tool result
  could mask a secret-bearing sibling result. Verified with focused Rust
  policy suites and
  `uv run python -m pytest tests/capsem-e2e/test_framed_mcp_mitm.py tests/capsem-e2e/test_policy_v2_http_dns_mitm.py tests/capsem-e2e/test_model_policy_mitm.py -q -s`
  (20 passed).
- T6 regression found two MITM body handling bugs: decompression was keyed
  off gzip magic bytes, so binary non-HTTP payloads beginning with gzip
  magic could be decoded accidentally; and gzip-decoded responses kept the
  stale compressed `Content-Length`/size hint, risking guest-visible
  truncation. Fixed by honoring `Content-Encoding: gzip` instead of magic
  bytes for HTTP decompression and dropping stale body size hints after
  decompression.
- Remaining T5 runtime gaps are explicit: model response/tool-call policy
  now has response-body/stream enforcement before guest delivery on the
  host MITM fixture path and deterministic VM E2E proof through a local
  OpenAI-compatible upstream harness. Local and HTTPS hooks still need
  product policy-to-runtime dispatch plus VM E2E from `user.toml`/`corp.toml`
  once that config path is wired; hook Spec0 validation, audit rows,
  timeout/auth/body-cap handling, and fail-closed fallback behavior are
  covered by the runtime/unit contract suite.
- T7 verification on 2026-05-09 fixed one real infra bug and one real
  suspend/resume recovery bug before going green. Full smoke initially
  failed because three independent pytest invocations in `just smoke`
  shared `tests/leak-attribution.jsonl`; a completed pytest process could
  falsely accuse another still-running pytest process's session service of
  leaking. Fixed by namespacing leak attribution/report logs with
  `CAPSEM_TEST_RUN_ID` and setting unique IDs for the smoke parallel and
  serial pytest phases; focused regression coverage lives in
  `tests/test_leak_detection.py`.
- Suspend/resume investigation found Apple VZ warm restore still returning
  `VZErrorDomain Code=12 permission denied` for a checkpoint on this host,
  and also found two recovery bugs: `.vzsave` fsync had regressed out of
  `capsem-process`, and `capsem-service` cleared the suspended checkpoint
  registry fields before the resumed process was actually ready. Fixed by
  fsyncing the checkpoint before process exit, clearing registry state only
  after readiness, and archiving a failed warm checkpoint before cold-booting
  the persistent session so workspace/overlay state remains recoverable.
  Residual gap: this is a recovery fallback, not proof that Apple VZ warm
  memory restore works on this host.
- T7 verification results: `just smoke` passed in 227s; in that run
  `capsem-doctor --fast` reported 307 passed / 4 skipped, the doctor MCP
  subset reported 94 passed / 2 skipped / 216 deselected, the integration
  session DB audit reported 40 passed / 0 failed / 3 warnings (Gemini key
  absent), the smoke parallel pytest phase reported gateway 88 passed,
  MCP 62 passed / 50 skipped / 18 deselected, service+CLI 139 passed /
  5 skipped, and the serial suspend/resume phase reported MCP state
  12 passed plus service suspend/resume 7 passed. Extra adversarial stress
  run `CAPSEM_STRESS=1 tests/capsem-mcp/test_stress_suspend_resume.py`
  passed 50/50 in 139.37s.
- Wrap-up pass on 2026-05-10 closed the explicit release-prep debt that was
  still blocking the sprint: Policy Hook Spec0 now has strict Rust wire
  types, a checked-in OpenAPI artifact, service export at
  `/policy-hook/spec`, strict endpoint config, hardened HTTPS/auth/body-cap/
  schema-version fail-closed runtime behavior, and `policy_hook_events`
  session DB audit rows with endpoint id, spec version/hash, decision id,
  callback, latency, error, fallback, trace id, and session id.
- The same wrap-up added a deterministic VM E2E fixture for
  OpenAI-compatible model response/tool-call policy. The test routes
  `https://api.openai.com` through the MITM to a local fixture upstream and
  proves response block/rewrite plus provider-emitted tool-call block/rewrite
  before guest delivery with no original secret in guest output or
  `net_events` previews.
- Scoped Policy V2 microbenchmarks now live in
  `crates/capsem-core/benches/policy_v2.rs` with a captured reference in
  `benchmarks/policy-v2/README.md`. Short Criterion sample:
  HTTP request match 1.61-1.76 us, DNS query match 960-967 ns, model
  response match 1.32-1.37 us, model tool-call match 2.11-2.12 us,
  hook-decision match 1.51-1.52 us, hook response decode 330-335 ns.
- Final release gate passed on 2026-05-10:
  `env UV_CACHE_DIR=/private/tmp/capsem-uv-cache just test`. Key results:
  frontend check/build passed; cargo audit found no known vulnerabilities
  with 14 allowed warnings; clippy/workspace warnings-as-errors passed;
  Rust coverage passed; Python main matrix reported 1307 passed / 69
  skipped; build-chain tests reported 21 passed; injection tests reported
  5 passed; integration/doctor subset reported 94 passed / 2 skipped /
  216 deselected; session DB audit reported 40 passed / 0 failed; Docker
  install E2E reported 30 passed / 34 skipped; cross-compile and benchmark
  gates completed.
- Release prep stamped `1.0.1778378133`, moved the changelog body under
  that release heading, regenerated `LATEST_RELEASE.md`, and verified the
  docs release page with `cd docs && pnpm run build`.
- Remaining named debt after this wrap-up is limited to productizing hook
  invocation from user/corp policy config and adding local/HTTPS hook VM E2E
  once that policy-to-runtime dispatch is wired.
