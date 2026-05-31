# Sprint: security-event

## Tasks

- [x] Create `security-event` branch from `origin/main`.
- [x] Create sprint plan/tracker/master docs.
- [x] T0: Map every live enforcement callback to the event shape it evaluates.
- [x] T0a: Map every live detection callback/source to the event shape it evaluates.
- [x] T1: Define and test the canonical `SecurityEvent` CEL projection for every event family.
- [x] T2: Rewire live model/MCP detection and enforcement callbacks to canonical events.
  Model request, model response, provider-emitted tool-call, and live MCP
  request/response enforcement are wired; request-side model tool-result
  enforcement is now proven before upstream dispatch.
- [x] T3: Remove model/MCP-to-HTTP rule lowering.
- [ ] T4: Add abstraction-level regression tests.
  First semantic object-search regression added for HTTP, DNS, file, MCP
  arguments, model tool-call arguments, and model response bodies.
- [ ] T5: Add integration/e2e proof and telemetry/session assertions.
- [ ] T6: Add fast and full benchmark proof for the security spine. Fast
  Criterion coverage added; full `just benchmark` artifact gate remains open.
- [x] T7: Add typed security-event identity contract for the network telemetry
  lane.
- [x] Update `CHANGELOG.md` under `## [Unreleased]` when code changes begin.
- [ ] Final testing gate.
- [ ] Commit at functional milestones.

## Callback-To-Event Map

| Callback / Source | Current Event Evaluated | Target Event | Status |
| --- | --- | --- | --- |
| HTTP request MITM | Canonical `http.request` `SecurityEvent` via `telemetry_hook::build_http_security_event` | Canonical network/HTTP request event | Mapped |
| HTTP response MITM | Canonical `http.response` `SecurityEvent` via `telemetry_hook::build_http_response_security_event` | Canonical network/HTTP response event | Mapped |
| DNS | Canonical `dns.request` `SecurityEvent` via `capsem-process/src/vsock.rs` before DNS transport; fallback ledger event via `build_dns_resolved_security_event` | Canonical DNS request event | Live inline enforcement mapped |
| MCP request/tool | Canonical `mcp.request` `SecurityEvent` built from parsed JSON-RPC frame before MCP dispatch | Canonical MCP request event | T2 MCP request slice wired |
| MCP response/result | Canonical `mcp.response` `SecurityEvent` built from parsed JSON-RPC response before guest delivery | Canonical MCP response event | T2 MCP response slice wired |
| Model request | Canonical `model.request` `SecurityEvent` from parsed provider request body before upstream dispatch | Canonical model request event | T2 request slice wired |
| Model response | Canonical `model.response` `SecurityEvent` from parsed provider response body before guest delivery | Canonical model response event | T2 response slice wired |
| Provider tool call | Canonical `model.response` event carries provider-emitted tool calls at `model.request.tool_calls[...]` before guest delivery | Canonical model tool-call event | T2 tool-call slice wired |
| Provider/request tool result | Canonical `model.request` event carries returned tool results at `model.response.tool_results[...]` before upstream dispatch | Canonical model tool-result projection | T2 tool-result slice wired |
| File activity | Canonical `file.activity` resolved event via `capsem-core/src/fs_monitor.rs` and MCP file-tool restore/delete logging; no inline file enforcement callback yet | Canonical file event | Ledger/session-detection mapped; inline enforcement not currently produced |
| Process activity | Canonical `process.exec` `SecurityEvent` via `capsem-process-engine::evaluate_exec_security_event` before guest exec delivery | Canonical process event | Live inline enforcement mapped |
| Credential activity | No live producer found; canonical projection and session reconstruction support `credential.activity` / `credential.request` if persisted | Canonical credential event | Contract/session-detection mapped; producer gap |
| VM lifecycle | No live producer found for `vm.start` / `vm.create`; service session reconstruction supports persisted VM lifecycle events | Canonical VM event | Contract/session-detection mapped; producer gap |
| Profile update | No live producer found for `profile.update`; service session reconstruction supports persisted profile events | Canonical profile event | Contract/session-detection mapped; producer gap |
| Conversation activity | No live producer found for `conversation.message`; service session reconstruction supports persisted conversation events | Canonical conversation event | Contract/session-detection mapped; producer gap |
| Snapshot activity | No live producer found for `snapshot.create`; service session reconstruction supports persisted snapshot events | Canonical snapshot event | Contract/session-detection mapped; producer gap |

## Detection Source Map

