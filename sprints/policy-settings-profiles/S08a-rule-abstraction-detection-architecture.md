# S08a - Rule Abstraction And Detection Architecture

## Status

Done. Inserted during the 2026-05-19 regroup as an architecture discussion
gate before more CLI, telemetry, plugin, rules UI, or Confirm UX
implementation.

First decision slice landed on 2026-05-21:

- Enforcement and detection are separate profile-owned rule families.
- Enforcement rules are synchronous blocking rules and use real CEL through the
  Rust `cel` crate family (`cel` 0.13.x / cel-rust), replacing the current
  Capsem-only CEL-like shortcut.
- Detection rules are event/finding rules. Sigma enters as a detection
  authoring/import format, not as an enforcement language. Runtime detection
  evaluates normalized Capsem security events and attaches findings to the
  resolved event before audit/logging, telemetry, export, UI, and timeline
  sinks receive it.
- Capsem adopts a Sigma-compatible detection-pack path: Pydantic validates the
  signed profile/detection-pack envelope in `capsem-admin`; Sigma YAML syntax
  is validated with pySigma for corp authoring; Rust runtime evaluation uses
  `rsigma-parser`/`rsigma-eval` over the normalized event JSON shape. Parity
  fixtures are mandatory wherever pySigma and Rust evaluation both apply.
- S08b owns the engine split and must implement the single normalized Security
  Engine path: preprocessor plugins -> enforcement/CEL -> ask/confirm -> detection
  -> postprocessor plugins -> resolved-event emitter -> telemetry/audit/logging
  sinks.

Second decision slice landed on 2026-05-21:

- Defined the first concrete enforcement-pack, detection-pack, compiled detection
  IR, finding, and normalized event contracts.
- Defined initial offline `capsem-admin policy validate|schema|check` and
  `capsem-admin detection validate|schema|compile|check` requirements.
- Fixed the initial Sigma logsource mapping for Capsem event families.
- Marked downstream sprint deltas for S07b, S12, S13, S14, S15, S16a, and S19.

Third decision slice landed on 2026-05-21:

- Closed the ADR: Capsem keeps separate profile-owned enforcement and detection
  packs; S07b implements admin schemas/validation first; S08b implements the
  Rust runtime contracts next; S12/S13/S14/S15/S16a/S19 consume those contracts.
- Added implementation ordering and a testing/proof matrix for enforcement,
  detection, normalized events, plugin separation, telemetry, and UI/Confirm
  behavior.

Fourth decision slice landed on 2026-05-21:

- Public runtime surfaces use separate nouns and route groups:
  `/enforcement/*` for blocking-capable decisions and `/detection/*` for
  findings, backtest, stats, and forensic hunt. Do not expose a generic
  `/rules/*` surface for the post-S08b API.
- The service/security-engine stack is runtime authority when Capsem is
  installed. It owns runtime parsing, validation, compilation, hot-swappable
  rule registries, listing, match stats, enforcement backtest, detection
  backtest, and detection hunt.
- `capsem-admin` must still work without Capsem installed. It is an offline
  authoring/CI/package tool that produces valid enforcement CEL packs and valid
  Sigma detection packs. It may optionally call a running service, but it must
  not require one.
- Offline admin behavior and Rust runtime behavior are pinned by a shared
  event/rule corpus in S08c. Python and Rust consume the same fixtures and
  expected backtest outputs so rule semantics cannot drift silently.
- Backtest is first-class for both enforcement and detection. Detection also
  supports hunt against historical resolved-event journals.
- Backtest returns exact event-level rows by default, not only aggregate
  counters. The default result set returns up to 100 matched events and
  deduplicates on a simple evidence signature to show useful diversity.
- Local backtest and hunt return full matched field values/evidence. Redaction
  is an export/support-bundle concern, not the default local debugging
  behavior.

Reference implementations checked during this slice:

- CEL: <https://docs.rs/cel/latest/cel/>
- CEL project: <https://github.com/cel-rust/cel-rust>
- rsigma parser/evaluator: <https://docs.rs/rsigma-parser/latest/rsigma_parser/>,
  <https://docs.rs/rsigma-eval/latest/rsigma_eval/>
- pySigma: <https://sigmahq-pysigma.readthedocs.io/en/latest/>

## Decision V1

### Rule Families And Surfaces

