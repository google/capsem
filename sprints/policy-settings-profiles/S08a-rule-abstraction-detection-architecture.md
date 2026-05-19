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

We also want detection: suspicious behavior, audit findings, policy
recommendations, and enterprise detection-as-code. Sigma is the obvious
industry reference point for detection rules, but Sigma is a detection/log
format, not a synchronous enforcement-policy format. Its model starts from
logged events and produces findings; Capsem policy starts from a live callback
and must decide before traffic/tool/model/file behavior proceeds.

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

- **Policy rules**: Capsem-native, synchronous, runtime-enforced rules.
  Decisions include `allow`, `block`, `ask`, `rewrite`, and any future
  enforcement actions. Inputs are live callback subjects.
- **Detection rules**: event/log-oriented rules, potentially Sigma-compatible
  or Sigma-derived. Outputs are findings/alerts/audit events/recommendations,
  not direct runtime decisions.

Promotion is explicit:

- a detection finding may propose a policy rule;
- an ask/confirm event may propose a policy rule;
- nothing silently becomes enforcement.

This is a hypothesis, not yet the final design. S08a exists to validate or
replace it.

## Questions To Answer

- What is the normalized Capsem event schema that detection rules evaluate?
- Which event families need detection support first: DNS, HTTP, MCP, model,
  filesystem, process, credentials, VM lifecycle, profile changes?
- Which fields must be stable enough for Sigma-style content?
- Do we import Sigma as-is, compile Sigma into Capsem detection rules, or only
  provide a Sigma-compatible bridge for selected event families?
- How do detection findings appear in telemetry, debug reports, UI, CLI, and
  corp export?
- Can detections trigger `ask`, or do they only suggest/promote policy rules?
- How does a remote policy plugin consume events versus return runtime
  decisions?
- What is the schema for a policy-rule suggestion generated from a detection or
  confirm event?
- How do package/profile assumptions affect rule availability?
- What gets signed in profiles: policy rules, detection rules, or both?

## Surfaces Affected

- S09 CLI: must expose policy rules and detections without confusing them.
- S11 status/debug/provenance: must explain active policy, detections, and
  findings separately.
- S12 telemetry: must define normalized event/finding metrics and labels before
  OTel names freeze.
- S13 remote policy plugin: must separate event streaming from synchronous
  decisions.
- S14 rules UI: must know whether it edits policy rules, detection rules, or
  two tabs/modes.
- S15 Confirm UX: promote-allow/promote-deny must create policy rules; it may
  also annotate detection findings, but should not silently author detections.
- S16 Profile UI and S19 docs: must present enterprise policy/detection
  semantics coherently.

## Deliverables

- Architecture decision record documenting the final split or unified model.
- Event taxonomy for detection-ready Capsem events.
- Policy rule schema changes, if any.
- Detection rule schema or Sigma bridge decision.
- Telemetry/logging requirements for detections and findings.
- Plugin contract impact notes.
- Updated S12/S13/S14/S15/S19 sprint specs with the chosen abstraction.
- Testing matrix for policy evaluation, detection evaluation, promotion, and
  telemetry attribution.

## Coverage Requirements For Later Implementation

This sprint is design-first, but the implementation it creates must require:

- policy-rule unit tests for synchronous allow/block/ask/rewrite behavior;
- detection-rule unit tests over normalized event fixtures;
- adversarial tests for detection false positives, missing fields, and schema
  drift;
- telemetry tests proving findings are attributable without high-cardinality
  label leaks;
- plugin tests separating event delivery from decision authority;
- UI/CLI tests proving policy and detection are not conflated.

## Done Means

- We can say exactly what a Capsem policy rule is.
- We can say exactly what a Capsem detection rule/finding is.
- We know whether and how Sigma enters the product.
- S12/S13/S14/S15 are updated to consume that model.
- No Confirm UX or rule editor work proceeds on ambiguous rule semantics.
