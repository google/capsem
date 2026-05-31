# Security Event CEL Plan

## What

Rework Capsem CEL policy evaluation so live detection and enforcement evaluate
rules directly against canonical `SecurityEvent` data.

The first target is model/MCP policy enforcement because that is where the old
policy-v2 audit exposed the likely mismatch: Capsem already records rich model
request/response/tool-call evidence, but live enforcement may still translate
some model rules into HTTP request/response body checks.

## Why

The user-facing mental model is correct and should become the implementation:

`engine parse/normalize -> SecurityEvent -> pre-transform -> CEL rules -> block/detect/log`

If model rules are secretly rewritten into HTTP predicates, or if any event
family only works for detection but not enforcement, the system has two
abstractions:

- The security/logging abstraction says `model.response`, `model.request`,
  `model.tool_calls`, `mcp.name`.
- The live enforcement abstraction may be checking `http.response.body.text` or
  `http.request.body.text`.
- Some event families may be huntable only through hand-authored CEL while
  Sigma/Detection IR or enforcement cannot address them.

That is dangerous because tests can pass by exercising the wrong abstraction,
and provider/parser hardening may not affect the path that actually blocks.

## Key Decisions

- Treat `origin/main` as authoritative.
- Do not port old `policy_v2_model.rs` as a second policy engine.
- Do not bulk merge the old branch.
- Remove model-to-HTTP lowering from live enforcement.
- Put any missing fields into the canonical event/projection layer.
- Keep provider-specific parsing inside network/provider adapters, not policy
  compilation.
- Make regression tests assert the abstraction, not just the behavior.
- Add fast, accurate benchmarks for the security spine before making
  performance claims.
- Treat every `SecurityEvent` family as part of the security surface for both
  detection and enforcement.
- Treat event identity as a closed typed contract: producers, profile callback
  validation, CEL guards, and SQLite storage must consume the same
  `SecurityEventType` registry.

## Files To Inspect First

- `/Users/elie/.codex/worktrees/5fcb/capsem/crates/capsem-process/src/security_engine/`
- `/Users/elie/.codex/worktrees/5fcb/capsem/crates/capsem-process/src/mcp_runtime.rs`
- `/Users/elie/.codex/worktrees/5fcb/capsem/crates/capsem-core/src/net/mitm_proxy/mod.rs`
- `/Users/elie/.codex/worktrees/5fcb/capsem/crates/capsem-core/src/net/mitm_proxy/telemetry_hook.rs`
- `/Users/elie/.codex/worktrees/5fcb/capsem/crates/capsem-proto/src/policy_context.rs`
- `/Users/elie/.codex/worktrees/5fcb/capsem/crates/capsem-security-engine/src/lib.rs`
- `/Users/elie/.codex/worktrees/5fcb/capsem/crates/capsem-security-engine/src/detection_ir.rs`
- `/Users/elie/.codex/worktrees/5fcb/capsem/crates/capsem-network-engine`
- `/Users/elie/.codex/worktrees/5fcb/capsem/crates/capsem-security-engine/benches/security_engine_cel.rs`
- `/Users/elie/.codex/worktrees/5fcb/capsem/crates/capsem-security-engine/benches/detection_ir.rs`
- `/Users/elie/.codex/worktrees/5fcb/capsem/justfile`
- `/Users/elie/.codex/worktrees/5fcb/capsem/scripts/integration_test.py`

## T0: Event-Flow Map

Trace every live security decision entrypoint:

- HTTP request allow/block.
- HTTP response allow/block.
- DNS allow/block.
- MCP request/tool invocation.
- Model request.
- Model response.
- Provider-emitted tool call.
- Provider-emitted tool result.

For each callback, record:

- The parsed source object.
- The `SecurityEvent` variant or fields produced.
- The CEL context actually evaluated.
- Whether logging/evidence receives the same object.

Exit criteria: a short callback-to-event map in `tracker.md`, with every model
and MCP callback either canonical or marked as a blocker.

## T1: Canonical Projection Contract

Define the CEL-visible shape from `SecurityEvent` in one place for both
detection and enforcement.

The contract should cover at least:

- `common.event_type`
- `network.*` or `http.*` fields that remain valid for network events.
- `dns.*`
- `mcp.name`, `mcp.method`, `mcp.tool`, `mcp.arguments`
- `model.provider`, `model.name`, `model.request`, `model.response`
- `model.tool_calls`, `model.tool_results`
- `credential.*`
- `vm.*`
- `conversation.*`
- `snapshot.*`
- Existing AI evidence/session identifiers needed for audit correlation.
- Semantic search verbs on policy objects so rules can use concise
  `contains()`, `match()`, and `matches()` checks across body text, arguments,
  file paths, DNS names, HTTP fields, and model/MCP tool metadata without
  writing CEL list/map closures for the common case.

