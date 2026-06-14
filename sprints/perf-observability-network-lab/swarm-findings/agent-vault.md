# Finding: Infisical agent-vault Patterns

Status: completed

Agent: Erdos (`019e999f-7099-7fc2-824d-6595ee10373f`), Firecrawl (`019e99a0-30ac-7212-a4d3-81d1b362c11d`, not relied on), local clone review

Upstream revision reviewed: `234dbf0d27d4749b35690c91713fd2789c810cd7`

## Scope

Review `https://github.com/Infisical/agent-vault` for patterns relevant to
Capsem's credential broker and security-event pipeline:

- Secret interception or brokering for AI agents.
- Token/reference/substitution model.
- Secret storage boundary.
- Audit/logging behavior.
- Policy, allowlist, or approval controls.
- Local agent/runtime integration.
- Tests or docs proving the model.

Capsem architecture remains authoritative: one security-event/CEL rail,
BLAKE3 substitution references, Keychain-backed secrets, DB logging through the
logger path, and provider-owned rules compiled into Policy V2.

## Concise Summary

`agent-vault` is a standalone HTTP/HTTPS forward proxy and vault. It keeps real
credentials outside the agent, authenticates the agent to the proxy with a
session token, matches the outbound request against service rules, decrypts the
credential server-side, injects or substitutes the credential into declared
request surfaces, forwards upstream, and records secret-free request metadata.

The useful Capsem port is conceptual: explicit substitution surfaces, injected
credentials always winning over client-provided auth, broker-scoped headers
never forwarding, strict no-secret audit tests, actionable deny metadata, and
scoped actor/session metadata. We should not port its separate service matcher,
request-log sink, proposal engine, or vault/config authority as second engines.

## Key Ideas With Upstream Anchors

| Area | Upstream paths/functions | Pattern | Capsem relevance |
| --- | --- | --- | --- |
| Brokered credential injection | `internal/brokercore/credential.go:33` `InjectResult`; `StoreCredentialProvider.Inject` at `internal/brokercore/credential.go:101` | Request match resolves safe metadata first, decrypts credentials only in the broker, returns headers/substitutions with comments marking secret fields. | Capsem broker action should produce a mutated security event/boundary materialization with secret-bearing values kept out of logs and DB rows. |
| Fail-closed service lookup | `internal/brokercore/credential.go:101-143` | Store/config errors become no-service failures; unmatched can be deny or passthrough by policy. | Capsem corp/provider rules should fail closed for broker/action lookup failures when enforcement says the credential is required. |
| Injection precedence and header stripping | `internal/brokercore/brokercore.go:30-120` `ApplyInjection`, `IsBrokerScopedRequestHeader`, `ShouldStripResponseHeader` | Hop-by-hop and broker-scoped headers are stripped; injected headers replace client-supplied auth slots. `Set-Cookie` is stripped from upstream responses. | Port as tests/invariants in the HTTP action materializer: Capsem/session auth headers never go upstream, and brokered auth wins. |
| Surface-scoped substitutions | `internal/broker/broker.go:55-61`, `SubstitutionSurfaces` at `internal/broker/broker.go:103-110`; runtime in `internal/brokercore/substitution.go:14-155` | Placeholder replacement is only allowed in declared surfaces: path, query, header, body, websocket. Body replacement is content-type-aware; header replacement rejects CRLF. | Capsem credential broker actions should carry explicit target surfaces and encode per surface. The security event should log surface/ref/hash metadata, never the raw value. |
| Proxy hot path order | `internal/mitm/forward.go:183-415` | Build request event, authenticate/session-resolve, rate-limit, inject, materialize body only when needed, apply substitutions, forward, emit terminal log. | Good ordering model for Capsem: parse to security event, evaluate rules/actions, mutate event/materialization, forward, then emit terminal outcome through logger. |
| Actor/vault scoping | `internal/brokercore/session.go:20-150` | Proxy scope binds actor id, vault id/name, role; scoped sessions cannot be silently retargeted. | Capsem session/workspace/provider context should be explicit fields on security events and broker actions. No implicit provider retargeting. |
| Secret-free audit shape | `internal/brokercore/logging.go`; `internal/requestlog/sink.go:17-108`; `internal/store/migrations/039_request_logs.sql` | Logs persist method, host, path, matched service, credential key names, status, latency, error. No header values, query strings, or bodies. | Capsem should log provider id, credential ref/BLAKE3, action ids, surfaces, decision, status, and latency; never raw credential, raw auth header, raw query secret, or body secret. |
| No-secret tests | `internal/brokercore/logging_test.go`; `internal/brokercore/substitution_test.go`; `internal/mitm/forward_test.go`; `internal/mitm/websocket_test.go` | Tests prove no raw credential in logs, injected auth wins, broker-scoped headers strip, substitution encoding/scoping, WebSocket text-frame substitution constraints. | Port test categories directly into Capsem credential-broker/security-event tests. |
| OAuth refresh singleflight | `internal/oauth/refresher.go`; `maybeRefreshOAuth` at `internal/brokercore/credential.go:236` | Concurrent refresh for same vault/key is deduplicated. | Useful for Capsem Keychain-backed OAuth refresh if multiple model calls hit an expiring credential. Keep it inside broker action, logged through security events. |
| Network guard | `internal/netguard/netguard.go:93-202` | Blocks metadata/private ranges by default, checks all resolved IPs, dials the validated IP to avoid DNS rebinding. | Capsem should express this as DNS/HTTP security-event policy and network-engine dial guard, not a separate broker-side allowlist. |
| Actionable deny hints | `ForbiddenHintBody` at `internal/brokercore/brokercore.go:131-149`; docs `docs/agents/protocol.mdx` | Denial responses tell the agent what access/proposal path is available. | Capsem deny events can carry remediation metadata for UI/settings suggestions, while enforcement remains CEL/security-event based. |

