# S19b - Reporting Setup

Status: S24 child sprint

## Product Goal

Provide a clear, shippable reporting setup package for teams that want to send
Capsem observability data to external reporting systems such as OpenTelemetry
collectors, Prometheus-compatible pipelines, and Grafana dashboards.

This is an operations enablement child sprint of
[S24 - Post-Ship Profile V2 Meta Sprint](S24-post-ship-profile-followup.md).
The core Profile V2 runtime shipped first, but users who need reporting should
have an official path instead of piecing together metrics, dashboards, and
privacy guidance themselves.

Dashboard packaging ideas from retired `analytics-dashboard` are folded here
only after S12 exposes stable fields. S19b may provide Grafana/dashboard
examples, but it must not define a competing runtime stats API.

## Problem

S12 defines the runtime observability architecture, but runtime metrics alone
do not tell an operator how to collect, route, retain, visualize, or explain
the data in a real environment.

The missing product capability is an official reporting setup guide and
packaged examples that connect Capsem's metric/event outputs to external
reporting tools with sane defaults and clear privacy boundaries.

## Users

- Operators setting up team or enterprise observability.
- Security reviewers validating what Capsem exports.
- Admins configuring telemetry for profile-managed deployments.
- Developers debugging VM/model/MCP/network behavior through dashboards.

## Core Scenarios

1. An operator wants to collect Capsem metrics from a local or team deployment.
2. An admin wants to know which telemetry fields may contain sensitive data.
3. A team wants a Grafana dashboard showing VM health, model usage, MCP usage,
   enforcement outcomes, detection findings, and error states.
4. A developer wants a minimal local reporting setup for debugging.
5. A cloud deployment owner wants to understand which external project,
   collector, token, or endpoint must be provisioned before enabling export.

## Required Product Capabilities

- Document the reporting architecture in product/operator language.
- Explain which Capsem outputs are metrics, logs, traces, events, or audit
  records.
- Provide example collector configuration for supported export paths.
- Provide Prometheus-compatible scrape guidance where applicable.
- Provide Grafana dashboard definitions or dashboard-building instructions.
- Document privacy and redaction expectations.
- Document low-cardinality label rules and why they matter.
- Document how profile/VM identity appears in reporting.
- Document model/provider/token/cost reporting once S12 supplies the runtime
  data.
- Document MCP, HTTP, DNS, enforcement, detection, and VM health reporting once
  the owning runtime sprints expose stable fields.
- Explain that backtest/hunt APIs may return full local event evidence, while
  reporting exports use aggregate counters, bounded summaries, and
  low-cardinality labels.
- Make cloud/project prerequisites explicit instead of hiding them in setup
  prose.

## UI/Site Requirements

The docs/site should include a dedicated reporting page or section that covers:

- what can be collected,
- what should not be collected,
- how to configure collection,
- how to verify data is flowing,
- how to disable or restrict reporting,
- how to read the provided dashboards,
- how to troubleshoot missing data.

This sprint may include static dashboard artifacts and screenshots if the
runtime metrics are stable enough when it starts.

## Non-Goals

- No new runtime metrics contract beyond what S12 and engine sprints define.
- No requirement to host a managed cloud reporting service.
- No requirement to make Grafana or any external system mandatory.
- No promise that reporting setup is required for core release readiness.
- No collection of secrets or high-cardinality raw payloads.

## Dependencies

- S12 OpenTelemetry Metrics Architecture for stable runtime metric names and
  labels.
- S08b resolved event contract for event/finding/audit vocabulary.
- S11 status/debug/provenance for operator explanation.
- S19 documentation/site structure.
- External cloud/project setup, if a hosted reporting example is included.

## Acceptance Criteria

- Operators can find a single official reporting setup entry point.
- The page explains what data exists and what each category is for.
- Example collection configuration works against the supported local/runtime
  output shape.
- Privacy/redaction boundaries are explicit.
- Dashboard examples or dashboard construction guidance cover VM health, model
  usage, MCP usage, network activity, enforcement outcomes, and detection
  findings where runtime data is available.
- Verification steps let an operator confirm reporting is active without
  guessing.
- The sprint is clearly marked non-blocking for core runtime shipment unless a
  release owner explicitly promotes it.

## Coverage Ledger

- Docs: reporting setup page, privacy/redaction guidance, verification steps.
- Functional: example collector/scrape config tested against available local
  outputs.
- UI/site: navigation entry and rendered docs build.
- Telemetry: field names and labels match S12/runtime contracts.
- Missing/deferred: hosted cloud project proof if credentials/project setup are
  not available during the sprint.
