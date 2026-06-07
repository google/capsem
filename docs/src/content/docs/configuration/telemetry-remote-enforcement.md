---
title: Telemetry And Remote Enforcement
description: Configure telemetry export, VM health summaries, remote enforcement boundaries, and future quota inputs.
sidebar:
  order: 8
---

Telemetry is derived from resolved Security Events. Remote enforcement is a
future extension lane that must consume the same resolved-event contract rather
than inventing another policy path.

## Telemetry Settings

Service Settings V2 owns telemetry export configuration:

```toml
[telemetry]
enabled = true
endpoint = "https://otel.example.com/v1/traces"
batch_max_events = 64
flush_interval_ms = 1000
redact_secrets = true
retry_attempts = 2
failure_mode = "drop"

[telemetry.headers]
x-capsem-tenant = "example"
```

Validation rules:

- `telemetry.endpoint` is required when telemetry is enabled.
- `failure_mode` is `drop`, `disable`, or `backpressure`.
- headers must be bounded, explicit strings.
- secrets are redacted from exported telemetry when `redact_secrets = true`.
- full local evidence stays in timeline, backtest, hunt, and session APIs.

## Export Shape

Every emitted event has already passed through:

```text
preprocessors -> enforcement -> ask/confirm -> detection -> postprocessors -> resolved event emitter
```

OpenTelemetry and future exporters receive summaries from the resolved event:

| Field class | Examples |
|---|---|
| Attribution | `vm_id`, `session_id`, `profile_id`, `profile_revision`, `user_id`, accounting owner. |
| Event identity | event id, trace id, event family, event type, process id, turn id when present. |
| Security | enforcement decision, rule id, detection ids, severity, latest block/detection summaries. |
| AI usage | provider, model, input/output tokens, model call count, estimated cost. |
| Transport | HTTP/DNS/MCP/file/process counters and bounded status summaries. |

Metrics labels must stay low-cardinality. Do not export prompt text, file
paths, tool arguments, raw headers, or arbitrary URLs as OTel labels.

## VM Status

VM status is the live operator surface for health:

- model/provider/token/cost counters;
- HTTP/DNS/MCP/file/process counts;
- enforcement decisions, block counts, latest block;
- detection finding counts, latest detection;
- profile id/revision/status and asset readiness.

Persistent VMs seed/recompute the accumulator once at load time from
`session.db`. Hot status reads do not scan SQLite.

## Remote Enforcement Boundary

Remote enforcement uses the same action vocabulary as local enforcement:
`allow`, `ask`, `block`, and `rewrite`.

```toml
[remote_policy]
enabled = false
endpoint = "https://policy.example.com/capsem/decision"
auth_token = "env:CAPSEM_POLICY_TOKEN"
timeout_ms = 1500
failure_mode = "fail-closed"
```

For the bedrock release, the settings shape and attribution fields are
reserved. S13 owns shipped remote plugin behavior. Until S13 passes its gate,
docs and product UI must not claim centralized remote decisions are available.

When enabled by a later sprint, a remote decision must:

- receive a fully typed Security Event;
- return explicit decision and mutation fields;
- preserve deterministic resolved-event logging;
- obey timeout and failure-mode settings;
- write remote endpoint, latency, error, and rule attribution to debug output.

## Future Quotas And Budgets

S22 owns rate limits, quotas, and budget enforcement. The bedrock release
exposes the dimensions S22 needs:

- accounting owner: VM or host/service;
- profile id and revision;
- provider/model;
- MCP server/tool;
- HTTP/DNS/file/process event families;
- token counts, estimated cost, request counts, and match counters.

Do not document budget enforcement as shipped until S22 lands. The current
contract is measurement and attribution, not throttling.

## Credential Brokerage

S10 owns credential brokerage. Service settings and profiles reserve credential
references, but the bedrock docs must not claim runtime credential release
unless S10 has passed the release gate. Use credential references instead of
embedding secrets directly in profile or image inputs.
