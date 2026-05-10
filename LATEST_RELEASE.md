version: 1.0.1778378133
---
### Added (policy rules)
- Added the MCP policy sprint plan and tracker to productize MCP
  rules as typed `allow`, `ask`, and `block` decisions across TOML,
  settings, MITM enforcement, telemetry, and VM E2E tests.
- Expanded policy planning beyond MCP to cover HTTP and DNS with the
  same typed decision model, including capture-aware `rewrite`, HTTP
  method/URL path/query/header rules, header stripping, DNS rewrite rules,
  credential-broker-safe redaction expectations, and explicit E2E/session
  proof for `mcp_calls`, `net_events`, and `dns_events`.
- Expanded policy planning again to include model request/response,
  model tool-call/tool-response policy, and Policy Hook Spec0: an
  OpenAPI 3.1 export generated from runtime wire types so third-party
  HTTPS hook servers can receive normalized policy requests and return
  typed allow/ask/block/rewrite decisions.
- Clarified the policy rule shape as named
  `policy.<type>.<rule_name>` TOML tables with `on`, CEL `if`,
  `decision`, `priority`, and capture-aware
  `rewrite_target`/`rewrite_value` fields; simple UI allow/block/header
  controls must compile into the same policy rule IR.
- Added the first policy settings slice: settings files can now parse,
  preserve, return, and save priority-bearing named policy rules through
  the `/settings` API so frontend policy editors can post rule objects.
- Hardened policy config validation with adversarial rewrite tests:
  bogus rewrite shapes, malformed regex targets, callback/table
  mismatches, invalid rule names, invalid policy key saves, header-strip
  normalization, and atomic rejection now fail closed before settings are
  written.
- Added strict policy condition validation for the documented
  CEL-compatible subset: conjunctions, comparisons, `has(...)`, string
  helper methods, regex `matches(...)`, and per-callback subject fields
  are checked before TOML or `/settings` policy saves can persist.
- Added the first policy rule evaluator over normalized subjects, with
  priority/name-ordered rule selection for MCP argument, HTTP path, and
  model response conditions.
- Wired merged policy rules into the framed MITM MCP endpoint: named
  MCP request `block` rules now stop dispatch and record `policy.mcp.*`
  in `mcp_calls`, while `ask` rules fail closed without aggregator
  dispatch and record `policy_action=ask`.
- Added framed MITM MCP response enforcement for `mcp.response`
  block rules: secret-bearing tool results are replaced with policy
  errors before reaching the guest and the original result is omitted from
  `mcp_calls.response_preview`.
- Added `mcp.response` rewrite enforcement for framed MITM MCP:
  regex/capture rewrite targets mutate matched response text before it
  reaches the guest and telemetry records only the rewritten payload.
- Added `mcp.request` rewrite enforcement for framed MITM MCP:
  argument regex rewrites mutate dispatch payloads before the aggregator
  sees them, request telemetry records only redacted arguments, and
  rewrite-target errors fail closed without leaking original arguments to
  `session.db`.
- Added the first HTTP policy enforcement path in the MITM hook
  pipeline: named `http.request` block and ask rules stop before upstream
  dispatch, rewrite rules can mutate request URLs and strip request
  headers before telemetry/upstream construction, and `net_events` now
  carries typed policy mode/action/rule/reason fields.
- Added HTTP response policy enforcement in the MITM hook pipeline:
  named `http.response` rewrite rules can strip response headers and
  rewrite response header/status targets before guest delivery and
  telemetry capture, while unsupported response rewrite targets fail
  closed without leaking upstream response headers or bodies.
- Added DNS query policy enforcement: named `dns.query` allow rules now
  dispatch with audit fields, block and ask rules fail closed before
  upstream resolution, rewrite rules synthesize configured A/AAAA answers
  without touching upstream DNS, live policy reload is checked before
  cached answers, and `dns_events` now carries typed policy
  mode/action/rule/reason fields.
- Added model request policy enforcement before provider dispatch:
  named `model.request` allow rules dispatch with audit fields, block
  and ask rules fail closed before upstream connection, unsupported
  request rewrite rules fail closed without dispatch, and `net_events`
  records policy fields plus byte counts without retaining denied request
  bodies.
- Added adversarial and VM E2E coverage for model request policy:
  truncated JSON matching, invalid runtime conditions, non-LLM path
  bypass, `/settings` model-policy saves, callback/type mismatch
  rejection, and a real guest OpenAI-shaped HTTPS request blocked from
  `user.toml` with `session.db` no-leak assertions.
- Added configured MCP Policy V2 VM E2E coverage: a saved
  `policy.mcp.*` argument-name block now goes through `/settings`,
  `/reload-config`, the real guest framed MCP relay, and `session.db`
  assertions for decision, rule, reason, process attribution, and
  redacted previews.
- Added more configured MCP Policy V2 VM E2E coverage for T5:
  argument-value `ask`, request-argument `rewrite`, external stdio MCP
  request `block` with no dispatch, and external MCP return-value `block`
  with no response-preview leak are now proven through `/settings`, the
  real guest framed MCP relay, and `session.db`.
- Added a policy product-surface subsprint covering docs site updates,
  session database references, just recipe documentation, and settings UI
  work so the framed MITM MCP and policy user-facing surfaces stay in sync
  with the implementation.
- Added the policy product surface: a docs reference page, refreshed
  framed-MITM MCP/settings/session/just recipe docs, settings import/export
  of named policy rules, and a settings UI panel that edits, deletes, and
  stages generated `policy.<type>.<rule_name>` rules.
- Added Policy V2 T5 VM proof for HTTP, DNS, and model traffic: real guest
  sessions now cover configured HTTP method/path/query/header blocks,
  HTTP request/response header stripping with no-leak `net_events`,
  configured DNS block/rewrite with `dns_events`, model request ask/rewrite
  fail-closed no-leak behavior, and model tool-response block/rewrite
  telemetry redaction.
- Added model `tool_response` Policy V2 enforcement before provider
  dispatch: OpenAI-shaped tool-result messages can now be blocked or
  rewritten before local tool output reaches the model provider, with
  rewritten request bodies updating `Content-Length` and redacted
  `net_events`, `model_calls`, and `tool_responses` previews.
- Added model response and provider-emitted model tool-call Policy V2
  enforcement before guest delivery: OpenAI-shaped responses can now be
  blocked, asked, or rewritten with no-leak `net_events`, redacted
  `model_calls.text_content`, and redacted nested `tool_calls` session
  rows on the host MITM fixture path.
- Added Policy Hook Spec0 as checked-in OpenAPI generated from Rust wire
  types, exposed it from `GET /policy-hook/spec`, and added a strict hook
  endpoint runtime with HTTPS/auth/body-cap/schema-version fail-closed
  handling plus `policy_hook_events` session DB audit rows.
- Added deterministic VM E2E coverage for model response block/rewrite and
  provider-emitted tool-call block/rewrite through a local OpenAI-shaped
  upstream fixture, with guest-visible no-leak assertions and `net_events`
  policy proof.
- Added scoped Policy V2 Criterion microbenchmarks for HTTP, DNS, model
  response, model tool-call, hook-decision matching, and Policy Hook response
  decoding, with sample results recorded under `benchmarks/policy-v2/`.

### Fixed (policy rules)
- Fixed model telemetry parsing for explicit/local OpenAI-compatible
  provider paths by carrying the request's provider classification through
  the MITM chunk-hook metadata, so enforcement and SSE interpretation use
  the same provider decision instead of relying only on the network domain.
- Fixed builtin MCP HTTP policy propagation: `capsem-process` now passes
  merged domain allow/block lists to `capsem-mcp-builtin`, so configured
  builtin HTTP denials fail at the policy boundary, avoid upstream
  resolution, and write both `mcp_calls` and `net_events`.
- Fixed a model tool-response policy bypass found during adversarial unit
  testing: an allow rule matching one tool result can no longer let a
  separate secret-bearing tool result in the same provider request bypass a
  block rule.
- Fixed a policy evaluator safety bug found during adversarial testing:
  a missing field no longer satisfies a negative comparison such as
  `provider != "local"`.
- Fixed policy settings UI crashes found during browser verification by
  tolerating omitted live metadata arrays and deduplicating generated rule
  keys before rendering Svelte keyed rows.
- Fixed MITM integration fixture discipline: fake upstreams now drain the
  full `Content-Length` body and upstream task panics fail the test instead
  of only printing noisy background panics.
- Fixed warnings-as-errors issues found during policy verification by
  removing a redundant setup detection closure and switching settings
  endpoint env-serialization tests to an async mutex.
- Fixed a Policy V2 MCP telemetry leak: pre-dispatch `policy.mcp.*`
  block/ask denials now redact original request arguments before writing
  `mcp_calls.request_preview`.
- Fixed MITM body handling regressions found during T6 verification:
  HTTP decompression now honors `Content-Encoding: gzip` instead of raw
  gzip magic bytes, and decoded responses drop stale compressed
  `Content-Length`/size hints so guest delivery cannot truncate.
- Fixed suspend/resume recovery hardening found during T7: `.vzsave`
  checkpoints are fsynced before process exit, service registry suspended
  state is cleared only after resume readiness, and failed Apple VZ warm
  checkpoints are archived before a persistent cold-boot fallback recovers
  workspace/overlay state.
- Fixed the smoke leak-detector false positive where concurrent pytest
  invocations shared one leak-attribution file and could report another
  still-running pytest process's service fixture as a leak; `just smoke`
  now gives each pytest phase a distinct leak-log namespace.
- Fixed clean ephemeral session shutdown cleanup so non-persistent session
  directories are removed on expected process exit while unexpected process
  deaths remain available for postmortem inspection.
- Fixed local release gate recipes so `just test` can complete on macOS:
  optional Tauri signing arguments no longer trip Bash 3.2 nounset in
  `just cross-compile`, and `just test-install` recreates the Docker host
  builder base image if cross-compile cleanup pruned it.

### Fixed (mitm-mcp-unification T4 coverage hardening)
- Preserved all JSON-RPC request id shapes in framed MCP telemetry:
  string, numeric, and null ids now populate `mcp_calls.request_id`
  instead of only unsigned numeric ids.
- Corrected the sprint tracker T5 scope: configured external MCP tool
  calls are inspected at the framed MITM MCP boundary; any remaining
  downstream host-side egress concern must be named separately.
- Expanded framed MCP coverage across Rust, VM E2E, and in-VM doctor
  diagnostics for malformed JSON recovery, oversized guest requests,
  corrupted-frame recovery after an established MCP frame stream,
  notification interleaving, non-`tools/call` timeout telemetry, and
  persistent stop/resume reconnect.
- Updated the T4 coverage review notes and benchmark log with the bugs
  found during review, `session.db` sanity evidence, and fresh
  `mcp-load` numbers after the hardening pass.

### Changed (mitm-mcp-unification T4 cutover)
- **Guest MCP now uses framed MITM transport by default.**
  `/run/capsem-mcp-server` relays stdio JSON-RPC over bounded MCP
  frames on `vsock:5002`, carries per-frame process attribution, emits
  explicit disconnect errors for in-flight JSON-RPC requests, and avoids
  automatic replay of non-idempotent `tools/call` requests after a
  transport drop.
- Removed the legacy guest MCP router on `vsock:5003`: deleted
  `capsem-core/src/mcp/gateway.rs`, removed `VSOCK_PORT_MCP_GATEWAY`,
  removed 5003 vsock dispatch/classification, and updated guest
  diagnostics, docs, skills, and benchmarks to describe the MITM MCP
  endpoint as the canonical guest MCP path.
- Added the `mitm.mcp_disconnects_total` metric and VM E2E coverage
  proving the default guest relay writes populated `mcp_calls` rows,
  live policy reload affects an existing connection, concurrent parent
  processes preserve `mcp_calls.process_name`, tool timeouts record
  terminal errors, external stdio MCP tools still dispatch, and legacy
  `vsock:5003` refuses guest connections.
- Fixed `scripts/check_session.py` so `just inspect-session <id>` works
  with current run-session directories and older system Python versions.

### Changed (development process)
- Strengthened the Capsem sprint/testing skills to require an explicit
  functional-slice proof matrix for non-trivial work: unit/contract,
  functional, adversarial, E2E/VM, telemetry, and performance evidence
  must be named in sprint trackers, with any missing coverage recorded
  as visible debt instead of implied by benchmarks or unit tests.
- Expanded the MCP development skill with the framed MITM MCP hardening
  matrix: parser/interpreter adversarial cases, dispatch coverage,
  policy rule enforcement, telemetry assertions, VM E2E checks, and the
  aggregator DB-free boundary.

### Fixed (mitm-mcp-unification T3 hardening)
- **Framed MCP now consumes request stream ids before JSON parsing,**
  so a valid frame with invalid JSON cannot reuse the same stream id for
  a later request. Parser-level failures still return JSON-RPC parse
  errors, complete the stream id, and avoid writing misleading
  `mcp_calls` rows.
- **`capsem-service` now forwards framed MCP runtime knobs to
  `capsem-process`.** The child-process env allowlist includes
  `CAPSEM_HOME`, `CAPSEM_MCP_DEFAULT_TIMEOUT_SECS`,
  `CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS`, and
  `CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS`, keeping service/process
  config roots aligned and allowing E2E tests to exercise real timeout
  limits.
- Added framed MCP VM E2E coverage for builtin `tools/call`, configured
  external stdio tools, live policy reload on an already-open connection,
  concurrent process attribution, slow-tool timeout telemetry, and
  `session.db` policy/preview assertions.
- Added a static regression guard proving the low-privilege
  `capsem-mcp-aggregator` crate remains free of session DB dependencies
  and audit writes.

### Added (mitm-mcp-unification T3 MITM MCP endpoint)
- **Framed MCP now dispatches through a real MITM-owned endpoint
  instead of borrowing the legacy MCP gateway handler.** `MitmProxyConfig`
  owns `McpEndpointState`; the framed path routes initialize,
  tool/resource/prompt list, tool calls, resource reads, and prompt gets
  through the low-privilege `AggregatorClient`; and the MITM frame layer
  writes `mcp_calls` telemetry directly through the session `DbWriter`.
  The aggregator remains DB-free.
- Added method-aware framed MCP timeouts: non-`tools/call` methods default
  to 60s, `tools/call` defaults to 300s, tool-call catalog timeout
  overrides are clamped by a 300s ceiling, and timeout failures return
  JSON-RPC errors while recording terminal `mcp_calls` rows with
  `decision=error`.

### Added (mitm-mcp-unification T2 decision provider)
- **Framed MCP calls now record audit-only policy decisions in
  `mcp_calls`.** The MITM MCP frame path builds an owned decision
  request from the interpreter summary, preserving process name,
  method classification, request preview, and BLAKE3 request hash for
  future remote corp forwarding. The local v1 provider emits only
  `allow` or `deny` actions, maps warning policy to `allow`, evaluates
  tool calls at per-tool granularity, evaluates resource/prompt reads
  at server granularity, and stores `policy_mode`, `policy_action`,
  `policy_rule`, and `policy_reason` through the logger schema,
  writer, reader, and session triage output.
- Added the T2 policy test matrix for exact tool name, exact MCP
  resource URI, prompt/tool argument name, prompt/tool argument value,
  nested return value, deny-over-allow precedence, live policy mutation,
  response-time decisions, actual framed request blocks, and sanitized
  framed response blocks. The framed tests now drive the MCP frame
  transport into a real `session.db` and assert both telemetry previews
  and policy fields on the resulting `mcp_calls` rows.
- Framed MCP deny decisions now enforce as well as log: request-rule
  denies short-circuit before aggregator dispatch, and return-value
  denies replace the original MCP result with a policy error before it
  reaches the guest.

### Added (mitm-mcp-unification T1 parser/interpreter)
- **Framed MCP over `vsock:5002` now has a bounded parser and
  interpreter instead of relying on the T0 spike shape.** The MITM
  MCP frame path validates frame length/flags, enforces monotonic
  nonzero request `stream_id`s while reserving `stream_id=0` for
  notifications, bounds JSON-RPC payload parsing before deserialize,
  classifies MCP request/notification methods, extracts server/tool/
  resource/prompt names for the known MCP call families, emits method
  metrics, and recovers from corrupt-but-bounded frames by returning
  JSON-RPC invalid-request errors before continuing the stream.

### Changed (exec timeout contract)
- **`capsem exec` and `capsem run` no longer impose a hidden default
  command timeout.** Omitting `--timeout` now waits for command
  completion, which matches long-running user jobs such as builds,
  installs, migrations, and `capsem-bench mcp-load`. Explicit
  `--timeout <seconds>` still applies a service-side deadline. The
  process-layer exec watchdog was removed; transport delivery remains
  covered by the control bridge's Ack/AckReply replay layers.

