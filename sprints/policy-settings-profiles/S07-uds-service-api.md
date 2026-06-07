# S07 - UDS Service API

Status: **Done** as of 2026-05-19. HTTP mirroring remains owned by
[S08](S08-http-gateway-api.md), CLI lift by [S09](S09-cli-integration.md),
the production confirm resolution path by [S15](S15-confirm-ux.md), and
profile/UI lift by [S16](S16-profile-ui.md).

Post-S08b note: S07 closed the original Profile V2 policy-rule UDS surface as
`/rules/*`. S08b replaces that generic successor surface with distinct
`/enforcement/*` and `/detection/*` route groups. Future UDS/API work must not
extend `/rules/*` for new enforcement or detection behavior.

## Goal

Expose typed settings, profiles, profile catalog state, and profile-backed VM
creation through the service UDS API.

## Tasks

- Add service settings endpoints.
- Add profile list/get/create/fork/update/delete endpoints.
- Add profile resolve and VM-effective settings endpoints.
- Add `POST /profiles/catalog/reconcile` for signed catalog lifecycle
  reconciliation. The route accepts a validated profile catalog manifest plus
  profile payload public key material, installs/updates current `active`
  revisions, keeps installed `deprecated` revisions available for existing VMs,
  removes launchable/current state for installed `revoked` revisions, and
  returns typed per-revision outcomes plus summary counts.
- Continue filling the UDS shape for profile catalog/revision endpoints; the backing
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
  identity. Initial UDS/CLI request fields have landed for fresh VM create;
  source clones reject override attempts and inherit the source VM profile pin.
- Add profile-backed VM create proof: selected profile resolves, required assets
  are present or queued for first-use download, and persistent VM registry pins
  profile id/revision plus asset hashes. The first service proof now verifies
  selected profile/revision create, first-use selected asset reconciliation,
  selected VM-effective attachment, and complete installed-payload trust.
- Add MCP list/add/delete endpoints in the new model. Landed: the old
  `/mcp/{servers,tools,policy}` and `/mcp/tools/*` management surface is
  removed; `/mcp/connectors` now lists/adds Profile V2 MCP servers, and
  `/mcp/connectors/{id}` deletes direct user servers while rejecting locked
  or inherited servers. The capsem-mcp debug tools now mirror the same
  server surface. The profile file format is the industry-standard top-level
  `mcpServers` map (`command`/`args`/`env` or `url`/`headers`/`bearerToken`);
  Capsem governance lives under `mcpServers.<id>.capsem`. The dead
  service-to-process MCP management IPC variants are deleted.
- Add skills list/add/delete endpoints in the new model. Landed:
  `GET /skills`, `POST /skills`, and `DELETE /skills/{id}` resolve
  `groups` / `enabled` / `disabled` from the selected effective profile,
  mutate direct user profile entries only, materialize default built-in
  overrides when needed, reject duplicate direct/inherited same-kind entries,
  reject inherited deletes, and move a skill between `enabled` and `disabled`
  rather than leaving contradictory state.
- Add the Rules API (see below): list / get / add / remove / evaluate
  policy rules through a dedicated route group, separate from the
  bulk `/settings/<profile>/rules` write path. The resolve side of
  the Rules API (answer a pending ask by id) is owned by
  [S15 - Confirm UX](S15-confirm-ux.md); S07 makes sure the rest of
  the API is shaped to plug into the same surface. Landed: list/get/evaluate
  plus create/delete user-rule mutations with duplicate protection, default
  built-in user override materialization, and locked-rule delete failures.
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

- `GET  /profiles/catalog` -> signed catalog status: configured source,
  persisted manifest path/presence, profile ids, current revision, installed
  revision/payload hash, and per-revision lifecycle status.
- `GET  /profiles/{id}/revisions` -> signed catalog revisions for one profile:
  current revision, installed revision/payload hash, canonical lifecycle status,
  and current/installed markers. Missing catalog manifests and unknown profile
  ids fail as absence/not-found, never as a synthetic `removed` lifecycle.
- `POST /profiles/{id}/revisions/install` body `{ revision?: string }` ->
  install the selected active signed revision, defaulting to `current_revision`.
  Revoked/deprecated installs fail closed instead of becoming launchable.
- `POST /profiles/{id}/revisions/update` body `{ revision?: string }` ->
  reconcile the selected signed revision lifecycle. Active revisions install or
  refresh, deprecated installed revisions stay available for pinned VMs, and
  revoked installed revisions lose launchable state.
- `POST /profiles/{id}/revisions/remove` body `{ revision?: string }` ->
  remove local launchable/current state for the selected installed revision,
  defaulting to the installed revision, while preserving archived payload bytes.
- `GET  /rules?profile=<id>&callback=<type>` -> list rules.
  Returns the canonical `security.rules.<type>.<name>` id, the rule
  body (typed, not raw TOML), the source profile, the priority, and
  the rule's match condition. Filterable by callback type and by
  profile.
- `GET  /rules/{rule_id}` -> single rule with full provenance
  (profile, layer, derived-from-ask metadata if any).
- `POST /rules` body: `{ id, profile?: string, ...typed rule }` -> create a
  rule under the user profile. If `profile` is omitted and the default profile
  is built-in, the service materializes the user override before writing.
  Duplicate direct user rules fail with `409 rule_exists`.
- `DELETE /rules/{rule_id}` -> remove a rule from the user profile.
  Removing a built-in, locked, generated, or inherited rule fails closed with a
  typed `rule_is_builtin` error; the caller must `POST /rules` an overriding
  user rule instead.
