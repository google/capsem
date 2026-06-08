# Route E2E Gate

This gate exists because route presence is not route completion. A route can be
explicitly registered in the service and gateway and still be read-only,
dry-run, synchronous-status-only, or fail-closed with `501`. The release gate
must distinguish those states.

## Status Legend

- `real`: route has production behavior and focused tests.
- `dry_run`: route intentionally evaluates/validates without mutating runtime
  state or writing a session ledger.
- `read_only`: route reflects current state/config but does not mutate.
- `fail_closed_stub`: route exists so callers get an explicit contract error,
  but product semantics are not implemented.
- `needs_e2e`: route has useful behavior but still needs a black-box service,
  CLI, VM, or session-db proof.

## Current Route Truth

| Area | Routes | Status | Notes |
| --- | --- | --- | --- |
| VM lifecycle | `/vms/create`, `/vms/list`, `/vms/{id}/info`, `/status`, `/start`, `/resume`, `/pause`, `/stop`, `/delete`, `/save`, `/fork` | real, needs_e2e | Existing service/VM suites cover much of this; final route gate must name exact tests. |
| VM edit/restart/reload | `/vms/{id}/edit`, `/restart`, `/reload-profile` | fail_closed_stub | `edit` validates immutable/unknown fields and rejects real edits; restart/reload-profile return `501`. |
| VM operation status | `/vms/{id}/save/status`, `/fork/status` | real-minimal | Returns truthful synchronous `idle` state; no async progress yet. |
| VM files/history/timeline | `/vms/{id}/files/*`, `/history/*`, `/timeline` | real, needs_e2e | File import/export ledger behavior has service tests; final gate must include black-box route coverage. |
| Service ledger | `/security/latest|status`, `/enforcement/latest|status`, `/detection/latest|status` | real, needs_e2e | Service-wide DB-backed views exist. |
| VM ledger | `/vms/{id}/security/latest|status`, `/detection/latest|status`, `/enforcement/latest|status` | real | Bridge test proves route-authored detection can trigger runtime ledger rows and be read back from VM latest route. |
| Profile ledger | profile-filtered latest/status | absent | Do not claim this route exists until implemented. |
| Profiles read/status | `/profiles/list`, `/profiles/status`, `/profiles/reload`, `/profiles/{id}/info`, `/profiles/{id}/validate`, `/profiles/{id}/reload` | real/read_only | Uses the typed profile catalog and reload broadcast path. |
| Profiles write | `/profiles/create`, `/profiles/{id}/edit`, `/delete`, `/clone` | fail_closed_stub | Returns explicit `501 profile file persistence not enabled yet`. |
| Profile assets | `/profiles/{id}/assets/status`, `/info`, `/ensure` | real, needs_e2e | Profile-owned asset status/info/ensure exists. |
| Profile assets edit | `/profiles/{id}/assets/edit` | fail_closed_stub | Returns explicit `501`. |
| Enforcement rules | `/profiles/{id}/enforcement/info`, `/rules/list`, `/evaluate`, `/reload`, `/rules/{rule_id}/edit|delete` | real/dry_run | Rule edit/delete persists user profile rules. `evaluate` is dry-run and does not write a session ledger. |
| Detection rules | `/profiles/{id}/detection/info`, `/rules/list`, `/evaluate`, `/reload`, `/rules/{rule_id}/edit|delete` | real/dry_run | Same rule rail as enforcement; detection edit requires `detection_level`. |
| Plugins | `/profiles/{id}/plugins/list`, `/info`, `/{plugin_id}/info`, `/{plugin_id}/edit` | real, needs_e2e | Handler tests cover dynamic plugin enable/disable and evaluation effects. |
| Skills read | `/profiles/{id}/skills/info`, `/list` | read_only | Reads profile manifest paths. |
| Skills write | `/profiles/{id}/skills/add`, `/{skill_id}/edit|delete` | fail_closed_stub | Validates input then returns explicit `501`. |
| MCP mechanics | `/profiles/{id}/mcp/info`, `/servers/list`, `/servers/{server}/tools/list`, `/refresh`, `/tools/{tool}/edit|call` | real, needs_e2e | Profile-scoped MCP mechanics only; no MCP security decision provider. |
| Settings | `/settings/info`, `/settings/edit` | real | UI/app settings only. |
| Corp | `/corp/info`, `/corp/edit`, `/corp/validate`, `/corp/reload` | real, needs_e2e | Corp validation/edit/reload exists. |
| Gateway parity | explicit service routes | real | Gateway has explicit allowlist; unknown and retired paths 404 instead of fallback-forwarding. |

## First Bridge Proof

Implemented in `crates/capsem-service/src/tests.rs`:

- `route_authored_detection_rule_triggers_runtime_ledger_and_latest_routes`

The test:

1. Creates isolated user/corp settings.
2. Calls the mounted HTTP route
   `PUT /profiles/code/detection/rules/openai_http_observed/edit`.
3. Loads the persisted settings and compiles them as runtime rules.
4. Emits a matching `http.request` security event into a test VM `session.db`
   through `emit_matching_security_rules`.
5. Reads the row back through mounted HTTP routes
   `GET /vms/route-ledger-vm/security/latest` and
   `GET /vms/route-ledger-vm/detection/latest`.
6. Asserts `event_id`, `event_type`, `rule_id`, `rule_action`,
   `detection_level`, `rule_json`, `event_json`, and `trace_id`.

Proof command:

```bash
cargo test -p capsem-service route_authored_detection_rule_triggers_runtime_ledger_and_latest_routes -- --nocapture
```

## Dry-Run Guard

Implemented in `crates/capsem-service/src/tests.rs`:

- `route_enforcement_evaluate_is_dry_run_and_does_not_write_ledger_rows`

This test calls mounted route `POST /profiles/code/enforcement/evaluate` and
proves it may return a blocking decision in the response event, but it does not
write session ledger rows. Runtime boundaries, not evaluation previews, own
ledger emission.

Proof command:

```bash
cargo test -p capsem-service route_enforcement_evaluate_is_dry_run_and_does_not_write_ledger_rows -- --nocapture
```

## Remaining Gate

- Add a generated/maintained route inventory so service, gateway, frontend API,
  CLI, and TUI cannot drift silently.
- For each `real` route, name at least one functional test and one adversarial
  test.
- For every route marked `fail_closed_stub`, add a test asserting the explicit
  error shape and keep it visible in release notes until real semantics land.
- Add at least one black-box service/VM route test for:
  - detection rule -> actual runtime boundary -> session DB/latest route,
  - enforcement block -> boundary refuses action/network/tool,
  - plugin enable/disable -> runtime effect plus ledger detection vector,
  - file import/export ledger route,
  - MCP server/tool mechanics route.
