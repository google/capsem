# S06-pre - Network Contract And Confirm Wiring

Last updated: 2026-05-14

## Goal

Create a strict pre-sprint gate before S06 that:

- normalizes network rule callback and field contracts,
- wires `ask` decisions through a shared `confirm()` path with telemetry,
- delivers instant propagation of policy changes to running sessions, and
- replaces the prior 5 MiB hard-cap body proposal with a streaming, sliding-
  window body inspector with bounded memory.

S06 must not start resolver cutover work until this sprint passes.

## Architectural Contract

### Path normalization

- Canonical profile rule path: `security.rules.<type>.<rule_name>`.
- `<type>` is `mcp|http|dns|model|hook`.
- Default rule priority when unspecified: `1`.

### Callback contract

- DNS callbacks: `dns.request`, `dns.response`. `dns.query` is removed and
  rejected at parse time.
- HTTP body fields: `request.data` (request body), `response.content` (response
  body). Old names (`request.body`, `response.body`) rejected at parse.
- MCP request argument grammar: `arguments` (entire args object) and
  `arguments.<dotted.json.path>` for nested access.
- `<type>` to callback mapping is exhaustive and validated.

### Universal `ask` semantics

`ask` is valid on every callback type, including all hook callbacks. No
asymmetry, no special case. Engine never decides on the user's behalf whether
asking is sensible for a given callback — that judgment belongs to the
decision authority (UI prompter, remote policy plugin, future automated
resolver).

The flow is uniform:

1. Engine evaluates rules, matches one with `action = "ask"`.
2. Engine calls `Confirmer::confirm(ConfirmArgs)`.
3. `confirm()` returns `Decision::Accept | Decision::Deny`.
4. Engine enforces the returned decision as if it were the original action.
5. Engine emits a `policy_confirm_events` row capturing `original_action="ask"`
   and `resolved_decision=accept|deny`.

`Confirmer` is an async trait owned by `capsem-core` so that
`capsem-process`, the MITM proxy, and future remote-plugin integrations all
use one path. The S06-pre placeholder implementation returns `Decision::Accept`
unconditionally, keeping existing test paths green until S14/S17 land real UI
and S13 lands the remote authority.

### Instant propagation

Policy changes propagate to running sessions immediately. There is no
"snapshot lifetime" tied to session start. A user tightening rules to clamp
down a misbehaving VM, loosening rules to authorize a new domain, or a remote
plugin pushing an enforcement decision must all take effect on the very next
rule evaluation in every affected session.

Mechanism:

1. Source of truth on disk: `vm-effective-settings.toml`, written
   atomically (temp + rename) by `capsem-service` whenever any input
   changes.
2. Inputs that trigger regeneration: profile edit, corp directive change,
   capability toggle, persisted confirm decision (future), remote policy
   plugin push (S13), service settings reload.
3. After write, `capsem-service` sends `ReloadConfig` over UDS to every
   `capsem-process` whose session is affected.
4. `capsem-process` holds `Arc<PolicyState>` (via `ArcSwap` where not already
   in place). Reload = atomic pointer swap. Each in-flight evaluation grabs
   the current `Arc` at start and runs against a consistent snapshot for
   the lifetime of that one evaluation.
5. Streaming body inspection captures the `Arc` at request start *and*
   re-checks pointer equality between chunks. If the `Arc` has changed and
   re-evaluation of the matched rules under the new state would now `deny`,
   the in-flight stream is aborted with reason `policy_revoked` and a
   telemetry event is emitted.

### Streaming HTTP body inspection

The prior 5 MiB hard-cap with fail-closed overflow is replaced. Bodies of any
size are inspected fully via a sliding window. Memory per connection
direction is bounded at `window + overlap`, independent of body size.

Parameters:

- `window = 2 MiB` (sliding scan window).
- `overlap = 128 KiB` (carried from chunk N into chunk N+1).
- `max_decompression_ratio = 100` (decompressed:wire). Deny with reason
  `body_decompress_ratio` if exceeded.

Contract:

- Bodies are scanned only when at least one active rule needs body content
  (`request.data` or `response.content` referenced by a matched rule's
  condition or rewrite target). No body-dependent rule active → no buffering.
- Compressed bodies (`gzip|deflate|zstd|br`) are decompressed first; the
  scanner sees the decompressed stream. Decompression failures mid-stream:
  deny with reason `body_decompress_failed`.
- Pattern max-match-length contract: every regex/literal in a body-targeted
  rule has its maximum possible match length computed at TOML parse time. If
  that exceeds `overlap`, the profile load fails with typed error
  `pattern_max_len_exceeds_scan_overlap` (rule id, pattern_max, overlap).
  Patterns with unbounded repetition are reduced to their bounded
  "interesting span" before the check.
- Rewrites under streaming (S06a scope): regex-substitute only. Structural
  rewrites (JSON-aware field rewrites etc.) are rejected at parse time with
  a typed error pointing to the future sprint that would add them.
- No paranoid memory backstop. If streaming bookkeeping is correct, resident
  bytes per direction are bounded by `window + overlap`. Backstops would
  hide bugs rather than catch them.