## What Capsem Should Port Conceptually

1. Add explicit credential-broker substitution surfaces to the action contract:
   `http.header`, `http.query`, `http.path`, `http.body`, `websocket.text`,
   and later model/file surfaces. Each surface must define encoding rules.

2. Log broker results as first-party security-event metadata:
   provider id, endpoint id, credential name/ref, BLAKE3 substitution ref,
   action id, surface list, match rule id, decision, actor/session/workspace,
   status, and latency. No raw secret, no raw replaced header, no raw secret
   query/body.

3. Add “injected value wins” and “broker/session auth never forwards” tests to
   Capsem HTTP materialization. This is high value because prompt-injected code
   may try to override `Authorization` or leak Capsem internal auth headers.

4. Add surface-scoped substitution tests:
   path encodes path segments, query URL-encodes, header rejects CR/LF, body
   handles JSON and form encoding, multipart/compressed bodies are skipped or
   denied with an explicit event, and WebSocket substitutions only apply to
   declared text frames when WebSocket work lands.

5. Add no-secret audit tests across all sinks:
   security events, `session.db`, logger rows, CLI `capsem log`, debug spans,
   denial responses, and panic/error paths.

6. Add credential-missing diagnostic metadata:
   when a rule matches but a credential ref cannot resolve, the event should
   name the provider/rule/credential ref/BLAKE3 placeholder and emit a strong
   warning/error without exposing the secret.

7. Add scoped runtime identity fields:
   session id, workspace id, actor/tool/process context, provider id, endpoint
   id, and credential ref should be on the event/action input so broker actions
   cannot silently retarget a different provider or endpoint.

8. Consider OAuth refresh singleflight inside the credential broker, keyed by
   provider/endpoint/credential ref, with refresh attempts and failures emitted
   as security events.

9. Keep actionable denial metadata, but make it a field on Capsem denial
   events and UI responses. It should suggest the settings/rule/provider fix;
   it must not bypass enforcement.

## What Capsem Should Avoid

- Do not port Agent Vault's `Service` host/path matcher as a second engine.
  In Capsem this belongs in provider-owned rules compiled into Policy V2/CEL.