Exit criteria: unit/contract tests prove CEL can evaluate every emitted
`SecurityEvent` family from canonical events, for detection and enforcement,
without using HTTP body fields except for true HTTP events.

Status: complete for the current event-family surface. The shared projection
now exposes credential, VM, conversation, and snapshot roots in addition to the
existing HTTP, DNS, MCP, model, file, process, and profile roots. Detection IR
lowering also recognizes those canonical roots, including `snapshot`.
MCP request arguments and model tool-call arguments are exposed as searchable
body contexts, file activity has an optional searchable content body, and
`contains()`/`match()`/`matches()` now recursively search canonical policy
objects.

## T7: Typed Event Identity Contract

Replace stringly event identity with a closed `SecurityEventType` contract that
network telemetry and other producer lanes can depend on.

Required behavior:

- `SecurityEventCommon.event_type` is typed, not an arbitrary string.
- The type exposes `as_str()`, `family()`, strict parsing, and serde-as-string.
- Runtime constructors assert the event type belongs to the subject family.
- Rule callback validation consumes the same contract as runtime guards.
- New SQLite `security_events` tables reject unknown event types and
  family/type mismatches.
- Stale pseudo-callbacks such as `dns.response`, `http.read`, `http.write`,
  `mcp.tool_call`, and `credential.read` are rejected or normalized into a
  real event type plus CEL predicate.

Status: complete for the current contract. `model.tool_call` and
`model.tool_response` are intentionally reserved future typed callbacks;
read/write distinctions now live as CEL predicates over `http.request.method`
instead of fake event types.

## T2: Live Enforcement Rewire

Change MITM/provider/MCP enforcement and detection callbacks to build and
evaluate canonical events.

Important behavior:

- Model request blocking evaluates a `model.request` event.
- Model response blocking evaluates a `model.response` event.
- Provider-emitted tool calls evaluate a model tool-call event.
- MCP blocking evaluates an MCP event.
- Network HTTP blocking still evaluates network/HTTP events.
- Detection findings and enforcement decisions see the same canonical event.

Exit criteria: the live callbacks call the shared security engine with
canonical events, and provider classification metadata is carried on the event
instead of re-derived in CEL lowering code.

Status: in progress. The MITM request path now evaluates a canonical
`model.request` event, derived from the provider-normalized request body, before
upstream dispatch. The MITM response path now evaluates a canonical
`model.response` event, derived from the decompressed provider response and
parsed SSE summary, before guest delivery. Provider-emitted tool-call blocking
is proven on the same canonical model response event through
`model.request.tool_calls[...]`. Request-side model tool-result blocking is
proven before upstream dispatch on the canonical `model.request` event through
`model.response.tool_results[...]`. The framed MCP path now evaluates
canonical `mcp.request` and `mcp.response` events over parsed JSON-RPC frames
and response bodies.

## T3: Rule Compilation Cleanup

Remove rule compilation that rewrites semantic policy resources into HTTP
predicates.

Known suspect patterns:

- `model.request` -> `common.event_type == 'http.request'`
- `model.response` -> `common.event_type == 'http.response'`
- `model.tool_call` -> HTTP response-body text matching
- `model_rule_condition` style body-text lowering
- MCP-specific condition mini-parsers or local decision providers that bypass
  `SecurityEvent -> PolicyContext -> CEL`.
- Builtin domain allow/block environment authority that acts as a second
  network policy engine beside CEL.
- Broad HTTP/DNS/model/MCP SecurityEngine installation hidden inside
  `mcp_runtime.rs`.

Exit criteria: no model or MCP rule is compiled into an HTTP predicate. If a
rule targets `model.*` or `mcp.*`, the evaluated event must contain that
canonical family.

Status: the process-side security runtime now lives under
`crates/capsem-process/src/security_engine/`, split into rule compilation,
match recording, guest boot config, and MCP settings extraction. `mcp_runtime.rs`
is scoped back to MCP aggregator/server wiring plus builtin MCP env assembly.

## T4: Regression Tests

Add tests that would fail under the old abstraction:

- A `model.response` rule blocks when `model.response.text` matches.
- The same rule does not require `http.response.body.text`.
- A provider-emitted tool-call rule blocks on canonical tool-call fields.
- An MCP rule blocks on canonical `mcp.*` fields.
- Default HTTP, DNS, and MCP settings rules compile into the runtime
  SecurityEngine, priority-0 allow rules win over catch-all blocks, and
  non-matching events fall through to the defaults.
- HTTP rules still work for true HTTP events.
- Detection and enforcement both work for every emitted `SecurityEvent`
  family.
- Semantic object-search rules work over HTTP, DNS, file path/content, MCP
  argument, model tool-call argument, and model response objects without
  closure boilerplate.
- Malformed/compressed/streamed provider responses cannot bypass canonical
  model extraction.

Exit criteria: tests prove both positive behavior and the absence of
model-to-HTTP dependency.

