---
title: Add Enforcement
description: Author, validate, backtest, publish, and verify realtime CEL enforcement.
sidebar:
  order: 28
---

Enforcement is synchronous. A rule can allow, block, ask, or rewrite a
Security Event before the Network/File/Process transport continues.

## Workflow

1. Choose the enforcement point: `http.request`, `dns.request`,
   `mcp.request`, `model.request`, `file.activity`, or `process.exec`.
2. Write CEL over canonical roots, such as
   `http.request.host.contains("google")`.
3. Validate and backtest with `capsem-admin enforcement`.
4. Publish the pack through a signed profile, or use `/enforcement/*` for a
   runtime overlay.
5. Verify match counters, resolved events, logs, VM health, and UI state.

Never author against `event.*`; that is internal representation.

## Runtime API

| Route | Purpose |
|---|---|
| `POST /enforcement/validate` | Compile-check a candidate rule. |
| `POST /enforcement/compile` | Return the compiled plan metadata. |
| `POST /enforcement/backtest` | Replay a rule over supplied events. |
| `GET /enforcement` | List live profile/user/corp/runtime rules. |
| `POST /enforcement` | Add or update a runtime overlay. |
| `DELETE /enforcement/{id}` | Delete a runtime overlay. |
| `GET /enforcement/stats` | Inspect match counters. |

Backtest returns counts plus up to 100 evidence-diverse rows by default.