- Do not add a separate `requestlog` package/sink that can drop security truth.
  Agent Vault drops request logs under backpressure; Capsem security events
  must go through the logger-owned DB writer with explicit loss/backpressure
  semantics and counters.

- Do not port passthrough as a broad default for credentialed provider traffic.
  Capsem can allow traffic, but credential brokering/enforcement should be
  explicit and auditable.

- Do not copy their proposal engine as-is. Capsem can later add ask/approve UI,
  but requests to change provider/credential policy must compile into our
  settings/provider-rule/security-action model.

- Do not store encrypted credential blobs in Capsem settings or session DB.
  Capsem's durable secret boundary should remain Keychain-backed, with BLAKE3
  refs and security-event metadata in DB rows.

- Do not make external secret-store sync an alternate source of truth before
  the broker/action rail is proven. If added later, it should populate
  Keychain/provider refs through audited events.

## Risks And Open Questions

- P1: Capsem still needs runtime callbacks/actions for credential and file
  import/export rules if provider profiles are expected to cover those event
  families, not only HTTP/DNS/model.

- P1: Body and WebSocket substitution can create large materialization or
  streaming hazards. Capsem should require explicit size caps, encoding rules,
  and denial events before enabling those surfaces.

- P1: If debug OTEL records raw headers/query/body by default, it can violate
  the BLAKE3 reference-only logging model. Span attributes need allowlists.

- P2: OAuth capture/refresh needs provider-specific edge cases, but the
  singleflight refresh pattern is sound if it stays inside broker actions.

- P2: “Actionable deny hints” are useful for UX, but they must not become a
  self-service rule mutation path that bypasses corp priority/locks.

- P2: Network guard concepts overlap with Capsem's network engine. Use them as
  invariants/tests for the network-engine dial path, not as a new side guard in
  credential broker code.

## Recommended Implementation Order

1. Freeze Capsem credential-broker action contract: input security event,
   matched provider rule, credential ref, BLAKE3 substitution ref, surface list,
   and mutated security event/materialization output.

2. Add tests for no raw secret across security events, logger DB rows,
   `capsem log`, spans, denial responses, and error paths.

3. Add HTTP header/query/path substitution materialization with encoding,
   injected-auth-wins, and broker-scoped-header-strip tests.

4. Add credential-missing and credential-capture events with provider/rule/ref
   metadata and strong warning/error severity.

5. Add body substitution only after local-lab tests cover JSON/form/multipart,
   compressed body behavior, size caps, and raw-secret redaction.

6. Add WebSocket text-frame substitution during the WebSocket sprint, with
   frame-size, compression, fragmentation, and no-secret tests.

7. Add OAuth refresh/capture as a broker action with singleflight and
   Keychain-backed token updates, emitted through the security-event rail.

8. Add UI/settings remediation hints sourced from denial/security events,
   preserving corp priority/locks and avoiding a parallel proposal/config
   authority.

## Tests Or Evidence

- Reviewed upstream clone at `234dbf0d27d4749b35690c91713fd2789c810cd7`.
- Static evidence inspected from:
  - `README.md`
  - `docs/learn/security.mdx`
  - `docs/learn/services.mdx`
  - `docs/agents/protocol.mdx`
  - `internal/brokercore/*`
  - `internal/broker/broker.go`
  - `internal/mitm/forward.go`
  - `internal/mitm/websocket.go`
  - `internal/requestlog/*`
  - `internal/store/migrations/039_request_logs.sql`
  - `internal/oauth/*`
  - `internal/netguard/*`
  - `internal/proposal/*`
- Attempted local upstream tests:
  `go test ./internal/broker ./internal/brokercore ./internal/mitm ./internal/netguard ./internal/oauth ./internal/proposal ./internal/requestlog`
  but this host lacks `go` on `PATH`, so no upstream tests were executed
  locally.
- Firecrawl review agent `019e99a0-30ac-7212-a4d3-81d1b362c11d` was polled
  repeatedly and was still `processing`; it was not relied on for the captured
  finding.
