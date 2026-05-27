# F07 - Metrics Status And Reporting Foundation

## Goal

Make dashboards, runtime health, metrics, export, reporting, and alert logging
explain the same foundation truth.

## Scope

- Live per-VM metrics accumulator and `/metrics/json`.
- Model/provider/token/cost counters.
- Security action, detection, MCP, HTTP, DNS, file, and process counters.
- Dashboard improvements for profile health, VM/session health, security
  events, provider/model usage, token/cost totals, and actionable offline/error
  states.
- OpenTelemetry export and Prometheus-compatible output.
- Remote alert logging for enforcement blocks, detection findings, credential
  denials, quota throttles, plugin decisions, and runtime health transitions.
- Reporting setup docs and dashboard packaging, including local verification
  and privacy/redaction guidance.

## Acceptance Criteria

- UI/status/CLI counters match runtime data.
- Host/service AI accounting is separate from VM accounting.
- Remote alert logs are bounded, redacted, correlated to canonical event ids,
  and testable without a mandatory hosted service.
- Reporting setup has a supported local verification path.