### Confirm telemetry

A dedicated `policy_confirm_events` event struct, schema, writer, and reader
land in `capsem-logger`. Each row captures:

- `event_id`, `session_id`, `trace_id`, `ts` for correlation.
- `callback` (e.g. `http.request`, `dns.response`, `mcp.request`).
- `rule_id` (matched rule path: `security.rules.<type>.<rule_name>`).
- `original_action` (always `"ask"` in v1 but typed so future actions can
  be observed here).
- `resolved_decision` (`accept` | `deny`).
- `confirmer` (`placeholder` | `user_ui` | `remote_plugin` | `automated`).
- `args_snapshot` (JSON: subject of the eval — domain, host, method, path,
  args projection, model request metadata, etc., redacted per existing
  redaction policy).
- `reason` (optional human-readable).

This table is the durable source of truth for the policy "ask" event
boundary. S12 (OpenTelemetry Metrics Architecture) defines the live
counter rollup of these events into the per-VM in-memory accumulator
exposed via `VmMetricsSnapshot.ask`:

- `total_asks` = COUNT(*) of `policy_confirm_events`.
- `asks_allowed` = WHERE `resolved_decision = 'accept'`.
- `asks_denied` = WHERE `resolved_decision = 'deny'`.
- `asks_errored` = confirmer error / timeout / panic (recorded in the
  same table with a sentinel `resolved_decision` value or a sibling
  error column — exact field shape decided in S06-pre slice 7 design).

The Confirmer trait's binary `Accept | Deny` outcome is the engine
contract; the legacy MCP `ToolDecision::Warn` UX concept does not get a
third ask outcome (see S12 "Decision on `asks_warned`").

### Body inspection telemetry

Each body-bearing evaluation emits a `policy_body_inspection_events` row:

- `bytes_scanned`, `bytes_emitted`, `decompressed: bool`.
- `window_size`, `overlap` (so tuning changes show up in history).
- `matched_rule_ids`.
- `outcome` (`allow` | `deny` | `rewritten` | `policy_revoked`
  | `body_decompress_failed` | `body_decompress_ratio`).

### Documentation handoff

Capture exact callback, field, confirm, propagation, and streaming
semantics so S19 can document the final rule engine contract without
re-deriving them.

### Model request rewrite

S06-pre keeps the existing fail-closed behavior for `model.request` rewrite
unchanged. Full `model.request` rewrite support is owned by
`S06a-model-request-rewrite-support.md` and runs after S06-pre lands.

## Per-Slice Test Discipline (read first)

**Every remaining slice in this sprint must add tests across multiple
categories. A slice shipped with only happy-path unit tests is
incomplete.** This rule is enforced by `/dev-sprint`; the requirement
is written down here too because the cost of getting it wrong inside
the policy/confirm stack is high (a missed adversarial test is a
production-grade trust hole, not an aesthetic gap).

For each slice you commit, the tracker block for that slice must have
checkmarks against:

1. **Unit/contract tests** -- at least one new test in the smallest
   meaningful logic boundary (the new helper, the new resolver, the
   new parser arm). Default minimum: one Accept-path and one Deny-path
   test for any new confirmer routing.
2. **Adversarial tests** -- at least one test covering a hostile or
   error path. For confirm wiring the menu is:
   - confirmer returns `Deny` for an ask rule (already a default test)
   - confirmer panics or returns an error (must not crash the whole
     request; must fail closed and surface a typed reason)
   - confirmer hangs past a sane timeout (must not block the request
     forever; document the bound)
   - malformed/oversized `ConfirmArgs.args_snapshot` (must not panic
     or leak)
   - redaction check: secrets present in the live subject must NOT
     appear in `args_snapshot`
   - concurrent confirms on the same VM/session must not deadlock or
     interleave decisions across rules
3. **Functional or E2E test** -- at least one test that exercises the
   production-facing path. For S06-pre slices that path is one of:
   - integration test that runs the eval through the real MITM
     pipeline or the real MCP dispatcher with a mock confirmer
   - capsem-doctor / `just smoke` exercise that fires the callback
     from inside a running VM and asserts the persisted telemetry
   - real session.db inspection after a session that exercised the
     callback
   "I tested it manually" is not enough. The test must be runnable
   automatically.
4. **Telemetry check** -- mandatory for slices that touch
   `policy_confirm_events`, `policy_body_inspection_events`, or any
   `*_calls` row.
5. **Performance check** -- mandatory for slices that claim a
   performance property (streaming body inspector window size,
   instant-propagation overhead). Optional otherwise.

If a category is genuinely not applicable for a slice (e.g. there is
no E2E surface to test against yet because no producer fires the
callback), the tracker block must say so explicitly with a one-line
justification AND open a follow-up task in the sprint-wide rollup --
do not leave it blank.

The historic slices 6b/6c/6d/6e shipped with only the Accept/Deny
unit-test pair. The honest adversarial and E2E debt for those slices
is tracked in the sprint-wide rollup as follow-up tasks; no further
slices should ship with that debt.

## Tasks

- [ ] Update callback enums/parsers/validators: accept `dns.request` and
      `dns.response`, reject `dns.query` with explicit typed error.
