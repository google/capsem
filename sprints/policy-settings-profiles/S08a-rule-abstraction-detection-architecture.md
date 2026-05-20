# S08a - Rule Abstraction And Detection Architecture

## Status

Not started. Inserted during the 2026-05-19 regroup as an architecture
discussion gate before more CLI, telemetry, plugin, rules UI, or Confirm UX
implementation.

## Goal

Decide the long-term abstraction boundary between Capsem runtime policy rules
and detection rules before we harden public surfaces around the wrong model.

## Problem Statement

Capsem currently uses canonical profile rules for synchronous enforcement:
`allow`, `block`, `ask`, and `rewrite` across DNS, HTTP, MCP, and model
callbacks. That is necessary for containment.

The current implementation does not yet have the final rule substrate:

- policy conditions use a small Capsem-owned CEL-like subset, not a real CEL
  implementation;
- detection is not a first-class subsystem yet;
- Sigma is not wired as a real detection format/engine;
- profile payloads do not yet clearly own both enforcement policy and detection
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
- remote policy plugin contracts;
- Sigma import/export;
- UI rule editor mental model;
- docs for enterprise admins.

## Working Hypothesis

Capsem should likely have two related but separate rule families:

- **Policy rules**: Profile-owned, signed, synchronous, runtime-enforced rules.
  Conditions use a real CEL implementation, not a homegrown CEL-like subset.
  Decisions include `allow`, `block`, `ask`, `rewrite`, and any future
  enforcement actions. Inputs are live normalized security events/subjects.
- **Detection rules**: Profile-owned, signed, event-oriented rules using a real
  Sigma-compatible representation/engine or a documented Sigma import/compile
  path. Outputs are findings/alerts/audit events/recommendations, not direct
  runtime decisions.

Promotion is explicit:

- a detection finding may propose a policy rule;
- an ask/confirm event may propose a policy rule;
- nothing silently becomes enforcement.

This is a hypothesis, not yet the final design. S08a exists to validate or
replace it.

## Questions To Answer

- Which Rust CEL implementation do we standardize on, and what exact CEL
  profile/function set is allowed for policy rules?
- How do we reject or migrate today's Capsem CEL-like subset so there is no
  second rule language?
- What is the normalized Capsem event schema that detection rules evaluate?
- Which event families need detection support first: DNS, HTTP, MCP, model,
  filesystem, process, credentials, VM lifecycle, profile changes?
- Which fields must be stable enough for Sigma-style content?
- Which Sigma implementation/path do we use: native Sigma YAML validation +
  compilation, an embedded engine, or a curated Sigma-compatible Capsem
  detection schema with strict import/export semantics?
- Which Sigma logsource/product/category vocabulary maps to Capsem event
  families?
- How do detection findings appear in telemetry, debug reports, UI, CLI, and
  corp export?
- Can detections trigger `ask`, or do they only suggest/promote policy rules?
- How does a remote policy plugin consume events versus return runtime
  decisions?
- What is the schema for a policy-rule suggestion generated from a detection or
  confirm event?
- How do package/profile assumptions affect rule availability?
- What gets signed in profiles: policy rules, detection rules, rule packs,
  Sigma imports, findings configuration, or all of them?
- How do profile inheritance, corp locks, and package/tool contracts affect
  detection rule enablement?

## Profile Ownership Requirement

Policy and detection content belongs to profiles, not loose runtime state:

- profiles declare enabled policy rule packs and detection rule packs;
- profile revisions sign the exact rule content or signed references to rule
  packs;
- VM-effective settings materialize the resolved policy + detection set for a
  specific VM profile revision;
- VMs pin the profile revision and rule-pack identity used at creation time;
- profile updates do not silently mutate running VM enforcement or detection
  behavior unless an explicit update/reload contract says so;
- corp governance can lock, require, disable, or replace policy/detection packs
  through the same profile governance model.

Detection is therefore part of the profile contract. It may emit findings
through telemetry/export sinks, but the authority for what detections run comes
from the signed profile.

## Surfaces Affected

- S08b security event engine: must turn the chosen policy/detection model into
  concrete Network Engine, File Engine, Process Engine, Security Engine, and
  Resolved Event Emitter boundaries.
- S09 CLI: must expose policy rules and detections without confusing them.
- S11 status/debug/provenance: must explain active policy, detections, and
  findings separately.
- S12 telemetry: must define normalized event/finding metrics and labels before
  OTel names freeze, including detection finding counters and model/provider/
  cost attribution surfaced in VM health without high-cardinality labels.
- S13 remote policy plugin: must separate event streaming from synchronous
  decisions.
- S14 rules UI: must know whether it edits policy rules, detection rules, or
  two tabs/modes.
- S15 Confirm UX: promote-allow/promote-deny must create policy rules; it may
  also annotate detection findings, but should not silently author detections.
- S16 Profile UI and S19 docs: must present enterprise policy/detection
  semantics coherently.
- S07b `capsem-admin`: must validate/schema/check policy and detection packs
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
- `capsem-admin` validation/schema requirements for policy and detection packs.
- Plugin contract impact notes.
- Profile payload changes for policy/detection rule packs.
- Updated S12/S13/S14/S15/S19 sprint specs with the chosen abstraction.
- Updated S08b engine-boundary sprint with the chosen normalized event,
  detection, and resolved-event journal contracts.
- Testing matrix for policy evaluation, detection evaluation, promotion, and
  telemetry attribution.

## Coverage Requirements For Later Implementation

This sprint is design-first, but the implementation it creates must require:

- policy-rule unit tests for synchronous allow/block/ask/rewrite behavior;
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
- admin-tool tests proving policy/detection packs validate through typed
  Pydantic models and schema artifacts;
- plugin tests separating event delivery from decision authority;
- UI/CLI tests proving policy and detection are not conflated.

## Done Means

- We can say exactly what a Capsem policy rule is.
- We can say exactly what a Capsem detection rule/finding is.
- We have chosen real CEL and real Sigma-compatible detection paths.
- We know how policy and detection packs live in signed profiles.
- We know whether and how Sigma enters the product.
- S08b has enough specificity to split engines and crates without inventing a
  second policy/event model during implementation.
- S12/S13/S14/S15 are updated to consume that model.
- No Confirm UX or rule editor work proceeds on ambiguous rule semantics.