Capsem has two rule families with different authority and different user-facing
route groups:

- **Enforcement packs** are synchronous, blocking-capable policy. They evaluate
  before a transport/file/process/model action commits. An enforcement decision
  is a `SecurityDecision`: `allow`, `block`, `ask`, `rewrite`, or any later
  explicitly modeled action such as quarantine.
- **Detection packs** are asynchronous-from-the-transport-point-of-view but
  still run inside the Security Engine before the event is emitted. They produce
  `DetectionFinding` records attached to the resolved event. Detections do not
  directly allow/block/rewrite/ask. They may propose policy through explicit
  suggestion/promote flows.

The split is not optional. Sigma-style content never becomes runtime blocking
without a generated or hand-authored enforcement rule that passes the
enforcement-pack schema, CEL validation, profile signing, and governance locks.

Runtime routes preserve that split:

```text
/enforcement/validate
/enforcement/compile
/enforcement/backtest
/enforcement
/enforcement/{id}
/enforcement/stats

/detection/validate
/detection/compile
/detection/backtest
/detection
/detection/{id}
/detection/stats
/detection/hunt
/sessions/{id}/detection/hunt
```

Backtest is a quality/debugging verb for candidate rules over known event
corpora. Hunt is detection-only and searches historical resolved-event journals
for forensic review.

### Policy CEL

Policy rules use real CEL, compiled and type-checked at profile/install time,
then cached with the VM-effective profile revision. The allowed CEL surface is
deliberately small at first:

- scalar comparisons, boolean operators, list membership, string helpers, and
  `matches()` with bounded regex;
- no custom user functions in profile content for S08b;
- no wall-clock/network/file-system side effects;
- event fields are accessed through the normalized typed event subject, not raw
  ad hoc maps.

The old Capsem CEL-like evaluator becomes a migration target only. New tests
must assert the real CEL behavior and must reject expressions that only worked
because of the shortcut parser.

### Sigma-Compatible Detection

Detection packs are signed profile content. A pack contains metadata,
governance, event-family bindings, and one or more Sigma-compatible YAML rules
or compiled Capsem detection rules.

The authoring path accepts Sigma because enterprise detection teams already
know it. The runtime path is Capsem-normalized:

1. `capsem-admin detection validate` validates the pack envelope with Pydantic
   and pySigma for offline authoring/CI. This must work without Capsem
   installed.
2. The Capsem service validates and compiles the same detection pack at runtime
   through `/detection/validate` and `/detection/compile`.
3. Capsem compiles supported Sigma selections/conditions into the S08b-selected
   runtime predicate plan. The likely direction is Sigma-to-CEL over normalized
   events so enforcement and detection can share one matching engine while
   keeping separate semantics.
4. Rust service tests and Python admin tests consume the same S08c fixtures and
   expected outputs.
5. Unsupported Sigma constructs fail closed at validation/import time with
   typed diagnostics; they are not silently ignored.

### Backtest And Evidence

Enforcement and detection both require backtest before a rule pack is trusted.
Backtest evaluates candidate rules over a known normalized event corpus and
returns aggregate counts plus event-level rows.

Default behavior:

- return up to 100 matched events;
- deduplicate rows by a simple evidence signature so the default output shows
  match diversity;
- include `corpus`, `session_id`, `event_id`, `sequence`, timestamp, rule id,
  pack id, actual decision/finding, expected label when present, full matched
  field values, and pass/fail/mismatch outcome;
- do not redact local evidence by default.

Redaction belongs to export/support-bundle/reporting flows. If an operator can
query the local Capsem service or local fixture corpus, they are inside the
debugging trust boundary and need full evidence.

### Security Engine Ordering

The Security Engine owns the single decision path for every event family:

```text
engine event
  -> normalize to SecurityEvent
  -> preprocessor plugins
  -> enforcement CEL evaluation
  -> ask/confirm if needed
  -> detection evaluation
  -> postprocessor plugins
  -> ResolvedSecurityEvent
  -> emitter sinks: audit/logging, telemetry/OTel, timeline/session DB, export
```

Detection runs after enforcement and confirm because it needs the final enforcement
decision. It still runs before sinks so audit logs, telemetry, timeline rows,
and exports all receive the same resolved event with `enforcement_results`,
`confirm_result`, `detection_findings`, and `postprocessor_results` attached.