### Changed (rustfmt sweep)
- Ran a one-time workspace `cargo fmt` sweep while landing T1 so future
  sprint diffs start from the same formatter baseline.

### Added (mitm-mcp-unification T0 wire gate)
- **Framed MCP-over-MITM transport is now benchmark-gated for the
  MCP unification sprint.** Added a bounded `MC` frame envelope in
  `capsem-proto`, a MITM classifier branch for framed MCP on
  `vsock:5002`, and an explicit `CAPSEM_MCP_TRANSPORT=framed`
  mode in the guest MCP relay. The T0 spike still routes through
  the existing aggregator/policy/MCP telemetry path so the wire
  comparison stays fair. Fresh same-hardware `mcp-load` artifacts
  are recorded at
  `benchmarks/mcp-load/baseline-pre-mitm-unification.json` and
  `benchmarks/mcp-load/baseline-framed-mitm-unification-t0.json`.
  Framed selected: rps +8.6% / +4.8% / -6.4% / +5.4% and p99
  -31.9% / -23.9% / +7.8% / -31.0% at concurrency 1/10/50/200,
  with zero errors on both transports.

### Fixed (mcp/file_tools: truncate_path panic on non-ASCII paths -- AB-007)
- **`truncate_path` no longer panics on paths whose suffix
  byte offset lands inside a multibyte UTF-8 sequence.** The
  legacy implementation used `path.len()` (bytes) and
  `&path[path.len() - (max - 3)..]` (byte slice). For example,
  a path of 40 `日` chars + 1 ASCII char (121 bytes) with
  max = 33 panicked with `start byte index 91 is not a char
  boundary; it is inside '日'`. Both call sites
  (`render_changes` and the snapshot list renderer) walk
  user-supplied paths, so any non-ASCII path could crash
  snapshot rendering for the whole VM. The new implementation
  counts and slices by character, falling back to a
  no-ellipsis suffix for `max <= 3` so ill-typed callers
  cannot bring down the tool. Eight regression tests cover
  ASCII-under, ASCII-over, Unicode-under (keeps as-is even
  when byte length exceeds max), Unicode-boundary panic
  repro, Unicode-over (correct char count), empty path,
  `max == 3`, and `max == 0`.

### Fixed (security: deep-link JS injection -- AB-003)
- **`capsem-app::dispatch_deep_link` no longer interpolates
  `--connect` / `--action` values into JavaScript that runs in
  the desktop webview.** The previous code only escaped single
  quotes and embedded the values into a single-quoted JS
  literal that was passed to `window.eval`. A trailing
  backslash, a newline, or a payload like
  `x\'); alert(1); //` broke out of the string and ran as
  code -- in a webview that holds the gateway auth token, so
  effective full local capsem control. New helpers
  `build_deep_link_payload` (returns a `serde_json::Value`)
  and `build_deep_link_script` embed the payload via JSON
  serialization, which is a strict subset of valid JS object/
  string literals; every backslash, quote, control char, and
  high-bit code point is escaped by construction. Tests added
  cover plain values, single quote, backslash, newline, the
  injection-payload repro, and a JSON round-trip across a
  high-entropy input string.

