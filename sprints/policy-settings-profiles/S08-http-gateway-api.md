# S08 - HTTP Gateway API

## Goal

Wire HTTP API to the already-tested UDS behavior.

## Tasks

- Add HTTP endpoints backed by UDS behavior.
- Preserve typed errors and provenance payloads.
- Add service/gateway mismatch reporting to debug report.
- Test app settings, profile CRUD, resolve, MCP, and skills over HTTP.
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

- Unit/contract: HTTP response shape tests (incl. the Rules API
  routes; assert the typed-error envelope matches UDS).
- Functional: HTTP CRUD and resolve tests; a Rules API roundtrip
  (`POST /rules` -> `POST /rules/evaluate` -> assert same
  `matched_rule_id` comes back).
- Adversarial: malformed requests, locked mutations (built-in rule
  delete attempt, profile lock), gateway/service mismatch.
- E2E/VM: session created through HTTP uses selected profile.
  Rules API + S15 resolve E2E is the prerequisite that un-defers
  the [S06-pre slice 6f capsem-doctor ask probe](tracker.md#slice-6f---exit-tests).
- Telemetry: debug report includes gateway-visible state.
- Performance: not primary; `POST /rules/evaluate` must remain a
  read-only operation that does not block concurrent rule writes
  (same `Arc<PolicyConfig>` snapshot contract as the UDS side).
