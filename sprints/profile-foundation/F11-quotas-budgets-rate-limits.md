# F11 - Quotas Budgets And Rate Limits

## Goal

Turn quota dimensions into enforceable product controls.

## Scope

- HTTP, MCP, model, token, cost, file, process, profile, VM, session, and user
  quota dimensions.
- Local engine versus plugin-backed provider decision.
- Throttle, delay, deny, and explain actions.
- UI/status/API/CLI for limits and budget state.
- Telemetry and reporting integration.

## Acceptance Criteria

- Quota decisions produce resolved-event steps and final actions.
- Limits are testable through public surfaces.
- Exhaustion, reset, concurrency, and stale-counter cases are adversarially
  tested.
