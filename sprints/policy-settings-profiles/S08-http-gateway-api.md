# S08 - HTTP Gateway API

## Status

In progress. Gateway Profile V2 proxy contract coverage now spans
catalog/revision routes, profile CRUD/resolve, skills, MCP servers,
rules/evaluate, confirm-pending read, profile-selected VM create response
payloads, `/status` profile/asset provenance, `/setup/assets` profile-scoped
download progress, `/debug/report` profile asset provenance, exact typed-error
passthrough, and service debug-report gateway runtime mismatch diagnostics.

S08 is not closed yet. Remaining scope is the live service/VM proof for
profile-selected HTTP create/download/boot, broader adversarial failure cases,
and the S15 confirm resolution routes/stream when S15 makes those routes real.

## Goal

Wire HTTP API to the already-tested UDS behavior, including profile catalog
state and profile-backed VM creation.

## Tasks

- Add HTTP endpoints backed by UDS behavior.
- The gateway fallback already exposes the service `POST
  /profiles/catalog/reconcile` route to authenticated local HTTP callers; S08
  must add HTTP contract tests and client-facing docs for that exact payload
  and typed outcome summary instead of inventing a gateway-only shape.
- Preserve typed errors and provenance payloads.
- Add service/gateway mismatch reporting to debug report.
- Test app settings, profile CRUD, profile catalog/revision state, profile
  package/tool contracts, asset readiness, resolve, MCP, and skills over HTTP.
- Mirror the UDS `ProfileRevisionStatus` enum exactly in HTTP response models:
  `active`, `deprecated`, and `revoked`. Do not invent gateway-only status
  names, `removed`, or boolean substitutes.
- Add/extend VM create HTTP surface so callers can pass `profile_id` and
  optional `profile_revision`. Response must echo resolved profile id/revision,
  package contract hash, pinned asset hashes, and asset readiness/download
  state.
- Stream or poll first-use profile asset download progress through the same
  typed envelope as `/setup/assets`, but scoped to the selected profile
  revision.
- **Mirror the [S07 Rules API](S07-uds-service-api.md#rules-api) on
  the gateway**: `GET /rules`, `GET /rules/{rule_id}`,
  `POST /rules`, `DELETE /rules/{rule_id}`, `POST /rules/evaluate`,
  plus the [S15 resolve routes](S15-confirm-ux.md) (`GET /confirm/pending`,
  `POST /confirm/pending/{ask_id}/{accept|deny|promote-allow|promote-deny}`,
  SSE on `/confirm/pending/stream`). This is the public surface a
  Python E2E test harness (capsem-doctor ask probe, external CI)
  talks to; it must use the same typed-error envelope as the UDS
  side so a client only needs to learn one schema.

## Coverage Ledger

- Unit/contract: first slice covered by
  `tests/capsem-gateway/test_gw_profile_v2_surface.py` and
  `crates/capsem-gateway/src/status/tests.rs`. It verifies exact
  `active|deprecated|revoked` revision status values, catalog/revision
  lifecycle summaries, profile CRUD/resolve envelopes, skills/MCP/rules
  gateway proxy routes, `GET /confirm/pending`, profile-selected VM create
  response identity/pin/asset health, `/status` profile identity plus
  per-asset provenance, `/setup/assets` download progress parity, and
  exact typed-error status/body passthrough for denied profile revision
  operations. Remaining: malformed request and locked mutation parity across
  the rest of profile/rule/asset surfaces.
- Functional: first slice covers HTTP CRUD and resolve tests; a Rules API
  roundtrip (`POST /rules` -> `POST /rules/evaluate` -> assert same
  `matched_rule_id` comes back); and HTTP VM create response echoing selected
  `profile_id`, `profile_revision`, profile pin hashes, and asset state.
  Remaining: prove a real service HTTP create with `profile_id` triggers the
  profile-scoped first-use download and boots with the selected revision.
- Adversarial: malformed requests, locked mutations (built-in rule
  delete attempt, profile lock), gateway/service mismatch, revoked profile,
  stale catalog, incompatible revision, interrupted download, and repeated
  create requests while a download is already in progress.
- E2E/VM: session created through HTTP uses selected profile revision, downloads
  missing verified assets on first use, and pins profile id/revision plus asset
  hashes before boot.
  Rules API + S15 resolve E2E is the prerequisite that un-defers
  the [S06-pre slice 6f capsem-doctor ask probe](tracker.md#slice-6f---exit-tests).
- Telemetry: covers gateway `/status` profile identity and asset provenance
  preservation, `/debug/report` Profile V2 asset provenance preservation, and
  service debug-report issues for invalid/stale/mismatched gateway runtime
  files. Remaining: richer debug report package/catalog/VM-pin summaries across
  live profile-selected HTTP create flows.
- Performance: not primary; `POST /rules/evaluate` must remain a
  read-only operation that does not block concurrent rule writes
  (same `Arc<PolicyConfig>` snapshot contract as the UDS side). Profile catalog
  and readiness endpoints use cached local state and do not perform network
  fetches on list/status paths.

## Verification

- `cargo fmt --all`
- `cargo test -p capsem-gateway status -- --nocapture`
- `cargo build -p capsem-gateway`
- `cargo test -p capsem-gateway`
- `cargo test -p capsem-service debug_report -- --nocapture`
- `uv run pytest tests/capsem-gateway/test_gw_profile_v2_surface.py -q`
- `uv run pytest tests/capsem-gateway/test_gw_profile_v2_surface.py tests/capsem-gateway/test_gw_proxy.py tests/capsem-gateway/test_gw_proxy_advanced.py tests/capsem-gateway/test_gw_status.py tests/capsem-gateway/test_gw_status_advanced.py tests/capsem-gateway/test_gw_auth.py -q`

Full `uv run pytest tests/capsem-gateway -q` is not an S08 closeout yet: the
mock/contract gateway suites pass, but the full directory currently has live
VM/MITM failures waiting for exec-ready or sandbox creation in the real
environment. Keep those visible until the live HTTP VM proof is closed.
