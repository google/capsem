# S07 - UDS Service API

## Goal

Expose typed settings, profiles, profile catalog state, and profile-backed VM
creation through the service UDS API.

## Tasks

- Add service settings endpoints.
- Add profile list/get/create/fork/update/delete endpoints.
- Add profile resolve and VM-effective settings endpoints.
- Reserve the UDS shape for profile catalog/revision endpoints; the backing
  manifest/profile/assets implementation lands in
  [S07a - Profile Manifest, Packages, And Assets](S07a-profile-manifest-assets.md).
  The surface lists catalog profiles, lists revisions, shows lifecycle status,
  shows package/tool contract, shows asset readiness, installs/updates/removes
  local profile revisions, and surfaces revoke/deprecated warnings.
- Use the canonical `ProfileRevisionStatus` enum from S07a for every catalog
  revision status field. Do not expose loose strings or derived boolean flags;
  typed errors and response models must distinguish `active`, `deprecated`, and
  `revoked`. Missing catalog revisions are absence/unknown-revision errors, not
  a `removed` status.
- Extend VM create/provision request shape with explicit profile selection:
  `profile_id` required by UI/CLI flows, `profile_revision` optional for
  advanced/debug use. Absence may default to the service-selected profile, but
  responses must always echo the resolved profile id/revision and pinned asset
  identity.
- Add profile-backed VM create proof: selected profile resolves, required assets
  are present or queued for first-use download, and persistent VM registry pins
  profile id/revision plus asset hashes.
- Add MCP list/add/delete endpoints in the new model.
- Add skills list/add/delete endpoints in the new model.
- Add the Rules API (see below): list / get / add / remove / evaluate
  policy rules through a dedicated route group, separate from the
  bulk `/settings/<profile>/rules` write path. The resolve side of
  the Rules API (answer a pending ask by id) is owned by
  [S15 - Confirm UX](S15-confirm-ux.md); S07 makes sure the rest of
  the API is shaped to plug into the same surface.
- Include provenance and typed validation errors in responses.
- Feed UDS-visible state into debug report.
- Add `capsem_proto::metrics` module (`VmMetricsSnapshot` and family)
  and the `ServiceToProcess::GetMetricsSnapshot` /
  `ProcessToService::MetricsSnapshot` bincode IPC variants. These are
  foundational types for S12 (OpenTelemetry Metrics Architecture);
  landing them in S07 means S12 can start with proto in place. See
  [S12 - Observability plugin](S12-observability-plugin.md) for the
  contract.

## Rules API

A dedicated route group so external tooling -- Python E2E harnesses,
the capsem-doctor ask probe deferred from
[S06-pre slice 6f](tracker.md#slice-6f---exit-tests), the rule editor
UI in [S14 - Rules UI](S14-rules-ui-components.md), and external
remote-policy plugins in [S13](S13-remote-policy-plugin.md) -- can
script the full ask lifecycle without poking at `[security.rules.*]`
TOML by hand.

Routes (UDS; mirrored on the gateway in [S08](S08-http-gateway-api.md)):

- `GET  /rules?profile=<id>&callback=<type>` -> list rules.
  Returns the canonical `security.rules.<type>.<name>` id, the rule
  body (typed, not raw TOML), the source profile, the priority, and
  the rule's match condition. Filterable by callback type and by
  profile.
- `GET  /rules/{rule_id}` -> single rule with full provenance
  (profile, layer, derived-from-ask metadata if any).
- `POST /rules` body: typed rule -> create a rule under the user
  profile. Validation errors are typed (same shape as the rest of S07).
- `DELETE /rules/{rule_id}` -> remove a rule from the user profile.
  Removing a built-in rule fails closed with a typed
  `rule_is_builtin` error; the caller must `POST /rules` an
  overriding rule with higher priority instead.
- `POST /rules/evaluate` body: `{ subject, callback, [profile] }` ->
  run the V2 evaluator against the supplied synthetic subject without
  enforcing. Returns `{ matched_rule_id, decision, would_ask: bool,
  reason }`. This is the test-harness primitive that lets Python /
  capsem-doctor / external CI exercise the rule engine without
  having to drive a real DNS/HTTP/MCP/model request through a VM.
  Implementation note: evaluate must be a pure function of the
  current `Arc<PolicyConfig>` -- it never mutates state, never calls
  the confirmer, and never touches `session.db`.
- **resolve (`POST /confirm/pending/{ask_id}/{accept|deny}` etc.)**
  is owned by [S15 - Confirm UX](S15-confirm-ux.md). S07 just makes
  sure the listing side (`GET /confirm/pending`) shares the same
  typed error shape and provenance fields as the rest of the Rules
  API, so a Python client treats list + add + remove + evaluate +
  resolve as one coherent surface.

The Rules API is the prerequisite for un-deferring the
[S06-pre slice 6f capsem-doctor ask probe](tracker.md#slice-6f---exit-tests):
the probe stages an ask rule via `POST /rules`, drives traffic
through the VM that matches it, picks up the pending ask via
`GET /confirm/pending`, and calls `POST /confirm/pending/{id}/accept`
to resolve. Listing + evaluate also unlocks Python contract tests
for the rule engine that do not need a VM.

## Coverage Ledger

- Unit/contract: request/response shape tests for every route
  (settings, profiles, profile catalog/revisions, profile package/tool contract,
  profile asset readiness, VM create profile selection, MCP, skills,
  **rules list/get/add/remove/evaluate**, provenance, typed errors).
  `ProfileRevisionStatus` enum serialization/deserialization is covered for
  `active`, `deprecated`, and `revoked`. Profile catalog errors must
  distinguish revoked, deprecated, absent/unknown revision, stale catalog,
  incompatible binary, unsupported arch, and verification failure.
- Functional: UDS CRUD and resolve tests, including a roundtrip that
  stages a rule via `POST /rules`, evaluates a synthetic subject via
  `POST /rules/evaluate`, and asserts the same `matched_rule_id`
  comes back. Profile-backed VM create test asserts the selected profile
  revision and pinned asset hashes are echoed.
- Adversarial: invalid payloads, locked mutations (built-in rule
  delete attempt, profile lock), revoked profile selection, unknown profile
  revision, incompatible binary, stale catalog rollback rejection, unsupported
  arch, asset readiness failure, interrupted first-use download, concurrent
  duplicate downloads, concurrent updates, oversize rule bodies, condition
  strings that fail closed at parse time.
- E2E/VM: service-level create/fork/delete profile proof and service-level
  profile-backed VM create. **Rules
  API end-to-end** is the prerequisite that un-defers the
  capsem-doctor ask probe -- track it as the slice that gates that
  E2E re-entry.
- Telemetry: debug report includes UDS-visible config, profile catalog state,
  installed revision, package/tool contract, asset readiness, VM pins, and
  user-authored rules and their provenance.
- Performance: concurrent update behavior tested; the evaluate route
  must run on the read-only `Arc<PolicyConfig>` snapshot so
  concurrent evaluates do not block writers.
