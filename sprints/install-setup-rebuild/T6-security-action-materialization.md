# T6: Security Action Materialization

## Why

T4 proved the brokered credential invariant for the current MITM path, but the
current implementation still resolves brokered credentials through direct
MITM helpers. That works for the first broker case, but it is not the final
security architecture.

The final contract is:

```text
raw/parser observation
  -> instantiate canonical SecurityEvent
  -> CEL/Sigma rule match
  -> action plugins run
  -> each plugin receives (rule, SecurityEvent)
  -> each plugin returns SecurityEvent
  -> one auditable emitter handles logging/detection/enforcement output
  -> outbound materializers read the final SecurityEvent when wire effects apply
```

This sub-sprint makes that contract real for HTTP first, then extends the
same pattern to model/MCP/file/process/DNS where wire materialization or event
logging applies.

## Key Decisions

- Rules own matching. Plugins do not invent another detector engine.
- Rule configuration carries `actions: Vec<ActionId>`. It must not duplicate
  source, target, provider, replacement, or field-selection metadata.
- `decision = "action"` is the typed rule shape for plugin-only work. Action
  rules may match and mutate the `SecurityEvent`, but they are not enforcement
  verdicts and must not shadow later block/ask/rewrite/allow decisions.
- Plugin contract is:

  ```rust
  plugin.apply(rule, security_event) -> security_event
  ```

- A plugin may write to its owned external store, such as Keychain, but the
  only security-pipeline output is the returned `SecurityEvent`.
- CEL and Sigma stay the rule languages. Sigma must validate/compile into the
  same event vocabulary and action identifiers as native CEL rules.
- Outbound requests must be built from the post-action event, not from helper
  side channels.
- Every parser/runtime path should directly instantiate the canonical
  `SecurityEvent` and pass it into the same auditable emit system. That emit
  system owns batching, DB writer handoff, future multiprocess transport, and
  fanout to detection/enforcement/logging consumers.
- Side writers are forbidden: protocol code may add typed context to the
  `SecurityEvent`, but it must not bypass the emitter with direct table writes
  or private audit rows.
- L1 HTTP request head/body is the first materialization target because it is
  already mutable and wire-visible. L2/L3 semantic events must not pretend
  mutations reach the wire until they have explicit reserialization.

## Closed Runtime Handoff

HTTP request handling now constructs a canonical `SecurityEvent`, runs matched
security action plugins, emits the final post-action event, and materializes
the upstream request from that final event. Telemetry keeps the broker
reference while only the upstream materialized copy resolves raw credentials.

The invariant is now:

```text
post-plugin SecurityEvent -> materialized upstream request
```

Runtime/session DB rows now enter through
`capsem_core::security_engine::{emit_security_write, emit_security_write_blocking}`
for HTTP/net, model, MCP, DNS, file, process exec/audit/completion, broker
substitution, and snapshot rows. `RuntimeSecurityEventType` is the closed
runtime emitted-row identity contract (`as_str`, `family`, strict parse).
`PolicyCallback` remains the CEL/rule callback identity.

The pipeline still must not pretend that L2/L3 semantic mutations reserialize
to the wire until those paths have explicit serializers; unsupported wire
mutation remains an explicit boundary, not hidden behavior.

## Scope

### T6.1: Rule Action Contract

- Add typed action identifiers to `PolicyRuleConfig`.
- Add a typed `action` decision for plugin-only rule matches.
- Validate action identifiers through one action registry.
- Preserve existing `decision` semantics for allow/deny/detect/rewrite.
- Allow multiple actions per matched rule.
- Reject unknown action identifiers at settings/policy load time.
- Add tests proving stale action names fail validation.
- Add tests proving action-only rules do not shadow enforcement decisions.

### T6.2: Security Action Plugin Registry

- Add a small registry owned by the security/policy path.
- Define the contract as `apply(rule, event) -> event`.
- Keep the plugin interface event-oriented; do not pass ad hoc source/target
  metadata.
- Make plugin execution ordered and deterministic.
- Add tests proving multiple actions run in rule order and each receives the
  event returned by the previous action.

### T6.2b: Auditable Security Event Emitter

- Add or consolidate the single emitter API that accepts a canonical
  `SecurityEvent`.
- Ensure parser/runtime paths create `SecurityEvent` directly and submit it to
  this emitter.
- Make the emitter the only owner of DB/logging handoff, batching, fanout, and
  future multiprocess transport.
- Add tests proving protocol code cannot silently write audit/security tables
  outside the emitter path for new events.

### T6.3: HTTP Event Materializer

- Define the HTTP request security event shape that owns method, authority,
  path/query, headers, body preview/body materialization state, credential
  reference, and policy metadata.
- Build that event before action execution.
- After actions run, materialize the upstream HTTP request from the final event.
- Keep direct credential-substitution helper paths out of the MITM request
  builder path.
- Preserve telemetry/logging view from the same final event, not from a second
  formatted-header path.
- Add adversarial tests proving raw credentials are not logged but are present
  only in the actual upstream bytes when the broker action resolves them.

### T6.4: Credential Broker As Action Plugin