### Profile Ownership And Pins

Profiles own both enforcement and detection packs:

- profile revisions declare pack ids, versions, hashes, signatures, status, and
  governance locks;
- VM creation pins the effective profile revision plus enforcement/detection pack
  identities;
- running VMs do not silently change enforcement/detection behavior on profile
  update;
- forks inherit the same effective enforcement/detection pack pins unless an
  explicit profile update flow creates a new VM-effective configuration.

### Finding Shape

Every detection emits a typed finding, not an unstructured log string:

- `finding_id`, `event_id`, `vm_id`, `profile_id`, `profile_revision`;
- `pack_id`, `pack_version`, `rule_id`, optional `sigma_id`;
- `severity`, `confidence`, `status`, `tags`;
- `event_family`, `event_type`, `field_refs`;
- bounded `labels` suitable for OTel;
- optional `policy_suggestion_id` when a finding proposes enforcement.

Prompt text, full URLs with secrets, raw headers, command output, and stack
traces are not OTel labels. They live in the session/timeline/audit payload
with redaction and access controls.

## ADR Summary

Decision: adopt a split enforcement/detection architecture.

Rationale:

- Runtime enforcement must answer before an action proceeds. That requires a
  synchronous policy language, typed event subjects, deterministic evaluation,
  and fail-closed behavior.
- Detection content is retrospective/analytic over normalized events. Sigma is
  strong as an authoring/import format for findings, but treating it as a
  direct blocking language would obscure the confirmation, rewrite, and
  governance semantics Capsem needs.
- Profile ownership and VM-effective pins keep enforcement/detection behavior
  reproducible for forensics, forks, updates, and enterprise rollout.
- A single `ResolvedSecurityEvent` gives audit, telemetry, timeline, export,
  and UI one evidence object instead of divergent domain-specific stories.

Consequences:

- The existing Capsem CEL-like parser is not a release contract and must be
  replaced by real CEL tests before new rule UI/Confirm work claims support.
- S07b is unblocked to implement Pydantic/schema/admin tooling for enforcement and
  detection packs.
- S08b is unblocked to implement runtime Rust contracts and engine boundaries.
- S12/S13/S14/S15/S16a/S19 must consume this contract instead of inventing
  parallel models.

Rejected alternatives:

- **Use Sigma for blocking:** rejected because Sigma is detection-oriented and
  does not model Capsem's synchronous `ask`, `rewrite`, and transport response
  semantics.
- **Keep the homegrown CEL subset:** rejected because it creates a second rule
  language, weakens corp-admin tooling, and makes external validation harder.
- **Let detections auto-promote to enforcement:** rejected because it creates
  silent behavior changes. Promotion must be explicit and profile-governed.

## Implementation Ordering

The next work should land in this order:

1. **S07b admin schema slice:** add Pydantic models and JSON Schema artifacts
   for `capsem.policy-pack.v1`, `capsem.detection-pack.v1`,
   `capsem.detection.ir.v1`, `SecurityEvent` fixtures, `DetectionFinding`, and
   validation/check reports.
2. **S07b admin command slice:** add offline `capsem-admin policy
   validate|schema|check` and `capsem-admin detection
   validate|schema|compile|check` with Pydantic-only JSON I/O and pySigma
   validation for detection YAML. These commands cannot require a running
   Capsem service.
   S08c adds the post-regroup `capsem-admin enforcement ...` parity/backtest
   surface over the shared corpus.
3. **S08b Rust contract slice:** add shared Rust types for `SecurityEvent`,
   `ResolvedSecurityEvent`, `EnforcementResult`, `ConfirmResult`,
   `DetectionFinding`, pack identity, and family-specific subjects.
4. **S08b real CEL slice:** integrate the Rust `cel` crate family behind a
   enforcement evaluator trait, port existing enforcement tests, and add adversarial
   tests proving old shortcut-only expressions fail.
5. **S08b detection runtime slice:** load compiled detection IR, evaluate
   normalized event fixtures with the Rust Sigma-compatible path, and emit
   typed findings.
6. **S08b engine path slice:** wire Network/File/Process/Conversation engine
   outputs into Security Engine -> Resolved Event Emitter, preserving one event
   id through audit/logging, telemetry, timeline/session DB, and export sinks.
