# S15 - Confirm UX (Ask)

Last updated: 2026-05-15

## Why this exists

[S06-pre](S06-pre-network-contract-and-confirm.md) wired every Policy V2
ask callsite (DNS, HTTP, MCP, model) through a `Confirmer` trait, and
shipped a placeholder implementation that always returns
`Decision::Accept`. That unblocked the rest of the runtime without
breaking existing tests, but it leaves `decision = "ask"` advertising
"the user will be prompted" while in reality there is no prompter.

S15 delivers the real production answer path:

- A pending-ask queue that survives stacking when multiple asks land
  back-to-back (typical during agent burst traffic).
- A UI prompter that consumes the queue and renders one prompt per ask
  with full context (callback type, subject snapshot, matched rule
  label, reason).
- A CLI prompter at parity with the UI (`capsem confirm list/accept/
  deny/promote-allow/promote-deny`) so headless and remote operators
  can drive the same flow.
- **Auto-rule derivation** so a single Accept-and-add-rule click
  produces a sensible pre-filled rule the user can tweak before
  saving (rather than dumping the user in an empty rule editor).
- Reuse of the [S14 Rules UI](MASTER.md#sprint-board) rule editor
  component embedded inside the confirm prompter. No second editor.
- `policy_confirm_events` integration so every decision is durable and
  attributable, including forward-rule-created (yes/no, new rule id).

The placeholder confirmer remains as a fallback (e.g. for headless
benchmarks and dev VMs where no operator is available) but ceases to
be the production answer path the moment S15 lands.

## Dependency On S08a

[S08a - Rule Abstraction And Detection Architecture](S08a-rule-abstraction-detection-architecture.md)
must settle the rule taxonomy before Confirm promotion lands. `accept` and
`deny` resolve one pending ask. `promote-allow` and `promote-deny` create
synchronous Capsem policy rules unless S08a explicitly decides otherwise.
Detection findings may inform or annotate prompts, but S15 must not silently
turn detections into blocking policy.

## Hard constraints

- **No duplicated rule editor.** The Confirm prompter embeds the
  reusable rule editor introduced in [S14 - Rules UI
  Components](MASTER.md#sprint-board). If S14 has not yet shipped that
  component, S15 blocks on S14 -- it does not fork a second editor.
- **Forward-rule decisions are explicit.** The user must distinguish
  "allow this one call" from "allow all future calls matching this
  pattern". The UI shows four buttons, not two; the CLI exposes
  separate `accept` and `promote-allow` (and `deny` / `promote-deny`)
  verbs. No silent learning.
- **Auto-derived rules are suggestions, not commitments.** The derived
  rule pre-fills the editor; the user must review and confirm before
  it persists. We never auto-create a durable rule on the user's
  behalf.
- **Stacking is observable.** The UI shows the pending-ask count in
  the toolbar at all times. The CLI `list` subcommand returns the
  same queue. An ask that has been waiting longer than the
  per-call `RetryOpts.timeout` budget (from
  [`confirm_with_backoff`](S06-pre-network-contract-and-confirm.md))
  surfaces as a `Decision::Deny` from the engine's side, and the
  pending-ask entry transitions to "expired" in the queue so the user
  knows their attention lapse caused a deny.
- **CLI and UI share state.** Two operators answering asks from the
  same session must not see ghost entries; the queue is the single
  source of truth and both surfaces subscribe to it.

## Architecture

```
Engine matches ask rule
  -> calls Confirmer::confirm(args)
  -> the new ServiceConfirmer impl enqueues a PendingAsk into the
     service's queue (keyed by trace_id + rule_id, dedup-safe)
  -> awaits a tokio::sync::oneshot::Receiver for this ask's Decision
  -> when an operator answers (UI or CLI), the queue dispatches the
     Decision into the oneshot and (optionally) creates a forward
     rule via the existing /settings/<profile>/rules write path
  -> the Confirmer's await resumes; engine enforces the Decision
  -> a policy_confirm_events row is written with the resolution

The placeholder confirmer stays available for headless / benchmark /
dev contexts; the service decides which to install based on a
`confirm_authority` setting.
```

Queue:

```rust
pub struct PendingAsk {
    pub ask_id: AskId,                // ULID; stable for the lifetime of the ask
    pub session_id: SessionId,
    pub callback: PolicyCallback,
    pub rule_id: String,              // canonical security.rules.<type>.<name>
    pub rule_label: String,           // human-readable
    pub reason: Option<String>,
    pub subject_snapshot: serde_json::Value,  // already redacted by S06-pre
    pub derived_rule: DerivedRuleSuggestion,  // see auto-rule derivation
    pub enqueued_at: SystemTime,
    pub expires_at: SystemTime,       // enqueued_at + RetryOpts.timeout
}
```

`AskId` is ULID so the queue order is observable and the API is stable
for replay/audit.

## Auto-rule derivation

When the user picks "Allow forward" or "Deny forward", the UI/CLI
needs a starting rule. We derive one from the ask context. Each
callback type has its own derivation strategy, owned by a single
module (`capsem-core::policy::ask_to_rule`) so the logic is testable
in isolation.

| Callback | Derived condition | Rule type |
| --- | --- | --- |
| `dns.request` | `qname == "<exact qname>"` (or `qname.endsWith(".<parent>")` if user-selected scope=parent) | dns |
| `http.request` | `request.host == "<host>" && request.path.startsWith("<path-prefix>")` | http |
| `http.response` | `request.host == "<host>" && response.status == "<status>"` | http |
| `mcp.request` | `method == "<method>" && server.name == "<server>" && tool.name == "<tool>"` (subset of fields present in snapshot) | mcp |
| `mcp.response` | same scope as the matched request, plus `response.is_error` if relevant | mcp |
| `model.request` | `provider == "<provider>" && model == "<model>"` | model |
| `model.response` | same scope as request, plus the relevant response signal | model |
| `model.tool_call` | `provider == "<provider>" && tool.name == "<name>"` | model |
| `model.tool_response` | `provider == "<provider>" && tool.call_id == "<id>"` (often the user will broaden this) | model |
| `hook.<name>` | best-effort from hook args; user always reviews | hook |

The derivation never widens beyond the ask's actual subject. The user
broadens it in the editor if they want. Narrowing (down to a single
request id) is always available.

`DerivedRuleSuggestion` carries the derived rule TOML plus a list of
"scope knobs" the UI exposes (e.g. "exact qname" / "parent domain"
toggle for DNS, "exact path" / "path prefix" / "host only" toggle for
HTTP).

## Surfaces

### Service

- New `capsem-core::policy::ask_queue` module owning `PendingAsk`,
  `AskId`, and the `Arc<Mutex<HashMap<AskId, PendingAskState>>>`
  shared between the engine-facing `ServiceConfirmer` and the
  operator-facing UDS/HTTP APIs.
- New UDS routes (consumed by both UI and CLI; these are the
  **resolve** side of the [S07 Rules API](S07-uds-service-api.md#rules-api),
  shaped to share its typed error envelope and provenance fields):
  - `GET /confirm/pending` -> list of `PendingAsk`
  - `GET /confirm/pending/{ask_id}` -> single ask + derived rule
  - `POST /confirm/pending/{ask_id}/accept` -> resolve once
  - `POST /confirm/pending/{ask_id}/deny` -> resolve once
  - `POST /confirm/pending/{ask_id}/promote-allow` body: optional
    edited `DerivedRuleSuggestion` -> create rule + resolve once
  - `POST /confirm/pending/{ask_id}/promote-deny` body: same
  - WebSocket / SSE on `/confirm/pending/stream` -> push enqueue +
    resolve + expire events to subscribers
- New service setting `confirm_authority: placeholder | user_ui`
  (typed enum). On boot the service installs the matching Confirmer
  impl. Defaults to `placeholder` so the upgrade path is opt-in.
- Re-use existing `/settings/<profile>/rules` write path for forward
  rules; no new rule-write API.

### UI

- New top-bar bell + pending-ask count badge, always visible. Click ->
  drawer.
- Drawer lists pending asks, newest first; each row shows callback
  icon + rule label + truncated reason + age.
- Selecting an ask opens a detail pane with:
  - Subject snapshot (read-only JSON tree)
  - Matched rule label + reason + originating profile
  - Four decision buttons: Accept once / Deny once / Allow forward /
    Deny forward
  - When forward is chosen: the S14 rule editor embeds inline,
    pre-filled with the derived rule. Editor shows the scope knobs
    surfaced by `DerivedRuleSuggestion`.
- Expired asks are visually distinct and surface a "deny-by-timeout"
  banner with the elapsed time -- so the user understands what
  happened.

### CLI

- `capsem confirm list [--session <id>] [--format json|table]`
- `capsem confirm show <ask_id>`
- `capsem confirm accept <ask_id>` / `deny <ask_id>`
- `capsem confirm promote-allow <ask_id> [--scope <derived-knob>] [--edit]`
- `capsem confirm promote-deny <ask_id> [--scope <derived-knob>] [--edit]`
- `capsem confirm watch` -> tails the stream endpoint for live
  enqueue/resolve/expire events (useful in agent demos).

`--edit` opens `$EDITOR` on the derived rule TOML before submitting,
so power users can refine the rule beyond what the scope knobs
expose.

### Telemetry (`policy_confirm_events`)

Each resolution writes a row with:

- `ask_id`, `session_id`, `trace_id`, `ts`
- `callback`, `rule_id`, `rule_label`, `reason`
- `original_action = "ask"`
- `resolved_decision = accept | deny | expired`
- `confirmer = placeholder | user_ui | cli | remote_plugin`
- `forward_rule_id` (Some(rule_id) when the user promoted, None
  otherwise)
- `decision_latency_ms` (enqueue -> resolve elapsed)

This table is shared with [S12 - OpenTelemetry Metrics
Architecture](S12-observability-plugin.md); see that spec for the
live-counter rollup.

## Slice plan (per [/dev-sprint](../../skills/dev-sprint/SKILL.md))

Slices are sketched here. Each slice's tracker block must enumerate
its per-slice test tasks per the test-discipline rule (unit +
adversarial + functional/E2E + telemetry as applicable).

1. **Slice 15a -- Queue + ServiceConfirmer skeleton.** PendingAsk /
   AskId / queue mutex + the ServiceConfirmer impl that enqueues and
   awaits a oneshot. Unit tests on enqueue/resolve/expire race
   semantics. Adversarial: confirmer dropped before resolve, two
   asks colliding on same `(trace_id, rule_id)`, queue overflow
   handling.
2. **Slice 15b -- Auto-rule derivation module.** `ask_to_rule` per
   callback type. Unit tests with golden fixtures across every
   callback. Adversarial: snapshot missing fields, hostile
   subject_snapshot (already length-capped per S06-pre slice 6b-6e
   backfill).
3. **Slice 15c -- UDS routes + stream endpoint.** Service-side
   handlers + Axum routes + a contract test that exercises the
   enqueue -> stream -> resolve loop.
4. **Slice 15d -- CLI.** `capsem confirm list/show/accept/deny/
   promote-allow/promote-deny/watch`. Snapshot tests of CLI output
   plus an E2E test that drives a real ask end-to-end through the
   CLI.
5. **Slice 15e -- UI drawer + rule-editor embed.** Frontend
   components: bell + drawer + detail pane + S14 rule editor
   embed. Reuses Svelte runes pattern. Visual verification via
   Chrome DevTools MCP per [/dev-testing-frontend](../../skills/dev-testing-frontend/SKILL.md).
6. **Slice 15f -- Telemetry integration + capsem-doctor E2E.**
   `policy_confirm_events` write path, capsem-doctor probe that
   fires one ask per callback through the CLI and asserts the
   recorded row attributes correctly.
7. **Slice 15g -- Settings cutover + docs.** Switch the default
   `confirm_authority` for installed deployments, document the
   operator workflow, deprecate the placeholder for production
   contexts.

## Open questions (decide before slice 15a)

- **Single-host vs multi-host queue scope.** Is the pending-ask
  queue per-VM or service-global? A service-global queue gives one
  inbox per operator; a per-VM queue isolates asks. Likely
  service-global with VM tag, but confirm before slice 15a.
- **Auth on the confirm endpoints.** The UDS endpoints inherit
  local-uid auth. The HTTP gateway endpoints need a token /
  pairing. Probably reuse the existing gateway token scheme; verify
  in [S08 - HTTP gateway API](MASTER.md#sprint-board).
- **Editor handoff.** When `--edit` opens `$EDITOR`, do we round-trip
  through the same canonical-TOML serialization the rule editor
  uses? Confirm parity so a CLI-edited rule looks identical to a
  UI-edited one.
- **Replay safety.** If the operator runs `capsem confirm accept
  <ask_id>` twice (e.g. flaky network), the second call must be an
  idempotent no-op, not a panic. Test it in slice 15c.

## Non-goals

- Not a remote policy plugin authority. That is [S13 - Remote
  Policy Plugin](MASTER.md#sprint-board); when it lands, the
  Confirmer trait gains a `RemotePluginConfirmer` impl that uses
  the same queue plumbing but answers programmatically.
- Not a "learn my preferences" automated resolver. The
  `ConfirmerKind::Automated` slot in the existing enum is for the
  future automated-resolver work; S15 leaves it as a typed slot but
  ships only the `UserUi` and `Cli` authorities.
- Not the `policy_confirm_events` durable table schema itself --
  that is a S06-pre slice 7 deliverable. S15 consumes the schema;
  it does not define it.
- Not the streaming body-cap inspector. That stays in S06-pre.