- Convert current broker capture/substitute behavior into registered actions:
  `credential_broker.capture` and `credential_broker.substitute`.
- Register built-in priority-0 broker substitute rules as `decision = "action"`
  defaults on the merged runtime policy so startup and reload paths share the
  same broker materialization contract.
- The action receives the matching rule and current event, rewrites the event,
  writes broker/substitution rows through the broker-owned store/log path, and
  returns the event.
- For capture: raw observed credential material becomes
  `credential:blake3:<hex>` in the returned event.
- For substitute: broker references become upstream-only raw material in the
  materialized transport representation while the security/logging event keeps
  the reference.
- Tests must prove the plugin does not decide allow/deny outside the security
  engine.

### T6.5: Sigma/CEL Integration

- Extend rule validation so Sigma-derived rules and native CEL rules use the
  same event-field registry and action registry.
- Add tests proving Sigma cannot name unknown event fields or unknown actions.
- Add tests proving CEL-matched HTTP header rules can invoke broker actions
  without provider/source/target YAML duplication.
- Ensure callback/event-type validation remains tied to the typed security
  event contract.

### T6.6: Cross-Family Boundary Check

- Audit model, MCP, DNS, file, and process paths.
- For paths that only log/enrich events, ensure the final logged event is the
  post-action event.
- For paths that can affect outbound bytes, require an explicit materializer
  before enabling mutation.
- Document unsupported wire mutation explicitly so future work cannot assume
  L2/L3 changes reach the VM/network.

## Files Expected

- `crates/capsem-core/src/net/policy_config/types.rs`
- `crates/capsem-core/src/net/policy_config/tests.rs`
- `crates/capsem-core/src/net/mitm_proxy/mod.rs`
- `crates/capsem-core/src/net/mitm_proxy/events.rs`
- `crates/capsem-core/src/net/mitm_proxy/pipeline.rs`
- `crates/capsem-core/src/credential_broker.rs`
- `crates/capsem-core/src/credential_broker/`
- `crates/capsem-core/src/security_engine/` or the current security/CEL module
  location if the engine has not yet been moved.
- the single security-event emitter module once located or created
- `crates/capsem-logger/src/events.rs`
- `crates/capsem-logger/src/writer.rs`
- `tests/capsem-e2e/test_brokered_ai_credentials.py`

## Proof Matrix

- Unit/contract:
  - action identifiers validate through one registry
  - unknown actions are rejected
  - plugin chain order is deterministic
  - plugin receives and returns `SecurityEvent`
  - parser/runtime paths submit canonical events through the emitter
  - new security/audit rows cannot bypass the emitter path
  - broker action rewrites event without leaking raw material
- Functional:
  - HTTP request with broker reference is materialized upstream with raw header
  - telemetry/session DB stores only `credential:blake3:<hex>`
  - HTTP response token capture returns a redacted event and substitution row
- Adversarial:
  - unknown broker reference fails closed without raw logging
  - malformed headers/body do not panic
  - plugin failure cannot silently allow a partially-mutated event
  - duplicate actions do not double-store or double-substitute
- E2E/VM:
  - Claude/Gemini run with broker refs in guest config and no raw VM credential
  - fake provider endpoint receives raw credential only after host materializer
  - `session.db` contains references and substitution rows, never raw secrets
- Telemetry:
  - hook/action counters show matched rule and action names
  - session DB event rows share the same credential reference
  - no parallel helper path writes a different credential identity
- Performance:
- benchmark HTTP request rule match + no-op action
- benchmark broker substitute action with header and query refs
- benchmark action-chain overhead with 1, 2, and 4 actions
- keep numbers fast enough for the bank: focused benches, no slow VM gate

Current fast bench harness:

```bash
cargo bench -p capsem-core --bench security_actions -- --quick
```

Latest quick-mode smoke numbers on this Mac:

- decision rule match: ~542 ns
- action chain: ~41 ns for 1 action, ~61 ns for 2, ~107 ns for 4
- broker substitute header ref through materializer: ~11.6 us

Latest hard-test quick-mode smoke numbers on this Mac:

- decision rule match: ~569 ns
- action chain: ~42 ns for 1 action, ~61 ns for 2, ~103 ns for 4
- broker substitute header ref through materializer: ~11.1 us

Quick mode proves the harness executes and gives a local smoke baseline. Final
release comparisons should run Criterion without `--quick`.

## Done

- Rules can declare actions.
- Runtime/parser paths instantiate canonical `SecurityEvent`s directly.
- A single auditable emitter owns batching, DB writer handoff, fanout, and
  future multiprocess transport.
- Action plugins run only after rule match.
- Plugins consume `(rule, SecurityEvent)` and return `SecurityEvent`.
- HTTP upstream request materialization consumes the final post-action event.
- Credential broker capture/substitute are action plugins, not direct MITM
  helper paths.
- CEL/Sigma validation shares field/action registries.
- Runtime emitted rows use `RuntimeSecurityEventType`, while CEL/Sigma rule
  callbacks use `PolicyCallback`; both are strict typed contracts.
- Tests prove raw secrets cannot leak through logs/session DB while upstream
  dispatch still receives the real credential when a broker rule allows it.
