# S13 - Remote Enforcement Plugin

## Goal

Add a service-scoped remote enforcement plugin and observer integration.

This is also the centralized forward-plugin sprint. A central controller can
receive realtime enforcement requests in decision mode and receive resolved
events/detection findings in observer mode, but the local Capsem service remains
the API owner for `/enforcement/*`, `/detection/*`, match stats, backtest, hunt,
and resolved-event journaling.

## Dependency On S08a

[S08a - Rule Abstraction And Detection Architecture](S08a-rule-abstraction-detection-architecture.md)
defines the boundary between event streaming/detection and synchronous
enforcement decisions. A remote plugin can have two explicit modes:

- **Decision mode:** consumes a redacted `SecurityEvent` plus enforcement
  context and returns a bounded `SecurityDecision` for synchronous enforcement.
- **Observer mode:** consumes `ResolvedSecurityEvent` records after local
  enforcement/confirm/detection/postprocessing and may export or enrich findings,
  but cannot block or rewrite the already-resolved event.

The plugin contract must never let a detection finding silently become a
blocking decision.

Post-regroup route contract: the plugin consumes the S08b service-owned
`/enforcement/*` and `/detection/*` model. Decision mode participates only in
the enforcement path. Observer mode may receive resolved events with attached
detection findings and match stats, but it does not mutate `/detection/*` rules
or convert findings into enforcement. Runtime add/update/delete/list/stats,
backtest, and detection hunt remain owned by the Capsem service APIs.

## Tasks

- Add service settings for endpoint, auth, timeout, and failure behavior.
- Define forwarded events/context.
- Define fail-open/fail-closed behavior by decision surface.
- Define separate decision-mode and observer-mode payloads.
- Define centralized forward-plugin payloads for:
  - realtime enforcement decision requests and replies;
  - observer-mode resolved events with attached detection findings;
  - registry/stat deltas needed by centralized dashboards without bypassing
    local `/enforcement/*` and `/detection/*` APIs.
- Wire remote decisions into enforcement paths without profile TOML depending
  on the endpoint.
- Ensure remote-origin decisions, confirms, timeouts, denials, and observer
  exports are attached to the resolved event before telemetry/audit/logging
  fan-out.
- Add docs requirements for centralized operation: how admins configure the
  forward plugin, how realtime enforcement differs from detection, how
  centralized review sees the same event ids as local timeline/backtest/hunt,
  and how VM status/OTel expose remote decision and observer health.
- Test allow/block/ask, endpoint failure, timeout, auth failure, redaction, and
  audit output.

### Confirmer integration

The remote enforcement plugin plugs in behind the same `Confirmer` trait
introduced in S06-pre, alongside the placeholder and the
[S15 UI/CLI prompter](S15-confirm-ux.md). All three are switchable
authorities for the same `decision = "ask"` resolution path:

- The service setting `confirm_authority` (added in S15) gets a
  third variant: `remote_plugin`.
- A `RemotePluginConfirmer` impl forwards `ConfirmArgs` to the
  configured endpoint and maps the response onto `Decision::Accept`
  / `Deny`. Redaction is already enforced at snapshot construction
  time (see the S06-pre adversarial backfill) -- the plugin
  receives the same redacted snapshot the UI would.
- `confirm_with_backoff` already wraps each attempt in a
  per-attempt timeout and fails closed on the overall deadline, so
  endpoint failure / timeout / hang are bounded the same way for
  the remote plugin as for any other authority.
- Auth failures and invalid decisions map to `Decision::Deny`
  (fail-closed) with a typed audit reason so telemetry attribution
  surfaces *why* the resolution flipped.

## Coverage Ledger

- Unit/contract: request/decision shape tests.
- Functional: remote decision tests.
- Functional: centralized forward-plugin decision and observer payload tests.
- Adversarial: endpoint failure, timeout, invalid decision, auth failure.
- E2E/VM: remote block/allow proof.
- Telemetry: audit output proves remote decisions, observer exports, and
  detection findings remain separate from enforcement decisions; VM status
  exposes forward-plugin health and last-error summaries without high-cardinality
  labels.
- Performance: timeout budget tested.