### Fixed (mitm-redesign T3 closure -- production bug, dns-load reveal)
- **DNS cache returned the original query id for every cache hit.**
  The TTL-honoring answer cache (T3.f) stored wire-format response
  bytes verbatim, including the 16-bit DNS transaction id in
  bytes 0-1. Cache hits returned those bytes without rewriting the
  id, so subsequent queries to the same `(qname, qtype, qclass)`
  always echoed the FIRST query's id. Downstream resolvers (which
  match responses to outstanding queries by id, RFC 1035 sec 4.1.1)
  would discard the cached response as not-mine, causing 100%
  query failure once the cache warmed up. Surfaced by the
  `capsem-bench dns-load` in-VM run during T3 closure: the run
  reported ~99.999% errors, and an inline diagnostic showed the
  exact pattern -- 5 sequential queries with random ids all
  returned the same id (the first query's). Fix: `DnsAnswerCache::get`
  takes a new `query_id: u16` parameter and patches the response
  bytes' id field on every hit before returning. New regression
  tests `cache_hit_patches_query_id_into_response` (asserts the
  patch happens with two different ids on the same key) and
  `cache_hit_with_zero_query_id_zeroes_bytes` (defensive: id=0
  must overwrite, not skip the patch). Existing 18 cache tests
  updated to pass through the new arg. capsem-core lib at 1693
  tests now (+2 regression). Workspace clippy clean.

### Fixed (mcp: corp precedence -- AB-002)
- **Corp-defined MCP servers can no longer be shadowed by a
  same-name user manual entry.** The build pipeline in
  `crates/capsem-core/src/mcp/mod.rs::build_server_list_with_builtin`
  used a first-wins HashSet but processed entries in the order
  builtin → auto-detected → user → corp, so corp was last and
  was silently skipped on collision. A user typing the same
  name as a corp-injected server would win the URL, headers,
  and bearer token, contradicting the documented `corp > user
  > defaults` policy in `docs/architecture/settings.md` and
  the "corp_locked" model. Corp definitions are now processed
  first, so the first-wins rule enforces the documented trust
  order. Same-name user entries are skipped; unique-name user
  and auto-detected entries are unaffected. Tests added:
  `build_server_list_corp_shadows_user_on_same_name`,
  `build_server_list_unique_user_server_survives_with_corp_present`,
  `build_server_list_corp_enabled_override_on_user_server`.
  `docs/src/content/docs/architecture/mcp-aggregator.md`
  reordered to match the new processing order.

### Fixed (security: gateway CORS -- AB-001)
- **Gateway CORS now does an exact-host check on the Origin
  header instead of a string prefix match, closing a path that
  could leak the gateway auth token to attacker-controlled
  pages.** The previous predicate accepted any origin starting
  with `http://localhost`, `http://127.0.0.1`, `https://...`,
  or `tauri://`, so origins like `http://localhostevil.com`,
  `http://127.0.0.1.evil.example`, and `tauri://evil.example`
  passed CORS. Combined with `GET /token` being exempted from
  the auth middleware (it is gated only by loopback peer IP --
  which a victim's own browser satisfies), a malicious page
  could read `gateway.token` cross-origin and drive the local
  capsem service. The new
  `crates/capsem-gateway/src/cors.rs::is_allowed_origin` parses
  the Origin as a URI and accepts only exact matches for
  `http`/`https` to `localhost`, `127.0.0.1`, or `::1`, plus
  `tauri://localhost`; any path/userinfo/query/fragment, any
  unknown scheme, and any host suffix attack are rejected.
  22 unit tests cover the positive and negative matrix and the
  predicate is now shared between production and the
  integration test in `main.rs` so they cannot drift.

### Fixed (mitm-redesign T3 closure -- in-VM gate)
- **Host vsock listener registration was missing
  `VSOCK_PORT_DNS_PROXY` (5007) and `VSOCK_PORT_AUDIT` (5006).**
  In-VM smoke surfaced the DNS half: `capsem-dns-proxy` queries
  failed with "Connection reset by peer (os error 104)" because
  the host kernel had no listener for vsock port 5007 to accept
  on. `crates/capsem-core/src/vm/boot.rs::vsock_ports` now
  includes both 5006 and 5007 alongside the existing 5000-5005,
  so the Apple VZ + KVM hypervisor backends register listeners
  on every port `dispatch_aux_connection` knows how to handle.
  The audit case was a latent bug -- `audit_events` had been
  silently empty in every session since the audit feature
  landed -- now incidentally fixed alongside the DNS one.
- **Diagnostics: `test_dns_resolves_to_local` (test_sandbox.py)
  and `test_allowed_domain` still asserted the legacy
  `10.0.0.1` dnsmasq sentinel.** Updated to match the T3.4
  cutover: DNS now resolves to a real upstream IP via the
  capsem proxy (accepting either IPv4 or IPv6 first-token
  shape, since some upstreams return AAAA-only). The
  `test_allowed_domain` step-by-step diagnostic now uses the
  resolved hostname for TCP/TLS steps instead of hard-coding
  10.0.0.1. `test_dns_blocked_domain_returns_nxdomain` was
  policy-dependent (the user's `~/.capsem/user.toml` may
  override `api.openai.com.allow`); replaced with
  `test_dns_nxdomain_propagates_from_upstream` which uses an
  RFC 2606 `.invalid` TLD that no upstream can resolve --
  a clean policy-independent NXDOMAIN E2E test that pre-T3
  dnsmasq would have wrongly answered with 10.0.0.1.
- **In-VM E2E gate result.** With the boot.rs fix + diagnostic
  updates: `capsem-doctor -k 'dns or proxy_listening or
  iptables_redirect'` returns 14/14 PASS in a temp VM. The
  full DNS path is validated end-to-end: libc -> iptables nat
  53 -> 1053 -> capsem-dns-proxy -> vsock 5007 -> host hickory
  handler -> upstream forward (1.1.1.1) OR NXDOMAIN
  short-circuit -> answer back. `dns_events` rows populate with
  `trace_id`, source_proto, upstream_resolver_ms.
  `pgrep dnsmasq` returns nothing.

### Added (mitm-redesign T3 follow-up `f.proptest`)
- **proptest property-based tests for the DNS wire codec.** New
  `crates/capsem-core/src/net/parsers/dns_parser/proptests.rs`
  with 7 properties (256 random cases each by default) closing
  the loop alongside the cargo-fuzz targets:
  - `parse_query_round_trip`: build a query with arbitrary
    name + qtype + id, parse it back, assert id / qname / qtype
    / qclass / extra_questions match.
  - `build_nxdomain_preserves_question`: NXDOMAIN response built
    from an arbitrary query parses back to a question with the
    same id / qname / qtype / qclass.
  - `build_servfail_preserves_question`: same shape, ServFail
    rcode.
  - `build_redirect_preserves_question_for_a`: redirect response
    with N arbitrary IPv4 IPs lands all N as A records (no
    cross-family filter loss).
  - `build_redirect_filters_cross_family`: redirect with
    1 IPv4 + 1 IPv6 + an A query yields exactly 1 answer
    (the IPv4) -- the cross-family filter holds.
  - `parse_query_does_not_panic_on_arbitrary_bytes`: 0..2000
    arbitrary bytes never panic. Mirrors the cargo-fuzz target's
    safety contract so a regression surfaces in `cargo test`
    even without nightly + cargo-fuzz installed locally.
  - `build_nxdomain_does_not_panic_on_arbitrary_bytes`: same.
  Strategies: `dns_name_strategy()` produces 2-3 label
  syntactically-valid lowercase DNS names; `qtype_strategy()`
  picks from A/AAAA/TXT/MX/CNAME/SRV/CAA/NS/SOA/PTR/HTTPS/ANY.
  New dev-dep `proptest = "1"` (test-only, no production
  surface). capsem-core lib at 1691 tests now (was 1684).

### Added (mitm-redesign T3 follow-up `f.cache`)
- **TTL-honoring LRU answer cache for the DNS proxy.** New
  `crates/capsem-core/src/net/dns/cache.rs` shipping
  `DnsAnswerCache`: bounded LRU (default 1024 entries) keyed on
  `(qname, qtype, qclass)`, value is the wire-format answer bytes
  + `expires_at` derived from `min(answer_TTL, max_cache_ttl)`
  with `[60s, 300s]` clamp (DEFAULT_MAX_TTL_SECS / MIN_TTL_SECS).
  Lazy expiry: an expired entry is popped on the next lookup +
  counted as a miss. Cache **eligibility**: only `Decision::Allowed`
  responses with rcode=0 are inserted -- block + redirect
  re-evaluate every query (admin can change either at any moment),
  and SERVFAIL / NXDOMAIN from upstream are not persisted (avoids
  amplifying a transient upstream blip into 5 minutes of wrong
  answers). Cache **coherence**: `cache.get()` re-checks
  `is_fully_blocked` AND `find_dns_redirect` on every hit -- a
  domain that becomes blocked or redirected after we cached its
  answer is invalidated lazily on the next access (the entry is
  popped + counted as a miss). Three new metrics:
  `mitm.dns_cache_hits_total`, `mitm.dns_cache_misses_total`,
  `mitm.dns_cache_evictions_total`. New `lru = "0.18"` capsem-core
  dep (small pure-Rust crate). Wired into `DnsHandler` via the
  new `with_cache` constructor; `with_default_resolver` enables
  it by default with default config. `new` (no cache) constructor
  is preserved so existing tests can assert the upstream path
  always runs without cache-hit interference. 18 cache unit tests
  (insert/get round-trip, qtype/qclass key independence, capacity
  eviction with LRU order, TTL clamps to MIN/MAX bounds,
  garbage-input falls back to MIN, NoData answer falls back to
  MIN, min-across-records, clear, default constants pinned) + 8
  handler integration tests (cache hit short-circuits upstream
  via blackhole-after-warmup, policy-now-blocks invalidates
  lazily, policy-now-redirects invalidates lazily, block path
  still NXDOMAINs without consulting cache, cache_hits_total +
  cache_misses_total metrics fire, NXDOMAIN-from-upstream is not
  cached, with_default_resolver enables caching, new() leaves
  cache=None). capsem-core lib at 1684 tests now (was 1658).
  Workspace clippy clean.

### Added (mitm-redesign T3 follow-up `f.observability`)
- **DNS path metrics + structured tracing span.** Three new
  metric names registered alongside the existing MITM ones:
  `mitm.dns_queries_total{decision}` (allowed / denied /
  redirected / error), `mitm.dns_handle_duration_ms` (histogram,
  end-to-end), `mitm.dns_upstream_duration_ms` (histogram,
  upstream-forward path only -- absent on policy short-circuit),
  `mitm.dns_upstream_failures_total`. `DnsHandler::handle` is now
  wrapped in a `mitm.dns.query` info-span recording `qname`,
  `qtype`, `decision`, `rcode`, and `upstream_ms` on exit so a
  single `RUST_LOG=capsem::net::dns=debug` traces one query from
  parse to answer. The handler was refactored to a thin
  `handle()` (span + metric emission) wrapping `handle_inner()`
  (the decision tree) so every exit path goes through the same
  observability stamp -- no drift between block / redirect /
  forward / error branches. 5 new tests against
  `metrics_util::DebuggingRecorder` assert the right counter
  fires per decision label, the upstream histogram is absent on
  policy short-circuit but present on the forward path, and
  `dns_upstream_failures_total` increments on resolver error.
  `metrics_util` was already a dev-dep from the T1 sprint;
  facade-only emission means a no-op overhead in production
  until T5 wires the OTel exporter (same shape as the existing
  MITM metrics).

### Added (mitm-redesign T3 follow-up `e`)
- **`capsem-bench dns-load` harness.** New
  `guest/artifacts/capsem_bench/dns_load.py` mirrors the
  mitm-load shape: drives the DNS proxy at concurrency
  1/10/50/200, measures rps + p50/p95/p99/p999 latency, counts
  errors, and reports a per-level rcode distribution
  (`{"denied": 1234}` for the policy-block path,
  `{"allowed": 1234}` for the upstream-forward path) so the
  output dovetails with `dns_events.decision` for cross-checks.
  Defaults to `api.openai.com` (a fully-blocked domain in the
  dev policy) so every query hits the NXDOMAIN short-circuit
  path -- isolates the proxy's per-query cost from real upstream
  variance. Override via `CAPSEM_BENCH_DNS_QNAME` /
  `_QTYPE` / `_DURATION` / `_TIMEOUT`. The harness builds DNS
  wire-format queries by hand (no dns-python dep needed) so the
  guest's bundled python is enough; the encoder helpers
  (`_encode_qname`, `_build_query`, `_decode_rcode`,
  `_RCODE_DECISION` map) come with 7 host-side unit tests
  pinning the wire format + the rcode-to-Decision lock-step.
  Wired into `__main__.py` as the new `dns-load` mode (gated
  off `all` like mitm-load -- 40s of pure proxy stress would
  dominate a casual `capsem-bench all` run). Baseline JSON
  capture deferred to junior who owns the bench runner this
  session per the resume prompt.

### Added (mitm-redesign T3 follow-up `d`)
- **`DnsRedirect` policy rule -- admin-configured DNS overrides.**
  New `DnsRedirect { matcher, qtype, answers, ttl }` rule kind on
  `NetworkPolicy::dns_redirects` lets an admin override DNS
  resolution for a specific qname (and optionally a specific
  qtype). The DNS handler checks redirects AFTER `is_fully_blocked`
  (a blocked domain stays NXDOMAIN; redirect never weakens block)
  and BEFORE the upstream forward (no network round-trip when the
  answer is pinned locally). Use cases: redirect telemetry domains
  to a local trap, simulate an unreachable name with a deterministic
  IP for test runs, /etc/hosts-style overrides without modifying
  the guest. New `Decision::Redirected` variant on
  `capsem_logger::events::Decision` (string `"redirected"`) so
  `dns_events` rows surface override hits via
  `WHERE decision = 'redirected'`. Builder
  `dns_parser::build_redirect_response(query_bytes, &[IpAddr],
  ttl) -> Result<Vec<u8>>` synthesizes A/AAAA answer records
  filtered by qtype (cross-family IPs silently skipped, yielding
  the standard "name exists, no record of that type" NoError +
  zero-answers shape). 9 new policy unit tests + 11 new handler
  integration tests + 8 new builder unit tests covering exact /
  wildcard match, qtype filter, qtype=None matches anything,
  cross-family filtering, mixed-family yields only matching,
  block-overrides-redirect (block path runs first), TTL
  propagation, multiple IPs, empty-answers nodata, and
  no-match-falls-through-to-upstream. capsem-core lib at 1653
  tests now (was 1591). Workspace clippy clean.

### Added (mcp-concurrency T3 angle 2)
- **Pooled rmcp stdio peers for the local builtin MCP server.** The
  gateway can now spawn N independent stdio subprocesses for one
  MCP server and round-robin tool calls across them, removing
  rmcp 1.6's per-`Peer` mpsc → driver-task → stdin funnel as a
  singleton bottleneck. New fields on `McpServerDef`: `pool_size`
  (None / 0 / 1 = no pool, current behavior; >1 = N peers) and
  `pool_safe_tools` (allowlist of tool names safe to round-robin;
  others pin to `peers[0]` so per-process state stays consistent).
  HTTP servers ignore `pool_size` (HTTP/2 multiplexes natively).
  Builtin pool defaults to `min(available_parallelism, 4)` (matches
  the inflight-cap rule from `d88a714`). `CAPSEM_MCP_BUILTIN_POOL`
  overrides for tuning / debugging (set to 1 to force pre-pool
  behavior; clamped [1, 16]). `pool_safe_tools = [echo, fetch_http,
  grep_http, http_headers]`; snapshot tools stay pinned to
  `peers[0]` (their `AutoSnapshotScheduler` is per-process and N
  peers would diverge silently). Single-shot smoke at the dynamic
  default on M5 Max (pool=4): c=200 mcp-load p99 = 28.2 ms (vs
  sprint gate ≤ 35 ms), rps = 9591 (vs sprint gate ≥ 8000); c=10
  rps 3628 → 8794 (+143 %) — the rmcp stdio funnel disappearing
  at low contention.
- **`CAPSEM_BUILTIN_PEER_INDEX` env var** on `capsem-mcp-builtin`.
  Peer 0 keeps the original `mcp-builtin.lock` singleton; peers
  1..N use `mcp-builtin-{idx}.lock` so the `capsem_guard::install`
  per-session-dir guard doesn't make pool peers exit 0 with
  "another instance holds the lock".
- **`CAPSEM_MCP_BUILTIN_POOL` added to capsem-service env-allowlist**
  (both create and resume paths) so ops/bench can tune without
  rebuilding.

### Added (mitm-redesign T3 follow-up `c`)
- **cargo-fuzz harnesses for the DNS wire-format codec.** Four
  libFuzzer targets at `crates/capsem-core/fuzz/fuzz_targets/`:
  `parse_query`, `build_nxdomain`, `build_servfail`, and
  `round_trip` (asserts that if `parse_query` succeeds then
  `build_nxdomain` succeeds AND the response re-parses to the
  same qname/qtype/qclass -- catches divergence between the parse
  and rebuild paths that would let malformed queries escape
  NXDOMAIN gating). Each `corpus/<target>/` is pre-seeded with
  the T3.b `.bin` fixtures for fast structural coverage. The
  `fuzz/` directory is a standalone cargo workspace so libFuzzer's
  instrumentation flags don't leak into the parent workspace's
  normal builds. Plan acceptance from `T3-dns-proxy.md`: each
  target must survive `cargo +nightly fuzz run <target> --
  -max_total_time=60` clean (run path documented in
  `crates/capsem-core/fuzz/README.md` alongside the triage
  workflow for any crash artifact).

### Added (mitm-redesign T3 follow-up `b`)
- **dns_parser on-disk wire-format fixture corpora.** 13 raw DNS
  wire-byte `.bin` fixtures live at
  `crates/capsem-core/src/net/parsers/dns_parser/fixtures/`,
  covering simple A / AAAA / TXT / MX / CAA / HTTPS queries, the
  multi-question case, NXDomain + ServFail synthetic responses,
  truncated query, header-only, lying-qdcount, and the
  compression-self-loop adversarial case. Loaded via
  `include_bytes!()` at compile time so test runs don't hit the
  filesystem. 13 round-trip tests + an `all_fixtures_have_nonzero_length`
  pin (catches "include_bytes! pointed at an empty file" failure
  modes) wire them into the existing dns_parser test suite.
  Bootstrapped + regenerated by a new
  `crates/capsem-core/examples/dns_fixture_gen.rs` (separate
  compilation unit so the include_bytes! / regen chicken-and-egg
  doesn't bite). Plain English: a hickory-proto upgrade that
  changes the on-the-wire encoding of any of these query shapes
  lights up in the test diff before it bites a real query, and
  cargo-fuzz can corpus-seed from these exact bytes.

### Added (mitm-redesign T3 follow-up `a`)
- **dns_parser test breadth: record types + adversarial.** 32 new
  unit tests covering CNAME / NS / SOA / PTR / SRV / CAA / HTTPS /
  ANY / NULL / HINFO / AXFR / IXFR record types, all five DNS
  classes (IN / CH / HS / NONE / ANY), and risk-shape inputs:
  empty / single-byte / header-only / lying-qdcount / oversized
  qdcount=65535 / label compression self-loop / forward pointer
  past EOF / label > 63 bytes / NUL byte in label / truncated
  question section / max-label (63 bytes) accepted / NXDOMAIN
  preserves obscure qtype (CAA) and non-IN qclass / SERVFAIL
  rejects undecodable input. Total dns_parser tests: 46 (was 14).
  No production code changed -- pure additive coverage so a
  hickory-proto upgrade that quietly drops a record-type variant
  or breaks compression-bomb defense lights up before it bites a
  real query.

### Changed (mitm-redesign T3.4)
- **Guest cutover from dnsmasq to capsem-dns-proxy.** The
  in-guest dnsmasq fake (which resolved every name to the sentinel
  `10.0.0.1` so the MITM proxy could intercept connections) is
  gone. `capsem-init` now launches `capsem-dns-proxy` (T3.2) and
  installs iptables nat rules redirecting UDP/TCP port 53 to the
  proxy's `127.0.0.1:1053` listener. DNS queries now traverse the
  vsock envelope to the host's hickory-backed handler (T3.1)
  which applies the shared `NetworkPolicy` and forwards to a real
  upstream nameserver. `dig anthropic.com` from a guest returns a
  real answer; `dig api.openai.com` returns NXDOMAIN with the
  decision logged in `dns_events` (T3.3). The `dnsmasq` package
  is dropped from `guest/config/packages/apt.toml`, so the next
  rootfs rebuild leaves the binary out of the squashfs entirely.
  Diagnostics updated: `test_sandbox::test_dnsmasq_running` is
  replaced with `test_dns_proxy_running` plus a new
  `test_dnsmasq_not_running` that pins the cutover.
  `test_network` swaps the dnsmasq sentinel checks for two new
  acceptance tests: `test_dns_resolves_via_capsem_proxy` (a
  policy-allowed name resolves to a real IP, not the legacy
  10.0.0.1) and `test_dns_blocked_domain_returns_nxdomain` (the
  host policy short-circuits api.openai.com to NXDOMAIN before
  hitting the upstream resolver). Boot-stage marker added:
  `dns_proxy` between `net_proxy` and the rest of the boot
  sequence.

  End-to-end VM validation + `mitm-load` regression check still
  pending: the dev `capsem` binary needs codesigning (handled by
  the `just` recipes) and the `~/.capsem/assets/` install needs
  a `just install` to pick up the rebuilt initrd. Both fall
  under the junior-dev-owned bench runner this session, so the
  final acceptance gate is staged but not yet executed -- code,
  cross-compile, initrd repack (validated end-to-end via the
  Docker `agent` recipe), workspace clippy, and full Rust test
  suite are all green.

### Added (mitm-redesign T3.3)
- **`dns_events` telemetry table + per-query event row + trace_id
  correlation.** New `dns_events` schema in `capsem-logger`
  (timestamp, qname, qtype, qclass, rcode, decision, matched_rule,
  source_proto, process_name, upstream_resolver_ms, trace_id) with
  indexes on `(timestamp, qname, trace_id, decision)` for the
  inspect-session join. New `DnsEvent` event struct +
  `WriteOp::DnsEvent` + `insert_dns_event` writer; idempotent
  schema migration so existing DBs pick up the new table without a
  rebuild. New free function
  `capsem_core::net::dns::build_dns_event(result, source_proto,
  process_name, trace_id) -> DnsEvent` (pure, sqlite-free) +
  `serve_dns_session` in `capsem-process::vsock` calls it after
  every handler invocation and pushes the row through the shared
  `DbWriter` via `try_write` (matches the audit-event back-pressure
  pattern). `trace_id` is the ambient capsem trace id, so a single
  agent action joins across `dns_events` and `net_events` -- a
  `curl https://anthropic.com/` shows up as one `dns_events` row
  ("anthropic.com" allowed, qtype=A, rcode=0) plus one `net_events`
  row, both stamped with the same trace_id. 6 new
  capsem-core::net::dns::telemetry tests (allowed, denied,
  undecodable, decision strings round-trip with logger convention,
  source_proto optional, process_name passthrough) + 2 new
  capsem-logger writer tests (dns_event_insert_populates_row,
  dns_events_indexed_by_trace_id_for_join) + 3 new schema tests
  (create includes dns_events, migrate idempotent, indexes
  present). Bench gate still deferred to T3.4 (zero MITM hot-path
  code touched).

### Added (mitm-redesign T3.2)
- **vsock DNS envelope + guest `capsem-dns-proxy` listener.** New
  vsock port `VSOCK_PORT_DNS_PROXY = 5007` (`capsem-proto`)
  carries length-framed `rmp-serde` `DnsRequest` / `DnsResponse`
  envelopes between the guest agent and the host's `DnsHandler`.
  The host side (`capsem-process::vsock::serve_dns_session`)
  performs one envelope round-trip per vsock connection: read a
  `DnsRequest`, run `DnsHandler::handle` (T3.1), write a
  `DnsResponse`, close. The guest side is a new agent binary
  `capsem-dns-proxy` that listens on `127.0.0.1:1053` (UDP + TCP
  on the same port; iptables NAT will redirect 53 -> 1053 in
  T3.4) and opens a fresh vsock conn per query. The `DnsHandler`
  was retrofitted to take the same `Arc<RwLock<Arc<NetworkPolicy>>>`
  hot-swappable shape as `MitmProxyConfig` so an admin policy
  edit propagates to both protocols at once. The agent crate
  stays hickory-free -- it forwards raw bytes only. 9 new
  capsem-proto envelope tests (port-distinctness, request /
  response roundtrip, no-process-name path, compactness,
  garbage rejection, IPC-frame disjointness) + 5 new agent-bin
  unit tests pinning the listen port (1053), vsock port (5007),
  EDNS payload size, proto labels. Pre-T3.4 the `capsem-dns-proxy`
  binary is built and packaged but NOT launched -- T3.4 wires it
  into `capsem-init` alongside the iptables redirect for port 53
  and removes the dnsmasq invocation. Until then dnsmasq is still
  the guest's DNS server.

### Added (mitm-redesign T3.1)
- **Host-side DNS handler + UDP forwarder + wire-format parser.**
  New `capsem-core::net::dns` module (`server`, `resolver`) plus
  `capsem-core::net::parsers::dns_parser`. The `DnsHandler` is the
  bytes-in / bytes-out async processor that decodes a DNS query,
  consults the shared `NetworkPolicy::is_fully_blocked` rule, and
  either synthesizes an NXDOMAIN response (`Decision::Denied`),
  forwards the bytes verbatim to one of N upstream nameservers
  (default `1.1.1.1:53`, `8.8.8.8:53`; `Decision::Allowed`), or
  returns a synthetic SERVFAIL when every upstream is
  unreachable (`Decision::Error`). Read-only domains still
  resolve so the MITM proxy keeps its verb-level audit trail.
  Built on `hickory-proto = "0.26"` (workspace dep,
  `default-features = false, features = ["std"]`) -- the agent
  crate stays hickory-free; it'll forward raw bytes when T3.2
  wires the vsock envelope. 14 parser unit tests + 10 handler
  end-to-end tests against a fake `127.0.0.1:0` UDP upstream.
  Not yet wired into anything; T3.2 brings the vsock bridge,
  T3.3 the `dns_events` schema + telemetry hook, T3.4 cuts the
  guest image over from dnsmasq to iptables redirect.

### Performance (mcp-concurrency)
- **MCP gateway in-flight cap now scales with host CPU.** Default
  `DEFAULT_MCP_INFLIGHT` constant replaced with
  `default_inflight_cap()` = `available_parallelism * 4`. Anchors
  to the empirical sweet spot we measured on Apple M5 Max (18 cores,
  64 permits optimal) and tracks host shape automatically.
  `CAPSEM_MCP_INFLIGHT` continues to override the computed default.
  Sample mappings: 8-core -> 32, 16-core -> 64, 18-core (M5 Max) ->
  72, 32-core -> 128. Fallback when `available_parallelism()` itself
  fails: 8 cores -> 32 permits.
- **mcp-load throughput +62 % at concurrency 200; tail -24 %.**
  Three changes shipped together so the regression we measured when
  T1.2 + T1.3 were tried alone (p99@200: 40 → 358 ms, mitm rps -40 %)
  cannot land on its own again:
  1. **T1.2: aggregator subprocess pipelined.** `capsem-mcp-aggregator`
     no longer reads-then-handles-then-writes in one task; the reader
     spawns `handle_request` per incoming msgpack frame and a single
     writer task drains an `mpsc<AggregatorResponse>(256)` to stdout.
     `Shutdown` is acked synchronously on the reader path before the
     drain so we can't lose the ack to a stuck handler.
  2. **T1.3: hot manager lock eliminated.** `McpServerManager` now
     exposes `dispatch_call_tool` / `dispatch_read_resource` /
     `dispatch_get_prompt` that perform the lookup synchronously and
     return owned `impl Future + Send + 'static` futures. The
     aggregator wraps the manager in `std::sync::RwLock`; the sync
     read guard drops before the rmcp RPC is awaited, so concurrent
     dispatches never serialise on the manager.
  3. **T1.5: bounded concurrency at the gateway.** The MCP gateway in
     `capsem-core::mcp::gateway::serve_mcp_session` now acquires a
     `tokio::sync::Semaphore` permit BEFORE `tokio::spawn`-ing each
     handler. Default cap 64 (override via `CAPSEM_MCP_INFLIGHT`,
     forwarded through the capsem-service env-allowlist). Without
     this cap, T1.2 + T1.3 turn the MCP path into a CPU-starvation
     source for the rest of capsem-process (notably the MITM proxy on
     the same tokio runtime).
  Bench (Apple M5 Max, 2 vCPU bench VM, vs T1.1-only baseline at
  HEAD): mcp-load c=10 rps 3370 → 9160 (+172 %), c=50 rps 3081 →
  8633 (+180 %), c=200 rps 5224 → 8464 (+62 %), p99@200 57.1 →
  43.4 ms (-24 %), p999@200 67.9 → 53.4 ms (-21 %). mitm-load
  c=200 rps 2845 → 2968 (+4.3 %), p99 177 → 170 ms (-3.8 %) — both
  paths better, neither path regressed. Sprint MCP rps@200 gate
  (≥ 8000) cleared; the 35 ms p99@200 gate is still 8 ms over and
  is tracked as T3 in `sprints/mcp-concurrency/tracker.md`.

### Added (mitm-redesign)
- **T2 plain-HTTP coverage: adversarial / risk-shape tests.**
  Five more tests on top of the parsing-correctness ones, each
  hitting a real failure mode the proxy could plausibly meet in
  the wild:
    * `…body_larger_than_preview_cap_forwards_full_but_caps_preview`
      -- 16 KB request body (4x default `max_body_capture`).
      Asserts upstream receives the full body byte-for-byte,
      `NetEvent.bytes_sent == 16384`, but
      `NetEvent.request_body_preview` length <= 4096 and starts
      with the first 4 KB block (no later block leaked through
      the cap).
    * `…ipv6_host_header_does_not_silently_succeed` -- inbound
      `Host: [::1]:8080`. The host parser explicitly bails on
      `[`-prefixed hosts; the proxy must NOT 200 on the implicit
      ("", 80) fallback. Asserts response is 502 or 403, never
      200, with a non-Allowed `Decision`.
    * `…corrupted_gzip_response_doesnt_crash` -- upstream sends
      `Content-Encoding: gzip` plus a valid 10-byte gzip header
      followed by 61 bytes of garbage payload. With a 5s read
      deadline, the test asserts: (a) the proxy still emits
      exactly one `NetEvent` (= `on_response_end` fired = no
      panic on the response path), and (b) `bytes_received == 0`
      because `flate2::Decompress` yields nothing on a
      fully-corrupt deflate body. Future regressions that would
      leak pre-decode bytes here get caught.
    * `…truncated_upstream_response_doesnt_hang` -- upstream
      advertises `Content-Length: 1000` but writes only 33 bytes
      then closes. With a 5s read deadline. Asserts the proxy
      doesn't hang AND `bytes_received <= 33` AND `< 1000` (i.e.
      we record the actual bytes received, not the lying
      Content-Length).
    * `…zero_length_response_body_emits_netevent` -- 200 OK with
      `Content-Length: 0`. Asserts the chunk-hook chain still
      fires `on_response_end` on an empty body and emits exactly
      one `NetEvent` with `bytes_received == 0`.
  26 mitm_integration tests pass (17 plain-HTTP + 8 TLS + 1
  ignored throughput); 1542 lib tests pass; clippy clean.
- **T2 plain-HTTP coverage: verbs, query strings, header
  passthrough + secret redaction.** Four more integration tests
  on top of the structural ones, closing the parsing-correctness
  gap:
    * `mitm_proxy_plain_http_records_every_http_method` -- sends
      GET / HEAD / OPTIONS / POST / PUT / DELETE / PATCH on one
      keep-alive connection, asserts seven separate `NetEvent`
      rows each with the right `method` + `path` + `204` status.
      Validates verb parsing across both read-classified and
      write-classified methods.
    * `mitm_proxy_plain_http_records_query_string_with_parameters`
      -- `GET /search?q=hello%20world&page=2&filter=active&tag=a&tag=b`.
      Asserts the upstream sees the full request line verbatim
      AND `NetEvent.path == "/search"` (no `?`) +
      `NetEvent.query == "q=hello%20world&page=2&filter=active&tag=a&tag=b"`.
      Repeated keys, equals signs, and percent-encoded values
      preserved verbatim.
    * `mitm_proxy_plain_http_forwards_custom_headers_to_upstream`
      -- sends `User-Agent` (allowlisted), `X-Trace-Id`,
      `X-Custom-Flag`, `Authorization: Bearer ...` (custom).
      Asserts the upstream receives every header by name + value
      verbatim, and that `accept-encoding` was rewritten to `gzip`
      (we only forward what we can decompress).
    * `mitm_proxy_plain_http_telemetry_hashes_non_allowlisted_headers`
      -- security-focused. Sends real-shaped secrets:
      `Authorization: Bearer SUPER-SECRET-...`,
      `X-Api-Key: live_pk_DEADBEEF_...`,
      `Cookie: session=ROTATE_ME_...`. Asserts
      `NetEvent.request_headers` does NOT contain any of those
      verbatim values (each is replaced with `hash:<12-hex>`),
      while the header NAMES still appear and allowlisted
      `User-Agent` + `Host` appear verbatim. Locks down the
      "secrets in telemetry" surface.
  Also tightened the keep-alive test's response reader to drain
  head + body per request rather than relying on one-shot
  `tcp.read()` (was order-flaky on a busy CI). 21 mitm_integration
  tests pass; 1542 lib tests pass; clippy clean.
- **T2 plain-HTTP integration coverage extended.** Five new
  integration tests close the "ad-hoc verification" gap left by
  the earlier Ollama smoke. The new tests share a
  `spawn_fake_upstream(serve)` helper + a `read_http11_request`
  drainer so each test parameterizes the upstream's behavior:
    * `mitm_proxy_plain_http_post_forwards_body_and_records_bytes_sent`
      -- POST with body. Asserts the upstream sees the JSON body
      verbatim + `NetEvent.bytes_sent` covers the body.
    * `mitm_proxy_plain_http_chunked_streaming_response_aggregates_bytes`
      -- fake upstream sends `Transfer-Encoding: chunked` with 4
      data frames. Asserts the client sees every chunk +
      `NetEvent.bytes_received` equals the concatenated payload
      length (proves the ChunkDispatchBody runs the sync
      ChunkHook chain across multiple frames and the
      end-of-stream NetEvent emission fires).
    * `mitm_proxy_plain_http_keep_alive_emits_one_netevent_per_request`
      -- single client TCP connection, three back-to-back GETs to
      `/a`, `/b`, `/c`. Asserts three separate `NetEvent` rows,
      each with the right path/method/status/port/conn_type.
      Validates the per-connection cached upstream sender +
      keep-alive on the plain-HTTP branch.
    * `mitm_proxy_plain_http_preserves_host_header_to_upstream`
      -- captures the bytes the upstream observed. Asserts the
      inbound `Host: 127.0.0.1:<port>` header is forwarded
      verbatim. (TLS path rewrites Host from SNI; HTTP must not.)
    * `mitm_proxy_plain_http_unresolvable_upstream_emits_502_netevent`
      -- targets `nonexistent.invalid` (RFC 6761). Asserts 502
      back to the client + one `NetEvent` with `Decision::Error`,
      status 502, conn_type http-mitm, and the dial error in
      `matched_rule`. No silent drop on dial failure.
  17 mitm_integration tests pass (8 plain-HTTP + 8 TLS + 1
  ignored throughput); 1542 lib tests pass; clippy clean.
- **T2 verified end-to-end against real Ollama on
  `127.0.0.1:11434`.** From inside an air-gapped VM, `curl
  http://127.0.0.1:11434/api/tags` rides the full new pipeline:
  iptables redirect (port 11434 → 10080), agent listener on
  10080, vsock bridge, host first-byte sniff (T2.1), Host header
  parse + port allowlist (T2.2), plain TCP upstream dial, 357-byte
  JSON response forwarded verbatim to the guest. NetEvent recorded
  with `port=11434, conn_type=http-mitm, decision=allowed,
  status=200`. As part of the verification,
  `DEFAULT_HTTP_UPSTREAM_PORTS` is bumped from `[80]` to
  `[80, 11434]` so the host policy default mirrors the iptables
  rules in `capsem-init` -- otherwise port 11434 traffic gets
  redirected to 10080, hits the host proxy, and is rejected by
  the policy gate, which is the wrong default for the canonical
  local-LLM workflow this protocol path was designed for. New
  ports get added by editing both lists in tandem until the
  policy_config plumb (deferred follow-up) lands.
- **T2 (agent-side): plain-HTTP listener + iptables redirects.**
  `capsem-net-proxy` now listens on `127.0.0.1:10080` in addition to
  the original `:10443`; a `run_listener(port)` helper drives the
  per-port accept loop, and both targets the same vsock port
  `VSOCK_PORT_SNI_PROXY` (5002) -- the host's first-byte sniff
  (T2.1) classifies on wire bytes, so the guest-side listener split
  is just an iptables-target convenience. `capsem-init` adds two
  `iptables -t nat -A OUTPUT -p tcp --dport <N> -j REDIRECT
  --to-port 10080` rules for `:80` (plain HTTP) and `:11434`
  (Ollama default); the post-launch readiness poll waits for both
  `:10443` and `:10080` before declaring the proxy ready. Three
  new in-VM diagnostics cover the wiring:
  `test_iptables_redirect_80_to_10080`,
  `test_iptables_redirect_11434_to_10080`, and
  `test_net_proxy_http_listening`. Three new agent unit tests pin
  the new constant + cross-port distinctness. Cross-compile
  (`aarch64-unknown-linux-musl`) clean. The configurable
  guest-side allowlist (read from `policy_config`) is deferred --
  the host-side `NetworkPolicy.http_upstream_ports` is the
  authoritative gate, and adding a config plumb to the guest-side
  iptables list is its own follow-up.
- **T2.3: Ollama-shaped end-to-end test for the plain-HTTP path.**
  `mitm_proxy_plain_http_ollama_shape_records_telemetry` spins a
  fake plain-HTTP upstream on `127.0.0.1:0`, configures the proxy
  with that OS-assigned port on its `http_upstream_ports` allowlist
  + `127.0.0.1` on the domain allowlist, sends `POST /api/generate`
  with the typical Ollama request shape (model + prompt JSON body),
  and asserts: (a) the upstream's response body is forwarded
  verbatim, (b) the resulting `NetEvent` records
  `method=POST`, `path=/api/generate`, `status=200`,
  `domain=127.0.0.1`, `port=<upstream_port>`,
  `conn_type=http-mitm`, `decision=Allowed`, with non-zero
  `bytes_sent` / `bytes_received`. Adds `make_proxy_config_full`
  helper to override the `http_upstream_ports` allowlist
  (existing tests stay on the default `[80]`). 12 mitm_integration
  tests pass.
- **T2.2 (host-side): plain HTTP serves through the same hyper
  pipeline as TLS.** When the first-byte sniff (T2.1) classifies a
  connection as `Protocol::Http`, the listener now skips rustls
  entirely and runs `hyper::server::conn::http1::Builder::new()
  .serve_connection(io, svc)` directly on the vsock stream
  (`ReplayReader` carries the buffered first bytes). Per-request
  domain + upstream port are parsed from the inbound `Host` header
  by `parse_http_host_target` (T2.2 helper in `mitm_proxy/util.rs`)
  and threaded through `handle_request` as a new `upstream_port:
  u16` parameter; the inbound `host` header is preserved (it's
  authoritative for plain HTTP), unlike the TLS path which still
  rewrites it from the SNI domain. The hyper service closure runs
  the same PolicyHook and ChunkHook chain as TLS, so domain
  policy, decompression, SSE parsing, AI interpreters and
  Telemetry all apply uniformly. Upstream dials branch on
  `protocol`: TLS does TCP+rustls+http1::handshake, HTTP does
  TCP+http1::handshake (no TLS step). Telemetry: every
  `TelemetryRequestContext` carries `port: u16` + `conn_type:
  &'static str` (`https-mitm` / `http-mitm`); `NetEvent` rows now
  reflect the actual upstream port and transport label so
  operators can split HTTPS vs plain-HTTP traffic in `session.db`.
  `MitmProxyConfig::handle_inner` is split into `serve_tls`,
  `serve_plain_http`, and a shared `serve_pipeline` helper that
  drives the hyper server over either an `IO: hyper::rt::Read +
  hyper::rt::Write`. New `NetworkPolicy::http_upstream_ports:
  Vec<u16>` (default `[80]`) gates plain-HTTP upstream ports
  before the dial -- a request whose `Host` header carries an
  allowlist-missing port is rejected with a 403 + Decision::Denied
  + `matched_rule = "http-port-not-allowlisted({port})"`. The TLS
  path is unaffected by the allowlist (always uses 443).
  Two new integration tests cover the path:
  `mitm_proxy_plain_http_denies_disallowed_host` (PolicyHook 403
  on a disallowed Host) and
  `mitm_proxy_plain_http_denies_port_not_in_allowlist` (port-gate
  403). 1539 lib tests + 11 mitm_integration tests pass; clippy
  clean. Agent-side multi-port listener and iptables rules ship
  separately so the in-VM test (T2.3) can drive them.
- **T2.1: first-byte protocol sniff (TLS vs plain HTTP) on the vsock
  listener.** New `mitm_proxy::protocol` module with `Protocol` enum
  (`Tls` / `Http` / `Unknown`) and `detect(&[u8]) -> Option<Protocol>`
  classifier. The `vsock:5002` accept path now peeks the first
  post-meta payload byte: `0x16` -> TLS (existing path, unchanged);
  uppercase ASCII (`0x41..=0x5A`, the HTTP method set) -> plain HTTP
  classified but routed to a "T2.2-pending" connection-level error
  (the actual hyper plain-HTTP server lands in T2.2); other bytes ->
  `Unknown` connection-level error. The `mitm.connections_total`
  counter, previously hard-coded to `protocol="tls"` on every accept,
  is now incremented post-sniff with the correct label so operators
  can distinguish TLS / HTTP / unknown traffic. `mitm.requests_total`
  + the upstream-error increments propagate the same label.
  `ConnMeta` carries a `protocol: Protocol` field set from the sniff;
  every hook reads it through `ctx.conn().protocol`. 8 unit tests in
  `protocol/tests.rs` cover the byte-level rules (record types
  `0x14`/`0x15`/`0x17` rejected; lowercase methods rejected; high-bit
  junk rejected) plus 2 integration tests in `mitm_integration.rs`
  asserting the plain-HTTP and unknown-byte paths each emit the
  right `NetEvent`.

### Changed (mitm-redesign)
- **T1 closes -- legacy async body chain deleted; sync ChunkHook
  pipeline owns the response path end-to-end.** Slice 9 cleanup.
  Removes `mitm_proxy/telemetry.rs` (`TelemetryEmitter` +
  `TelemetryBody`, ~390 lines), `ai_traffic/ai_body.rs`
  (`AiResponseBody`, ~155 lines), `body::DecompressBody` +
  `body::BodyStream` + `body::RespStatsKind` (one
  `async_compression::tokio::bufread::GzipDecoder` adapter, one
  `tokio_util::io::StreamReader`, one `Body→Stream` shim). The
  inline `if is_gzip { DecompressBody::new(...) }` block in
  `handle_request` is gone -- the inline `if is_gzip` now only
  strips Content-Encoding / Content-Length headers (a few field
  accesses on the parts struct, kept inline because moving it to
  an async hook would re-introduce the same plumbing the slice
  removed). All four ChunkHooks are pure sync: `DecompressionHook`
  (`flate2::Decompress::new(false)`), `SseParserHook`, three
  `InterpreterHook`s, `TelemetryHook` -- per-chunk work runs inline
  from `poll_frame` with no `.await`, no channel hop, no async
  wrapper. `TelemetryHook` is wired into
  `make_production_pipeline` + reads its per-request context out
  of a `HookState` slot seeded by `handle_request` (new
  `HookState::set::<T>()` + `ChunkDispatchBody::seed::<T>()`
  builder). `MitmProxyConfig` is refactored to hold
  `Arc<TelemetryDeps> { db, pricing, trace_state }` instead of
  by-value `pricing` + `Mutex<TraceState>` -- the `Arc` breaks
  the would-be config↔pipeline↔hook reference cycle (the hook
  points at `TelemetryDeps`, not the surrounding config).
  `make_production_pipeline` signature now takes the
  `Arc<TelemetryDeps>`; `capsem-process` construction site +
  in-tree test fixtures + the integration test in
  `crates/capsem-core/tests/mitm_integration.rs` updated. The
  redundant `TelemetryEmitter` / `TelemetryBody` / `DecompressBody`
  / `emit_model_call` / `trace_chains_across_tool_use` test
  fixtures in `mitm_proxy/tests.rs` are deleted -- the same
  surfaces are covered by the per-hook tests in
  `telemetry_hook/tests.rs` (NetEvent + ModelCall builders),
  `decompression_hook/tests.rs` (gzip streaming), and the
  remaining integration tests still exercise the full path
  end-to-end via `handle_connection`.

  **Bench: SSE parser microbench at 478-488 MiB/s (up from 449-472
  MiB/s in the T0 pre-rewrite baseline; criterion reports
  "Performance has improved" with p<0.05).** Sync ChunkHooks are
  structurally faster than the async wrappers they replace.
  `capsem-bench mitm-load` against
  `benchmarks/mitm-load/baseline.json` is the integration gate;
  it requires a built VM image and is run on a real-machine
  session (this commit's verification rests on the criterion
  micro-bench + the 8 in-tree integration tests through the
  full MITM path).

  1531 capsem-core lib tests pass (down from 1547 -- the deleted
  redundant fixtures); 8/8 mitm_integration tests pass; clippy
  clean.

### Performance (mcp)
- **Pipelined the MCP gateway loop**
  (`crates/capsem-core/src/mcp/gateway.rs`). The per-vsock-connection
  serial `read → handle → write` loop is replaced with a reader that
  spawns one `tokio::spawn(handle_json_rpc)` per request and a
  dedicated writer task that drains an `mpsc::Receiver<Vec<u8>>`(256).
  Out-of-order responses are fine — JSON-RPC `id` lets the client
  demux. mcp-load (single fastmcp Client over one vsock) gains
  **+30 % rps@200 (4 252 → 5 551) and -44 % p99@200 (70.95 → 39.73 ms)**;
  mitm-load unchanged (±2.6 %). Next ceiling is the aggregator
  subprocess loop (T1.2 in `sprints/mcp-concurrency/`).

### Fixed (mcp)
- **`capsem_host_logs` / `capsem_panics` / `capsem_triage` /
  `capsem_timeline` no longer corrupt query values with reserved
  characters.** Each tool built its URL via raw
  `format!("k={}&", value)` interpolation. Two failure modes,
  both reproduced via live MCP:
  1. Any value containing whitespace (e.g. `grep="capsem-gateway
     spawned"`) failed with `invalid uri character` because the
     URL parser rejects unencoded spaces. **Multi-word grep was
     completely broken.**
  2. Any value containing `&` (e.g. `grep="foo&bar"`) was silently
     truncated to `foo` because the server's query parser saw the
     unescaped `&` as a separator and treated `bar` as a stray
     empty param.
  Same risk on `=`, `+`, `#`, `%`, `?`, and other reserved chars
  in `since`, `id`, `trace_id`, `layers`. Fix in
  `crates/capsem-mcp/src/main.rs`: new `query_string` helper
  builds the query from a list of `(key, Option<value>)` pairs,
  percent-encoding each value with an explicit RFC 3986
  query-value set (CONTROLS plus all reserved/unsafe ASCII;
  ALPHA/DIGIT and the unreserved `-._~` round-trip plain).
  Refactored the 4 tools to use it; trailing-`&` cosmetic issue
  fixed as a side effect. 8 new unit tests cover empty/single/
  multiple/None-skipping/space/`&`/multi-reserved-chars/unreserved-
  passthrough. `capsem_service_logs` was unaffected (does
  client-side filtering); the other 21 tools use JSON bodies or
  path-only URLs and don't take untrusted query values.

### Fixed (build)
- **`just _pack-initrd` no longer corrupts the hash-named hardlink
  while a stress run is mid-`VmConfig::build`.** The recipe wrote
  the gzipped cpio archive via shell redirect (`gzip > "$INITRD"`),
  which truncates the existing inode in place. `create_hash_assets.py`
  later gives `initrd.img` a hash-named hardlink (e.g.
  `initrd-<hex16>.img`, sharing the inode). An in-place rewrite
  mutates that hardlink's content too, so any concurrent VM mid-
  `VmConfig::build` reading the old hash-named path computes a hash
  of the NEW bytes and rejects with `hash mismatch for ...img:
  expected X, got Y` -- a stress run hit by a parallel `just
  _pack-initrd` lost two cycles per race (observed in
  `target/stress-acceptance-logs/iter-6.log` cycles 48-49 with
  unified-log evidence of `cpio` running at the exact failure
  timestamp). Fix in `Justfile`: write to `${INITRD}.tmp.$$` and
  `mv` to the final path. The atomic rename leaves the old inode
  (and its hash-named hardlink) intact until `_cleanup_stale` in
  `create_hash_assets.py` explicitly unlinks the old alias.

### Fixed (resume,protocol)
- **Stress-cycle "doesn't have entitlement" cascade now self-recovers
  via launchd-cleanup-aware retry.** Apple's
  `Virtualization.framework` runs a per-VM XPC helper
  (`com.apple.Virtualization.VirtualMachine.<UUID>`); when
  capsem-process dies, launchd schedules that XPC's cleanup with a
  9-second delay (observed in `log show`: `scheduling cleanup in 9
  sec after sending Killed: 9` followed by `internal event:
  PETRIFIED`). Under rapid VM churn (~3s/cycle) the cleanup queue
  grows; once `syspolicyd` saturates (`Unable to get certificates
  array: (null)` in the unified log just before the failure
  window), the next freshly-spawned capsem-process's
  `VZVirtualMachineConfiguration.validateWithError()` returns
  NSError code 2 with the misleading
  `localizedDescription = "...The process doesn't have the
  'com.apple.security.virtualization' entitlement."` -- even though
  the binary IS entitled. We saw this fire as 2-cycle cascades at
  ~cycle 37-40 of the 50-cycle stress (iter-2 cycles 37-38; iter-6
  cycles 39-40 post-Bug-C-fix). Two-part fix in
  `crates/capsem-service/src/main.rs`:
  (1) New `is_launchd_cleanup_transient` helper pattern-matches
  the full VZ-specific phrase (`com.apple.security.virtualization`
  + `entitlement`) on the failed-attempt's process.log tail. Does
  NOT match a bare `entitlement` mention so a real codesign
  regression still surfaces.
  (2) `handle_provision` extracts the per-attempt logic into
  `provision_attempt` and wraps it in `capsem_core::poll::poll_until`
  with `timeout=8s, initial_delay=200ms, max_delay=500ms`. On
  `LaunchdTransient` outcome the loop unregisters the failed
  attempt's persistent-registry entry + clears the instances map,
  then retries; everything else (`BootCrash`, `ProvisionError`,
  `Ready`) bails or succeeds immediately. Retry-decision routing
  is a pure function (`classify_attempt_decision`) so the retry
  logic is unit-testable without spawning a real VM. Worst-case
  user-visible latency on a healthy launchd is unchanged
  (single attempt, ~3-5s); under cascade the retry adds ~500ms-1s
  of backoff per failed attempt, amortized against the launchd
  drain. Unit coverage: 4 matcher tests + 6 routing tests covering
  Ready/StillBooting/LaunchdTransient/BootCrash/already-exists
  /generic-provision-error.
- **Post-resume `vsock_connect` ECONNRESET no longer poisons the agent's
  exec dedup cache.** After `restoreMachineStateFromURL` the host's
  vsock listener for the EXEC port (5005) is registered but the
  kernel-side accept queue can briefly reset incoming connections
  while VZ attaches it. The agent's `run_exec` opened that connection
  with a single-shot `vsock_connect`; one ECONNRESET → `run_exec`
  returned 126 → `exec_done` cached `id → 126` → every host-watchdog
  retry of the same Exec id replayed `ExecDone {exit_code: 126}`,
  even after the transport recovered. Captured in serial.log as
  `exec[N] vsock connect failed: Connection reset by peer (os error
  104)` followed by `exec[N] duplicate (already done, exit=126);
  replaying ExecDone`. Two-part fix in
  `crates/capsem-agent/src/main.rs`: (1) new
  `vsock_connect_with_econnreset_retry` helper retries on
  `ErrorKind::ConnectionReset` only (5 attempts × 20ms backoff =
  ~100ms ceiling, well under the host's 1s watchdog window);
  non-ECONNRESET errors bail immediately so misconfiguration
  (refused / address-family-unsupported) is not papered over. (2)
  `run_exec` now returns `ExecOutcome::{Done(i32), TransportFailed}`;
  `control_loop` only inserts into `exec_done` when
  `outcome.should_cache()` -- transport failures stay uncached so
  the next host-watchdog retry gets a fresh attempt against the
  recovered vsock. The host still receives `ExecDone {exit_code:
  126}` so its watchdog resolves with a real ExecResult instead of
  hanging. Verified in real-VM stress: pre-fix cycle 1 hit this in
  the very first failure; post-fix 39 consecutive cycles pass before
  a different (separately-tracked) failure mode appears. Unit
  coverage: 7 new tests covering retry-success, retry-recovery,
  bail-on-other-kinds, exhaustion, cache-decision matrix.
- **Symmetric guest-side replay buffer with `HostToGuest::AckReply`.**
  Closes the bidirectional silent-drop hole: the prior bridge replay
  layer covered the host→guest forward path; this adds the matching
  guest→host return path. The agent now keeps every ackable
  `GuestToHost` response (`ExecDone` / `FileOpDone` / `FileContent` /
  `Error`) in a `pending_responses` map keyed by `id`, lifted to
  outer scope in `capsem-agent/src/main.rs` so it survives
  reconnects (the writer thread is per-`run_bridge`). On every
  fresh control conn the writer thread first replays every entry
  still in the map, then resumes normal writes. The host bridge in
  `capsem-process/src/vsock.rs` emits `HostToGuest::AckReply { id }`
  immediately on receipt of an ackable response; `control_loop`
  removes the entry. Without this, an ExecDone (or FileContent --
  worse, since the agent doesn't cache file bytes) lost on the
  Apple VZ silent-drop path was unrecoverable except via the
  host's watchdog re-sending the original `Exec`, which only worked
  for `Exec` (cached `exit_code`) and not for `FileRead`'s
  `FileContent`. Verified directionally with a 50-cycle
  `CAPSEM_STRESS=1 test_stress_suspend_resume.py` run, 50/50 passed.
- **Bridge replay layer with `GuestToHost::Ack` for ackable
  HostToGuest messages.** The control bridge in
  `capsem-process/src/vsock.rs` now keeps every ackable outbound
  message (`Exec` / `FileWrite` / `FileRead` / `FileDelete`) in a
  pending map keyed by `id` (`JobStore::pending_acks`). The agent
  emits `GuestToHost::Ack { id }` immediately on receipt, *before*
  any processing -- the bridge clears the entry. On every fresh
  control conn after a re-key, the bridge re-writes every entry
  still in the map. This is the protocol-level cover for Apple
  VZ's post-restoreState silent-drop pattern: the host's
  `write_control_msg` returns success while the bytes never
  propagate, so the previous single-slot `held: Option<HostToGuest>`
  (which only fired on write *errors*) couldn't catch them. The
  multi-slot map also recovers a message whose Ack was lost on the
  return path -- the message stays pending across reconnects until
  an ack actually lands. Agent dedup ensures a re-sent message that
  did land twice doesn't double-execute.
- **Watchdog recalibrated to 1s × 16 retries (16s budget)** -- with
  the bridge replay layer now handling forward-path losses, the
  watchdog only exists to cover the asymmetric return-path case
  (agent processed and sent ExecDone / FileOpDone, those bytes were
  silently dropped). 1s gives ~6× headroom over the longest
  observed healthy round-trip (~150ms for `bash -c "mkdir+echo+cat"`)
  without sitting idle for 3s of dead time.
- The earlier "8 × 3s = 24s budget" config (commit `8cc76e2`) is
  superseded -- the storm-derivation-based number was correct in
  intent but the bridge replay layer is the structurally right fix
  for forward-path drops.

### Added (mitm-redesign)
- **`TelemetryHook` -- per-request `NetEvent` + optional `ModelCall`
  emission as a sync `ChunkHook`.** T1 slice 8 (additive). Carries
  the entire emit surface that lives in `telemetry::TelemetryEmitter`
  today, packaged as a `ChunkHook` that fires on `on_response_end`.
  The hook owns its own response-side byte counting + preview, so
  once the legacy chain is removed in the cleanup slice it
  replaces both `TelemetryEmitter` (the per-request scratch
  struct) and `TelemetryBody` (the body wrapper that decided
  *when* to fire). Per-request context is read out of a typed
  `HookState` slot (`Option<TelemetryRequestContext>`); a missing
  slot puts the hook in shadow mode (no allocation, no emit). The
  per-call `LlmEventStream` populated by the interpreter hooks is
  read at end-of-stream and folded into the `ModelCall` via the
  existing `collect_summary` path. Pure builder helpers
  (`build_net_event` and `maybe_build_model_call`) are split out
  so tests verify the field-mapping logic without spinning up an
  async runtime or a real `DbWriter`. Trace-correlation
  (tool-use chains across requests) goes through a shared
  `Arc<Mutex<TraceState>>` exactly the way `TelemetryEmitter`
  does today, so existing trace-grouping behavior is preserved
  byte-for-byte. Hook is **not** yet registered in
  `make_production_pipeline` and `handle_request` is **not** yet
  rewired; those changes ship together with the deletion of
  `telemetry.rs`, the legacy `AiResponseBody` /
  `DecompressBody` wrappers, and the benchmark gate in slice 9
  cleanup. Eight unit tests covering: `NetEvent` field mapping,
  HEAD probe filter, non-LLM path filter, non-AI provider
  filter, `LlmEvent` flow into `ModelCall`, tool-use trace
  chaining across two requests, shadow-mode skip when context
  unseeded, byte counting + preview tally with seeded context.
  1547 capsem-core lib tests pass; clippy clean.

### Added (mitm-redesign)
- **`DecompressionHook` -- streaming gzip decompression as a sync
  `ChunkHook`.** T1 slice 7. Replaces the
  `async_compression::tokio::bufread::GzipDecoder` driving
  `body::DecompressBody` with the lower-level
  `flate2::Decompress` raw-deflate state machine plus a small
  hand-rolled gzip-header parser. gzip streaming-decode is
  fundamentally sync, so the async wrapper was plumbing-only
  (one `tokio::io::AsyncRead` adapter, one `StreamReader`, one
  `Body -> Stream` shim) -- removing it is the goal of the cleanup
  slice. The hook detects gzip from the first two bytes' magic
  prefix (`0x1f 0x8b`) since the per-request `HookState` slot map
  carried by `ChunkDispatchBody` isn't shared with async
  `Hook::on_event`'s state, so a `Content-Encoding: gzip` flag
  can't bridge from `RawResponseHead` into the chunk pass through
  that map. Magic detection sidesteps the issue without changing
  the hook trait. The header parser handles the standard 10-byte
  prefix plus FEXTRA / FNAME / FCOMMENT / FHCRC optional fields
  (RFC 1952 §2.3.1). After the header, the deflate body streams
  through `flate2::Decompress::new(false)` (`zlib_header=false` =
  raw deflate); the decoder retains state across chunks so partial
  blocks split anywhere decode correctly. Registered in
  `make_production_pipeline` BEFORE the SSE parser hook so the
  hook order is correct once the legacy inline `DecompressBody` is
  removed in slice 9 (today the hook is essentially a no-op
  because `DecompressBody` decompresses upstream of the
  `ChunkDispatchBody` and the hook sees plaintext bytes; that's
  intentional -- this slice ships the surface, the cleanup slice
  flips the switch). Six unit tests: single-chunk decompress,
  decompressed-bytes split across two chunks, plain non-gzip
  passthrough, classification stickiness (a chunk that happens to
  start with `0x1f 0x8b` after a non-gzip first chunk is left
  alone), byte-by-byte chunking, and one-byte-first-chunk
  classification deferred. 1539 capsem-core lib tests pass;
  clippy clean.

### Added (mitm-redesign)
- **Provider interpreter `ChunkHook`s -- Anthropic / OpenAI /
  Google.** T1 slice 6. Three concrete `ChunkHook`s that consume
  parsed `SseEvent`s from the upstream `SseEventStream` slot and
  emit provider-agnostic `LlmEvent`s into a shared `LlmEventStream`
  slot. Each interpreter gates on its provider's domain
  (`api.anthropic.com`, `api.openai.com`,
  `generativelanguage.googleapis.com`) so registering all three in
  the production pipeline is essentially free for non-AI traffic --
  the unmatched hooks short-circuit on a single string compare
  before touching state. Internally, each hook reuses the existing
  `ProviderStreamParser` impl
  (`AnthropicStreamParserWithState` / `OpenAiStreamParser` /
  `GoogleStreamParser`) -- no parsing logic is duplicated, so all
  the existing per-provider tests still cover the parse semantics.
  The interpreter takes the parser out of its slot via
  `mem::take`, drains `SseEventStream`, runs each event through
  the parser, then puts the parser back -- this releases the slot
  map for the SSE/LLM slot accesses inside (single-borrow at a
  time on the slot map). `LlmEventStream` carries an optional
  `provider: ProviderKind` set by the matching interpreter on
  first push, so downstream consumers can dispatch on provider
  without re-parsing the domain. `on_response_end` runs the same
  drain so trailing SSE events flushed by `SseParserHook` reach
  the interpreter. All three registered in
  `make_production_pipeline` after `SseParserHook`. Six unit tests
  covering: end-to-end Anthropic SSE → text delta + summary,
  OpenAI text delta, Google multi-part chunk, three-hooks-coexist
  routing (only matching one drains), wrong-domain skip leaves
  queue untouched, on_response_end trailing flush. 1533
  capsem-core lib tests pass; clippy clean.

### Added (mitm-redesign)
- **`SseParserHook` -- the first concrete `ChunkHook` consumer.** T1
  slice 5. Wraps the existing `parsers::sse_parser::SseParser` as a
  sync `ChunkHook` and writes parsed `SseEvent`s into a public
  per-request `SseEventStream` slot via `ChunkCtx::state`. The slot
  is the bridge to the provider-specific interpreter hooks landing
  in the next slice -- they drain new events on every chunk pass to
  build `ModelCall` summaries. The hook gates internally on AI
  domains (`api.anthropic.com`, `api.openai.com`,
  `generativelanguage.googleapis.com`) so registering it in the
  production pipeline is free for non-AI traffic: the `is_ai` check
  caches in the parser-state slot on first chunk and a non-AI
  connection bails before allocating the parser. `on_response_end`
  flushes any trailing event without a terminating blank line --
  matches the behavior of the inline `AiResponseBody` path that
  this hook is replacing. Now registered in
  `make_production_pipeline`. Six unit tests cover single-chunk,
  multi-chunk-split, multi-event accumulation, non-AI bypass,
  trailing-event flush, and the `[DONE]` sentinel filter for
  OpenAI. 1527 capsem-core lib tests pass; clippy clean.

### Fixed (resume,protocol)
- **Host-side watchdog around HostToGuest::Exec / FileWrite / FileRead
  with j_rx-based retry and 24s budget.** Apple VZ post-restoreState
  occasionally drops a successfully-written vsock frame (the host's
  `write_control_msg` returns success; the bytes never reach the
  guest), and the existing single-slot replay buffer in the control
  bridge can't catch this -- it only triggers on a write *error*.
  The watchdog re-sends the payload every 3s if the host hasn't seen
  the result oneshot resolve. Direct measurement of one stress-suite
  failure (`process.log` from
  `20260503-220608/.../susp-10f1a6c7`) showed the storm lasted 9.13s
  before any message arrived end-to-end, so the budget is set to 8
  attempts × 3s = 24s, leaving 6s of headroom under the 30s IPC
  envelope. The watchdog's signal is the j_rx oneshot resolving
  (i.e. ExecResult / FileOp ack), not ExecStarted -- the latter
  fires while ExecDone is still in flight, and ExecDone can be lost
  on the same torn return path the original Exec was lost on.
- **Agent-side dedup with cached ExecDone replay.** Exec ids
  observed during a session are tracked in two maps shared across
  reconnects: `exec_inflight` (still running -- skip duplicate, the
  original will send ExecDone) and `exec_done: HashMap<id,
  exit_code>` (finished -- replay GuestToHost::ExecDone with the
  cached code so the host's j_rx resolves even when the original
  reply was lost on the return path). The maps are hoisted out of
  `control_loop` into the parent's outer reconnect scope so a retry
  that lands on a *new* control conn after the previous one was
  torn still hits the dedup logic. File ops are intentionally not
  deduped -- write/read/delete are idempotent enough to re-process
  and re-ack on every receipt, which is correct for a FileOpDone
  that was lost on the return path (dedup-with-skip there would
  deadlock the host watchdog).

### Known limitations (resume,protocol)
- **Stress-suite flakiness floor: ~30% iteration fail rate remains.**
  10x runs of the back-to-back stress suite
  (`test_svc_resume_paths.py` + `test_svc_suspend_corruption.py` +
  `TestSuspendResume`) score 6-7/10 with these fixes, vs 7/10 for
  the unfixed baseline at HEAD~1 -- within the same noise band.
  Direct measurement (one ovl-test failure) showed the post-resume
  storm can last 21s of constant vsock re-keying, dropping
  bidirectional traffic for the entire window. Neither host-side
  retries nor guest-side response replay survive a storm that
  spans the whole 30s IPC envelope, because the bytes for the
  retried Exec *and* its replayed ExecDone are both subject to
  silent-drop on every conn. Closing this requires either: (a)
  application-level reliability (per-message ACKs over vsock with
  exponential backoff and a longer envelope), (b) a guest-side
  replay buffer for GuestToHost messages analogous to the host's
  bridge replay buffer (held across the agent's reconnect rather
  than dropped when the writer thread breaks), or (c) detecting and
  pausing sends during a storm. Followup beyond this sprint's scope.

### Fixed (test-infra)
- **`/delete` now routes through `preserve_failed_session_dir`.**
  Previously the only paths that preserved `process.log` /
  `serial.log` / `session.db` for post-mortem were three
  host-detected failure routes; a Python-side test assertion that
  fired after `/exec` but before the test's `finally:
  client.delete()` left only `service.log` archived, which doesn't
  show what the per-VM process or the guest were doing. The cull
  is bumped from 5 to 32 most-recent failed sessions so a
  10-iteration stress run that creates 1-3 VMs per iteration
  doesn't lose earlier failures to the LRU. Disk usage stays
  bounded by the cull regardless.

### Added (mitm-redesign)
- **Pipeline observability contract: every hook call is logged,
  timed, and counted.** Closes the "what is blocking?" gap. Async
  `Hook::on_event` is now wrapped in a `mitm.hook` info-span carrying
  fields `hook`, `kind`, `layer`, `decision` (recorded after the
  future resolves -- one of `continue`/`rewrote`/`stop_drop`/
  `stop_reject`/`stop_dns_reject`), and `duration_ms`. Counter
  `mitm.hook_invocations_total{hook}` increments per call;
  histogram `mitm.hook_duration_ms{hook}` samples the wall time.
  Trace events bracket the call: `on_enter` + `on_exit` at trace!
  level (filter via `RUST_LOG=mitm.hook=trace`). Stop-outcomes
  promote to debug! at target `mitm.hook.cause` so triage tooling
  surfaces them at default RUST_LOG=info filtering. Sync
  `ChunkHook` iteration gets the same counter + histogram (no span,
  trace! events at `mitm.hook.chunk` -- per-chunk spans would
  dominate the bench budget). New unit test installs a
  `metrics_util::DebuggingRecorder` via `set_default_local_recorder`
  and asserts the counter + histogram both fire on a single
  dispatch. 1521 tests pass; clippy clean.

### Added (mitm-redesign)
- **`ChunkHook` -- sync per-body-chunk hook trait + pipeline
  registration.** T1 slice 3 foundation. `ChunkHook` is a sync
  companion to the async `Hook` trait: methods
  `on_request_chunk(&mut Bytes, &mut ChunkCtx)` /
  `on_response_chunk(...)` / `on_request_end` /
  `on_response_end`. Body wrappers iterate registered ChunkHooks
  inline from `poll_frame` -- no async overhead, no channel hop.
  Sync is correct here because per-chunk work is fundamentally
  CPU-bound byte transformation: decompression, regex
  match-and-replace, streaming parsers, byte counting. None need
  `.await`. Per-connection state lives in the same typed slot
  map the async `Hook`s use, accessed via `ChunkCtx::state::<T>()`.
  `Pipeline` gains `register_chunk(ArcChunkHook)` builder method,
  `has_chunk_hooks()` short-circuit predicate, and
  `dispatch_request_chunk` / `dispatch_response_chunk` /
  `dispatch_request_end` / `dispatch_response_end` iteration
  helpers. Two new unit tests prove the surface: registration-order
  iteration with one hook rewriting bytes that the next hook then
  observes, and the empty-pipeline short-circuit. Slices 3b
  (DecompressionHook), 3c (TelemetryHook), 3d (SseParserHook) are
  now unblocked. 1520 tests pass; clippy clean.

### Added (mitm-redesign)
- **`RawResponseHead` dispatch + per-request `mitm.request` span.**
  T1 slice 3a (observer surface). After upstream returns headers,
  `handle_request` now dispatches `Event::RawResponseHead(&mut parts)`
  through the pipeline so future hooks can observe the response head
  before any wrapping (decompression, telemetry, AI parsing) takes
  place. Hooks that want to react to status codes or content-encoding
  / content-type live here. Today observer-only -- the Stop outcome
  is intentionally dropped because handing the upstream sender
  partially-used would leak. Plus a `#[instrument(target="mitm.request")]`
  decoration on `handle_request` itself recording fields domain,
  method, path, decision, status; every log line in a request now
  carries those as structured fields. Pure addition; no behavior
  change. 1518 tests pass; clippy clean.

### Added (mitm-redesign)
- **Metrics + tracing decision contract wired on the hot path.** T1
  slice 4. Every TLS connection now increments
  `mitm.connections_total{protocol="tls"}` and the
  `mitm.active_connections` gauge (RAII-decremented on drop, even on
  panic). Every request increments
  `mitm.requests_total{protocol="tls", decision}` partitioned by
  outcome (`allow` / `deny` / `upstream_error`). TLS handshake time
  histograms via `mitm.tls_handshake_ms`; full upstream-dial path
  (TCP + TLS) via `mitm.upstream_dial_ms`. `handle_connection` now
  in a `#[instrument(target="mitm.connection")]` span. No recorder
  registered yet, so each emission is one relaxed atomic add against
  the global no-op recorder (~4 ns per call per the T0 baseline).
  Two new smoke tests assert the metric names are unique and
  `describe_all` is idempotent. 1518 capsem-core lib tests pass;
  clippy clean.

### Fixed (virtio-blk-overlay-migration)
- **System overlay moved off loop-on-VirtioFS onto a real virtio-blk
  device.** rootfs.img is now attached to the guest as `/dev/vdb` and
  mounted directly as the overlayfs upper, bypassing the prior
  loop-device-on-VirtioFS sandwich whose closed-source virtiofsd
  returned EIO under writeback pressure on resume. Closes
  `loop-device-io-after-resume`: heavy directory churn + suspend +
  resume no longer leaves `EXT4-fs (loop0): failed to convert
  unwritten extents` / `I/O error, dev loop0` in dmesg. Universal --
  ephemeral and persistent VMs both use the new path; legacy
  loop-on-VirtioFS fallback removed from `capsem-init`. Snapshot
  (APFS clonefile) path validated byte-for-byte against the
  virtio-blk-attached file. `BootOptions::scratch_disk_path` renamed
  to `system_overlay_disk` to reflect its new role.

### Fixed (resume-stability)
- **Resume API no longer hangs 30s when capsem-process dies during
  restore.** `wait_for_vm_ready` now races the `.ready` sentinel poll
  against an instance-presence check; when the resume-side child
  exits before signalling ready, the API fails fast (~5ms-50ms)
  instead of spinning out the full readiness budget. The exit
  handler also logs the child's `exit_status` so future failures are
  diagnosable from `service.log` alone (previously the resume-side
  exit silently dropped the status).
- **Apple VZ post-restoreState handshake EOF is now retryable.**
  `is_retryable_handshake_error` accepts `UnexpectedEof` alongside
  `BrokenPipe` / `ConnectionReset` -- empirically the dominant
  fingerprint when Apple VZ tears the new vsock conn down between
  guest frames. The host re-accepts a fresh terminal+control pair
  and re-runs the handshake within the existing
  `HANDSHAKE_RETRY_MAX` budget. Prior behaviour: process exited with
  code 1, leaving the resume API to time out at 30s.
- **Control bridge holds in-flight `HostToGuest` messages across
  re-key.** When Apple VZ kills the control vsock mid-write, the
  message that was being sent (often an `Exec` or `FileWrite`
  command) used to be silently dropped, and the corresponding
  `/exec` or `/write_file` call timed out at 30s waiting for a reply
  that would never come. The bridge now stashes the failed message
  and replays it on the next successfully re-keyed connection.

### Changed (mitm-redesign)
- **Inline `policy.evaluate` deny path removed; PolicyHook is now the
  source of truth.** T1 slice 2d. PolicyHook stashes its
  PolicyDecision (allowed + matched_rule + reason) in HookCtx::state
  via the typed slot mechanism. After dispatch, handle_request reads
  the record back and uses it to populate the TelemetryEmitter (allow
  + deny paths both). On Stop(Reject(_)) the hook's response is
  wrapped with TelemetryBody so a NetEvent still fires for denies
  (no telemetry regression). Test fixtures upgraded from
  make_default_pipeline() to make_production_pipeline(policy) so
  policy actually fires in unit + integration tests. 1516 lib tests +
  8 integration tests pass; clippy clean. Slice 2d closes T1's
  rewire of the policy stage; the pipeline now owns it end-to-end.

### Added (mitm-redesign)
- **Pre-rewrite `mitm-load` baseline captured.** T0 closes:
  `benchmarks/mitm-load/baseline.json` holds the live numbers from
  `capsem-bench mitm-load` against the un-redesigned proxy at
  concurrency 1/10/50/200 (10s per level). Highlights: rps
  1109/2862/2995/2701, p99 2.2/8.4/45.4/175.2 ms, 0 errors,
  RSS 26-230 MB. T5's CI gate compares against this file -- any
  level >2x p99 regression fails the build.

### Added (mcp + bench)
- **`local__echo` MCP tool + `capsem-bench mcp-load` mode + baseline.**
  New zero-I/O diagnostic tool: returns its `text` parameter verbatim.
  Lives in `capsem-mcp-builtin`; reachable as `local__echo` through
  the in-guest MCP server -> vsock:5003 -> aggregator -> builtin
  subprocess chain. New `capsem-bench mcp-load` mode hammers it from
  the guest with concurrent fastmcp Client calls (asyncio.gather over
  N workers per concurrency level) so we get a number for the MCP
  path's scaling shape, isolated from the MITM path. Pre-rewrite
  baseline at `benchmarks/mcp-load/baseline.json`: rps
  2162/3792/4061/3965 across concurrency 1/10/50/200, p99
  1.1/4.4/17.4/70.8 ms, 0 errors. Sub-linear scaling -- plateaus at
  ~4000 rps from concurrency 10 onwards. There IS a serialization
  point in the MCP path that needs debugging (suspect:
  stdio-framing in capsem-mcp-server, single vsock:5003 stream, or
  JSON-RPC dispatch in the aggregator). Sister to the MITM baseline,
  which plateaus around ~3000 rps with worse tails.

### Added (capsem CLI)
- **`capsem cp` -- file transfer between host and a session's
  workspace.** The service has had `GET/POST /files/{id}/content`
  upload/download endpoints for a while (used by the desktop app's
  Files tab). The CLI never exposed them. Now: `capsem cp foo.txt
  my-vm:foo.txt` (upload) / `capsem cp my-vm:bench.json
  ./bench.json` (download) / `capsem cp my-vm:log.txt -` (stdout).
  Exactly one of `<src>`/`<dst>` must be `SESSION:PATH`; PATH is
  relative to `/root` (workspace bind-mount in the guest). Errors
  loud: `guest-to-guest copy not supported`,
  `neither argument is SESSION:PATH`. New
  `UdsClient::request_bytes` returns raw response bytes + content-type
  for endpoints that don't speak JSON (the existing `request` method
  always tries to deserialize JSON, so couldn't be used for binary
  downloads).

### Added (mitm-redesign)
- **Pre-rewrite `mitm-load` baseline captured.** T0 closes:
  `benchmarks/mitm-load/baseline.json` holds the live numbers from
  `capsem-bench mitm-load` against the un-redesigned proxy at
  concurrency 1/10/50/200 (10s per level). Extracted via the new
  `capsem cp` command (write bench output to `/root/baseline.json`
  in the guest, `capsem cp` it to host). Highlights: rps
  1037/3043/3029/2699, p99 2.3/8.4/53.4/191.3 ms, 0 errors,
  RSS 27-260 MB. T5's CI gate compares against this file -- any
  level >2x p99 regression fails the build.

### Added (mitm-redesign)
- **Hook pipeline now dispatches from `handle_request`
  (parallel-deploy).** T1 slice 2c: every HTTPS request through the
  MITM now runs `pipeline.dispatch(Event::RawRequestHead, ...)` with
  the per-connection `ConnMeta` (domain + process_name + port=443)
  and the ambient `trace_id`. Production builds use
  `make_production_pipeline` so `PolicyHook` fires for every request,
  emitting the `mitm.policy_decisions_total` counter and the
  structured `mitm.policy` tracing event with rule + reason fields.
  The hook's `Stop(Reject(_))` outcome is intentionally dropped this
  slice -- the inline `policy.evaluate()` call below remains the
  source of truth for the actual stop/continue decision so behavior
  is provably unchanged. Subsequent slices land TelemetryHook +
  RejectHook plumbing that lets us safely remove the inline path.

### Added (mitm-redesign)
- **`PolicyHook` + `ConnMeta` + `make_production_pipeline`.** T1
  slice 2b: first concrete `Hook` impl. `mitm_proxy/policy_hook.rs`
  subscribes to `Event::RawRequestHead` (priority -1000 so it runs
  before any other L1 consumer), evaluates `NetworkPolicy::evaluate`
  against `ConnMeta::domain` + the request method, returns
  `Stop(Reject(403))` on deny. Tracing target `mitm.policy` records
  `decision` (allow|deny) + `rule` + `reason`; metric
  `mitm.policy_decisions_total{decision}` increments. New
  `ConnMeta` (`domain`, `process_name`, `port`) carried read-only
  through `HookCtx::conn()` so hooks can reach per-connection
  metadata not present in `RawRequestHead`. `make_production_pipeline`
  builds the registered set; `handle_request` does not yet dispatch
  through it (slice 2c). 4 new tests cover allow / deny / default-allow
  and the `evaluate_decision` rendering helper. 1516 passing.

### Added (mitm-redesign)
- **`MitmProxyConfig` carries a `pipeline: Arc<Pipeline>` field.**
  T1 slice 2a: the `Pipeline` from slice 1 is now plumbed through the
  proxy config so subsequent slices can dispatch from `handle_request`
  without changing the public type again. `make_default_pipeline()`
  returns an empty pipeline -- the inline call graph in
  `handle_request` still drives policy / decompression / AI parsing /
  telemetry. T1 slice 2b will register the production hooks; slice 3
  wires the metrics + tracing decision contract. Three call sites
  updated: `mitm_proxy/tests.rs`, `tests/mitm_integration.rs`,
  `capsem-process/src/main.rs`.

### Added (mitm-redesign)
- **Single `Hook` trait + `Event<'_>` ladder + dispatcher.** T1 slice
  1: pure-additive infrastructure for the new pipeline. Three new
  modules under `mitm_proxy/`: `events.rs` (15-variant `Event<'a>`
  enum across L1 raw transport / L2 protocol / L3 semantic, plus
  `EventKind` discriminator + `EventLayer` ordering + bitset
  `EventMask`), `hooks.rs` (the single `Hook` trait, `HookOutcome` =
  `Continue | Rewrote | Stop(StopAction)`, `StopAction` =
  `Drop | Reject(http::Response) | DnsReject(rcode)`, `HookCtx` with
  per-connection typed slot map for cross-call carry-over and
  `ctx.emit()`), `pipeline.rs` (registration-time-sorted dispatcher
  with O(1) per-kind plan, recursive `emit()` re-entry, layer-cycle
  prevention enforced at runtime: an L3 hook cannot emit L1/L2;
  `EmitError::CycleAttempt` returned). 16 new unit tests including:
  hook ordering by `(priority, registration_order)`, `Stop`
  short-circuit, L1->L2 emit dispatch, L3->L1 cycle rejection, typed
  state slot persistence across multiple chunk dispatches (the
  contract the future credential-rewrite hook will use), trace-id
  visibility. No production code wires the pipeline yet -- T1 slice 2
  rewires policy / decompression / AI parsing / telemetry as Hook
  impls.

### Added (mitm-redesign)
- **`capsem-bench mitm-load` mode.** New
  `guest/artifacts/capsem_bench/mitm_load.py` drives the MITM proxy at
  configurable concurrency levels (default 1 / 10 / 50 / 200) for
  `CAPSEM_BENCH_MITM_DURATION` seconds each (default 10s) against
  `CAPSEM_BENCH_MITM_TARGET` (default a non-routable domain so every
  request fails fast at upstream-dial, isolating proxy cost from
  upstream variance). Reports per-level rps, p50/p95/p99/p99.9 latency,
  RSS peak, and error count. T5's CI gate compares to
  `benchmarks/mitm-load/baseline.json`: any concurrency level >2x p99
  regression fails the build. Baseline JSON itself is deferred --
  requires `just run "capsem-bench mitm-load"` against the
  un-redesigned proxy and commit of the result.

### Added (mitm-redesign)
- **Criterion bench harness + pre-rewrite baselines.** `criterion`
  (dev-dep) plus four new benches under `crates/capsem-core/benches/`:
  `parser_sse`, `parser_jsonrpc`, `interp_anthropic`, `mitm_pipeline`.
  First-run numbers committed to `benches/baselines/T0-pre-rewrite.md`
  -- T5's regression gate compares against this file via `critcmp` and
  fails CI on >5% slower medians. Baseline highlights: SSE parser
  449-472 MiB/s on 1MB corpora (plan budget 500 MiB/s), Anthropic
  interpreter end-to-end 233 MiB/s on tool-use response, metrics-facade
  counter emission 3.89 ns with no recorder installed.
- **`metrics` facade dependency + `mitm_proxy::metrics` module.**
  All counter / histogram / gauge names from the plan declared with
  `describe_*` calls in `mitm_proxy/metrics.rs`. No recorder registered
  this sprint -- T5 wires an exporter (likely OTel via
  `opentelemetry-otlp`); until then emission is a single relaxed atomic
  add against the global no-op recorder. T0 slice 4 of
  `sprints/mitm-redesign/`.

### Fixed (observability)
- **W3 IPC handshake: respect tokio's non-blocking sockets.**
  `tokio::net::UnixStream::into_std()` returns the std handle still in
  non-blocking mode. The W3 handshake's `read_exact`/`write_all` then
  bailed with WouldBlock instantly, manifesting as 95 integration tests
  failing with "peer did not send Hello within 5000ms" the first time
  any IPC channel was used. `negotiate_initiator`/`negotiate_responder`
  now flip the socket to blocking mode for the handshake (saving the
  previous flag) and restore the original mode afterward so the bincode
  channel inherits the same tokio-non-blocking shape it expects. Builds
  + 1273 integration tests now pass.

### Added (observability follow-ups)
- **W6 writer-side population.** `trace_id` is now a column AND a
  field on every event struct. Writer INSERTs the column on every row.
  Construction sites populate via
  `capsem_core::telemetry::ambient_capsem_trace_id()`. `tool_calls` /
  `tool_responses` fall back to the parent `model_calls.trace_id`.
- **`capsem_triage --id <vm>` queries session.db** for `denied_net`,
  `mcp_errors`, `exec_failures` alongside the host-log scan.
- **`capsem_timeline` joins tool_calls -> mcp_calls** so a model
  tool_use shows its servicing MCP call inline.
- **`capsem support-bundle --max-session-bytes`** (default 50MB) drops
  oldest sessions when their session.db total exceeds the cap.
- **Hot-path `#[instrument]` coverage** on `wait_for_vm_ready`,
  `pause`, `resume`, `attach_disk`, `attach_virtiofs_share`.
- **`dump_frontend_logs` Tauri command + `recordWsEvent` wiring.**
  `__capsemDebug.dumpLogs()` now returns a real jsonl path;
  `__capsemDebug.lastWsEvents` actually fills as WS events arrive.
- **Triage panic parser + redactor adversarial fixtures.**
- **`capsem-app` emits `service.start`** so cross-version-mix detection
  covers all 9 binaries (adds capsem-proto leaf dep; capsem-core
  invariant preserved).
- **Skill updates: dev-mcp** (4 new tools in tool table), **dev-debugging**
  (MCP triage trio workflow + schema_hash hint),
  **references/mcp-wire.md** (W5 `_meta` envelope + BootConfig.traceparent).
- **C1: T3 timeline SQL allowlist** enforced before `format!()`.
- **C2: `app_error_logged!`** used in fork's clone-task error path.
- **T1 (test): `tests/capsem-service/test_protocol_handshake.py`**
  exercises the W3 handshake regression.
- **CLI parity: `support-bundle` added to `CLI_ONLY` allowlist.**

### Added (observability)
- **In-band W3C trace context on the host->guest control bridge and
  on MCP JSON-RPC.** `BootConfig` now carries an optional
  `traceparent: String` so the guest agent learns the host's trace_id
  on message #1 of boot; capsem-agent stamps every subsequent
  `blog_line` log line with `trace_id=<lower 16 hex>` so guest-side
  panics, kernel errors, and init script output correlate with
  host-side spans for the same VM boot.
  `JsonRpcRequest` and `JsonRpcResponse` gain an optional `_meta`
  envelope with `traceparent` + `tracestate` (W3C Trace Context) so a
  per-tool-call trace can ride alongside the JSON-RPC payload. Both
  fields are optional with serde defaults -- third-party MCP clients
  and pre-W5 capsem peers continue to round-trip cleanly.
  Also reorganizes the post-mitm-redesign rename: `net::ai_traffic::
  {anthropic,google,openai,sse}` are now re-exports of the new
  `net::interpreters::*_interpreter` and `net::parsers::sse_parser`
  modules so existing call sites compile while new code can use the
  fully-qualified path.

### Changed (mitm-redesign)
- **`mitm_proxy.rs` decomposed into submodules.** The 1421-line file
  is now `mitm_proxy/mod.rs` (614 lines: handle_connection +
  handle_inner + handle_request + MitmProxyConfig + helpers) plus four
  sibling submodules: `body.rs` (BodyStats, RespStatsKind, ProxyBoxBody,
  TrackedBody, BodyStream, DecompressBody), `telemetry.rs`
  (TelemetryEmitter + TelemetryBody + emit_model_call), `fd_stream.rs`
  (AsyncFdStream + ReplayReader + set_nonblocking), `util.rs`
  (is_llm_api_path + split_path_query + format_headers +
  HEADER_ALLOWLIST). Each submodule keeps `pub(super)` visibility so
  the public API of `crate::net::mitm_proxy::*` is unchanged. T0
  slice 3 of `sprints/mitm-redesign/`; zero behavior change.

### Changed (mitm-redesign)
- **All remaining inline `mod tests { }` blocks in `net/` extracted to
  sibling `tests.rs` per CLAUDE.md.** `mitm_proxy.rs` shrinks from
  2847 to 1421 lines (1426 lines of tests now in
  `mitm_proxy/tests.rs`); `ai_traffic/{events,pricing,ai_body,provider,
  mod}.rs` similarly cleaned. Production code is no longer buried under
  scroll-past test fixtures; every grep / Read of a parser shows just
  the parser.

### Changed (observability)
- **W6 trace_id wiring completed across capsem-logger / capsem-core /
  capsem-process.** The `trace_id` column on `net_events`, `mcp_calls`,
  `tool_calls`, `tool_responses`, `fs_events`, `snapshot_events`, and
  `audit_events` is now populated end-to-end. Write-side: every event
  emitter (`mitm_proxy`, `mcp/{gateway,builtin_tools,file_tools}`,
  `fs_monitor`, `capsem-process`'s snapshot/audit paths) calls
  `capsem_core::telemetry::ambient_capsem_trace_id()`. INSERT statements
  in `writer.rs` now include the new column. `tool_calls.trace_id` and
  `tool_responses.trace_id` fall back to the parent `model_calls.trace_id`
  when the per-row value is None (same agent turn). Read-side defaults
  to `None` until the SELECT clauses are extended in a follow-up.

### Changed (mitm-redesign)
- **AI parser tests extracted to sibling `tests.rs` per CLAUDE.md.**
  `parsers/sse_parser.rs`, `interpreters/anthropic_interpreter.rs`,
  `interpreters/openai_interpreter.rs`, and
  `interpreters/google_interpreter.rs` no longer carry inline
  `mod tests { }` blocks; their ~1100 lines of tests now live next to
  each prod file (e.g., `parsers/sse_parser/tests.rs`). Same pattern
  established by the obs sprint's earlier 18-file extraction.
- **Backwards-compat re-exports removed.** The transitional aliases
  `net::ai_traffic::{anthropic,google,openai,sse}` are gone; all
  internal callers (mitm_proxy, ai_body, events, provider, interpreter
  tests) reference the canonical
  `net::parsers::sse_parser` / `net::interpreters::<provider>_interpreter`
  paths. T0 slice of `sprints/mitm-redesign/`.

### Added (mitm-redesign)
- **`sprints/mitm-redesign/` scaffolded.** Meta-sprint plan to decompose
  the 2847-line `mitm_proxy.rs` monolith into a hookable pipeline with
  first-class plain HTTP, a real DNS proxy (hickory-server replaces the
  fake dnsmasq), MCP protocol awareness, and a single `Hook` trait + L1/
  L2/L3 `Event` ladder. Six phases (T0..T5) covering reorganization,
  hook traits, plain HTTP, DNS, MCP awareness, and hardening with
  performance regression CI gates. The future security engine
  (credential rewrite via regex body replace) is explicitly out of scope
  but the hook surface is shaped to host it without trait changes.

### Added (observability)
- **`capsem doctor --bundle` -- in-VM diagnostic tar wired into the
  support bundle.** `guest/artifacts/capsem-doctor` now accepts
  `--bundle [PATH]` and packages pytest output + junit XML, /var/log,
  dmesg, /proc/{mounts,cmdline}, /tmp/capsem-init.log, and
  session.db (when present) into a single tar at
  `/shared/doctor-bundle.tar` (default) or a caller-supplied path.
  Host-side `capsem doctor --bundle` lifts that file out of virtiofs
  to `~/.capsem/run/doctor-latest.tar` before the VM is destroyed.
  `capsem support-bundle` then embeds it as `doctor/bundle.tar`.
  Closes the "guest-side bug, but the bundle has only host context"
  gap in T1's bundle.

- **CI uploads `test-artifacts/` on red runs.** Both the `test-linux:`
  and `test:` jobs now have `upload-artifact@v4` steps gated on
  `if: failure()`. Reviewers get a downloadable bundle of
  `service.log`, `process.log`, `serial.log`, and `session.db` from
  every failed job without rerunning. Existing `preserve_tmp_dir_on_failure`
  in `tests/helpers/service.py` already populates the directory.
- **`just test-artifacts`** -- one recipe that finds the latest
  preserved failure dir under `test-artifacts/` and prints the file
  list with sizes. Saves digging through `ls -lt` after a red local
  run.
- **Frontend `window.__capsemDebug` console handle.** Exposed when
  the URL contains `?debug=1`. Methods: `versions()` (build_ts +
  version), `dumpLogs()` (returns the path to the latest jsonl via a
  reserved `dump_frontend_logs` Tauri command), `lastWsEvents` (small
  ring buffer; populated by api.ts when a WS event arrives via
  `recordWsEvent`). Console-only -- the visual HUD is punted to the
  frontend-rebuild sprint.

- **`capsem_timeline` MCP tool -- one tool call renders the unified
  time-ordered event stream for a session.** UNION across exec_events,
  mcp_calls, net_events, fs_events, and model_calls, ordered by
  timestamp. Filter by `traceId` to follow a single logical operation
  across layers (W6 added trace_id to every table; W4 propagates the
  id through the host process tree). Filter by `since` to scope the
  window. Optional `layers` arg accepts a comma-separated subset
  ("exec,mcp" etc.) when only some are interesting. Pre-W4 rows have
  NULL trace_id and are returned alongside matched rows so the user
  doesn't lose context that pre-dates the trace propagation.

- **`trace_id TEXT` column on every event table.** Added to
  `mcp_calls`, `net_events`, `fs_events`, `snapshot_events`,
  `tool_calls`, `tool_responses`, `audit_events` (model_calls and
  exec_events already had it). Indexes added on each. Fresh DBs get
  the column from `CREATE_SCHEMA`; existing DBs get it via
  idempotent `ALTER TABLE ADD COLUMN` on next open. Unblocks
  `capsem_timeline --trace_id <X>` to UNION across all event classes
  for one logical user action. Population through the writer API
  follows in a subsequent commit; pre-population rows are NULL and
  the timeline tool tolerates that gracefully.

- **W3C trace context propagated to every spawned capsem-* binary +
  per-stage timing on the suspend hot path.** capsem-service injects
  `CAPSEM_VM_ID`, `CAPSEM_TRACE_ID`, `TRACEPARENT`, `TRACESTATE` into
  capsem-process at spawn (cold-boot + resume paths); capsem-process
  forwards them when spawning capsem-mcp-aggregator. New helper
  `capsem_core::telemetry::child_trace_env(vm_id)` in one place; if
  this binary is itself a child of another capsem-* binary, the
  parent's traceparent is forwarded verbatim, so the whole tree shares
  one trace_id. Top-of-tree binaries synthesize a fresh
  `00-<32hex>-<16hex>-01` traceparent from blake3(vm_id + nanos).
  Suspend now emits `target=suspend op=apple_vz_pause`,
  `op=apple_vz_save_state`, `op=with_quiescence`, and
  `target=fs op=fsync path=rootfs.img` events with `duration_ms` --
  closes parent ISSUE.md pattern (6) and the today-2026-05-02
  "fsync timing was missing" debugging session.
- **Top-5 `_ => {}` enum arms now log instead of dropping.** vsock
  port dispatcher, lifecycle port, `handle_guest_msg`, and the MCP
  aggregator main match. An unknown variant now emits
  `tracing::warn!(target = "ipc", unhandled = ?other, "unknown
  variant; this binary may be older than its peer")` -- closes parent
  ISSUE.md pattern (3).

- **`capsem_panics`, `capsem_triage`, `capsem_host_logs` MCP tools.**
  AI agents (and developers via `capsem-mcp`) can now triage Capsem
  failures in one tool call without leaving the conversation:
  - `capsem_panics` -- structured panic + backtrace extractor across
    `~/.capsem/run/{service,mcp,gateway,tray}.log` and capsem-app's
    latest jsonl. Returns `[{ ts, binary, thread, location, message,
    frames }]` with `/Users/<x>/` paths redacted to `~/`. Run this
    FIRST when investigating an unexplained failure.
  - `capsem_triage` -- ranked summary of recent panics, dropped IPC
    frames (`target=ipc` warns from W1), 4xx/5xx server errors
    (`target=service` from W3.5), and slow operations (`target=fs
    op=fsync` etc., >500ms). Default lookback "30m"; accepts "5m",
    "1h", "24h", "7d", or RFC3339.
  - `capsem_host_logs` -- read any host log by symbolic name with
    grep + tail filtering. Hard-coded allowlist (no path traversal).
  Three new service HTTP endpoints (`/triage`, `/panics`,
  `/host-logs/{name}`) reuse the W2 JSON output shape, the W3 schema
  hash, and the W3.5 status field for deterministic ranking.

- **`capsem support-bundle` -- one command, one redacted tar.gz, ready
  to attach to a bug report.** Gathers `~/.capsem/run/*.log`,
  `~/.capsem/logs/*.jsonl`, the last N session directories
  (session.db + serial.log + process.log + metadata.json), assets
  manifest, redacted user.toml/corp.toml, version + OS info, dmesg
  (Linux), and a blake3 fingerprint of the MITM CA cert (the cert
  itself is NEVER bundled). Default output:
  `~/.capsem/support/capsem-support-<UTC-ts>-<host>.tar.gz`. Five
  redaction rules strip Bearer tokens, sk-/AIza/xoxb- API key prefixes,
  TOML/JSON keys named like a secret, and `/Users/<x>/` paths;
  `--no-redact` disables. `--include-rootfs` opt-in (off by default --
  rootfs.img is huge and rarely useful). Manifest schema v1 includes a
  ranked "next steps" list pointing at where to look in the bundle and
  which `target=` filters to grep for.

- **Every `AppError` returned by the capsem-service HTTP layer now
  emits a structured `tracing` event automatically.** Done in
  `IntoResponse` so all 104 `AppError(StatusCode, msg)` call sites are
  covered with zero codemod: 5xx → `error!`, 4xx → `warn!`, other →
  `info!` with `target = "service"` and the status code as a
  structured field. Pre-W3.5: the user got a 500 in the response with
  nothing in `service.log` to trace back from. Optional
  `app_error_logged!` macro lets a call site emit a SECOND event
  earlier (with the same status field) when an in-flight span is more
  informative than the late one fired at response-build time.

- **Versioned IPC handshake: cross-version mixes fail loudly in ~1s.**
  Every typed IPC connection between capsem-service and capsem-process
  now exchanges a `Hello { version, schema_hash, peer, traceparent }`
  frame on the raw UnixStream before the bincode channel takes over.
  `version` bumped to `1`. `schema_hash` is a build-script-emitted
  FNV-1a 64 hash of the protocol source bytes -- catches enum
  reordering / variant additions that don't bump version. On mismatch:
  `tracing::error!(target = "ipc", peer_id, ours_hash, peer_hash,
  "IPC handshake failed; refusing connection")` within 1 second instead
  of the pre-sprint 30-second silent timeout. Side-channel design
  (handshake on the raw stream before bincode) preserves the existing
  `Sender<ServiceToProcess>` / `Receiver<ProcessToService>` API; W1's
  `try_send!` codemod sites are unchanged. Pre-W3 binaries fail decode
  within 5 seconds (HELLO_TIMEOUT).

- **All host-side binaries now write JSON-per-line logs to
  `~/.capsem/run/{service,mcp,gateway,tray}.log`** -- consolidated
  through a single `capsem_core::telemetry::init()` entry point. Eight
  binaries (capsem-service, -process, -mcp, -mcp-aggregator,
  -mcp-builtin, -gateway, -tray, plus the macros consumer in capsem)
  now share one tracing-subscriber bootstrap. The four that previously
  emitted compact-format text (gateway, tray, mcp-builtin,
  mcp-aggregator) now emit structured JSON, so `capsem support-bundle`
  and the upcoming `capsem_panics` MCP tool can parse every host log
  with one decoder. Each binary's `service.start` line carries
  `protocol_version` + `schema_hash` so cross-version-mix can be
  detected from a single log read once W3 lands.
- **W3C `TRACEPARENT` env var captured at startup** and exposed via
  `capsem_core::telemetry::current_parent_traceparent()` /
  `ambient_capsem_trace_id()`. No OpenTelemetry runtime dep this
  sprint -- traceparent is a structured field in JSON for now;
  tracing-opentelemetry layer is a future-sprint addition. Adding it
  later is purely an additional `Layer` on the existing subscriber.

### Changed (observability)
- **Silent IPC drops in suspend/resume/exec/file paths now log at
  `target="ipc"`.** ~50 sites across `capsem-process/src/{vsock,ipc,
  main,terminal,job_store}.rs`, `capsem-service/src/main.rs`, and
  `capsem/src/main.rs` were `let _ = X.send(...)` -- a closed receiver
  silently swallowed the message with no trace. New `try_send!` macro
  in `capsem-core::macros` wraps every IPC/vsock send and emits a
  `tracing::warn!(target = "ipc", channel, error)` line on failure.
  Filter with `RUST_LOG=ipc=warn` to see only dropped-message events.
  Cleanup paths where a closed receiver is the documented design
  (e.g. broadcast publish into `TerminalOutputQueue`) keep the bare
  `let _ = ` and carry an inline `// channel-closed-ok: <reason>`
  marker so the audit grep can exclude them.

### Changed (persistent overlay)
- **EXT4 journal re-enabled on the persistent overlay-upper.** Previously
  formatted with `mke2fs -O ^has_journal`; switched to default
  `has_journal` and mount with `data=ordered`. Costs ~5-10% IOPS;
  enables metadata replay on resume so directory listings stay
  consistent after suspend/resume cycles where in-flight metadata
  writes hadn't been flushed. Verified via `tune2fs -l /dev/loop0`:
  `Filesystem features: has_journal ... metadata_csum`. Standard
  suspend/resume + heavy-churn directory listing now both work.
  (Heavy-churn DATA reads of a subset of files still hit
  `Input/output error` -- that's the loop-device-io-after-resume
  sprint's remaining work, fixable only by moving rootfs.img off
  VirtioFS to a real VZ block device.)

### Fixed (lifecycle)
- **Guest-initiated `shutdown` left persistent VMs marked Defunct
  instead of Stopped.** The lifecycle path (`capsem-sysutil shutdown`
  -> vsock:5004 -> `ProcessToService::ShutdownRequested`) had no
  service-side listener; the process just sent `Shutdown` to itself
  and exited cleanly. The cleanup task interpreted "instance still in
  the map at exit" as `unexpected_exit=true` and flipped the registry
  to `defunct`, so `capsem list` showed Defunct and the test
  `test_guest_shutdown_preserves_persistent_and_resume` failed.
  Distinguish: a clean `ExitStatus::success()` is graceful regardless
  of who initiated it; only non-zero exit / signal kill is a crash.

### Fixed (suspend/resume durability)
- **`cd /root && ls` after `capsem resume` failed with "cannot open
  directory '.': No such file or directory".** Apple VZ writes to the
  persistent overlay's `rootfs.img` were buffered in macOS's APFS page
  cache. After `save_state`, capsem-process exited before APFS flushed,
  so the next boot read a stale `rootfs.img` and the EXT4 overlay-upper
  served stale inodes -- the cwd handle in the resumed shell pointed at
  garbage. Three-stage flush now layered on suspend:
  1. Guest agent: `sync()` + `BLKFLSBUF` + `fsync(/dev/loop0)` (existed).
  2. Guest agent: `fsync(/mnt/shared/system/rootfs.img)` -- sends
     `FUSE_FSYNC` over VirtioFS so the host VirtioFS daemon flushes its
     own buffered writes against the real macOS file (NEW).
  3. Host capsem-process: `sync_all()` on `rootfs.img` after
     `save_state` returns -- catches APFS dirty pages (NEW).
  Confirmed end-to-end against the live service: simple suspend/resume
  + `cd /root && ls` works; suspend with churn across `/tmp /var /opt
  /etc /usr/local` survives; file *contents* on the EXT4 overlay are
  durable. Heavy directory churn (~50 new entries per dir then
  immediate suspend) can still leave EXT4 directory data blocks with
  stale checksums on resume -- file reads succeed but `readdir`
  returns I/O error. Tracked in
  `sprints/loop-device-io-after-resume/ISSUE.md`; the next step is
  forcing an `fsync` on each parent directory inside the guest before
  signalling SnapshotReady.
- **Failed suspend left VM marked "Suspended" with a corrupt checkpoint.**
  When `with_quiescence` failed (timeout, channel closed) the spawn task
  ignored the error, sent `StateChanged{Suspended}` anyway, and exited
  with code 0. The service then marked the VM as suspended; the next
  resume cold-booted against the half-written rootfs.img and kernel-
  panicked with `EXT4-fs error inode #N: iget: checksum invalid` ->
  `overlayfs failed`. Fix: only send the Suspended state and `exit(0)`
  when the operation actually succeeded; on failure, log the error and
  `exit(1)` so the service treats it as a crash and does not write the
  checkpoint marker.
- **Silent IPC connection close on protocol mismatch.** Two binaries
  built across an enum-variant addition (`StopTerminalStream`) talked
  past each other; the receive side closed the connection silently with
  the decoder error swallowed. Fix: log the rx error at `warn` level so
  the next protocol-skew bug surfaces in the first run instead of
  presenting as a "guest doesn't respond" timeout.

### Fixed (capsem shell)
- **Terminal garbage on shell exit.** Pressing Ctrl-C / typing `exit` in
  `capsem shell` could leave the user's parent terminal flooded with
  binary garbage (MessagePack frames -- `bootconfig`, `epoch_secs`,
  `Pong` repeated). Two compounding bugs:
  1. `output_task` (the spawned reader of `ProcessToService` IPC frames)
     was never aborted on exit. tokio `JoinHandle::drop` does NOT cancel
     -- the task lived on, kept holding `stdout`, and any in-flight
     `TerminalOutput` frame wrote to the user's now-cooked-mode shell.
  2. The host-side `capsem-process` kept queuing `TerminalOutput` for the
     dropped IPC connection because the client never told it to stop.
- Fix: `run_shell` now sends a new `ServiceToProcess::StopTerminalStream`
  before exit, aborts the local task, drops the IPC writer, and writes a
  minimal terminal reset (`\x1b[0m\x1b[?25h\r\n` -- SGR reset, show
  cursor, CRLF; deliberately no alt-screen toggle or screen clear so
  scrollback is preserved).
- Defenses: `capsem_proto::looks_like_ipc_frame` ships a detector for the
  `to_vec_named` adjacently-tagged enum prefix that produced the garbage;
  `capsem-process` calls it on every `TerminalOutput` payload and emits a
  loud `warn!` if a leak ever resurfaces. 15 unit tests in
  `crates/capsem/src/shell_exit/tests.rs` pin: the reset sequence shape,
  every variant of both `HostToGuest` and `GuestToHost` matching the
  detector, no false positives on ANSI/UTF-8/scrollback content, and
  the load-bearing tokio behavior (`JoinHandle::drop` does not cancel,
  `JoinHandle::abort` does).

### Changed (kernel)
- `guest/config/build.toml` ships `kernel_branch = "auto"` instead of a
  hardcoded `"6.6"`. `resolve_kernel_version("auto")` queries
  kernel.org/releases.json and picks the newest non-EOL longterm branch's
  latest patch (today: `6.18.26`). Pin to a specific branch by setting
  `kernel_branch = "X.Y"` (e.g. `"6.6"`) for reproducibility / security
  freeze. Killed the duplicated `"6.6"` literal in `models.py` /
  `scaffold.py` -- single source of truth is now `build.toml`.

### Changed (bootstrap)
- `bootstrap.sh` moved to the repo root (was `scripts/bootstrap.sh`).
- Phase 1 now auto-installs `rustup` (sh.rustup.rs) and `just` (just.systems
  -> `~/.local/bin`) instead of printing hints and bailing.
- Phase 2 auto-installs `uv` (astral.sh), `pnpm` (brew on macOS,
  get.pnpm.io on Linux), and on macOS `colima` + `docker` + `docker-buildx`
  with Rosetta-enabled VM start (`colima start --vm-type vz --vz-rosetta
  --memory 8 --cpu 8`). Linux docker stays manual (distro-specific, sudo,
  group, daemon -- prints clear apt/dnf hints instead).
- Each install gates on a `[Y/n]` prompt; **Enter accepts** (Y is the
  default). `--yes` and non-tty input both auto-accept for CI.
- Stopped silencing every installer (`--quiet`, `>/dev/null`). Real errors
  were getting swallowed -- `uv sync` failures showed up as a mystery
  `exit 1` with no diagnostic.
- Closing message no longer tells you to run `just build-assets` (it
  already ran as part of doctor's auto-fix in Phase 3).

### Fixed (bootstrap)
- `cargo install cargo-tauri` was wrong -- the crate is `tauri-cli` (the
  binary it produces is `cargo-tauri`). Fixed in `scripts/doctor-common.sh`.

### Fixed
- **Asset download URL.** `download_missing_assets` built the URL from the
  asset version (`v2026.0424.1`) instead of the binary version (`v1.0.{ts}`),
  so every fresh install 404'd against the GitHub Release. Releases are tagged
  by binary version; the asset version lives only inside the manifest.
- **Manifest schema mismatch.** The CI release pipeline writes
  `binaries.releases.<v> = {version, files}`, but the Rust `BinaryRelease`
  struct required `{date, min_assets}`. Every published manifest was
  unparseable -- the binary couldn't even *get* to the URL builder before
  failing. Made `date` / `min_assets` / `min_binary` optional, added
  `version` / `files` to round-trip pkg/deb metadata. `pick_asset_version`
  treats empty `min_assets` as "no constraint" and falls back to
  `assets.current`.
- **Removed broken Makefile.** The legacy Makefile bypassed `_pack-initrd`,
  `gen_manifest`, and `create_hash_assets`, so `make` produced a binary that
  couldn't resolve any VM asset at boot. Use `just` for everything.

### Added (defenses)
- Pinned the asset URL contract in `asset_download_url()` with unit tests so
  future drift between the downloader and `release.yaml`'s upload step
  (`gh release upload "$f#${arch}-${base}"`) is caught at compile time.
- `verify-release-downloads` post-flight job: after every release, downloads
  the published manifest, curl-checks every `<base>/v<tag>/<arch>-<name>` URL
  is reachable, AND runs the just-released binary's `capsem update --assets`
  against real GitHub. Closes the gap that hid the URL bug for one release.
- Fixed `tests/capsem-install/test_asset_download.py`: fake release dir was
  at `v<asset_version>` (mirroring the same buggy mental model as the code).
  Now at `v<binary_version>` so it actually models GitHub.
- Dropped the `_build-host` dependency from `just test-install`. The recipe
  builds host crates inside the container that has the GTK/glib -dev libs;
  the duplicate runner-side build was failing on Ubuntu 24.04 arm64 (no
  libglib2.0-dev), which masked the asset-URL bug because the e2e never ran.

### Security (frontend deps)
- **`marked` 18.0.0 -> 18.0.3** (GHSA-6v9c-7cg6-27q7, HIGH): infinite recursion
  in tokenizer. Direct dev dep; bumped to `^18.0.2`, lockfile resolved 18.0.3.
- **`postcss` >=8.5.10 enforced via pnpm override** (GHSA-qx2v-qp2m-jg93,
  MODERATE): XSS via unescaped `</style>` in CSS stringify output. Pulled
  transitively through `@sveltejs/vite-plugin-svelte > vite > postcss`.
  Override forces every node in the lockfile to >=8.5.10.

### Added (CI)
- **`just audit` recipe.** Fast standalone gate (cargo audit + pnpm audit only,
  no test/build). `just test` Stage 1 already runs both audits; this is the
  pre-push check that doesn't require ~15 min of full-suite work first.
- **`test-linux` job no longer hard-fails when `/dev/kvm` is missing.** The
  "Enable KVM" step is now `continue-on-error: true`, and the verification
  step emits a workflow warning instead of `::error::` + `exit 1`. Hosted
  ARM runners do not always expose nested virt; the compile + non-KVM unit
  tests still run, and real-KVM coverage runs in the release pipeline.
  Workflow comments link future readers to `sprints/done/ci-green` so the
  hard-fail doesn't get reintroduced.

### Changed (Colima default)
- **Bumped Colima default RAM from 8 GB to 16 GB** across `bootstrap.sh`,
  `scripts/doctor-macos.sh`, three skills (`dev-setup`, `dev-start`,
  `build-images`), and four docs pages (architecture/build-system,
  architecture/custom-images, development/getting-started, development/stack).
  The Tauri install-test cold build (`just test-install`) blew past 8 GB
  during cargo compile of the capsem-mcp crates and SIGTERM'd at exit 143.
  16 GB is the recommended floor; 12 GB is the absolute minimum.
- Bumped `@tauri-apps/api` from `^2.10.1` to `^2.11.0` to match the Rust
  `tauri` v2.11.0 crate (`cargo tauri build` refuses mismatched majors/minors).

### Fixed (install-test fixture)
- `tests/capsem-install/test_asset_download.py` hardcoded `serve_dir/v1.0.1/`
  for the fake release dir, but the installed binary builds asset URLs from
  its own `CARGO_PKG_VERSION` (e.g. `v1.0.1777065213`). Every run inside the
  install-test container 404'd. Replaced with `f"v{_binary_version()}"` -- a
  helper that runs `capsem --version` once and uses the result -- so the
  fixture always matches the binary under test, regardless of release tag.

### Deferred
- **Orthogonal asset/binary release cadence** (separate tag scheme + workflow
  for asset-only bumps) is still postponed -- revisit after this URL fix
  ships. The defenses above are designed to also guard the future split.