- [ ] Update HTTP condition field allowlists/subjects to `request.data` and
      `response.content`; reject `request.body`/`response.body`.
- [ ] Normalize MCP argument grammar/validation/documentation to
      `arguments` and `arguments.<path>`.
- [ ] Set default rule priority to `1` consistently across policy and
      profile canonical rule parsing.
- [ ] Migrate in-tree fixtures, built-in profiles, corp examples, docs, and
      tests off the deprecated callback/field names. No alias layer.
- [ ] Add `Confirmer` async trait to `capsem-core` with placeholder
      implementation returning `Decision::Accept`. Wire every callback site
      (DNS request/response, HTTP request/response, MCP request/response,
      model request/response, tool call/response, all hook callbacks)
      through `confirm()` for `ask`-matched rules.
- [ ] Allow `ask` on every callback type at parse time. Remove any prior
      callback-type asymmetry.
- [ ] Add `policy_confirm_events` event struct, sqlite schema, async writer,
      and reader queries in `capsem-logger`.
- [ ] Add `policy_body_inspection_events` event struct, schema, writer, and
      reader queries in `capsem-logger`.
- [ ] Implement streaming sliding-window body inspector with 2 MiB / 128 KiB
      defaults in the MITM body pipeline. Activate only when an active rule
      references `request.data` or `response.content`.
- [ ] Wire decompression-first scanning for gzip/deflate/zstd/br with 100×
      ratio ceiling and `body_decompress_failed`/`body_decompress_ratio`
      typed denials.
- [ ] Enforce pattern max-match-length contract at TOML parse time with
      typed error `pattern_max_len_exceeds_scan_overlap`. Compute bounded
      interesting span for patterns with unbounded repetition.
- [ ] Reject structural rewrite rules at parse time pending a dedicated
      future sprint.
- [ ] Plumb instant propagation: `capsem-service` atomic snapshot write +
      `ReloadConfig` push to affected `capsem-process` sessions.
      `capsem-process` `Arc<PolicyState>` pointer-swap on reload.
- [ ] Implement per-chunk `Arc` revalidation in the streaming scanner.
      Abort in-flight streams with reason `policy_revoked` when re-evaluation
      under the new state would deny.
- [ ] Add parser, behavior, telemetry, and propagation tests for all of the
      above.
- [ ] Sync tracker/MASTER/S06 dependency references after implementation.

## Verification Gate

Run after implementation:

```sh
cargo test -p capsem-core -p capsem-logger -p capsem-process -p capsem-service
just smoke
```

E2E gate against a real VM is required:

```sh
just run "capsem-doctor"
just inspect-session
```

The `policy_confirm_events` and `policy_body_inspection_events` tables must
contain rows from a real session after the smoke run.

## Coverage Ledger

- Unit/contract:
  - callback parsing (`dns.request`/`dns.response`, reject `dns.query`)
  - field parsing/allowlists (`request.data`, `response.content`,
    `arguments` + `arguments.<path>`)
  - default priority fallback = 1
  - confirm event schema serialization
  - body inspection event schema serialization
  - pattern max-match-length parse-time enforcement
  - `Decision` and `Confirmer` trait contracts
- Functional:
  - `ask` invokes `confirm()` and enforces returned `accept|deny`
    uniformly across DNS/HTTP/MCP/model/hook callbacks
  - placeholder `Confirmer` returns `accept` and existing flows remain green
  - confirm events are persisted and queryable
  - body inspection events are persisted and queryable
  - `ReloadConfig` push triggers `Arc<PolicyState>` swap and next eval sees
    new state
  - streaming scanner inspects full body without exceeding `window + overlap`
    resident bytes
  - in-flight stream is aborted with `policy_revoked` when policy tightens
    mid-stream
- Adversarial:
  - malformed callback names rejected
  - deprecated callback/field names rejected with actionable error
  - pattern whose bounded match length exceeds overlap rejected at parse
  - decompression bomb > 100× ratio denied with `body_decompress_ratio`
  - mid-stream decompression failure denied with `body_decompress_failed`
  - structural rewrite rule rejected at parse with future-sprint pointer
  - confirm telemetry writer failure fails closed for the affected event
- E2E/VM:
  - DNS/MCP/HTTP/model/hook `ask` paths emit confirm telemetry under real
    runtime flows
  - body-dependent HTTP rule matches across chunk boundary inside the overlap
  - body-dependent HTTP rule with massive (multi-hundred-MB) body stays
    within memory bound and inspects to completion
  - profile edit while session is running propagates to next request without
    restart; in-flight body stream observes `policy_revoked` when rule
    tightens to deny
- Telemetry:
  - confirm events include rule id, callback, original_action,
    resolved_decision, confirmer, args_snapshot, trace/session linkage
  - body inspection events include bytes_scanned, bytes_emitted,
    decompressed, window_size, overlap, matched_rule_ids, outcome
- Performance:
  - no body buffering when no body-dependent rules apply
  - bounded memory at `window + overlap` confirmed for arbitrarily large
    body
  - per-chunk `Arc` revalidation cost confirmed negligible (atomic load
    only, no allocation)