7. **S08c corpus/parity slice:** add the shared event/rule corpus, offline
   admin backtest, Rust runtime backtest, and real-session fixture generation
   once the S08b journal is stable.
8. **S12/S13/S14/S15/S16a/S19 consumption slices:** implement metrics/plugin/UI/
   Confirm/timeline/docs behavior only after the runtime contracts are present.

S07b can run in parallel with early S08b type work if write scopes stay
separate: Python admin models in `src/capsem/builder|admin`, Rust runtime
contracts in crates.

## Testing Matrix

S07b admin proof:

- Unit/contract: Pydantic JSON/TOML/YAML validation, schema golden tests, and
  unknown-field fail-closed tests for enforcement packs, detection packs, detection
  IR, findings, and reports.
- Functional: CLI validate/schema/compile/check commands over committed
  fixture packs and JSONL event fixtures.
- Adversarial: unsupported Sigma constructs, missing field mappings, policy
  rules referencing wrong event families, rewrite payload mismatch, detection
  rules declaring enforcement decisions, malformed CEL, and locked corp rule
  edits.

S08b runtime proof:

- Unit/contract: Rust serialization/roundtrip tests for `SecurityEvent`,
  `ResolvedSecurityEvent`, findings, pack identity, and subject variants.
- Policy: real CEL parser/type/evaluator tests for allow/block/ask/rewrite and
  explicit rejection of the old CEL-like shortcuts.
- Detection: Sigma-compatible fixture rules evaluated against normalized event
  fixtures, including nonmatch and malformed-event cases.
- Engine: every event family goes through preprocessor -> policy -> confirm ->
  detection -> postprocessor -> emitter with one resolved event id.
- Emitter: audit/logging, telemetry, timeline/session DB, and export sinks
  receive the same resolved event id and finding ids.

S12/S13/S14/S15/S16a/S19 consumption proof:

- Telemetry: OTel/status labels stay bounded; findings and model usage
  counters avoid prompts, paths, raw URLs, commands, and free-form errors as
  labels.
- Plugin: decision mode can allow/block/ask/rewrite; observer mode cannot
  change final action.
- UI: enforcement editor edits enforcement packs only; detection findings render as
  findings/suggestions.
- Confirm: promote flows create enforcement rules only after explicit operator
  action; detection suggestions never auto-enforce.
- Timeline: `/timeline/{id}` can group resolved events, findings, asks,
  conversations, tools, files, processes, snapshots, and profile provenance.

## Contract V1

This contract is intentionally small enough to implement and test in S08b/S07b,
while leaving room for richer detection content later.

### Policy Pack V1

Schema id: `capsem.policy-pack.v1`.

Required top-level fields:

- `id`: stable pack id, globally unique inside the profile/catalog namespace.
- `version`: semantic or calendar version string.
- `status`: `active`, `deprecated`, or `revoked`.
- `owner`: `corp`, `vendor`, or `user`.
- `profile_scope`: allowed profile ids, profile types, or package/tool
  assumptions required before the pack can run.
- `locks`: section editability and override rules consumed by corp governance.
- `rules`: ordered enforcement rules.

Enforcement rule fields:

- `id`, `name`, `description`, `enabled`.
- `event_family`: `dns`, `http`, `mcp`, `model`, `file`, `process`,
  `credential`, `vm`, `profile`, or `conversation`.
- `event_type`: family-specific type such as `http.request`,
  `file.write`, or `process.exec`.
- `priority`: integer; lower values evaluate first.
- `condition`: real CEL expression over the normalized event subject.
- `decision`: `allow`, `block`, `ask`, or `rewrite`.
- `rewrite`: typed rewrite payload, present only for `decision = "rewrite"`.
- `reason`, `tags`, `references`.
- `provenance`: generated-by, source pack, source profile revision, and
  optional confirm/detection suggestion id.

Validation requirements:

- Unknown fields fail closed.
- `decision = "rewrite"` requires a rewrite payload matching the event family.
- `decision != "rewrite"` rejects rewrite payloads.
- CEL must parse and type-check against the event-family schema before the pack
  can be installed or launched.
- An enforcement rule may not reference fields outside its `event_family` schema.
- A locked corp rule cannot be edited by user profile overlays.

### Detection Pack V1

Schema id: `capsem.detection-pack.v1`.

Required top-level fields:

- `id`, `version`, `status`, `owner`, `description`.
- `profile_scope`: allowed profile ids/types and required package/tool
  assumptions.
