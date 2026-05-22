# S08c - Rule Corpus, Backtest, And Admin Parity

## Status

In progress. Inserted on 2026-05-21 after the S08b rule-runtime regroup.

## Goal

Prove that Capsem's offline admin tooling and runtime service evaluate the same
enforcement and detection artifacts the same way.

S08b owns the runtime rule registry, normalized events, CEL engine, Sigma import
path, backtest route contracts, and resolved-event journal. S08c comes after
that runtime exists and builds the shared corpus, parity tests, and admin
workflows needed to make those contracts production-trustworthy.

## Product Contract

- `capsem-admin` must work without Capsem installed. It is an offline authoring,
  packaging, schema, validation, and CI tool.
- `capsem-admin` must produce valid enforcement CEL packs and valid Sigma
  detection packs, but it is not the runtime rule authority.
- The Capsem service is runtime authority when installed. It owns hot-loaded
  `/enforcement/*` and `/detection/*` state, match stats, backtest over
  normalized events, detection hunt over session journals, and atomic compiled
  rule-plan swaps.
- Enforcement CEL and Sigma-derived detection fixtures must use the canonical
  policy context roots from S08b, not internal `event.*` paths. The corpus must
  include negative fixtures proving `event`, `event.subject`, and raw envelope
  paths are rejected before install/backtest/hunt.
- Python/admin and Rust/runtime behavior must be pinned by a shared corpus,
  not by duplicate prose. The same committed data fixtures must be accepted by
  `capsem-admin` and by the Rust service/security-engine tests.
- The first corpus may be curated from synthetic fixtures. A later slice in
  this sprint must generate initial real-session fixtures from Capsem sessions
  once S08b's resolved-event journal is stable.

## Shared Corpus Shape

Use committed fixtures under a durable `data/` or `schemas/fixtures/` layout,
with exact paths chosen during implementation:

```text
data/security-events/
data/enforcement/cel/
data/enforcement/backtest-expected/
data/detection/sigma/
data/detection/backtest-expected/
data/detection/hunt-expected/
data/policy-context/
```

Every event fixture must include stable `session_id`, `event_id`, `sequence`,
event family/type, VM/profile/user identity where applicable, and enough full
payload evidence to debug a wrong match locally. Backtest and hunt fixtures are
not redacted by default; redaction is an export/support-bundle concern.
Every policy-context fixture must pin the typed object model that CEL and the
future high-level DSL mirror: `http.request.host`, `http.request.header(name)`,
`mcp.request.tool_name`, `model.request.provider`, `file.activity.path_class`,
and missing/redacted-value semantics.

## Backtest Contract

Both enforcement and detection support backtest.

Backtest is not a summary-only quality gate. It returns:

- aggregate counts;
- up to 100 matched event rows by default;
- event refs (`corpus`, `session_id`, `event_id`, `sequence`, timestamp);
- rule id and pack id;
- actual decision/finding;
- expected label when the corpus supplies one;
- full matched field values/evidence for local debugging;
- pass/fail/mismatch outcome.

Default result selection deduplicates on a simple evidence signature so the 100
returned matched rows show useful diversity instead of 100 copies of the same
field/value shape. More elaborate sampling, pagination, or result-query options
can wait until real usage demands them.

Detection also supports forensic hunt against historical resolved-event
journals. Enforcement does not use the word `hunt`; if we later need
session-scoped enforcement replay, it should be named and designed separately.

## Tasks

- [ ] Create shared event, enforcement, detection, expected-result fixture corpus.
- [x] Create shared policy-context fixtures and negative fixtures for rejected
  `event.*` authoring.
- [ ] Add `capsem-admin enforcement validate|compile|backtest` over the shared
  corpus without requiring a Capsem service.
- [x] Add `capsem-admin detection validate|compile|backtest` over the shared corpus
  without requiring a Capsem service.
- [ ] Keep `capsem-admin detection hunt` optional unless it can target a local
  service/session store explicitly; offline detection backtest is mandatory.
- [x] Add Rust runtime parity tests that consume the same corpus and expected
  outputs through the S08b service/security-engine evaluator.