## T5: Integration Proof

Extend or add integration proof around `scripts/integration_test.py` only after
the unit/contract path is stable.

The proof should show:

- A real or fixture-backed model response produces a canonical security event.
- CEL rules over `model.response` block and detect as configured.
- Non-network security events remain enforceable and detectable through the same
  engine path.
- The logged/session event matches the enforcement event.
- Current HTTP/DNS/MCP behavior is not regressed.

Exit criteria: reproducible integration command recorded in `tracker.md`, with
coverage debt explicit if VM/provider setup is not available locally.

## T6: Benchmark Proof

Measure the security spine with two benchmark tiers.

Fast microbench gate:

- `cargo bench -p capsem-security-engine --bench security_engine_cel`
- `cargo bench -p capsem-security-engine --bench detection_ir`

Current fast coverage added:

- `security_engine_cel` projects every current event family from
  `SecurityEvent` to `PolicyContext`.
- `security_engine_cel` evaluates detection and enforcement rules across every
  current family, plus a mixed-family 100-rule model tool-call/result case.
- `security_engine_cel` benchmarks runtime enforcement backtest, detection
  backtest, and 100-rule/100-event detection hunt through the engine API.
- `detection_ir` lowers every Detection IR family to CEL, lowers indexed
  model tool-call/result paths, lowers plus compiles 100 mixed-family rules,
  and measures direct IR matching plus lowered-CEL matching against canonical
  `SecurityEvent`.

Full artifact gate:

- `just benchmark`
- `just benchmark-compare` when comparing committed artifacts.

Coverage targets:

- `SecurityEvent -> PolicyContext` projection for every emitted event family:
  HTTP/network, DNS, MCP, model, file, process, credential, VM, profile,
  conversation, snapshot, and any new typed events.
- Enforcement CEL evaluation for inline blockable events across all enforceable
  event families.
- Detection CEL evaluation for one-rule, mixed-family, every-family, and
  100-rule cases.
- Sigma/Detection IR parse, validate, lower-to-CEL, lower-plus-compile, and
  evaluate paths across every family the Detection IR schema claims to support.
- Detection hunt over inline events and session-reconstructed events.
- MITM request/response callback overhead after canonical event construction.
- Model provider parsing/extraction for streamed, compressed, and provider
  tool-call responses.

Accuracy requirements:

- Benchmarks must measure the production code path, not duplicated test-only
  shortcuts.
- Inputs must include realistic small, medium, and adversarial payloads.
- Results must report the benchmark group/function names and event families used
  for any claim.
- If the full VM-originated `just benchmark` gate cannot run locally, record the
  reason and keep performance release debt open.

Speed requirements:

- Fast microbench gates should be suitable for routine sprint verification.
- Full `just benchmark` is the release/artifact gate, not the inner loop.
- Avoid adding benchmarks that depend on network availability, live providers,
  or unbounded fixture sizes.

Exit criteria: benchmark coverage is updated for every touched security-spine
path, fast benchmark commands are recorded in `tracker.md`, and full benchmark
artifact commands are recorded before release claims.

## Testing Matrix

| Slice | Unit/Contract | Functional | Adversarial | E2E/VM | Telemetry | Performance |
| --- | --- | --- | --- | --- | --- | --- |
| T0 | N/A | Manual map | N/A | N/A | Manual map | N/A |
| T1 | CEL projection tests | Security engine API | Missing/null fields | N/A | Event serialization | N/A |
| T2 | Callback construction tests | MITM/MCP/model detection + enforcement | Streaming/chunk/provider edge cases | Optional VM | Same-event logging | N/A |
| T3 | Compiler tests | Rule install/apply | Wrong-resource rules | N/A | N/A | N/A |
| T4 | Regression suite | Production-facing APIs | Bypass attempts | Optional VM | Event assertions | N/A |
| T5 | Existing unit suites | Integration script | Provider fixture attacks | VM if available | Session DB check | Smoke only |
| T6 | Bench harness compiles | Bench production paths | Pathological payloads | `just benchmark` if available | Artifact archive | Fast microbench + full artifact gate |

## Done Means

- Detection and enforcement CEL rules evaluate against canonical
  `SecurityEvent` fields.
- Every emitted `SecurityEvent` family has an explicit CEL projection contract,
  test coverage, and benchmark coverage or recorded release-blocking debt.
- No live model/MCP enforcement path depends on synthetic HTTP-body lowering.
- Logging/session evidence and enforcement observe the same event abstraction.
- Regression tests fail if someone reintroduces model-to-HTTP lowering.
- Benchmarks cover CEL enforcement, CEL detection, Sigma/Detection IR lowering,
  hunting, and touched MITM/model callback paths across the full event surface.
- Coverage debt is visible in `tracker.md`; no release claim hides missing E2E
  or benchmark proof.