- `POST /rules/evaluate` body: `{ subject, callback, [profile] }` ->
  run the V2 evaluator against the supplied synthetic subject without
  enforcing. Returns `{ matched_rule_id, decision, would_ask: bool,
  reason }`. This is the test-harness primitive that lets Python /
  capsem-doctor / external CI exercise the rule engine without
  having to drive a real DNS/HTTP/MCP/model request through a VM.
  Implementation note: evaluate must be a pure function of the
  current `Arc<PolicyConfig>` -- it never mutates state, never calls
  the confirmer, and never touches `session.db`.
- `GET /confirm/pending` -> typed pending-ask listing. In S07 this returns
  the settings-profiles-v2 envelope with an empty queue plus
  `resolve_available = false` / `resolve_owner = "S15-confirm-ux"` because
  the production resolver/prompter is deliberately owned by S15.
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

## S08b Successor Routes

The post-S08b UDS API must expose two distinct route groups.

Enforcement:

- `POST /enforcement/validate`
- `POST /enforcement/compile`
- `POST /enforcement/backtest`
- `GET /enforcement`
- `POST /enforcement`
- `PUT /enforcement/{id}`
- `DELETE /enforcement/{id}`
- `GET /enforcement/stats`

Detection:

- `POST /detection/validate`
- `POST /detection/compile`
- `POST /detection/backtest`
- `GET /detection`
- `POST /detection`
- `PUT /detection/{id}`
- `DELETE /detection/{id}`
- `GET /detection/stats`
- `POST /detection/hunt`
- `POST /sessions/{id}/detection/hunt`

Backtest defaults are part of the contract: return aggregate counts plus up to
100 matched event rows, deduplicated by simple evidence signature for match
diversity. Rows include event refs and full local matched field evidence.
Backtest is not redacted by default. Redaction belongs to export/support-bundle
flows.

## Coverage Ledger

- Unit/contract: request/response shape tests for every route
  (settings, profiles, profile catalog/revisions, profile package/tool contract,
  profile asset readiness, VM create profile selection, MCP, skills,
  **rules list/get/add/remove/evaluate**, provenance, typed errors).
  S07 closeout adds `skills_api_create_list_delete_roundtrip_updates_user_profile`,
  `handle_create_skill_rejects_duplicate_direct_skill`,
  `handle_create_skill_rejects_duplicate_inherited_skill`,
  `handle_create_skill_moves_skill_between_enabled_and_disabled_lists`,
  `handle_delete_skill_rejects_inherited_skill`,
  `handle_list_pending_confirms_returns_typed_empty_s07_surface`, and
  `s07_route_surface_chains_profiles_skills_mcp_rules_and_confirm_listing`.
  `ProfileRevisionStatus` enum serialization/deserialization is covered for
  `active`, `deprecated`, and `revoked`. Profile catalog errors must
  distinguish revoked, deprecated, absent/unknown revision, stale catalog,
  incompatible binary, unsupported arch, and verification failure.
- Functional: UDS CRUD and resolve tests, including Profile V2 MCP server
  list/create/delete over user profile state, plus a roundtrip that
  stages a rule via `POST /rules`, evaluates a synthetic subject via
  `POST /rules/evaluate`, asserts the same `matched_rule_id`
  comes back, deletes it via `DELETE /rules/{rule_id}`, and asserts evaluation
  no longer matches. S07 closeout adds `tests/capsem-service/test_svc_s07_surface.py`
  so the live service harness exercises `GET /confirm/pending` and
  `POST /skills` -> `GET /skills` -> `DELETE /skills/{id}` over the UDS HTTP
  surface. Profile-backed VM create test asserts the selected profile revision
  and pinned asset hashes are echoed.
- Adversarial: invalid payloads, locked mutations (built-in rule
  delete attempt, inherited MCP server delete, inherited skill delete, profile
  lock), duplicate rule create, duplicate MCP server create, duplicate direct
  and inherited same-kind skill create, contradictory enabled/disabled skill
  mutation cleanup, revoked profile selection, unknown
  profile revision, incompatible binary, stale catalog rollback rejection,
  unsupported arch, asset readiness failure, interrupted first-use download,
  concurrent duplicate downloads, concurrent updates, oversize rule bodies,
  condition strings that fail closed at parse time.
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

## Closeout Verification

Focused S07 closeout commands run on 2026-05-19:

- `cargo fmt --all`
- `cargo test -p capsem-service skills_api -- --nocapture`
- `cargo test -p capsem-service handle_create_skill -- --nocapture`
- `cargo test -p capsem-service handle_delete_skill_rejects_inherited_skill -- --nocapture`
- `cargo test -p capsem-service handle_list_pending_confirms -- --nocapture`
- `cargo test -p capsem-service s07_route_surface_chains_profiles_skills_mcp_rules_and_confirm_listing -- --nocapture`
- `cargo test -p capsem-service mcp_connector -- --nocapture`
- `cargo test -p capsem-service`
- `cargo test -p capsem-core profile_manifest --lib -- --nocapture`
- `cargo test -p capsem-core reconcile_profile_revision_from_manifest --lib -- --nocapture`
- `cargo build -p capsem-service`
- `uv run pytest tests/capsem-service/test_svc_s07_surface.py tests/capsem-service/test_svc_mcp_api.py -q`
- `cargo fmt --all -- --check`
- `git diff --check`

During the final package sweep, the real profile payload verification tests
surfaced a stale valid-payload minisign fixture. The fixture public key and
signature were regenerated together, verified with `minisign -Vm`, and then the
core/service profile install and reconcile tests were rerun green. The same
sweep also removed brittle assumptions in the asset-log and profile-root test
fixtures so the full `capsem-service` package can run cleanly.