- `sources`: embedded Sigma YAML documents, external signed references, or
  compiled Capsem detection IR.
- `field_mapping`: explicit mapping from Sigma fields to Capsem normalized
  event fields.
- `findings`: severity/confidence defaults, tags, and SOAR/export routing
  hints.
- `locks`: corp governance over enablement, severity changes, and suppression.

Detection rules produce findings only. They may carry a
`enforcement_suggestion_template`, but that template is inert until an explicit
operator/profile workflow converts it into an enforcement rule.

Validation requirements:

- Unknown fields fail closed.
- Sigma YAML must validate through pySigma and the S08a-supported subset.
- The same sample fixtures must validate through the Rust Sigma runtime path
  where runtime evaluation applies.
- Unsupported Sigma selection modifiers, aggregation, correlation, or backend
  query output fail at import/compile time with typed diagnostics.
- Every Sigma field must map to a known Capsem normalized event field.
- Detection rules cannot declare `allow`, `block`, `ask`, or `rewrite`.

### Detection IR V1

Schema id: `capsem.detection.ir.v1`.

This is the runtime-stable compiled form, not the corp authoring surface. It
contains:

- pack id/version/hash/signature provenance;
- rule id and optional Sigma id;
- event-family/type filters;
- normalized field matchers;
- condition expression/selection tree accepted by the chosen Rust evaluator;
- finding metadata defaults;
- source-location mapping back to the original detection pack.

S08b may initially store this in memory beside the VM-effective profile. S12
and S16a consume findings, not the raw IR.

### Normalized Event Taxonomy V1

Every engine emits a `SecurityEvent` with common fields:

- `event_id`, `trace_id`, `span_id`, `timestamp`;
- `vm_id`, `session_id`, `profile_id`, `profile_revision`;
- `profile_pack_ids`: effective enforcement/detection pack identities;
- `user_id` when available, otherwise a typed absent value;
- `process_id`, `parent_process_id`, `exec_id`, `turn_id`, `message_id`,
  `tool_call_id`, and `mcp_call_id` when known;
- `event_family`, `event_type`;
- `subject`: family-specific typed payload;
- `redaction_state`: raw, redacted, or summary-only.

Initial event families and Sigma logsource mapping:

| Event family | Capsem event types | Sigma product/category |
| --- | --- | --- |
| DNS | `dns.request`, `dns.response` | `capsem` / `dns` |
| HTTP | `http.request`, `http.response`, `http.stream_chunk` | `capsem` / `http` |
| MCP | `mcp.request`, `mcp.response`, `mcp.tool_call` | `capsem` / `mcp` |
| Model | `model.request`, `model.response`, `model.tool_call` | `capsem` / `model` |
| File | `file.read`, `file.write`, `file.delete`, `file.rename`, `file.quarantine`, `snapshot.create`, `snapshot.restore` | `capsem` / `file` |
| Process | `process.exec`, `process.exit`, `process.audit` | `capsem` / `process` |
| Credential | `credential.request`, `credential.inject`, `credential.denied` | `capsem` / `credential` |
| VM/Profile | `vm.create`, `vm.fork`, `vm.resume`, `profile.update`, `profile.revoked` | `capsem` / `vm` or `profile` |
| Conversation | `conversation.turn`, `conversation.message`, `conversation.artifact` | `capsem` / `conversation` |

The Sigma `logsource.product` for first-party rules is `capsem`. Importers may
accept external Sigma rules for other products only through an explicit mapping
profile. No implicit Windows/Linux/cloud mappings are allowed in S08b.

### Resolved Event V1

`ResolvedSecurityEvent` contains the original normalized event plus:

- `preprocessor_results`;
- `enforcement_results`;
- `confirm_result`;
- `detection_findings`;
- `postprocessor_results`;
- `final_action`;
- `emitter_results`.

The Resolved Event Emitter writes the same resolved event identity to all
sinks. Domain tables can remain projections, but the resolved event is the
canonical audit/timeline/security record.

### Admin Commands

S07b must add, after S08a format closeout:

