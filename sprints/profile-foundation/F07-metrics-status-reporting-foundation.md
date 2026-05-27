# F07 - Metrics Status And Reporting Foundation

## Goal

Make runtime health, metrics, export, and reporting explain the same foundation
truth.

## Scope

- Live per-VM metrics accumulator and `/metrics/json`.
- Model/provider/token/cost counters.
- Security action, detection, MCP, HTTP, DNS, file, and process counters.
- OTel/export and Prometheus-compatible output.
- Reporting setup docs and dashboard packaging.

## Acceptance Criteria

- UI/status/CLI counters match runtime data.
- Host/service AI accounting is separate from VM accounting.
- Reporting setup has a supported local verification path.