| Source | Event Shape | Detection Path | Status |
| --- | --- | --- | --- |
| Runtime detection/backtest request payloads | Caller-provided typed `SecurityEvent` / `RuntimeBacktestEvent` | `capsem-security-engine::run_detection_backtest` / `run_detection_hunt` | Mapped |
| Session `security_events` ledger | Reconstructed canonical `SecurityEvent` via `session_security_event_from_row` | `handle_session_detection_hunt` -> `run_detection_hunt` | Mapped |
| HTTP/DNS/MCP/model/file/process rows emitted by live engines | Persisted as `security_events` through `WriteOp::ResolvedSecurityEvent` | Session hunt and policy-context export reconstruct canonical events | Mapped |
| Credential/VM/profile/conversation/snapshot rows | Supported only when a `security_events` row exists; no live producer found in this slice | Session hunt and policy-context export reconstruct canonical events | Producer gap, not detection gap |

## Notes

- User clarified the intended Capsem architecture: parse/normalize in each
  engine, emit a single security/logging event, pre-transform that parsed event,
  then evaluate blocking/detection rules over that same abstraction.
- The sprint should not copy the old policy-v2 branch or revive
  `policy_v2_model.rs` as a separate enforcement path.
- No live compatibility shims for model/MCP enforcement. The fix is to remove
  model-to-HTTP lowering and use canonical events.
- Other agents may add fields to canonical events in parallel; this sprint
  should consume those through the shared projection layer.
- Benchmarks are part of the bank-facing release proof. Use fast Criterion
  benches for routine development and `just benchmark` for artifact-grade
  claims.
- No half ledger: detection and enforcement must be tracked together, and every
  emitted `SecurityEvent` family must be either covered or marked as
  release-blocking debt.
- T1 implementation added canonical `PolicyContext` roots for credential, VM,
  conversation, and snapshot events. Detection and enforcement CEL now share the
  same projection for every current `SecurityEvent` family.
- Detection IR now treats `snapshot` as a first-class family and lowers
  credential, VM, profile, conversation, and snapshot field paths to canonical
  CEL roots instead of rejecting or aliasing them.
- Fast Criterion benchmarks now compile for all-family policy projection,
  mixed-family CEL detection/enforcement, Detection IR all-family lowering, and
  indexed model tool-call/result paths.
- T3 removed `capsem-process` profile-rule lowering from `model.*` callbacks to
  synthetic `http.request`/`http.response` CEL predicates.
- T3 removed the MITM HTTP response body rewrite allowance for
  `tool.arguments.*`, so model tool-call rewrite compatibility cannot ride the
  HTTP response path.
- T3 intentionally does not complete live model enforcement: model rules now
  compile only against canonical `model.*` events, and T2 must wire the live
  provider callbacks that produce/evaluate those events.
- T2 first slice now evaluates a canonical `model.request` event, built from the
  provider-normalized request body, after true HTTP request enforcement and
  before upstream dispatch. This proves model-request rules no longer need an
  HTTP request-body predicate to block inline.
- T2 second slice now evaluates a canonical `model.response` event, built from
  the decompressed provider response and parsed SSE model summary, before guest
  delivery. The canonical projection exposes parsed model response text at
  `model.response.body.text`.
- T2 third slice proves provider-emitted tool calls can block from the same
  canonical `model.response` event using parsed tool-call metadata at
  `model.request.tool_calls[...]`.
- CEL now has semantic object-search verbs on first-party policy objects:
  `contains()` performs recursive object/list/map/body/scalar search, while
  `match()` and `matches()` perform recursive regex search. The proof covers
  HTTP request/body, DNS request, file path/content activity, MCP request
  arguments, model tool-call arguments, and model response body text.
- The old local MCP decision provider, MCP condition mini-parser, and builtin
  domain-policy environment authority are removed from the live path. Profile
  MCP allow/block/default rules now compile into canonical CEL rules over
  `mcp.request.*` and `mcp.response.*`.
- Default HTTP, DNS, and MCP settings rules now have focused proof:
  priority-0 specific allow rules compile into the runtime SecurityEngine and
  win over catch-all block rules at `RULE_CATCH_ALL_PRIORITY`; non-matching
  events fall through to the defaults.
- The process-side SecurityEngine glue moved out of `mcp_runtime.rs` into
  `crates/capsem-process/src/security_engine/`, split into rule compilation,
  match recording, guest config, and MCP settings extraction. `mcp_runtime.rs`
  now only owns MCP runtime/server wiring.
- T7 replaced stringly `SecurityEventCommon.event_type` with
  `SecurityEventType`, made profile callback validation consume the same typed
  registry, removed stale pseudo-callbacks from the contract, and added
  SQLite `security_events` checks for known types plus family/type consistency.