- [ ] Add cross-language drift tests proving Python-generated enforcement/detection
  artifacts use canonical policy roots, are accepted by Rust, and produce
  identical backtest outcomes.
- [ ] Generate initial real-session normalized event fixtures from S08b's
  resolved-event journal and add them to the corpus once stable.
- [ ] Document the corpus update workflow so future rule-language changes must
  update Python, Rust, and expected-result fixtures together.

## Implementation Notes

- Slice 1 landed `data/policy-context/canonical-policy-contexts.jsonl` as a
  shared fixture envelope. Each line contains a typed event ref, expected
  labels, and a `capsem_proto::PolicyContext` payload. It also added first CEL
  corpus expressions under `data/enforcement/cel/`, including a positive
  canonical-root rule and a rejected `event.subject.*` fixture.
- Python admin tooling now has typed Pydantic models for the policy-context
  envelope and loads the JSONL corpus without raw JSON dictionaries.
- Rust `capsem-security-engine` consumes the same fixture, parses the embedded
  context through `capsem_proto::PolicyContext`, validates canonical CEL roots
  such as `http.request.host`, `http.request.header(...)`, and
  `http.request.body.text`, and asserts the rejected `event.subject.*` root
  stays rejected before rule install.
- Slice 2 added `capsem-admin detection backtest`, a shared Sigma detection
  pack fixture under `data/detection/sigma/`, and documentation updates so
  offline detection checks now target policy-context JSONL instead of the old
  normalized-event/subject shape.
- Slice 3 added `capsem-admin policy backtest`, a shared enforcement policy
  pack fixture under `data/enforcement/policy/`, and first expected-result
  artifacts under `data/enforcement/backtest-expected/` and
  `data/detection/backtest-expected/`. The admin backtest accepts canonical
  policy-context roots with CEL-shaped clauses such as
  `http.request.host.contains("google")`,
  `http.request.header("authorization").exists()`, and
  `http.request.body.text.contains("secret")`, and rejects `event.*` /
  `subject.*` roots during replay.
- Important debt: the offline policy backtest is currently a constrained
  fixture replay evaluator for the committed corpus, not the full runtime CEL
  authority. S08c remains open until enforcement compile/parity uses the same
  CEL semantics as Rust runtime or an equivalent shared expected-row generator,
  and until real-session fixtures are generated from the resolved-event journal.
- Slice 4 added the first Rust expected-artifact parity test: the real CEL
  evaluator consumes the shared policy-context JSONL corpus and compares its
  enforcement backtest row shape to
  `data/enforcement/backtest-expected/http-google-secret.json`. The red pass
  caught header-case drift between fixture storage and canonical evidence keys,
  which is exactly the class of mismatch this corpus is meant to pin.
- Slice 5 added the compiled Detection IR artifact for the shared Sigma
  corpus under `data/detection/ir/`, made Rust Detection IR lowering require
  canonical policy roots such as `http.request.host` instead of legacy
  `subject.request.host`, and pinned the admin detection backtest expected
  artifact from Rust.

## Coverage Ledger

- Unit/contract: Python Pydantic and Rust serde/schema validation over the same
  enforcement/detection/event fixtures; first admin policy and detection
  backtest reports compare against committed expected-result JSON artifacts.
- Functional: admin offline backtest and Rust runtime backtest produce the same
  matched event refs, decisions/findings, and counts. This is proven for the
  first synthetic enforcement and detection corpus; broader corpus diversity
  and real-session rows remain open.
- Adversarial: unsupported Sigma constructs, invalid CEL, missing event fields,
  duplicate rule ids, mismatched expected labels, internal `event.*` /
  `subject.*` authoring, legacy Detection IR `subject.*` paths, and
  evidence-dedup behavior.
- E2E/VM or integration: real-session fixture generation after S08b journal
  lands; no live VM dependency for every corpus test.
- Telemetry/observability: backtest reports include event refs and full local
  evidence; export redaction is tested separately when export exists.
- Performance: corpus backtest has a basic timing budget and reports evaluated
  event/rule counts.