```text
capsem-admin policy validate policy-pack.toml|json [--profile profile.toml] [--json]
capsem-admin policy schema [--out schemas/capsem.policy-pack.v1.schema.json]
capsem-admin policy check policy-pack.toml|json --events fixtures/events.jsonl --json

capsem-admin detection validate detection-pack.toml|json|yml [--profile profile.toml] [--json]
capsem-admin detection schema [--out schemas/capsem.detection-pack.v1.schema.json]
capsem-admin detection compile detection-pack.yml --out detection.ir.json --json
capsem-admin detection check detection-pack.yml --events fixtures/events.jsonl --json
```

`validate` proves shape and static semantics. `compile` proves the supported
Sigma subset maps into Capsem detection IR. `check` evaluates fixture events
and emits a typed report with matched findings, nonmatches, errors, and timing
summary. All JSON I/O follows the Pydantic-only boundary used elsewhere:
`model_validate_json()` / `TypeAdapter.validate_json()` on input and
`model_dump_json()` / `TypeAdapter.dump_json()` on output.

## Goal

Decide the long-term abstraction boundary between Capsem runtime enforcement rules
and detection rules before we harden public surfaces around the wrong model.

## Problem Statement

Capsem currently uses canonical profile rules for synchronous enforcement:
`allow`, `block`, `ask`, and `rewrite` across DNS, HTTP, MCP, and model
callbacks. That is necessary for containment.

The current implementation does not yet have the final rule substrate:

- enforcement conditions use a small Capsem-owned CEL-like subset, not a real CEL
  implementation;
- detection is not a first-class subsystem yet;
- Sigma is not wired as a real detection format/engine;
- profile payloads do not yet clearly own both enforcement and detection
  content as signed, versioned configuration.

We want real detection: suspicious behavior, audit findings, policy
recommendations, and enterprise detection-as-code. Sigma is the industry
reference point for detection rules, but Sigma is a detection/log format, not a
synchronous enforcement-policy format. Its model starts from normalized events
and produces findings; Capsem policy starts from a live callback and must decide
before traffic/tool/model/file behavior proceeds.

If we merge those two concepts too early, we risk breaking all of these:

- runtime blocking semantics;
- policy confirm/promote flows;
- telemetry and log event schemas;
- remote enforcement plugin contracts;
- Sigma import/export;
- UI rule editor mental model;
- docs for enterprise admins.

## Working Hypothesis

Superseded by Decision V1 above, kept here as the original framing that the
decision validated: Capsem should have two related but separate rule families.

- **Policy rules**: Profile-owned, signed, synchronous, runtime-enforced rules.
  Conditions use a real CEL implementation, not a homegrown CEL-like subset.
  Decisions include `allow`, `block`, `ask`, `rewrite`, and any future
  enforcement actions. Inputs are live normalized security events/subjects.
- **Detection rules**: Profile-owned, signed, event-oriented rules using a real
  Sigma-compatible representation/engine or a documented Sigma import/compile
  path. Outputs are findings/alerts/audit events/recommendations, not direct
  runtime decisions.

Promotion is explicit:

- a detection finding may propose an enforcement rule;
- an ask/confirm event may propose an enforcement rule;
- nothing silently becomes enforcement.

Decision V1 keeps that split and turns it into concrete pack/event/finding
contracts.

## Questions To Answer

- Answered: standardize on the Rust `cel` crate family for enforcement CEL and
  replace the Capsem-only shortcut.
- Answered: normalized events are `SecurityEvent` values with family-specific
  typed subjects and common ids/provenance.
- Answered: initial detection families are DNS, HTTP, MCP, model, file,
  process, credential, VM/profile, and conversation.
- Answered: Sigma enters as detection authoring/import and compiles to
  `capsem.detection.ir.v1`; runtime findings attach to
  `ResolvedSecurityEvent`.
- Answered: first-party Sigma `logsource.product` is `capsem`; categories map
  to Capsem event families.
- Answered: detections do not trigger `ask`; they may provide explicit enforcement
  suggestions.
- Answered: remote enforcement plugin has separate decision and observer modes.
- Answered: profiles sign enforcement packs, detection packs, compiled IR or signed
  references, plus pack hashes/status/locks.
- Remaining: exact Rust/Python model field types, schema artifacts, fixture
  files, and implementation ordering for S07b/S08b.

## Profile Ownership Requirement

Enforcement and detection content belongs to profiles, not loose runtime state:

- profiles declare enabled enforcement rule packs and detection rule packs;
- profile revisions sign the exact rule content or signed references to rule
  packs;
