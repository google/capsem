# Sprint: 1.3 VM Stats Ledger

## Why

The VM Stats tab still queried an older narrow set of tables and did not show
the security rule ledger that now owns detection/enforcement truth. The tab must
use the current VM-scoped routes and session database tables, and it must make
raw inspection usable for forensic review.

## Scope

- Add typed frontend API helpers for VM-scoped runtime ledger routes:
  `/vms/{id}/security/latest`, `/vms/{id}/security/status`,
  `/vms/{id}/detection/latest`, `/vms/{id}/detection/status`,
  `/vms/{id}/enforcement/latest`, `/vms/{id}/enforcement/status`.
- Refactor `StatsView` around current session DB tables:
  `model_calls`, `mcp_calls`, `net_events`, `dns_events`, `fs_events`,
  `audit_events`, `exec_events`, `substitution_events`, `snapshot_events`, and
  `security_rule_events`.
- Show security/detection/enforcement rule counters and latest ledger rows from
  the VM-scoped ledger routes, not from live rules.
- Update inspector presets to real current table names.
- Fix inspector rendering/sorting for columnar inspect responses.

## Done

- Stats tab has first-class Model, MCP, HTTP, DNS, Files, Process, Security,
  and Snapshot views.
- Each row opens a detail payload suitable for DB-backed inspection.
- Security rows include event id, event type, rule id, action, detection level,
  trace id, rule JSON, and event JSON.
- Inspector can sort and render columnar inspect responses correctly.
- Frontend tests/check/build pass.

## Proof Matrix

- Unit/contract: API tests for new VM ledger route helpers and inspector
  response mapping.
- Functional: Svelte/Astro checks and production build.
- Adversarial: inspector validation remains select-only; stats uses fixed SQL
  strings plus typed route calls.
- E2E/UI: Browser smoke when service is available; full DB content proof belongs
  to the final release VM smoke.
- Telemetry: Not adding new telemetry, only reading existing session DB and
  ledger routes.
- Performance: Stats queries are bounded with `LIMIT` and aggregate in SQLite.