- T2 final slice proves OpenAI-shaped tool-result messages can block before
  upstream dispatch from the same canonical `model.request` event using parsed
  tool-result metadata at `model.response.tool_results[...]`.
- T0/T0a source map is complete for the current codebase. Remaining
  credential, VM, profile, conversation, and snapshot gaps are producer gaps:
  the typed contract, CEL projection, SQLite ledger, session reconstruction,
  and detection hunt support those families when rows exist, but this slice did
  not find live emitters for them.

## Benchmark Gate

Fast microbench commands:

- [x] `cargo bench -p capsem-security-engine --bench security_engine_cel --no-run`
- [x] `cargo bench -p capsem-security-engine --bench detection_ir --no-run`

Full artifact commands:

- [ ] `just benchmark`
- [ ] `just benchmark-compare`

Benchmark coverage checklist:

- [x] `SecurityEvent -> PolicyContext` projection for every emitted event
  family.
- [x] Enforcement CEL evaluation for inline blockable events across every
  enforceable event family.
- [x] Detection CEL evaluation for one-rule, mixed-family, every-family, and
  100-rule cases.
- [x] Sigma/Detection IR parse, validate, lower-to-CEL, compile, and evaluate.
- [x] Detection hunt over inline events and session-reconstructed events.
- [ ] MITM request/response callback overhead after canonical event creation.
- [ ] Model provider parser/extractor overhead for streamed, compressed, and
  provider tool-call responses.
- [ ] Benchmark result names and commands recorded before any performance
  claim.

## Coverage Ledger

Unit/contract:
- `cargo test -p capsem-proto policy_context` proves policy-context
  serialization/default contracts, including the new roots.
- `cargo test -p capsem-security-engine
  policy_context_cel_match_and_pass_smoke_covers_all_event_families` proves
  canonical projection fields can be matched from every event family.
- `cargo test -p capsem-security-engine
  detection_and_enforcement_cel_cover_every_security_event_family_root` proves
  detection and enforcement both evaluate those roots.
- `cargo test -p capsem-security-engine
  policy_context_cel_contains_and_match_are_semantic_object_search` proves
  first-party policy objects support direct `contains()`, `match()`, and
  `matches()` search without CEL closure boilerplate, including file content
  when a producer supplies a content preview.
- `cargo test -p capsem-security-engine` passes the broader
  security-engine unit suite with the new canonical roots.
- `cargo test -p capsem-security-engine runtime_` proves runtime enforcement
  backtest and detection hunt run inside `capsem-security-engine`, including
  canonical matched-field output.
- `cargo test -p capsem-security-engine --test detection_ir` proves Detection
  IR schema, direct matching, canonical `SecurityEvent` matching, CEL lowering,
  indexed model tool paths, and all-family rule lowering in the owning crate.
- `cargo test -p capsem-process security_engine::tests::` proves process
  SecurityEngine runtime
  profile-rule loading after removing model-to-HTTP lowering, including a
  regression that an OpenAI model rule no longer blocks `api.openai.com` HTTP,
  plus HTTP, DNS, and MCP default-rule priority/fallthrough behavior.
- `cargo test -p capsem-process mcp_runtime::tests::` proves the remaining MCP
  runtime module is scoped to builtin MCP env/server wiring and does not carry
  domain policy env authority.
- `cargo test -p capsem-process` passes after the module split, proving IPC
  reloads, startup wiring, MCP runtime, and the new process-side
  `security_engine/` module compile and work together.
- `cargo test -p capsem-file-engine` proves current file-event producers still
  emit normalized file security events with missing content by default.
- `cargo test -p capsem-logger writer` proves resolved security-event
  persistence still accepts the expanded file subject.
- `cargo check -p capsem-service` proves session reconstruction compiles with
  the expanded file subject and leaves content missing when historical session
  rows only contain path/class/size.
- `cargo test -p capsem-core
  mitm_runtime_source_has_no_model_tool_argument_http_rewrite_bridge` proves the
  MITM HTTP response rewrite path no longer accepts model `tool.arguments.*`
  compatibility mutations.
- `cargo test -p capsem-core mcp_frame` proves the live framed-MCP path builds
  canonical `mcp.request` and `mcp.response` events, evaluates CEL over MCP
  arguments/result bodies, and converts SecurityEngine blocks into MCP
  pre-dispatch/response policy denials.
- `cargo test -p capsem-core runtime_security_engine_blocks` proves HTTP
  request/body/response inline blocking still works and canonical
  `model.request` CEL rules block before upstream dispatch while canonical
  `model.response` and provider tool-call CEL rules block before guest delivery.
