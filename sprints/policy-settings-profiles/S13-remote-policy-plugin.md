# S13 - Remote Policy Plugin

## Goal

Add a service-scoped remote policy plugin.

## Dependency On S08a

[S08a - Rule Abstraction And Detection Architecture](S08a-rule-abstraction-detection-architecture.md)
must define the boundary between event streaming/detection and synchronous
policy decisions before this plugin ships. A remote plugin may receive
normalized events for detection or return decisions for live policy callbacks,
but those are different contracts and must not be conflated.

## Tasks

- Add service settings for endpoint, auth, timeout, and failure behavior.
- Define forwarded events/context.
- Define fail-open/fail-closed behavior by decision surface.
- Wire remote decisions into policy paths without profile TOML depending on the
  endpoint.
- Test allow/block/ask, endpoint failure, timeout, auth failure, redaction, and
  audit output.

### Confirmer integration

The remote policy plugin plugs in behind the same `Confirmer` trait
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
- Adversarial: endpoint failure, timeout, invalid decision, auth failure.
- E2E/VM: remote block/allow proof.
- Telemetry: audit output proves remote decisions.
- Performance: timeout budget tested.