- VM-effective settings materialize the resolved enforcement + detection set for a
  specific VM profile revision;
- VMs pin the profile revision and rule-pack identity used at creation time;
- profile updates do not silently mutate running VM enforcement or detection
  behavior unless an explicit update/reload contract says so;
- corp governance can lock, require, disable, or replace enforcement/detection packs
  through the same profile governance model.

Detection is therefore part of the profile contract. It may emit findings
through telemetry/export sinks, but the authority for what detections run comes
from the signed profile.

## Surfaces Affected

- S08b security event engine: must turn the chosen enforcement/detection model into
  concrete Network Engine, File Engine, Process Engine, Security Engine, and
  Resolved Event Emitter boundaries.
- S09 CLI: must expose enforcement rules and detections without confusing them.
- S11 status/debug/provenance: must explain active enforcement, detections, and
  findings separately.
- S12 telemetry: must define normalized event/finding metrics and labels before
  OTel names freeze, including detection finding counters and model/provider/
  cost attribution surfaced in VM health without high-cardinality labels.
- S13 remote enforcement plugin: must separate event streaming from synchronous
  decisions.
- S14 rules UI: must know whether it edits enforcement rules, detection rules, or
  two tabs/modes.
- S15 Confirm UX: promote-allow/promote-deny must create enforcement rules; it may
  also annotate detection findings, but should not silently author detections.
- S16 Profile UI and S19 docs: must present enterprise enforcement/detection
  semantics coherently.
- S07b/S08c `capsem-admin`: must validate/schema/check/backtest enforcement and detection packs
  with Pydantic models and JSON Schema artifacts once this sprint chooses the
  real CEL and Sigma-compatible detection formats.

## Deliverables

- Architecture decision record documenting the final split or unified model.
- Real CEL decision: selected crate/runtime, allowed functions/macros,
  type-mapping rules, validation errors, and replacement plan for the current
  CEL-like evaluator.
- Real Sigma decision: selected implementation/import/compile path,
  supported Sigma subset if any, schema validation, and event-field mapping.
- Event taxonomy for detection-ready Capsem events.
- Policy rule schema changes, if any.
- Detection rule schema or Sigma bridge decision, including profile ownership
  and signing semantics.
- Telemetry/logging requirements for detections and findings.
- OTel/VM-health requirements for detection findings and model usage
  attribution: provider, model, call count, token totals, and estimated cost
  must be typed live metrics with bounded labels.
- `capsem-admin` validation/schema requirements for enforcement and detection packs.
- Plugin contract impact notes.
- Profile payload changes for enforcement/detection rule packs.
- Updated S12/S13/S14/S15/S19 sprint specs with the chosen abstraction.
- Updated S08b engine-boundary sprint with the chosen normalized event,
  detection, and resolved-event journal contracts.
- Testing matrix for enforcement evaluation, detection evaluation, promotion, and
  telemetry attribution.

## Coverage Requirements For Later Implementation

This sprint is design-first, but the implementation it creates must require:

- enforcement-rule unit tests for synchronous allow/block/ask/rewrite behavior;
- real CEL parser/type/evaluator tests; no tests should depend on the old
  Capsem-only CEL-like shortcuts after cutover;
- detection-rule unit tests over normalized event fixtures;
- Sigma validation/import/compile tests over representative Sigma YAML;
- adversarial tests for detection false positives, missing fields, and schema
  drift;
- telemetry tests proving findings are attributable without high-cardinality
  label leaks;
- VM-health/OTel tests proving model usage and cost are attributed by bounded
  provider/model identity without raw prompt/error labels;
- admin-tool tests proving enforcement/detection packs validate through typed
  Pydantic models and schema artifacts;
- plugin tests separating event delivery from decision authority;
- UI/CLI tests proving enforcement and detection are not conflated.

## Done Means

- We can say exactly what a Capsem enforcement rule is.
- We can say exactly what a Capsem detection rule/finding is.
- We have chosen real CEL and real Sigma-compatible detection paths.
- We know how enforcement and detection packs live in signed profiles.
- We know whether and how Sigma enters the product.
- S08b has enough specificity to split engines and crates without inventing a
  second enforcement/event model during implementation.
- S12/S13/S14/S15 are updated to consume that model.
- No Confirm UX or rule editor work proceeds on ambiguous rule semantics.