- `cargo test -p capsem-core
  runtime_security_engine_blocks_model_tool_result_before_upstream_dispatch`
  proves OpenAI-shaped tool-result messages block before upstream dispatch from
  canonical `model.response.tool_results[...]` on the `model.request` event.
- `cargo test -p capsem-core settings_profiles::tests::` proves generated
  settings/Profile rules use canonical MCP CEL fields, including priority-0
  `allowed_tools` allow rules.
- `uv run pytest tests/test_security_packs.py -q` proves the Python builder
  schema/path layer accepts the updated family surface.
- `uv run pytest tests/test_benchmark_contract.py
  tests/test_archive_criterion_benchmarks.py -q` proves the canonical
  benchmark recipe and Criterion artifact archiver call/archive the
  security-engine-owned Detection IR harness.
- `cargo check -p capsem-core` proves the core compatibility re-export compiles
  without owning Detection IR implementation/tests/benches.
- `cargo check -p capsem-service` proves the service still compiles through
  the moved Detection IR and canonical security-event crates.
- `cargo test -p capsem-service handle_enforcement_backtest`,
  `cargo test -p capsem-service handle_detection_backtest`,
  `cargo test -p capsem-service handle_detection_hunt`, and
  `cargo test -p capsem-service handle_session_detection_hunt` prove the
  service endpoints remain compatible while delegating runtime backtest/hunt
  semantics to `capsem-security-engine`.
- `cargo test -p capsem-security-engine security_event_` proves typed event
  serde roundtrips, strict parse rejection of stale callbacks, callback guards,
  constructor-known event types, constructor family/type assertions, fixture
  coverage, and all-family CEL projection.
- `cargo test -p capsem-logger security_events_` proves new SQLite
  `security_events` tables reject unknown event types, reject family/type
  mismatches, and accept every `SecurityEventType` when paired with its family.
- `cargo test -p capsem-core
  profile_rule_callback_validation_rejects_unbacked_event_strings` proves
  profile callback validation rejects stale unbacked strings such as
  `dns.response`.
- `cargo test -p capsem-service --no-run` proves service tests compile with
  typed session reconstruction and event construction.

Functional:
- Partial. Security-engine evaluator APIs are covered by focused Rust tests,
  and the MITM live request path now has a fixture-backed functional test for
  canonical `model.request` blocking before upstream dispatch plus canonical
  `model.response` and provider tool-call blocking before guest delivery.
  The framed-MCP live request/response path now has fixture-backed proof for
  canonical `mcp.request` and `mcp.response` CEL enforcement. Production
  service/profile registry wiring for Sigma/Detection IR and live provider
  tool-result callbacks remain open under T2/T4.

Adversarial:
- Partial. Existing policy-context tests cover missing/redacted body semantics
  and unknown-field rejection. MITM tests cover request blocking before any
  upstream accept. Provider malformed/streaming/compressed response payloads
  remain open under T4/T5.

E2E/VM:
- Missing. Required once live callbacks are rewired under T2.

Telemetry:
- Missing. Required to prove logged/session event matches detection and
  enforcement events after T2.

Performance:
- Fast benchmark coverage added:
  `cargo bench -p capsem-security-engine --bench security_engine_cel --no-run`
  compiled the all-family CEL/projection bench binary, and `cargo bench -p
  capsem-security-engine --bench detection_ir --no-run` compiled the moved
  Detection IR harness. The Detection IR harness covers parse, lowering, direct
  matching, canonical `SecurityEvent` matching, and lowered-CEL matching.
- `security_engine_cel` now includes
  `security_engine_runtime_backtest_hunt` for enforcement backtest, detection
  backtest, and 100-rule/100-event detection hunt through the engine API.
- These are harness compile gates only; no new performance numbers were
  recorded in this milestone.
- Full artifact benchmark not run in this milestone. `just benchmark` and
  `just benchmark-compare` remain required before any performance claim.

Missing/deferred:
- T2 live callback rewire is complete for the current fixture-backed surface:
  canonical `model.request`, `model.response`, provider-emitted tool-call,
  request-side model tool-result, and framed-MCP request/response blocking are
  wired and tested. T3 lowering/removal is done.
- Credential, VM, profile, conversation, and snapshot remain live producer
  gaps. They are first-party in the contract/projection/session-detection path,
  but no live emitter was found in this T0/T0a mapping pass.
- T4/T5 provider-body hardening, integration proof, and session telemetry proof
  are still open.
- T6 still needs full benchmark artifact execution and callback/parser/hunt
  benchmark coverage.
- The current file watcher emits file path/class/size; file content search is
  available in the canonical policy context when richer file producers attach a
  content preview.
