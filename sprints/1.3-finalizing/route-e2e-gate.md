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
| VM edit/restart/reload | `/vms/{id}/edit`, `/restart`, `/reload-profile` | fail_closed_stub, mounted_proof | `mounted_fail_closed_stub_routes_return_explicit_errors` asserts the public `501` error shape. |
| VM operation status | `/vms/{id}/save/status`, `/fork/status` | real-minimal | Returns truthful synchronous `idle` state; no async progress yet. |
| VM files/history/timeline | `/vms/{id}/files/*`, `/history/*`, `/timeline` | real, partial_mounted_proof | `mounted_file_import_export_routes_log_boundary_events` proves mounted file import/export routes send ledger boundary IPC before bytes move. History/timeline still need mounted route proof. |
| Service ledger | `/security/latest|status`, `/enforcement/latest|status`, `/detection/latest|status` | real, mounted_proof | `mounted_service_ledger_routes_read_real_session_db_rows` proves service-wide latest/status read real session DB rows. |
| VM ledger | `/vms/{id}/security/latest|status`, `/detection/latest|status`, `/enforcement/latest|status` | real | Bridge test proves route-authored detection can trigger runtime ledger rows and be read back from VM latest route. |
| Profile ledger | profile-filtered latest/status | absent | Do not claim this route exists until implemented. |
| Profiles read/status | `/profiles/list`, `/profiles/status`, `/profiles/reload`, `/profiles/{id}/info`, `/profiles/{id}/validate`, `/profiles/{id}/reload` | real/read_only, partial_mounted_proof | `mounted_read_routes_reflect_profile_settings_corp_mcp_and_assets_contracts` covers list/status/info/validate. Reload routes still need named mounted proof. |
| Profiles write | `/profiles/create`, `/profiles/{id}/edit`, `/delete`, `/clone` | fail_closed_stub, mounted_proof | `mounted_fail_closed_stub_routes_return_explicit_errors` asserts the public `501` error shape. |
| Profile assets | `/profiles/{id}/assets/status`, `/info`, `/ensure` | real, partial_mounted_proof | Mounted read proof covers assets info. Status/ensure still need named mounted proof. |
| Profile assets edit | `/profiles/{id}/assets/edit` | unmounted | Asset references are authored by capsem-admin/materialized profiles; `profile_assets_edit_route_is_not_mounted` and `gateway_profile_assets_edit_is_not_forwarded` assert the route stays absent until a typed profile mutation exists. |
| Enforcement rules | `/profiles/{id}/enforcement/info`, `/rules/list`, `/evaluate`, `/reload`, `/rules/{rule_id}/edit|delete` | real/dry_run | Rule edit/delete persists user profile rules. `evaluate` is dry-run and does not write a session ledger. |
| Detection rules | `/profiles/{id}/detection/info`, `/rules/list`, `/evaluate`, `/reload`, `/rules/{rule_id}/edit|delete` | real/dry_run | Same rule rail as enforcement; detection edit requires `detection_level`. |
| Plugins | `/profiles/{id}/plugins/list`, `/info`, `/{plugin_id}/info`, `/{plugin_id}/edit` | real, mounted_proof | `mounted_plugin_routes_control_profile_evaluation` proves list/edit and evaluation effect through mounted routes. |
| Skills read | `/profiles/{id}/skills/info`, `/list` | read_only | Reads profile manifest paths; handler proof exists, mounted proof still needed. |
| Skills write | `/profiles/{id}/skills/add`, `/{skill_id}/edit|delete` | fail_closed_stub, mounted_proof | `mounted_fail_closed_stub_routes_return_explicit_errors` asserts the public `501` error shape. |
| MCP mechanics | `/profiles/{id}/mcp/info`, `/servers/list`, `/servers/{server}/tools/list`, `/refresh`, `/tools/{tool}/edit|call` | real, partial_mounted_proof | `mounted_mcp_routes_are_profile_scoped_mechanics_only` proves profile/server isolation and refresh. `local_http_mcp_e2e_uses_brokered_oauth_and_records_tool_call` proves the production MCP manager can connect to a local recording Streamable HTTP MCP server, resolve broker-owned auth, list a tool, and dispatch a call without remote services. Route-level tool edit/call still need named mounted proof. |
| Settings | `/settings/info`, `/settings/edit` | real, partial_mounted_proof | Mounted read proof covers `/settings/info`; edit still needs named mounted proof. |
| Corp | `/corp/info`, `/corp/edit`, `/corp/validate`, `/corp/reload` | real, mounted_proof | `mounted_corp_routes_validate_install_report_and_reload_inline_toml` proves validate/edit/info/reload with temp `CAPSEM_HOME`. |
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

## Mounted Route Matrix

Implemented in `crates/capsem-service/src/tests.rs`:

- `mounted_fail_closed_stub_routes_return_explicit_errors`
- `mounted_read_routes_reflect_profile_settings_corp_mcp_and_assets_contracts`
- `mounted_corp_routes_validate_install_report_and_reload_inline_toml`
- `mounted_plugin_routes_control_profile_evaluation`
- `mounted_mcp_routes_are_profile_scoped_mechanics_only`
- `mounted_service_ledger_routes_read_real_session_db_rows`
- `mounted_file_import_export_routes_log_boundary_events`

Proof command:

```bash
cargo test -p capsem-service mounted_ -- --nocapture
```

These are mounted Axum route tests, not direct handler calls. The file route
test uses a mock capsem-process IPC responder and proves import/export route
calls send `LogFileBoundary` before bytes are written or returned.

## Remaining Gate

- Add a generated/maintained route inventory so service, gateway, frontend API,
  CLI, and TUI cannot drift silently.
- For each remaining `real` route without mounted proof, name at least one
  functional test and one adversarial test.
- Add at least one black-box service/VM route test for:
  - enforcement block -> actual runtime boundary refuses action/network/tool,
  - MCP route-level tool edit/call with the local recording MCP target,
  - history/timeline mounted route reads with seeded DB data,
  - profile reload/assets status/assets ensure mounted routes,
  - settings edit mounted route.
